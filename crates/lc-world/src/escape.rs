//! When an escape meets a sphere of influence.
//!
//! [`em_sim::crossing::first_crossing_of`] keys its sampling on the traveller's time constant,
//! `2 pi sqrt(|a|^3 / mu)`. For a hyperbola at a fraction of `c` that is milliseconds — the arc
//! is a straight line and `|a|` is meters — so every candidate was sampled at that search's
//! ceiling across a whole year: most of a minute a solve, on both client and server, and at a
//! hundred and fifty seconds a step it stepped clean through every sphere in the system anyway.
//!
//! What an escape does have is a bound on its speed, from vis-viva at the closest it comes to
//! its primary, and that is all [`em_sim::crossing::next_crossing_bounded`] needs to take
//! large steps wherever a sphere is far away. Nothing here reads a clock or a cache, so client
//! and server fold the same answer.

use em_foundations::kepler::anomaly;
use em_foundations::time::Instant;
use em_sim::crossing::Crossing;
use em_sim::id::BodyIndex;
use em_sim::motive::MotiveSelection;

use crate::coast::Coast;
use crate::system::LocalSystem;

/// Samples per candidate sphere before the walk gives up on it. A path that comes nowhere near
/// a sphere spends a handful; this is the ceiling for one that lingers in its band.
pub const MAX_SAMPLES_PER_SPHERE: usize = 4096;

/// The first sphere among `candidates` an escaping `arc` crosses in `[from_s, until_s]`.
///
/// `None` for a closed arc, which [`em_sim::crossing::first_crossing_of`] handles well.
pub fn first_crossing(
    arc: &Coast,
    system: &LocalSystem,
    primary: BodyIndex,
    candidates: &[BodyIndex],
    from_s: f64,
    until_s: f64,
) -> Option<Crossing> {
    let own_speed = speed_bound(arc, from_s, until_s)?;
    let path = arc.path(system);
    let window = (
        Instant::from_seconds_since_j2000(from_s),
        Instant::from_seconds_since_j2000(until_s),
    );
    candidates
        .iter()
        .filter_map(|&body| {
            let carried = sphere_speed_bound(system, primary, body, window.0)?;
            em_sim::crossing::next_crossing_bounded(
                system.sim(),
                &path,
                body,
                window,
                own_speed + carried,
                MAX_SAMPLES_PER_SPHERE,
            )
        })
        .min_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// The fastest an escape moves relative to its primary over `[from_s, until_s]`: vis-viva at
/// the closest it comes in that window, which is periapsis if the window holds it and an end
/// otherwise.
fn speed_bound(arc: &Coast, from_s: f64, until_s: f64) -> Option<f64> {
    if !arc.is_escaping() {
        return None;
    }
    let (e, a) = (
        arc.elements.eccentricity,
        arc.elements.semi_major_axis.abs(),
    );
    let rate = (arc.mu / (a * a * a)).sqrt();
    let at_epoch = anomaly::hyperbolic_from_true(arc.elements.true_anomaly, e)?;
    let periapsis_s = arc.epoch_s - anomaly::mean_from_hyperbolic(at_epoch, e) / rate;
    let closest = if (from_s..=until_s).contains(&periapsis_s) {
        arc.periapsis_m()
    } else {
        let distance = |t: f64| arc.local_state_at(t).map(|(at, _)| at.length());
        distance(from_s)?.min(distance(until_s)?)
    };
    let speed = (arc.mu * (2.0 / closest + 1.0 / a)).sqrt();
    speed.is_finite().then_some(speed)
}

/// The fastest `body`'s sphere moves relative to `primary`. Zero for the primary's own.
fn sphere_speed_bound(
    system: &LocalSystem,
    primary: BodyIndex,
    body: BodyIndex,
    at: Instant,
) -> Option<f64> {
    if body == primary {
        return Some(0.0);
    }
    let sim = system.sim();
    let (_, selection) = sim.motive(body).motive_at(at);
    if let MotiveSelection::Keplerian(orbit) = selection
        && orbit.primary_id == sim.name(primary)
        && orbit.eccentricity() < 1.0
        && orbit.semi_major_axis() > 0.0
    {
        let (a, e) = (orbit.semi_major_axis(), orbit.eccentricity());
        let mu = em_sim::propagate::gravitational_parameter_at(sim, body, at);
        return Some((mu * (2.0 / (a * (1.0 - e)) - 1.0 / a)).sqrt());
    }
    // Nothing closed-form to bound it by, so its speed now. Doubled because it is a guess.
    let (_, velocity) = system.body_state_at(body, at.to_j2000_seconds())?;
    let (_, frame) = system.body_state_at(primary, at.to_j2000_seconds())?;
    Some(2.0 * (velocity - frame).length())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft::{Craft, CraftId, Kind};
    use crate::flight::C_M_S;
    use crate::motion::{Change, Motive, ShipState, repatch_at};
    use crate::system::M_PER_LY;
    use glam::DVec3;
    use std::sync::Arc;

    fn sol() -> Option<LocalSystem> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))?
            .clone();
        let mut system = LocalSystem::for_star(&sun)?;
        system.advance_to(0.0);
        Some(system)
    }

    /// Falling from `offset_m` off a body, at `beta` in the world frame.
    fn falling(system: &LocalSystem, body: &str, offset_m: DVec3, beta: DVec3) -> ShipState {
        let index = system.body_named(body).expect("the body");
        let (at_m, _) = system.body_state_at(index, 0.0).expect("a state");
        let position_ly = system.origin_ly + (at_m + offset_m) / M_PER_LY;
        let arc = Coast::from_state(system, position_ly, beta * C_M_S, 0.0).expect("an arc");
        let mut ship = ShipState::at(position_ly);
        ship.beta = beta;
        ship.motive = Motive::Falling(arc);
        ship
    }

    /// The case that hung the chase demo: a ship going ballistic at a good fraction of `c` on
    /// its way out. The old search took the best part of a minute per solve in a debug build,
    /// so the bound here is a regression tripwire three orders of magnitude clear of today.
    #[test]
    fn a_fast_escape_is_solved_quickly_and_meets_nothing() {
        let Some(system) = sol() else { return };
        let system = Arc::new(system);
        let ship = falling(
            &system,
            "Sol",
            DVec3::new(2.0e11, 1.0e11, 0.0),
            DVec3::new(0.3, 0.1, 0.0),
        );

        let mut craft = Craft::at(CraftId(1), Kind::Ship, ship.position_ly);
        craft.motion = ship;
        let started = std::time::Instant::now();
        craft.enter(Some(system.clone()), 0.0);
        for tick in 1..=120 {
            craft.advance(tick as f64 * 0.25, 0.25);
        }
        assert!(
            started.elapsed().as_secs_f64() < 10.0,
            "{:?} for 120 ticks",
            started.elapsed()
        );
        assert!(
            craft.patch_due_at().is_none(),
            "a straight line out past the planets meets nothing"
        );
        assert!(matches!(craft.motion.motive, Motive::Falling(_)));
    }

    /// Fast is not the same as blind: a ship at a tenth of `c` aimed at Earth enters its
    /// sphere, on the boundary, within the time the geometry says.
    #[test]
    fn a_fast_ship_aimed_at_earth_enters_its_sphere() {
        let Some(system) = sol() else { return };
        const APPROACH_M: f64 = 3.0e9;
        let ship = falling(
            &system,
            "Earth",
            DVec3::new(-APPROACH_M, 1.0e8, 0.0),
            DVec3::X * 0.1,
        );
        let Motive::Falling(arc) = &ship.motive else {
            unreachable!()
        };
        assert_eq!(arc.primary, "Sol");

        let event = repatch_at(&ship, &system, 0.0).expect("it meets Earth");
        assert_eq!(
            event.change,
            Change::Repatch {
                about: "Earth".into()
            }
        );
        let earth = system.body_named("Earth").unwrap();
        let radius =
            em_sim::influence::soi_at(system.sim(), earth, Instant::from_seconds_since_j2000(0.0))
                .expect("a sphere")
                .bounding_radius();
        let expected_s = (APPROACH_M - radius) / (0.1 * C_M_S);
        assert!(
            (event.at_t - expected_s).abs() < 0.2 * expected_s,
            "{} s, not {expected_s}",
            event.at_t
        );

        let distance_at = |t: f64| {
            em_sim::crossing::boundary_distance_of(
                system.sim(),
                &arc.path(&system),
                earth,
                Instant::from_seconds_since_j2000(t),
            )
            .expect("evaluable")
        };
        assert!(
            distance_at(event.at_t).abs() < 0.1 * C_M_S * 1.0e-2,
            "{} m off",
            distance_at(event.at_t)
        );
        assert!(distance_at(event.at_t - 1.0) > 0.0 && distance_at(event.at_t + 1.0) < 0.0);
    }

    /// And through the craft at two step sizes, into Earth's sphere and out the far side, to
    /// the same arc: the search has no state for a step size to leak into.
    #[test]
    fn a_fast_flyby_folds_to_the_same_arc_at_any_step() {
        let Some(system) = sol() else { return };
        let system = Arc::new(system);
        let ship = falling(
            &system,
            "Earth",
            DVec3::new(-3.0e9, 1.0e8, 0.0),
            DVec3::X * 0.1,
        );
        let run = |step: f64| {
            let mut craft = Craft::at(CraftId(1), Kind::Ship, ship.position_ly);
            craft.motion = ship.clone();
            craft.enter(Some(system.clone()), 0.0);
            let mut now = 0.0;
            while now < 300.0 {
                let next = (now + step).min(300.0);
                craft.advance(next, next - now);
                now = next;
            }
            craft.motion
        };
        let (coarse, fine) = (run(7.3), run(0.25));
        let Motive::Falling(arc) = &coarse.motive else {
            panic!("{:?}", coarse.motive)
        };
        assert_eq!(arc.primary, "Sol", "through Earth's sphere and out again");
        assert!(
            arc.epoch_s > 60.0,
            "it never went in: the arc is from {} s",
            arc.epoch_s
        );
        assert_eq!(coarse.motive, fine.motive);
        assert_eq!(coarse.position_ly, fine.position_ly);
    }
}
