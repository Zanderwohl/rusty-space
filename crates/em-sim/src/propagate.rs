//! Advancing a [`System`] through time.
//!
//! Free functions over `&mut System` rather than methods, so the orbital mechanics stays
//! separable from the container holding it.
//!
//! Two kinds of motion, resolved in that order:
//!
//! - **Hierarchical** (Fixed and Keplerian) is analytic. A body's position follows from
//!   its primary's, so these are evaluated parents-first in topological order, and any
//!   time can be evaluated directly without stepping through the times before it.
//! - **Newtonian** is integrated, so it must be stepped and cannot be jumped.

use em_foundations::gravity;
use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::id::BodyIndex;
use crate::motive::{MotiveSelection, TransitionEvent};
use crate::system::System;

/// Recompute parent links, gravitational parameters and traversal order for `time`.
///
/// Called automatically by [`evaluate_at`] and [`step`] when the system is dirty; call it
/// directly only to force a rebuild.
pub fn rebuild(system: &mut System, time: Instant) {
    system.rebuild_derived(time);
}

/// Place every analytically-defined body at `time`.
///
/// Fixed and Keplerian bodies only. Newtonian bodies are integrated and cannot jump, so
/// they keep whatever state they already had — use [`step`] to advance them.
///
/// This is what an editor scrubbing a timeline wants: any instant, at the same cost.
pub fn evaluate_at(system: &mut System, time: Instant) {
    if system.is_dirty() {
        system.rebuild_derived(time);
    }
    system.set_time(time);
    evaluate_hierarchical(system, time);
}

/// Advance the system by `dt`.
///
/// Hierarchical bodies are evaluated analytically at the new time; Newtonian bodies are
/// integrated across the interval under gravity from bodies flagged major.
///
/// `dt` should be small enough for the fastest Newtonian body present — integration error
/// grows with step size, and nothing here subdivides on your behalf.
pub fn step(system: &mut System, dt: TimeDelta) {
    let target = system.time() + dt;
    if system.is_dirty() {
        system.rebuild_derived(target);
    }
    system.set_time(target);
    evaluate_hierarchical(system, target);
    integrate_newtonian(system, target, dt);
}

// ---------------------------------------------------------------------------
// Hierarchical: Fixed and Keplerian
// ---------------------------------------------------------------------------

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
                        // A degenerate orbit (parabolic, or a zero semi-latus rectum)
                        // leaves the body at its primary rather than at NaN.
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

// ---------------------------------------------------------------------------
// Newtonian
// ---------------------------------------------------------------------------

/// Acceleration on `at` from every major body except itself.
fn acceleration(system: &System, at: DVec3, exclude: BodyIndex, g: f64) -> DVec3 {
    system
        .major_indices()
        .iter()
        .filter(|&&m| m != exclude)
        .map(|&m| gravity::one_body_acceleration(g * system.mass(m), at - system.position(m)))
        .sum()
}

fn integrate_newtonian(system: &mut System, time: Instant, dt: TimeDelta) {
    let g = system.gravitational_constant();
    let h = dt.to_seconds();

    for idx in 0..system.newtonian_indices().len() {
        let i = system.newtonian_indices()[idx];

        let (mut position, mut velocity) = match seed(system, i, time) {
            Some(seeded) => seeded,
            None => (system.position(i), system.velocity(i)),
        };

        if h != 0.0 {
            // Velocity Verlet. Second-order and symplectic, for one extra acceleration
            // evaluation over semi-implicit Euler — and the second evaluation is at the
            // new position, which is where it does the most good.
            let a0 = acceleration(system, position, i, g);
            position += velocity * h + 0.5 * a0 * h * h;
            let a1 = acceleration(system, position, i, g);
            velocity += 0.5 * (a0 + a1) * h;
        }

        system.write_state(i, position, velocity, None);
        system.set_newtonian_started(i, true);
    }
}

/// Initial state for a Newtonian body that has not been integrated yet.
///
/// `None` once it is running, so integration continues from its own state rather than
/// snapping back to the motive's stored values every step.
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

    // Released from a Fixed motive: the position comes from where that motive had put the
    // body, and the stored velocity is relative to the parent it was released from.
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
