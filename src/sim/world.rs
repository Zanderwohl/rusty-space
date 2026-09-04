//! The link between the simulation arena and the Bevy world.
//!
//! `em_sim::System` owns simulation state. Entities are **views** onto it: a body's entity
//! carries a [`BodyRef`] and its rendering components, and nothing else. Position,
//! velocity, motive and mass are read from the arena through the ref, so there is exactly
//! one place any of them can be wrong.

use std::collections::HashMap;

use bevy::math::DVec3;
use bevy::prelude::*;
use em_foundations::time::{Instant, TimeDelta};
use em_sim::id::BodyId;
use em_sim::propagate;
use em_sim::system::System;
use em_sim::time_map::TimeMap;

use crate::body::appearance::{AssetCache, PbrBundle};
use crate::sim::{SimTime, SimulationObject};

/// The simulation. The single source of truth for where anything is.
#[derive(Resource)]
pub struct SimSystem(pub System);

impl Default for SimSystem {
    fn default() -> Self {
        Self(System::new(6.6743015e-11))
    }
}

/// The only link an entity has to the simulation.
#[derive(Component, Copy, Clone, Debug)]
pub struct BodyRef(pub BodyId);

/// Which entity currently views which body, and the arena generation that was true for.
#[derive(Resource, Default)]
pub struct BodyEntities {
    pub map: HashMap<BodyId, Entity>,
    generation: Option<u32>,
}

/// Drawn trajectories, keyed by body.
///
/// A display artifact rather than physics — its resolution comes from `ViewSettings` — so
/// it lives here rather than in the arena.
#[derive(Resource, Default)]
pub struct Trajectories(pub HashMap<BodyId, TimeMap<DVec3>>);

/// What the last propagation cost, for the controls panel.
#[derive(Resource, Default, serde::Serialize)]
pub struct SimMetrics {
    pub bodies: usize,
    /// Bodies that must be integrated rather than evaluated.
    pub newtonian_bodies: usize,
    /// Integration steps taken last frame. Zero when the system is fully analytic and
    /// jumped straight to the target time.
    pub steps: usize,
    pub step_size_seconds: f64,
    pub propagation_ms: f64,
    pub analytic_jump: bool,
}

/// Advance the simulation to the clock's time.
///
/// Hierarchical bodies are analytic, so when nothing needs integrating the whole system
/// can jump straight to the target instant instead of walking there.
pub fn advance_simulation(
    mut system: ResMut<SimSystem>,
    mut sim_time: ResMut<SimTime>,
    mut metrics: ResMut<SimMetrics>,
    settings: Res<crate::gui::settings::Settings>,
) {
    let started = std::time::Instant::now();
    let target = sim_time.time;
    let system = &mut system.0;

    metrics.bodies = system.len();
    metrics.newtonian_bodies = system.newtonian_count();
    metrics.step_size_seconds = sim_time.step;

    // With nothing to integrate the whole system is analytic, so it can jump straight to
    // the target instant however far away it is. This is what makes scrubbing a timeline
    // cost the same as playing it.
    if !settings.simulation.newtonian || system.newtonian_count() == 0 {
        propagate::evaluate_at(system, target);
        sim_time.previous_times.clear();
        metrics.steps = 0;
        metrics.analytic_jump = true;
        metrics.propagation_ms = started.elapsed().as_secs_f64() * 1000.0;
        return;
    }

    // Something is being integrated, so the intervening times matter and must be walked.
    metrics.analytic_jump = false;
    let mut steps = 0;
    let mut last = system.time();
    for t in sim_time.previous_times.iter() {
        let t = Instant::from_seconds_since_j2000(t);
        propagate::step(system, t - last);
        last = t;
        steps += 1;
        if sim_time.frame_time_exceeded() {
            break;
        }
    }
    if last < target {
        propagate::step(system, target - last);
        steps += 1;
    }
    sim_time.previous_times.clear();
    metrics.steps = steps;
    metrics.propagation_ms = started.elapsed().as_secs_f64() * 1000.0;
}

/// Spawn an entity for every body, and despawn entities whose body has gone.
///
/// Runs only when the arena's structure has changed, which is what `generation` reports.
pub fn sync_body_entities(
    mut commands: Commands,
    system: Res<SimSystem>,
    mut tracked: ResMut<BodyEntities>,
    mut cache: ResMut<AssetCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let generation = system.0.generation();
    if tracked.generation == Some(generation) {
        return;
    }

    let live: std::collections::HashSet<BodyId> = system.0.iter_ids().collect();
    tracked.map.retain(|id, entity| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });

    for i in system.0.indices() {
        let id = system.0.id(i);
        if tracked.map.contains_key(&id) {
            continue;
        }
        let mut entity = commands.spawn((
            SimulationObject,
            BodyRef(id),
            Transform::default(),
            Visibility::default(),
            bevy::camera::visibility::NoFrustumCulling,
        ));
        match system.0.appearance(i) {
            em_sim::appearance::Appearance::DebugBall(ball) => {
                let (mesh, material) = ball.pbr_bundle(&mut cache, &mut meshes, &mut materials, &mut images);
                entity.insert((mesh, material));
            }
            em_sim::appearance::Appearance::Star(star) => {
                let (mesh, material, light) = star.pbr_bundle(&mut cache, &mut meshes, &mut materials, &mut images);
                entity.insert((mesh, material, light));
            }
            em_sim::appearance::Appearance::Empty => {}
        }
        tracked.map.insert(id, entity.id());
    }
    tracked.generation = Some(generation);
}

/// Copy positions from the arena into transforms.
pub fn sync_transforms(
    system: Res<SimSystem>,
    view_settings: Res<crate::body::universe::save::ViewSettings>,
    camera: Query<&crate::camera::Freecam, With<crate::camera::PlanetariumCamera>>,
    mut bodies: Query<(&BodyRef, &mut Transform)>,
) {
    use crate::presentation::render_space::ToRender;
    let Ok(freecam) = camera.single() else { return };
    let scale = view_settings.distance_factor();

    for (body, mut transform) in bodies.iter_mut() {
        let Some(i) = system.0.index_of(body.0) else { continue };
        transform.translation = system.0.position(i).to_render_relative(scale, freecam.bevy_pos);
        transform.scale = Vec3::splat(view_settings.body_scale_factor(system.0.radius(i)));
    }
}

/// Copy orientations from the arena into transforms.
pub fn sync_rotations(
    system: Res<SimSystem>,
    sim_time: Res<SimTime>,
    mut bodies: Query<(&BodyRef, &mut Transform)>,
) {
    use crate::presentation::render_space::ToRenderRotation;
    use em_sim::body::RotationMode;

    for (body, mut transform) in bodies.iter_mut() {
        let Some(i) = system.0.index_of(body.0) else { continue };
        let Some(rotation) = system.0.rotation(i) else { continue };
        let orientation = match &rotation.mode {
            RotationMode::Spinning { .. } => rotation.orientation_at(sim_time.time),
            RotationMode::TidallyLocked { .. } => {
                let Some(primary) = system.0.parent(i) else { continue };
                rotation.orientation_tidally_locked(system.0.position(i), system.0.position(primary))
            }
        };
        if let Some(orientation) = orientation {
            transform.rotation = orientation.to_render_rotation();
        }
    }
}

/// A body's index, resolved for this frame. Convenience for the read-only systems.
pub fn resolve<'a>(system: &'a System, body: &BodyRef) -> Option<em_sim::id::BodyIndex> {
    system.index_of(body.0)
}

/// Recompute drawn trajectories for the Keplerian bodies.
pub fn calculate_trajectories(
    mut calcs: MessageReader<crate::sim::CalculateTrajectory>,
    system: Res<SimSystem>,
    view_settings: Res<crate::body::universe::save::ViewSettings>,
    mut trajectories: ResMut<Trajectories>,
) {
    use crate::sim::BodySelection;
    use em_sim::motive::MotiveSelection;

    if calcs.is_empty() {
        return;
    }
    let now = system.0.time();
    let resolution = view_settings.trajectory_resolution.max(1);

    for calc in calcs.read() {
        // A named selection resolves by id rather than scanning every body — the editor
        // requests one body per keystroke, and scanning 221 for it adds up.
        let targets: Vec<_> = match &calc.selection {
            BodySelection::IDs(ids) => ids.iter().filter_map(|id| system.0.by_name(id)).collect(),
            _ => system.0.indices().collect(),
        };

        for i in targets {
            let info = system.0.info(i);
            let wanted = match &calc.selection {
                BodySelection::All | BodySelection::IDs(_) => true,
                BodySelection::Tag(tag) => info.tags.contains(tag),
            };
            if !wanted {
                continue;
            }
            let (_, selection) = system.0.motive(i).motive_at(now);
            let MotiveSelection::Keplerian(kepler) = selection else { continue };

            let mu = system.0.mu(i);
            let period = kepler.period(mu);
            let periapsis = kepler.time_at_periapsis_passage(mu);

            let mut map = TimeMap::new();
            if !kepler.is_open() {
                map.set_periodicity(periapsis, period);
            }
            for step in 0..=resolution {
                let offset = period * (step as f64 / resolution as f64);
                if let Some(d) = kepler.displacement(periapsis + offset, mu) {
                    map.insert(offset, d);
                }
            }
            trajectories.0.insert(system.0.id(i), map);
        }
    }
}

/// Forget which entity viewed which body.
///
/// Runs alongside the despawn on leaving the planetarium: the entities are gone, so the
/// map must not keep claiming they exist.
pub fn forget_body_entities(mut tracked: ResMut<BodyEntities>) {
    *tracked = BodyEntities::default();
}
