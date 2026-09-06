//! Sampling an orbit into a path.
//!
//! Samples are keyed by offset from periapsis, not absolute time, so a closed orbit's path
//! is reusable at any epoch.

use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::id::BodyIndex;
use crate::motive::MotiveSelection;
use crate::system::System;
use crate::time_map::TimeMap;

/// A sampled orbit: displacements from the primary, keyed by offset from periapsis.
pub struct Path {
    pub points: TimeMap<DVec3>,
    /// When the sampled cycle starts, for a closed orbit.
    pub periapsis: Instant,
    /// One full revolution, for a closed orbit.
    pub period: TimeDelta,
    /// `false` for a hyperbolic or parabolic orbit, which does not repeat.
    pub closed: bool,
}

/// How far an unbounded open arc is drawn either side of periapsis, in conic timescales.
///
/// A hyperbola has no period and runs to infinity, so something has to say where to stop.
/// This is the equivalent of a revolution: `2*pi` timescales is exactly one period for a
/// closed orbit of the same `|a|`.
const OPEN_ARC_TIMESCALES: f64 = std::f64::consts::TAU;

/// Sample the orbit of body `index` into `resolution` points.
///
/// `None` unless the body is Keplerian; fixed and integrated bodies have no closed form.
pub fn sample(system: &System, index: BodyIndex, resolution: usize) -> Option<Path> {
    sample_segment(system, index, system.time(), resolution)
}

/// Sample the arc in force at `at`, over the stretch of time that arc is actually flown.
///
/// An arc bounded by events — one leg of a patched chain — is drawn between them, so a
/// capture by a moon shows the piece of hyperbola travelled rather than a whole conic. An
/// unbounded arc, which is every ordinary orbit in a system, is drawn as a full revolution
/// exactly as before.
pub fn sample_segment(
    system: &System,
    index: BodyIndex,
    at: Instant,
    resolution: usize,
) -> Option<Path> {
    let resolution = resolution.max(1);
    let (_, selection) = system.motive(index).motive_at(at);
    let MotiveSelection::Keplerian(kepler) = selection else {
        return None;
    };

    // Not `System::mu`: on a patched chain that column describes whichever arc the clock is
    // in, which need not be this one.
    let mu = crate::propagate::gravitational_parameter_at(system, index, at);
    let periapsis = kepler.time_at_periapsis_passage(mu);
    let closed = !kepler.is_open();
    let period = kepler.period(mu);

    let (start, end) = system.motive(index).active_segment_range(at);
    let bounded = start.is_some() || end.is_some();

    // Offsets from periapsis, which is what `Path` is keyed by.
    let (from, to) = if let (Some(start), Some(end)) = (start, end) {
        (start - periapsis, end - periapsis)
    } else {
        // Half-bounded or unbounded. A closed orbit gets its revolution; an open one gets
        // a span either side of periapsis, since it has no revolution to get.
        let span = if closed {
            period
        } else {
            TimeDelta::from_seconds(OPEN_ARC_TIMESCALES * timescale(kepler.semi_major_axis(), mu))
        };
        match (start, end) {
            (Some(start), None) => (start - periapsis, start - periapsis + span),
            (None, Some(end)) => (end - periapsis - span, end - periapsis),
            // The ordinary case: one revolution from periapsis, or a hyperbola centred on it.
            (None, None) if closed => (TimeDelta::from_seconds(0.0), period),
            // `(Some, Some)` is handled above; this is the unbounded open arc.
            _ => (span * -0.5, span * 0.5),
        }
    };
    if !(to - from).is_finite() || (to - from).to_seconds() <= 0.0 {
        return None;
    }

    // Bulk-load then sort once; samples are generated in order.
    let mut points = TimeMap::with_capacity(resolution + 1);
    for step in 0..=resolution {
        let offset = from + (to - from) * (step as f64 / resolution as f64);
        if let Some(displacement) = kepler.displacement(periapsis + offset, mu) {
            if displacement.is_finite() {
                points.insert_unordered(offset, displacement);
            }
        }
    }
    points.finalize_unordered();
    if points.is_empty() {
        return None;
    }
    // Only an unclipped closed orbit repeats; a leg of a chain is flown once.
    if closed && !bounded {
        points.set_periodicity(periapsis, period);
    }

    Some(Path { points, periapsis, period, closed })
}

/// The natural time unit of a conic, `sqrt(|a|^3 / mu)`. Defined for a hyperbola, where a
/// period is not.
fn timescale(semi_major_axis: f64, gravitational_parameter: f64) -> f64 {
    let a = semi_major_axis.abs();
    (a * a * a / gravitational_parameter).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::solar_system;

    #[test]
    fn samples_a_closed_orbit_all_the_way_round() {
        let mut s = System::from_contents(&solar_system()).unwrap();
        crate::propagate::evaluate_at(&mut s, Instant::J2000);
        let earth = s.by_name("Earth").unwrap();

        let path = sample(&s, earth, 64).expect("Earth is Keplerian");
        assert!(path.closed);
        assert_eq!(path.points.len(), 65, "resolution + 1, closing the loop");
        assert!(path.points.periodicity().is_some());

        // Every sample sits between periapsis and apoapsis.
        let (_, sel) = s.motive(earth).motive_at(s.time());
        let MotiveSelection::Keplerian(k) = sel else { unreachable!() };
        let (peri, apo) = (k.periapsis(), k.apoapsis().unwrap());
        for (_, p) in path.points.iter() {
            let r = p.length();
            assert!(r >= peri * 0.999 && r <= apo * 1.001, "sample at {r:e} outside [{peri:e}, {apo:e}]");
        }
    }

    /// A capture by a moon is a hyperbola, and a hyperbola has no period. Sampling one
    /// against `period` gave NaN offsets, so the arc drew nothing at all and a craft inside
    /// a moon's sphere appeared to have no trajectory.
    #[test]
    fn a_hyperbolic_arc_still_produces_a_path() {
        use crate::presets::soi_test;
        let mut s = System::from_contents(&soi_test()).unwrap();
        crate::propagate::evaluate_at(&mut s, Instant::J2000);
        let craft = s.by_name("SC-LUN").unwrap();
        let report = crate::patch::solve(&mut s, craft, crate::patch::DEFAULT_PATCH_BUDGET);

        // Partway through the capture, where the arc about Luna is hyperbolic.
        let midpoint = report.joins[0] + (report.joins[1] - report.joins[0]) / 2.0;
        crate::propagate::evaluate_at(&mut s, midpoint);

        let path = sample(&s, craft, 64).expect("a hyperbolic arc is still a path");
        assert!(!path.closed, "a flyby does not repeat");
        assert!(path.points.len() > 32, "only {} samples", path.points.len());
        for (_, point) in path.points.iter() {
            assert!(point.is_finite(), "sample was {point:?}");
        }

        // And it is clipped to the leg actually flown, so it starts and ends on the
        // sphere's boundary rather than running off to the asymptotes.
        let luna = s.by_name("Luna").unwrap();
        let soi = crate::influence::soi_at(&s, luna, midpoint).unwrap().bounding_radius();
        for (_, point) in path.points.iter() {
            assert!(point.length() <= soi * 1.05,
                "sample {:e} m is outside Luna's {:e} m sphere", point.length(), soi);
        }
    }

    /// Clipping applies only to arcs that are actually bounded. Every ordinary orbit in a
    /// system has one unbounded arc and must still draw as a full, repeating ellipse.
    #[test]
    fn an_ordinary_orbit_is_still_a_whole_revolution() {
        let mut s = System::from_contents(&solar_system()).unwrap();
        crate::propagate::evaluate_at(&mut s, Instant::J2000);
        let earth = s.by_name("Earth").unwrap();

        let path = sample(&s, earth, 64).expect("Earth is Keplerian");
        assert!(path.closed);
        assert_eq!(path.points.len(), 65, "resolution + 1, closing the loop");
        assert!(path.points.periodicity().is_some(), "an unclipped orbit repeats");
    }

    #[test]
    fn a_fixed_body_has_no_path() {
        let mut s = System::from_contents(&solar_system()).unwrap();
        crate::propagate::evaluate_at(&mut s, Instant::J2000);
        let sol = s.by_name("Sol").unwrap();
        assert!(sample(&s, sol, 32).is_none());
    }
}
