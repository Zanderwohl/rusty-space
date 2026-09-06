//! Solving a body's future as a chain of conic arcs.
//!
//! A mission is not stepped, it is *described*: a timeline of arcs, each analytic, joined
//! at events. [`Motive`](crate::motive::Motive) already is that timeline, and
//! [`TransitionEvent::SOIChange`] already is the join. This module fills those joins in.
//!
//! So the clock is only a cursor. Evaluating any instant is a lookup into the arc in force
//! plus a closed-form solve, and scrubbing costs nothing. The expensive part — finding
//! where the arcs meet — happens once, when the plan changes, not once per frame.
//!
//! # Invalidation
//!
//! An edit at time `T` cannot change anything before `T`, so re-solving is a suffix
//! rewrite: [`Motive::remove_derived_events_after`] drops the solver's own events from `T`
//! on, leaves the player's alone, and the walk starts again from there.
//!
//! # Analytic only
//!
//! Every arc must be evaluable at an arbitrary instant, so an integrated body has no place
//! in a chain — [`propagate::position_at`] says as much by returning `None`. A Newtonian
//! traveller stops the walk rather than being approximated.

use em_foundations::kepler::state;
use em_foundations::time::{Instant, TimeDelta};

use crate::id::BodyIndex;
use crate::influence;
use crate::motive::kepler::{
    EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape,
    TrueAnomalyAtEpoch,
};
use crate::motive::{Frontier, MotiveSelection, TransitionEvent};
use crate::propagate;
use crate::system::System;

/// How many arcs a single solve will lay down before giving up.
///
/// A budget rather than a guarantee: a craft grazing a boundary can chatter across it, and
/// without a ceiling one solve could run forever. KSP calls the same thing a conic patch
/// limit.
pub const DEFAULT_PATCH_BUDGET: usize = 8;

/// How far past an arc's start to look for its end, in revolutions of that arc.
///
/// A hyperbolic arc has no revolutions, but it has the same time constant — see
/// [`conic_timescale`] — and a flyby needs a span just as much as an orbit does.
const SEARCH_REVOLUTIONS: f64 = 6.0;

/// How far to step past a join before looking for the next one.
///
/// At a join the traveller is exactly on a boundary, so a search starting there would find
/// that same crossing again and the walk would not advance. A second of flight puts it
/// about a kilometre clear — far outside the metre-scale tolerance the root was found to,
/// and far inside any arc.
const JOIN_CLEARANCE: TimeDelta = TimeDelta::from_seconds(1.0);

/// What a solve did. Enough to explain an empty chain without re-running it.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchReport {
    /// Joins laid down, in time order.
    pub joins: Vec<Instant>,
    /// Why the walk stopped. A chain also ends quietly when an arc is not analytic or a
    /// crossing cannot be expressed about the new primary; both leave the frontier
    /// complete with the joins found so far.
    pub outcome: PatchOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    /// No further crossing within the search span. The chain is complete as far as it goes.
    Settled,
    /// [`DEFAULT_PATCH_BUDGET`] arcs were laid down and the walk was still going.
    BudgetSpent,
}

/// Work on the traveller's chain for at most `sample_budget` boundary evaluations, then
/// stop wherever it got to.
///
/// This is the whole point of keeping the frontier in the timeline: a chain reaching years
/// into the future can cost seconds to work out, and a frame has milliseconds. So the
/// search is handed out a slice at a time and picks up next call from where it stopped,
/// leaving a partially-described timeline that is still evaluable everywhere — only its
/// *far* future is unknown, and it fills in over the following frames.
///
/// Returns the frontier as it now stands; [`Frontier::is_complete`] means there is no more
/// work to hand out.
pub fn advance(system: &mut System, traveller: BodyIndex, sample_budget: usize) -> Frontier {
    let mut frontier = system.motive(traveller).frontier();
    if frontier.is_complete() {
        return frontier;
    }

    let mut cursor = match frontier.resume_from() {
        Some(time) => time,
        // Never started: clear whatever an earlier solve left and begin at the epoch.
        None => {
            let start = first_event(system, traveller);
            system.motive_mut(traveller).remove_derived_events_after(start);
            start
        }
    };

    let mut spent = 0usize;
    while spent < sample_budget {
        let joins = system.motive(traveller).soi_changes().count();
        if joins >= DEFAULT_PATCH_BUDGET {
            frontier = Frontier::Complete;
            break;
        }

        match step(system, traveller, cursor, sample_budget - spent) {
            Step::Joined { time, arc, cost } => {
                system.motive_mut(traveller).insert_event(
                    time,
                    TransitionEvent::SOIChange,
                    MotiveSelection::Keplerian(arc),
                );
                spent += cost;
                cursor = time + JOIN_CLEARANCE;
                frontier = Frontier::Reached(cursor);
            }
            Step::Searched { to, cost } => {
                spent += cost;
                cursor = to;
                frontier = Frontier::Reached(cursor);
            }
            Step::Exhausted => {
                frontier = Frontier::Complete;
                break;
            }
        }
    }

    system.motive_mut(traveller).set_frontier(frontier);
    frontier
}

/// Work the chain to completion in one go.
///
/// The batch form, for tests and tools. Interactive callers want [`advance`], which will
/// not stall a frame.
pub fn solve(system: &mut System, traveller: BodyIndex, budget: usize) -> PatchReport {
    system.motive_mut(traveller).set_frontier(Frontier::Unsolved);

    // Generous enough that each call makes real progress, small enough that a runaway
    // cannot hang: the loop below bounds the total regardless.
    const SLICE: usize = 50_000;
    for _ in 0..(budget.max(1) * 64) {
        if advance(system, traveller, SLICE).is_complete() {
            break;
        }
    }

    let joins: Vec<Instant> = system.motive(traveller).soi_changes().map(|(t, _)| t).collect();
    let outcome = if joins.len() >= DEFAULT_PATCH_BUDGET {
        PatchOutcome::BudgetSpent
    } else {
        PatchOutcome::Settled
    };
    PatchReport { joins, outcome }
}

/// What one slice of searching did.
///
/// Every outcome carries what it cost. Finding a join is not free — the window was sampled
/// to find it — and letting joins through uncharged is exactly how a budgeted solve turns
/// back into a blocking one.
enum Step {
    /// A crossing was found; this arc ends here and a new one begins.
    Joined { time: Instant, arc: KeplerMotive, cost: usize },
    /// Searched up to `to` without finding anything.
    Searched { to: Instant, cost: usize },
    /// Nothing further to search: the arc is not analytic, or its span is used up.
    Exhausted,
}

/// Search a bounded slice of the arc in force at `cursor`.
fn step(
    system: &System,
    traveller: BodyIndex,
    cursor: Instant,
    sample_budget: usize,
) -> Step {
    let (_, selection) = system.motive(traveller).motive_at(cursor);
    let MotiveSelection::Keplerian(arc) = selection else {
        return Step::Exhausted;
    };
    let Some(primary) = system.by_name(&arc.primary_id) else {
        return Step::Exhausted;
    };

    // Not `System::mu`: that column holds one value per body, for the arena's last rebuild
    // time. Reading it for an arc the clock is not in dresses a heliocentric orbit in
    // Earth's mass and inflates its period from a year to six centuries.
    let mu = propagate::gravitational_parameter_at(system, traveller, cursor);
    let revolution = std::f64::consts::TAU * conic_timescale(arc, mu);
    if !revolution.is_finite() || revolution <= 0.0 {
        return Step::Exhausted;
    }

    // Anchor the span at the arc's own start, not at the cursor, or searching an arc in
    // slices would push its horizon ahead of itself and it would never be used up.
    let (segment_start, authored_end) = system.motive(traveller).active_segment_range(cursor);
    let arc_start = segment_start.unwrap_or_else(|| first_event(system, traveller));
    let horizon = arc_start + TimeDelta::from_seconds(revolution * SEARCH_REVOLUTIONS);
    let arc_end = authored_end.map_or(horizon, |end| end.min(horizon));

    if cursor >= arc_end {
        // This arc is searched out. An authored event starts another; otherwise the chain
        // is as long as the solver looks.
        return match authored_end {
            Some(next) if next > cursor => Step::Searched { to: next, cost: 0 },
            _ => Step::Exhausted,
        };
    }

    // Turn the sample budget into a span: `crossings` samples at a fixed density per
    // revolution, so this is that relation read backwards.
    let affordable = revolution * (sample_budget as f64 / influence::SAMPLES_PER_REVOLUTION);
    let window_end = (cursor + TimeDelta::from_seconds(affordable)).min(arc_end);
    if window_end <= cursor {
        return Step::Exhausted;
    }

    // Every candidate sphere is searched over the same window, so the window is sampled
    // once per candidate. Charging for one would let a planet with forty moons quietly
    // spend forty times its slice.
    let candidates = influence::crossing_candidates_about(system, traveller, primary);
    let per_candidate =
        ((window_end - cursor).to_seconds() / revolution * influence::SAMPLES_PER_REVOLUTION)
            .ceil()
            .max(1.0) as usize;
    let cost = per_candidate.saturating_mul(candidates.len().max(1));

    let Some(crossing) = earliest_crossing(system, traveller, &candidates, cursor, window_end)
    else {
        return Step::Searched { to: window_end, cost };
    };

    // Leaving the sphere it is in hands the traveller up to the next primary out; entering
    // a sibling's hands it down to that sibling.
    let new_primary = if crossing.body == primary {
        if crossing.entering {
            return Step::Searched { to: window_end, cost };
        }
        match system.parent(primary) {
            Some(grandparent) => grandparent,
            // The root's influence is unbounded; there is nowhere further out to go.
            None => return Step::Exhausted,
        }
    } else if crossing.entering {
        crossing.body
    } else {
        return Step::Searched { to: window_end, cost };
    };

    match arc_about(system, traveller, new_primary, crossing.time) {
        Some(arc) => Step::Joined { time: crossing.time, arc, cost },
        None => Step::Searched { to: window_end, cost },
    }
}

/// The natural time unit of a conic, `sqrt(|a|^3 / mu)`.
///
/// For a closed orbit this is the period over `2*pi`. A hyperbola has none — its
/// semi-major axis is negative and Kepler's third law returns NaN off it — but it has the
/// same time constant, and every capture into a moon's sphere is a hyperbola. Taking the
/// period directly meant a flyby got a NaN span, found nothing, and looked like a craft
/// that entered a sphere and never left.
fn conic_timescale(arc: &KeplerMotive, gravitational_parameter: f64) -> f64 {
    let a = arc.semi_major_axis().abs();
    (a * a * a / gravitational_parameter).sqrt()
}

/// The first crossing of any of `candidates` in `(cursor, end)`.
fn earliest_crossing(
    system: &System,
    traveller: BodyIndex,
    candidates: &[BodyIndex],
    cursor: Instant,
    end: Instant,
) -> Option<influence::Crossing> {
    candidates
        .iter()
        .copied()
        .filter_map(|target| {
            influence::crossings(system, traveller, target, (cursor, end))
                .into_iter()
                .next()
        })
        .min_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal))
}

/// The orbit the traveller is on about `primary`, taken from its state at `time`.
///
/// This is the join itself: the same instant described in a new frame. Position and
/// velocity are continuous across it by construction, which is what lets the two arcs be
/// evaluated interchangeably at the join.
fn arc_about(
    system: &System,
    traveller: BodyIndex,
    primary: BodyIndex,
    time: Instant,
) -> Option<KeplerMotive> {
    let (position, velocity) = propagate::state_at(system, traveller, time)?;
    let (primary_position, primary_velocity) = propagate::state_at(system, primary, time)?;

    let mu = system.gravitational_constant() * (system.mass(primary) + system.mass(traveller));
    let elements = state::from_state(mu, position - primary_position, velocity - primary_velocity)?;
    if !elements.semi_major_axis.is_finite() || !elements.eccentricity.is_finite() {
        return None;
    }

    Some(KeplerMotive {
        primary_id: system.name(primary).to_string(),
        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
            eccentricity: elements.eccentricity,
            semi_major_axis: elements.semi_major_axis,
        }),
        // Degrees: this crate's element structs store them, and only `em-foundations` is
        // radians-only.
        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
            inclination: elements.inclination.to_degrees(),
            longitude_of_ascending_node: elements.longitude_of_ascending_node.to_degrees(),
            argument_of_periapsis: elements.argument_of_periapsis.to_degrees(),
        }),
        // Anchored at the join rather than at J2000: the arc only exists from here.
        epoch: KeplerEpoch::TrueAnomaly(TrueAnomalyAtEpoch {
            epoch: time,
            true_anomaly: elements.true_anomaly.to_degrees(),
        }),
        // Derived from `a` by Kepler's third law. A patched arc is an osculating conic, so
        // it has no fitted period to carry.
        anomalistic_period: None,
        gravitational_parameter: Some(mu),
    })
}

/// The earliest event on the traveller's timeline, which is where its history starts.
fn first_event(system: &System, traveller: BodyIndex) -> Instant {
    system
        .motive(traveller)
        .iter_events()
        .next()
        .map(|(time, _, _)| time)
        .unwrap_or(Instant::J2000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::soi_test;

    fn built() -> System {
        let mut system = System::from_contents(&soi_test()).expect("the test system builds");
        propagate::evaluate_at(&mut system, Instant::J2000);
        system
    }

    /// The straddling craft must come out with a chain of arcs, alternating between Earth
    /// and Sol as it leaves and returns.
    #[test]
    fn a_straddling_orbit_solves_into_a_chain() {
        let mut s = built();
        let craft = s.by_name("SC-APO").unwrap();
        let report = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        assert!(!report.joins.is_empty(), "expected joins, got {report:?}");
        for pair in report.joins.windows(2) {
            assert!(pair[0] < pair[1], "joins must come out in time order");
        }

        // Each join hands the craft to a different primary than the arc before it.
        let primaries: Vec<String> = s
            .motive(craft)
            .soi_changes()
            .filter_map(|(_, selection)| selection.primary_id().map(str::to_string))
            .collect();
        for pair in primaries.windows(2) {
            assert_ne!(pair[0], pair[1], "a join must change primary: {primaries:?}");
        }
    }

    /// The join is the same instant in two frames, so the state either side of it must
    /// agree. If it does not, the chain is not a trajectory, it is a teleport.
    #[test]
    fn position_is_continuous_across_every_join() {
        let mut s = built();
        let craft = s.by_name("SC-APO").unwrap();
        let report = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);
        assert!(!report.joins.is_empty(), "nothing to check: {report:?}");

        let nudge = TimeDelta::from_seconds(0.001);
        for join in report.joins {
            let before = propagate::state_at(&s, craft, join - nudge).expect("analytic");
            let after = propagate::state_at(&s, craft, join + nudge).expect("analytic");

            // Straddling the join in absolute space means straddling the primary's own
            // motion too: across 2 ms Earth alone carries the craft some 60 m, which is
            // travel, not a jump. What must be zero is the part of the step that the
            // velocity does not account for.
            let travelled = (before.1 + after.1) / 2.0 * (2.0 * nudge.to_seconds());
            let jump = ((after.0 - before.0) - travelled).length();
            assert!(jump < 1.0, "position jumps {jump:e} m across the join at {join:?}");

            let slip = (after.1 - before.1).length();
            assert!(slip < 0.1, "velocity jumps {slip:e} m/s across the join at {join:?}");
        }
    }

    /// Solving twice must give the same timeline: the solver clears its own events first.
    #[test]
    fn solving_is_idempotent() {
        let mut s = built();
        let craft = s.by_name("SC-APO").unwrap();

        let first = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);
        let events_after_first = s.motive(craft).iter_events().count();
        let second = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        assert_eq!(first.joins, second.joins);
        assert_eq!(events_after_first, s.motive(craft).iter_events().count());
    }

    /// The whole point of the model: the player's own events survive a re-solve.
    #[test]
    fn a_resolve_keeps_authored_events() {
        let mut s = built();
        let craft = s.by_name("SC-APO").unwrap();
        solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        // Stand in for a maneuver: an authored event partway along the timeline.
        let burn = Instant::J2000 + TimeDelta::from_days(400.0);
        let arc = match s.motive(craft).motive_at(burn) {
            (_, MotiveSelection::Keplerian(k)) => k.clone(),
            _ => panic!("the craft should be on a conic there"),
        };
        s.motive_mut(craft).insert_event(burn, TransitionEvent::Impulse,
            MotiveSelection::Keplerian(arc));

        solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        let impulses = s.motive(craft).iter_events()
            .filter(|(_, event, _)| matches!(event, TransitionEvent::Impulse))
            .count();
        assert_eq!(impulses, 1, "the burn must survive re-solving");
    }

    /// The case patched conics exists for: a craft is captured by a moon, flies past, and
    /// comes out somewhere new. Every capture is a hyperbola, so this is also what catches
    /// a propagator that only handles closed orbits.
    #[test]
    fn a_lunar_flyby_solves_into_a_capture_and_an_escape() {
        let mut s = built();
        let craft = s.by_name("SC-LUN").unwrap();
        let report = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        let primaries: Vec<String> = s
            .motive(craft)
            .iter_events()
            .filter_map(|(_, _, selection)| selection.primary_id().map(str::to_string))
            .collect();
        assert_eq!(&primaries[..3], &["Earth", "Luna", "Earth"],
            "expected a capture by Luna and a release back to Earth, got {primaries:?}");

        // The passage through Luna's sphere is a flyby, not a stay: hours to days, not
        // years.
        let inside = report.joins[1] - report.joins[0];
        assert!(inside.to_days() > 0.1 && inside.to_days() < 5.0,
            "spent {} days inside Luna's sphere", inside.to_days());

        // And the arc while inside is hyperbolic, which is the whole reason this is hard.
        let midpoint = report.joins[0] + inside / 2.0;
        match s.motive(craft).motive_at(midpoint) {
            (_, MotiveSelection::Keplerian(k)) => {
                assert!(k.is_open(), "a flyby must be hyperbolic, got e = {}", k.eccentricity());
                let mu = propagate::gravitational_parameter_at(&s, craft, midpoint);
                let position = propagate::position_at(&s, craft, midpoint);
                assert!(position.is_some_and(|p| p.is_finite()),
                    "hyperbolic arc evaluated to {position:?} with mu {mu:e}");
            }
            _ => panic!("the craft should be on a conic about Luna"),
        }
    }

    /// Handing the work out in slices must not change the answer. If it did, a chain would
    /// depend on the frame rate.
    #[test]
    fn slicing_the_work_gives_the_same_chain() {
        let batch = {
            let mut s = built();
            let craft = s.by_name("SC-LUN").unwrap();
            solve(&mut s, craft, DEFAULT_PATCH_BUDGET).joins
        };

        // Deliberately mean slices: small enough that most calls find nothing at all, so
        // joins land across many separate calls.
        for slice in [64usize, 200, 1500] {
            let mut s = built();
            let craft = s.by_name("SC-LUN").unwrap();

            let mut calls = 0;
            while !advance(&mut s, craft, slice).is_complete() {
                calls += 1;
                assert!(calls < 200_000, "slice {slice} never finished");
            }

            let sliced: Vec<Instant> = s.motive(craft).soi_changes().map(|(t, _)| t).collect();
            assert_eq!(sliced.len(), batch.len(),
                "slice {slice} found a different chain: {sliced:?} vs {batch:?}");

            // Not bit-identical, and cannot be: which bracket a root falls in depends on
            // where the sample grid lands, bisection stops at a millisecond, and each arc
            // is derived from the previous join's exact time so that choice carries down
            // the chain. A second, on a chain spanning years, is the same trajectory.
            for (sliced, batch) in sliced.iter().zip(batch.iter()) {
                let drift = (*sliced - *batch).to_seconds().abs();
                assert!(drift < 1.0, "slice {slice} drifted {drift} s from the batch solve");
            }
            assert!(calls > 1, "slice {slice} did it all in one call, so this proves nothing");
        }
    }

    /// Progress lives on the timeline, so an interrupted solve is resumable rather than
    /// restarted, and a finished one is not re-done.
    #[test]
    fn the_frontier_records_progress_and_completion() {
        let mut s = built();
        let craft = s.by_name("SC-LUN").unwrap();
        assert_eq!(s.motive(craft).frontier(), Frontier::Unsolved);

        let after_one = advance(&mut s, craft, 64);
        assert!(matches!(after_one, Frontier::Reached(_)), "expected progress, got {after_one:?}");

        while !advance(&mut s, craft, 2_000).is_complete() {}
        assert!(s.motive(craft).frontier().is_complete());

        // A complete chain is not searched again, however many times it is asked.
        let joins = s.motive(craft).soi_changes().count();
        for _ in 0..5 {
            assert!(advance(&mut s, craft, 10_000).is_complete());
        }
        assert_eq!(s.motive(craft).soi_changes().count(), joins);
    }

    /// A craft that never leaves has nothing to join, and must say so rather than
    /// inventing arcs.
    #[test]
    fn an_orbit_that_never_leaves_settles_immediately() {
        let mut s = built();
        let craft = s.by_name("SC-INN").unwrap();
        let report = solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        assert!(report.joins.is_empty(), "{report:?}");
        assert_eq!(report.outcome, PatchOutcome::Settled);
    }

    /// Time is only a cursor: the chain answers any instant, in any order, identically.
    #[test]
    fn scrubbing_is_free_and_order_independent() {
        let mut s = built();
        let craft = s.by_name("SC-APO").unwrap();
        solve(&mut s, craft, DEFAULT_PATCH_BUDGET);

        let sample = |t: f64| {
            propagate::position_at(&s, craft, Instant::J2000 + TimeDelta::from_days(t))
        };
        let forwards: Vec<_> = (0..200).map(|i| sample(i as f64 * 3.0)).collect();
        let backwards: Vec<_> = (0..200).rev().map(|i| sample(i as f64 * 3.0)).collect();

        for (i, (a, b)) in forwards.iter().zip(backwards.iter().rev()).enumerate() {
            assert_eq!(a.is_some(), b.is_some(), "sample {i}");
            if let (Some(a), Some(b)) = (a, b) {
                assert!((*a - *b).length() < 1e-6, "sample {i} depends on scrub direction");
            }
        }
    }
}
