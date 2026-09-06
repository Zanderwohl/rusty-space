//! The link between the simulation arena and the Bevy world.
//!
//! `em_sim::System` is the single source of truth. Entities are **views**: a body's entity
//! carries a [`BodyRef`] plus rendering components, nothing more. Position, velocity,
//! motive and mass are read from the arena through the ref.

use std::collections::HashMap;

use bevy::prelude::*;
use em_foundations::time::Instant;
use em_sim::id::BodyId;
use em_sim::propagate;
use em_sim::system::System;

use crate::sim::{SimTime, SimulationObject};

/// The simulation arena.
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

/// Sampled orbital paths, keyed by body. Points come from `em_sim::trajectory`; when and
/// at what resolution to resample is a view decision.
#[derive(Resource, Default)]
pub struct Trajectories(pub HashMap<BodyId, em_sim::trajectory::Path>);

/// What the last propagation cost, for the controls panel.
#[derive(Resource, Default, serde::Serialize)]
pub struct SimMetrics {
    pub bodies: usize,
    /// Bodies integrated rather than evaluated.
    pub newtonian_bodies: usize,
    /// Integration steps last frame. Zero on an analytic jump.
    pub steps: usize,
    pub step_size_seconds: f64,
    pub propagation_ms: f64,
    pub analytic_jump: bool,
}

/// Advance the clock, then bring the simulation to it. Queueing the steps and walking them
/// stay in one system: split apart, the clock runs ahead of the bodies on a slow frame.
/// With nothing to integrate, the system jumps straight to the target instant.
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

    // Queue whole steps for the elapsed real time; partial steps accumulate, so a low
    // `gui_speed` creeps forward instead of stalling.
    if sim_time.playing {
        let step = sim_time.step;
        sim_time.accumulated_time += sim_time.gui_speed * time.delta_secs_f64();

        let full_steps = (sim_time.accumulated_time / step).floor() as usize;
        if full_steps > 0 {
            sim_time.accumulated_time -= full_steps as f64 * step;
            // Speed reduced: trim an oversized queue from the tail.
            sim_time.previous_times.truncate(full_steps);
            let already_queued = sim_time.previous_times.len();
            if already_queued < full_steps {
                let last = sim_time.previous_times.last()
                    .unwrap_or(sim_time.time.to_j2000_seconds());
                sim_time.previous_times.expand(last + step, full_steps - already_queued, step);
            }
        }
    }

    // Where the bodies should be: end of the queue when playing, the clock otherwise
    // (scrubbing while paused).
    let target = match sim_time.previous_times.last() {
        Some(t) => Instant::from_seconds_since_j2000(t),
        None => sim_time.time,
    };

    // Paused, nothing queued, arena already at that instant, no edits pending.
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

    // Fully analytic: jump straight to the target instant, however far away.
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

    // Integration in play: the intervening times matter and must be walked.
    metrics.analytic_jump = false;
    let mut steps = 0;
    let mut last = system.time();
    for t in sim_time.previous_times.iter() {
        let t = Instant::from_seconds_since_j2000(t);
        propagate::step(system, t - last);
        last = t;
        steps += 1;
        sim_time.step_completed();
        // Out of budget. The queue keeps for next frame, so the bodies fall behind
        // rather than skipping steps.
        if sim_time.frame_time_exceeded() {
            break;
        }
    }
    // Nothing queued (a scrub, or an edit while paused): close the gap directly.
    if steps == 0 && last < target {
        propagate::step(system, target - last);
        last = target;
        steps += 1;
        sim_time.step_completed();
    }

    // The clock follows what was simulated, not what was asked for.
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

/// Spawn an entity per body, despawn entities whose body has gone. Skipped unless the
/// arena's `generation` changed.
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
        // Only the entity and its link to the arena. Presentation systems pick up
        // anything with a `BodyRef` and attach their own components.
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

/// A body's index, resolved for this frame.
pub fn resolve<'a>(system: &'a System, body: &BodyRef) -> Option<em_sim::id::BodyIndex> {
    system.index_of(body.0)
}

/// Recompute the sampled paths for the requested bodies. Sampling is `em_sim::trajectory`;
/// geometry is `presentation::trajectory`.
/// Resample a body's path when the clock moves it onto a different arc.
///
/// Paths are sampled on request — at load, and when a body is edited — because an orbit
/// does not change on its own. An arc on a patched chain does: crossing a join swaps both
/// the conic and the frame it is measured in, and nothing else asks for a fresh sample, so
/// a craft entering a moon's sphere kept the path drawn for the orbit it had left.
pub fn refresh_trajectories_on_arc_change(
    system: Res<SimSystem>,
    mut previous: Local<Option<em_foundations::time::Instant>>,
    mut calcs: MessageWriter<crate::sim::CalculateTrajectory>,
) {
    let now = system.0.time();
    let Some(before) = previous.replace(now) else { return };
    if before == now {
        return;
    }

    let changed: Vec<String> = system
        .0
        .bodies_crossing_event(before, now)
        .map(|i| system.0.info(i).id.clone())
        .collect();
    if !changed.is_empty() {
        calcs.write(crate::sim::CalculateTrajectory {
            selection: crate::sim::BodySelection::IDs(changed),
        });
    }
}

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
        // Named selections resolve by id rather than scanning every body.
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
