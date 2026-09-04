//! Sampling an orbit into a path.
//!
//! Where the points come from, not how they are drawn. Turning a path into geometry is
//! presentation and belongs to whatever is rendering; producing it is orbital mechanics
//! and belongs here, so a headless tool can ask for a trajectory too.
//!
//! Samples are keyed by offset from periapsis rather than absolute time, which is what
//! makes a closed orbit's path reusable at any epoch: the shape does not move, only the
//! body's position along it.

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

/// Sample the orbit of body `index` into `resolution` points.
///
/// `None` if the body is not on a Keplerian orbit — a fixed or integrated body has no
/// closed-form path to sample.
pub fn sample(system: &System, index: BodyIndex, resolution: usize) -> Option<Path> {
    let resolution = resolution.max(1);
    let (_, selection) = system.motive(index).motive_at(system.time());
    let MotiveSelection::Keplerian(kepler) = selection else {
        return None;
    };

    let mu = system.mu(index);
    let period = kepler.period(mu);
    let periapsis = kepler.time_at_periapsis_passage(mu);
    let closed = !kepler.is_open();

    // Bulk-load then sort once: the samples are generated in order anyway, and a
    // binary-search insert per point is wasted work at this count.
    let mut points = TimeMap::with_capacity(resolution + 1);
    for step in 0..=resolution {
        let offset = period * (step as f64 / resolution as f64);
        if let Some(displacement) = kepler.displacement(periapsis + offset, mu) {
            points.insert_unordered(offset, displacement);
        }
    }
    points.finalize_unordered();
    if closed {
        points.set_periodicity(periapsis, period);
    }

    Some(Path { points, periapsis, period, closed })
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

    #[test]
    fn a_fixed_body_has_no_path() {
        let mut s = System::from_contents(&solar_system()).unwrap();
        crate::propagate::evaluate_at(&mut s, Instant::J2000);
        let sol = s.by_name("Sol").unwrap();
        assert!(sample(&s, sol, 32).is_none());
    }
}
