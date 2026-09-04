//! The link between the simulation arena and the Bevy world.
//!
//! `em_sim::System` owns simulation state. Entities are **views** onto it: a body's entity
//! carries a [`BodyRef`] and its rendering components, and nothing else. Position,
//! velocity, motive and mass are read from the arena through the ref, so there is exactly
//! one place any of them can be wrong.

use std::collections::HashMap;

use bevy::prelude::*;
use em_foundations::time::Instant;
use em_sim::id::BodyId;
use em_sim::propagate;
use em_sim::system::System;

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

/// Sampled orbital paths, keyed by body.
///
/// The points come from `em_sim::trajectory`; what makes this the app's rather than the
/// arena's is *when* and *at what resolution* to resample, which is a view decision.
#[derive(Resource, Default)]
pub struct Trajectories(pub HashMap<BodyId, em_sim::trajectory::Path>);

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

/// Advance the clock, then bring the simulation to it.
///
/// Two jobs, deliberately together: queueing the steps and walking them have to agree
/// about how far time actually got, and splitting them across systems is what lets the
/// clock run ahead of the bodies on a slow frame.
///
/// Hierarchical bodies are analytic, so when nothing needs integrating the whole system
/// jumps straight to the target instant instead of walking there. That is what makes
/// scrubbing a timeline cost the same as playing it.
pub fn advance_simulation(
    mut system: ResMut<SimSystem>,
    mut sim_time: ResMut<SimTime>,
    mut metrics: ResMut<SimMetrics>,
    time: Res<Time>,
    settings: Res<crate::gui::settings::Settings>,
) {
    let started = std::time::Instant::now();
    let system = &mut system.0;

    metrics.bodies = system.len();
    metrics.newtonian_bodies = system.newtonian_count();
    metrics.step_size_seconds = sim_time.step;

    // Queue whole steps for the real time that has passed. Partial steps accumulate
    // rather than rounding, so a low `gui_speed` creeps forward instead of stalling.
    if sim_time.playing {
        let step = sim_time.step;
        sim_time.accumulated_time += sim_time.gui_speed * time.delta_secs_f64();

        let full_steps = (sim_time.accumulated_time / step).floor() as usize;
        if full_steps > 0 {
            sim_time.accumulated_time -= full_steps as f64 * step;
            // If the speed was reduced, trim an oversized queue from the tail.
            sim_time.previous_times.truncate(full_steps);
            let already_queued = sim_time.previous_times.len();
            if already_queued < full_steps {
                let last = sim_time.previous_times.last()
                    .unwrap_or(sim_time.time.to_j2000_seconds());
                sim_time.previous_times.expand(last + step, full_steps - already_queued, step);
            }
        }
    }

    // Where the clock wants the bodies to be: the end of the queue when playing, and
    // whatever the clock says otherwise — which is how scrubbing while paused works.
    let target = match sim_time.previous_times.last() {
        Some(t) => Instant::from_seconds_since_j2000(t),
        None => sim_time.time,
    };

    // Paused, nothing queued, and the arena is already at that instant with no edits
    // pending. Re-propagating 221 bodies to where they already are is pure waste.
    if !sim_time.playing
        && sim_time.previous_times.is_empty()
        && !system.is_dirty()
        && system.time() == target
    {
        metrics.steps = 0;
        metrics.propagation_ms = 0.0;
        return;
    }

    sim_time.begin_frame();

    // With nothing to integrate the whole system is analytic, so it can jump straight to
    // the target instant however far away it is.
    if !settings.simulation.newtonian || system.newtonian_count() == 0 {
        propagate::evaluate_at(system, target);
        sim_time.time = target;
        sim_time.previous_times.clear();
        sim_time.step_completed();
        metrics.steps = 0;
        metrics.analytic_jump = true;
        metrics.propagation_ms = started.elapsed().as_secs_f64() * 1000.0;
        sim_time.end_frame();
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
        sim_time.step_completed();
        // Out of budget. The rest of the queue keeps for next frame, so the bodies fall
        // behind the wall clock rather than skipping the steps between.
        if sim_time.frame_time_exceeded() {
            break;
        }
    }
    // Nothing was queued (a scrub, or an edit while paused), so close the gap directly.
    if steps == 0 && last < target {
        propagate::step(system, target - last);
        last = target;
        steps += 1;
        sim_time.step_completed();
    }

    // The clock follows what was actually simulated, never what was asked for.
    sim_time.time = last;
    let queued = sim_time.previous_times.len();
    if steps >= queued {
        sim_time.previous_times.clear();
    } else {
        sim_time.previous_times.drain_front(steps);
    }

    metrics.steps = steps;
    metrics.propagation_ms = started.elapsed().as_secs_f64() * 1000.0;
    sim_time.end_frame();
}

/// Spawn an entity for every body, and despawn entities whose body has gone.
///
/// Runs only when the arena's structure has changed, which is what `generation` reports.
pub fn sync_body_entities(
    mut commands: Commands,
    system: Res<SimSystem>,
    mut tracked: ResMut<BodyEntities>,
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
        // Only the entity and its link to the arena. Meshes, materials, wireframes,
        // occluders and point sprites are attached by the presentation systems, which
        // pick up anything carrying a `BodyRef` and lacking their own link component —
        // so a body added at runtime gets dressed without this knowing how.
        let entity = commands.spawn((
            SimulationObject,
            BodyRef(id),
            Transform::default(),
            Visibility::default(),
            bevy::camera::visibility::NoFrustumCulling,
        ));
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

/// Recompute the sampled paths for the requested bodies.
///
/// The sampling itself is `em_sim::trajectory` — it is orbital mechanics, not rendering.
/// This just decides *which* bodies to resample and caches the result; turning a path into
/// geometry is `presentation::trajectory`'s job.
pub fn calculate_trajectories(
    mut calcs: MessageReader<crate::sim::CalculateTrajectory>,
    system: Res<SimSystem>,
    view_settings: Res<crate::body::universe::save::ViewSettings>,
    mut trajectories: ResMut<Trajectories>,
) {
    use crate::sim::BodySelection;

    if calcs.is_empty() {
        return;
    }
    let resolution = view_settings.trajectory_resolution.max(1);

    for calc in calcs.read() {
        // A named selection resolves by id rather than scanning every body — the editor
        // requests one body per keystroke, and scanning 221 for it adds up.
        let targets: Vec<_> = match &calc.selection {
            BodySelection::IDs(ids) => ids.iter().filter_map(|id| system.0.by_name(id)).collect(),
            _ => system.0.indices().collect(),
        };

        for i in targets {
            let wanted = match &calc.selection {
                BodySelection::All | BodySelection::IDs(_) => true,
                BodySelection::Tag(tag) => system.0.info(i).tags.contains(tag),
            };
            if !wanted {
                continue;
            }
            if let Some(path) = em_sim::trajectory::sample(&system.0, i, resolution) {
                trajectories.0.insert(system.0.id(i), path);
            }
        }
    }
}
