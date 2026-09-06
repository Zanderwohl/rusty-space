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
    // Crossing an event changes which arcs are in force, and the derived columns describe
    // arcs. Scrubbing is not an edit, so `is_dirty` alone would not catch it.
    if system.is_dirty() || system.crosses_event(system.time(), time) {
        system.rebuild_derived(time);
    }
    system.set_time(time);
    evaluate_hierarchical(system, time);
}

// === Evaluating without touching the arena ===
//
// [`evaluate_at`] places every body by writing the arena, which makes it useless for
// asking where something *will* be: a caller would have to save and restore the whole
// system around every question. These walk the parent chain instead and answer for one
// body at a time, which is what a search over future times needs.

/// How deep a parent chain may go before it is assumed to be a cycle.
const MAX_CHAIN_DEPTH: usize = 64;

/// Where `i` is at `time`, without touching the arena.
///
/// `None` when `i` or anything in its parent chain moves Newtonially: that state is
/// integrated rather than evaluated, so no closed form exists for an arbitrary instant.
/// That `Option` is the honest limit of what can be predicted.
pub fn position_at(system: &System, i: BodyIndex, time: Instant) -> Option<DVec3> {
    state_at(system, i, time).map(|(position, _)| position)
}

/// Position and velocity at `time`, without touching the arena. `None` on the same terms
/// as [`position_at`].
pub fn state_at(system: &System, i: BodyIndex, time: Instant) -> Option<(DVec3, DVec3)> {
    // Root-most first, so each body can be placed relative to one already placed.
    let mut chain = Vec::with_capacity(4);
    let mut walk = Some(i);
    while let Some(body) = walk {
        if chain.len() >= MAX_CHAIN_DEPTH || chain.contains(&body) {
            return None; // a cyclic parent column; `topological_order` tolerates them
        }
        chain.push(body);
        walk = primary_at(system, body, time);
    }

    let mut position = DVec3::ZERO;
    let mut velocity = DVec3::ZERO;
    for &body in chain.iter().rev() {
        let (local_position, local_velocity) = local_state_at(system, body, time)?;
        position += local_position;
        velocity += local_velocity;
    }
    Some((position, velocity))
}

/// State relative to the primary at `time`. `(ZERO, ZERO)` for a root. `None` for a
/// Newtonian body.
pub fn local_state_at(system: &System, i: BodyIndex, time: Instant) -> Option<(DVec3, DVec3)> {
    let (_, selection) = system.motive(i).motive_at(time);
    match selection {
        MotiveSelection::Fixed { position, .. } => Some((*position, DVec3::ZERO)),
        MotiveSelection::Newtonian { .. } => None,
        MotiveSelection::Keplerian(kepler) => {
            // Degenerate orbits sit on the primary rather than produce NaN, matching
            // `evaluate_hierarchical`.
            Some(kepler.state_vectors(time, mu_at(system, i, time))
                .unwrap_or((DVec3::ZERO, DVec3::ZERO)))
        }
    }
}

/// The primary in force at `time`, resolved from the motive rather than from
/// [`System::parent`].
///
/// The parent column is derived for whatever time the arena last rebuilt at, so it is
/// wrong when the system is dirty or when a motive transition falls between then and
/// `time`. An unresolved primary is treated as a root, matching `rebuild_derived`.
fn primary_at(system: &System, i: BodyIndex, time: Instant) -> Option<BodyIndex> {
    let (_, selection) = system.motive(i).motive_at(time);
    system.by_name(selection.primary_id()?)
}

/// Gravitational parameter for `i` at `time`, computed rather than read from the derived
/// column, for the same reason as [`primary_at`]. Mirrors `System::rebuild_derived`.
///
/// Public because anything reasoning about an arc the clock is not currently in needs it:
/// [`System::mu`] holds one value per body, for the arena's last rebuild time, and using it
/// for another arc silently mixes the wrong primary's mass into the answer.
pub fn gravitational_parameter_at(system: &System, i: BodyIndex, time: Instant) -> f64 {
    mu_at(system, i, time)
}

fn mu_at(system: &System, i: BodyIndex, time: Instant) -> f64 {
    let (_, selection) = system.motive(i).motive_at(time);
    let MotiveSelection::Keplerian(kepler) = selection else { return 0.0 };
    if let Some(explicit) = kepler.gravitational_parameter {
        // Barycentric orbits: effective mu is not G(M+m), the motive says it.
        return explicit;
    }
    let primary_mass = primary_at(system, i, time).map(|p| system.mass(p)).unwrap_or(0.0);
    // Relative two-body motion: mu = G(M + m), not G*M.
    system.gravitational_constant() * (primary_mass + system.mass(i))
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
    if system.is_dirty() || system.crosses_event(start, target) {
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

#[cfg(test)]
mod non_mutating_tests {
    use super::*;
    use crate::appearance::Appearance;
    use crate::body::BodyInfo;
    use crate::presets::solar_system;
    use crate::system::BodyDef;

    fn built() -> System {
        System::from_contents(&solar_system()).expect("the bundled system must build")
    }

    /// The whole point: the same answer as `evaluate_at`, without writing the arena.
    #[test]
    fn agrees_with_evaluate_at_across_the_timeline() {
        let day = TimeDelta::from_seconds(86_400.0);
        for offset in [0.0, 1.0, 36_525.0, -18_262.0] {
            let time = Instant::J2000 + day * offset;
            let quiet = built();
            let mut evaluated = built();
            evaluate_at(&mut evaluated, time);

            for i in quiet.indices() {
                let Some(position) = position_at(&quiet, i, time) else { continue };
                let expected = evaluated.position(i);
                let tolerance = expected.length().max(1.0) * 1e-9;
                assert!((position - expected).length() <= tolerance,
                    "{} at {offset} days: {position:?} vs {expected:?}", quiet.name(i));
            }
        }
    }

    #[test]
    fn velocities_agree_too() {
        let time = Instant::J2000 + TimeDelta::from_seconds(1.0e7);
        let quiet = built();
        let mut evaluated = built();
        evaluate_at(&mut evaluated, time);

        let earth = quiet.by_name("Earth").unwrap();
        let (_, velocity) = state_at(&quiet, earth, time).unwrap();
        let expected = evaluated.velocity(earth);
        assert!((velocity - expected).length() <= expected.length() * 1e-9,
            "{velocity:?} vs {expected:?}");
    }

    /// The reason the derived columns are recomputed rather than read: an edited arena has
    /// not rebuilt them, but the answer must still be right.
    #[test]
    fn answers_correctly_while_the_arena_is_dirty() {
        let time = Instant::J2000 + TimeDelta::from_seconds(5.0e6);
        let mut quiet = built();
        let expected = position_at(&quiet, quiet.by_name("Earth").unwrap(), time).unwrap();

        quiet.insert(BodyDef {
            info: BodyInfo { id: "Interloper".into(), mass: 1.0e20, ..Default::default() },
            motive: crate::motive::Motive::fixed(DVec3::new(1.0e12, 0.0, 0.0)),
            rotation: None,
            appearance: Appearance::Empty,
        }).unwrap();
        assert!(quiet.is_dirty(), "inserting must leave the derived columns stale");

        let earth = quiet.by_name("Earth").unwrap();
        let after = position_at(&quiet, earth, time).unwrap();
        assert!((after - expected).length() < 1.0, "{after:?} vs {expected:?}");
    }

    /// Integrated state has no closed form, and neither does anything hanging off it.
    #[test]
    fn a_newtonian_chain_has_no_analytic_answer() {
        let g = 6.6743015e-11;
        let mut s = System::new(g);
        s.insert(BodyDef {
            info: BodyInfo { id: "Probe".into(), mass: 1.0e3, ..Default::default() },
            motive: crate::motive::Motive::newtonian(DVec3::new(1.0e9, 0.0, 0.0), DVec3::Y),
            rotation: None,
            appearance: Appearance::Empty,
        }).unwrap();
        s.insert(BodyDef {
            info: BodyInfo { id: "Tag Along".into(), mass: 1.0, ..Default::default() },
            motive: crate::motive::Motive::fixed_with_parent(Some("Probe".into()), DVec3::X),
            rotation: None,
            appearance: Appearance::Empty,
        }).unwrap();

        let probe = s.by_name("Probe").unwrap();
        let follower = s.by_name("Tag Along").unwrap();
        assert!(position_at(&s, probe, Instant::J2000).is_none());
        assert!(position_at(&s, follower, Instant::J2000).is_none(),
            "a body parented to an integrated one is just as unpredictable");
    }
}
