//! Advancing a [`System`] through time.
//!
//! Two kinds of motion, resolved in that order:
//!
//! - **Hierarchical** (Fixed and Keplerian): analytic, evaluated parents-first in
//!   topological order. Any instant can be evaluated without stepping to it.
//! - **Newtonian**: integrated, so it must be stepped and cannot be jumped.

use em_foundations::gravity;
use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::id::BodyIndex;
use crate::motive::{MotiveSelection, TransitionEvent};
use crate::system::System;

/// Recompute parent links, gravitational parameters and traversal order for `time`.
/// [`evaluate_at`] and [`step`] do this when dirty; call it only to force a rebuild.
pub fn rebuild(system: &mut System, time: Instant) {
    system.rebuild_derived(time);
}

/// Place every analytically-defined body at `time`. Fixed and Keplerian only; Newtonian
/// bodies keep their current state — use [`step`].
pub fn evaluate_at(system: &mut System, time: Instant) {
    if system.is_dirty() {
        system.rebuild_derived(time);
    }
    system.set_time(time);
    evaluate_hierarchical(system, time);
}

/// Advance the system by `dt`. Hierarchical bodies are evaluated at the new time; Newtonian
/// bodies are integrated under gravity from bodies flagged major. Nothing subdivides `dt`,
/// so size it for the fastest Newtonian body present.
///
/// The hierarchy's advance happens *inside* the Verlet step, between the two acceleration
/// samples: `a0` is measured against the attractors where they were at the start of the
/// step and `a1` where they are at the end. Advancing them first and sampling both from
/// there is what costs the scheme its symplectic property.
pub fn step(system: &mut System, dt: TimeDelta) {
    let start = system.time();
    let target = start + dt;
    if system.is_dirty() {
        system.rebuild_derived(target);
        // The derived columns were stale, so the positions standing in the arena are not
        // trustworthy at `start` either — and `a0` is measured against them.
        evaluate_hierarchical(system, start);
    }

    let a0 = newtonian_drift(system, target, dt);

    system.set_time(target);
    evaluate_hierarchical(system, target);

    newtonian_kick(system, dt, &a0);
}

// === Hierarchical: Fixed and Keplerian ===

fn evaluate_hierarchical(system: &mut System, time: Instant) {
    for idx in 0..system.topo_order().len() {
        let i = system.topo_order()[idx];
        let parent = system.parent(i);
        let parent_position = parent.map(|p| system.position(p)).unwrap_or(DVec3::ZERO);
        let parent_velocity = parent.map(|p| system.velocity(p)).unwrap_or(DVec3::ZERO);

        let (local_position, local_velocity) = {
            let (_, selection) = system.motive(i).motive_at(time);
            match selection {
                MotiveSelection::Fixed { position, .. } => (*position, DVec3::ZERO),
                MotiveSelection::Keplerian(kepler) => {
                    match kepler.state_vectors(time, system.mu(i)) {
                        Some((r, v)) => (r, v),
                        // Degenerate orbit (parabolic, zero semi-latus rectum): sit on
                        // the primary rather than produce NaN.
                        None => (DVec3::ZERO, DVec3::ZERO),
                    }
                }
                MotiveSelection::Newtonian { .. } => continue,
            }
        };

        system.write_state(
            i,
            parent_position + local_position,
            parent_velocity + local_velocity,
            Some(local_position),
        );
    }
}

// === Newtonian ===

/// Acceleration on `at` from every major body except itself.
fn acceleration(system: &System, at: DVec3, exclude: BodyIndex, g: f64) -> DVec3 {
    system
        .major_indices()
        .iter()
        .filter(|&&m| m != exclude)
        .map(|&m| gravity::one_body_acceleration(g * system.mass(m), at - system.position(m)))
        .sum()
}

/// First half of velocity Verlet, with the attractors still where they were at the start
/// of the step: seed anything not yet running, sample `a0`, and drift positions to the end
/// of the step. Velocities are left alone until [`newtonian_kick`].
///
/// Sampling is a separate pass from writing, so a Newtonian body that is itself flagged
/// major cannot contribute its drifted position to another body's `a0`.
///
/// `time` selects which motive is in force, matching the rebuild above; the accelerations
/// it measures come from the arena as it stands.
fn newtonian_drift(system: &mut System, time: Instant, dt: TimeDelta) -> Vec<(BodyIndex, DVec3)> {
    let g = system.gravitational_constant();
    let h = dt.to_seconds();

    for idx in 0..system.newtonian_indices().len() {
        let i = system.newtonian_indices()[idx];
        if let Some((position, velocity)) = seed(system, i, time) {
            system.write_state(i, position, velocity, None);
        }
        system.set_newtonian_started(i, true);
    }

    if h == 0.0 {
        // Seeding was the whole job; nothing drifts and no acceleration is needed.
        return Vec::new();
    }

    let mut sampled: Vec<(BodyIndex, DVec3)> =
        Vec::with_capacity(system.newtonian_indices().len());
    for idx in 0..system.newtonian_indices().len() {
        let i = system.newtonian_indices()[idx];
        sampled.push((i, acceleration(system, system.position(i), i, g)));
    }

    for &(i, a0) in &sampled {
        let position = system.position(i) + system.velocity(i) * h + 0.5 * a0 * h * h;
        system.write_state(i, position, system.velocity(i), None);
    }
    sampled
}

/// Second half of velocity Verlet, with the attractors now at the end of the step: sample
/// `a1` at the drifted positions and complete the velocity update.
fn newtonian_kick(system: &mut System, dt: TimeDelta, a0: &[(BodyIndex, DVec3)]) {
    let g = system.gravitational_constant();
    let h = dt.to_seconds();
    if h == 0.0 {
        return;
    }

    // Sample before writing, for the same reason as the `a0` pass.
    let mut sampled: Vec<(BodyIndex, DVec3, DVec3)> = Vec::with_capacity(a0.len());
    for &(i, a0) in a0 {
        sampled.push((i, a0, acceleration(system, system.position(i), i, g)));
    }
    for (i, a0, a1) in sampled {
        let velocity = system.velocity(i) + 0.5 * (a0 + a1) * h;
        system.write_state(i, system.position(i), velocity, None);
    }
}

/// Seed state for a Newtonian body not yet integrated. `None` once running, so it does not
/// snap back to the motive's stored values each step.
fn seed(system: &mut System, i: BodyIndex, time: Instant) -> Option<(DVec3, DVec3)> {
    if system.newtonian_started(i) {
        return None;
    }
    let (event, selection) = system.motive(i).motive_at(time);
    let MotiveSelection::Newtonian { position, velocity } = selection else {
        return None;
    };
    let (position, velocity) = (*position, *velocity);

    if !matches!(event, TransitionEvent::Release) {
        return Some((position, velocity));
    }

    // Released from Fixed: stored velocity is relative to the parent it was released from.
    let Some((_, previous)) = system.motive(i).motive_before(time) else {
        return Some((position, velocity));
    };
    let MotiveSelection::Fixed { primary_id, position: fixed } = previous else {
        return Some((position, velocity));
    };
    let parent = primary_id
        .as_deref()
        .and_then(|name| system.by_name(name));
    let (parent_position, parent_velocity) = parent
        .map(|p| (system.position(p), system.velocity(p)))
        .unwrap_or((DVec3::ZERO, DVec3::ZERO));
    Some((parent_position + *fixed, parent_velocity + velocity))
}
