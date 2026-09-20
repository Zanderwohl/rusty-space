//! What the ship is doing when nothing is pushing it.
//!
//! Canceling a maneuvere does not stop the ship. It stops the *engine*, and the ship keeps
//! whatever velocity it had — which inside a system means it is now on a conic about whichever
//! body's sphere of influence it happens to be in. That can be a circular orbit, an ellipse
//! that grazes the atmosphere, or an escape.
//!
//! None of the mathematics is here. `em-foundations` turns a state vector into elements and
//! back, and `em-sim` finds the sphere of influence; this is the join, plus the patched-conic
//! rule that when the ship crosses into another body's influence the arc is re-solved about it.

use em_foundations::kepler::{anomaly, state::{self, Elements}};
use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::flight::C_M_S;
use crate::system::{LocalSystem, M_PER_LY};

/// How close to parabolic counts as parabolic.
///
/// Neither anomaly solver converges there — the elliptical one divides by `1 - e` and the
/// hyperbolic by `e - 1` — and a state vector landing exactly on the boundary is a measure-zero
/// accident of arithmetic rather than a trajectory anyone chose. Nudged to one side.
pub const PARABOLIC_TOLERANCE: f64 = 1e-6;

/// A ballistic arc: the conic the ship is on, about the body whose influence it is in.
#[derive(Clone, Debug, PartialEq)]
pub struct Coast {
    /// The body the arc is about, by the name `em-sim` knows it under.
    pub primary: String,
    /// `G m` of that body.
    pub mu: f64,
    /// The arc, in the primary's frame. `true_anomaly` is the value at [`Coast::epoch_s`].
    pub elements: Elements,
    /// Coordinate seconds at which the elements were taken.
    pub epoch_s: f64,
}

impl Coast {
    /// Solve the arc through a state, about whichever body holds the ship.
    ///
    /// `velocity_m_s` is in the world frame, the same frame positions are in. The primary's own
    /// motion is subtracted here: a ship matching Earth's orbit is at rest about Earth and
    /// moving at thirty kilometers a second about the Sun, and only one of those is the orbit
    /// it is on.
    pub fn from_state(
        system: &LocalSystem,
        position_ly: DVec3,
        velocity_m_s: DVec3,
        now_s: f64,
    ) -> Option<Self> {
        Self::about(system, system.holding(position_ly, now_s), position_ly, velocity_m_s, now_s)
    }

    /// Solve the arc about a *named* body, whether or not that body's sphere contains the ship.
    ///
    /// What a patch uses. At a join the ship is exactly on a boundary and containment is a
    /// coin toss — it answered "still inside" and the ship never left — so the crossing says
    /// which frame the new arc is in and this takes it as given.
    pub fn about(
        system: &LocalSystem,
        index: em_sim::id::BodyIndex,
        position_ly: DVec3,
        velocity_m_s: DVec3,
        now_s: f64,
    ) -> Option<Self> {
        let at_m = (position_ly - system.origin_ly) * M_PER_LY;
        let (center, carried) = system.body_state_at(index, now_s)?;
        let mu = system.sim().gravitational_constant() * system.sim().mass(index);
        if mu <= 0.0 || !mu.is_finite() {
            return None;
        }
        let local = at_m - center;
        let relative = velocity_m_s - carried;
        let mut elements = state::from_state(mu, local, relative)?;
        elements.eccentricity = nudged(elements.eccentricity);
        Some(Self {
            primary: system.sim().name(index).to_string(),
            mu,
            elements,
            epoch_s: now_s,
        })
    }

    /// Where the ship is and how fast, at a coordinate time. Light-years and meters a second,
    /// world frame.
    ///
    /// The primary is placed at `now_s` analytically rather than read from the arena, so the
    /// answer does not depend on where `system`'s own clock happens to be. It is what makes an
    /// arc a worldline — something a light-delay solve can evaluate at whatever time its root
    /// lands on — rather than a thing that is only correct at the present.
    pub fn at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let (at_m, velocity) = self.sim_state_at(system, now_s)?;
        Some((system.origin_ly + at_m / M_PER_LY, velocity))
    }

    /// The same, in simulation space: meters from the system's own origin, and meters a
    /// second. What `em-sim` measures in, and what the crossing search wants.
    pub fn sim_state_at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let index = system.body_named(&self.primary)?;
        let (center, carried) = system.body_state_at(index, now_s)?;
        let (local, relative) = self.local_state_at(now_s)?;
        Some((center + local, carried + relative))
    }

    /// Where round the conic the ship is, measured from the primary. Meters.
    pub fn local_state_at(&self, now_s: f64) -> Option<(DVec3, DVec3)> {
        let mut elements = self.elements;
        elements.true_anomaly = self.true_anomaly_at(now_s)?;
        state::to_state(self.mu, &elements)
    }

    /// The arc as something [`em_sim::crossing`] can search.
    pub fn path<'a>(&'a self, system: &'a LocalSystem) -> ConicPath<'a> {
        ConicPath { coast: self, system }
    }

    /// Where round the conic the ship is, radians, at a coordinate time.
    pub fn true_anomaly_at(&self, now_s: f64) -> Option<f64> {
        let (a, e) = (self.elements.semi_major_axis, self.elements.eccentricity);
        let dt = now_s - self.epoch_s;
        let rate = (self.mu / a.abs().powi(3)).sqrt();
        if !rate.is_finite() {
            return None;
        }
        if e < 1.0 {
            let at_epoch = anomaly::mean_from_eccentric(
                anomaly::eccentric_from_true(self.elements.true_anomaly, e),
                e,
            );
            let mean = at_epoch + rate * dt;
            let eccentric = anomaly::eccentric_from_mean_newton(
                mean,
                e,
                anomaly::DEFAULT_TOLERANCE,
                anomaly::DEFAULT_MAX_ITERATIONS,
            );
            Some(anomaly::true_from_eccentric(eccentric, e))
        } else {
            let at_epoch = anomaly::mean_from_hyperbolic(
                anomaly::hyperbolic_from_true(self.elements.true_anomaly, e)?,
                e,
            );
            let mean = at_epoch + rate * dt;
            let hyperbolic = anomaly::hyperbolic_from_mean_newton(
                mean,
                e,
                anomaly::DEFAULT_TOLERANCE,
                anomaly::DEFAULT_MAX_ITERATIONS,
            );
            Some(anomaly::true_from_hyperbolic(hyperbolic, e))
        }
    }

    /// Re-solve about whatever body holds the ship now, if it is no longer this one.
    ///
    /// Patched conics, done when the patch is reached rather than predicted: one conic is exact
    /// only inside one sphere of influence, and a ship falling away from a planet crosses into
    /// the star's at some point on the way out. Returns `None` when nothing changed.
    pub fn repatched(
        &self,
        system: &LocalSystem,
        position_ly: DVec3,
        velocity_m_s: DVec3,
        now_s: f64,
    ) -> Option<Self> {
        let holding = system.holding(position_ly, now_s);
        if system.sim().name(holding) == self.primary {
            return None;
        }
        Self::from_state(system, position_ly, velocity_m_s, now_s)
    }

    pub fn periapsis_m(&self) -> f64 {
        self.elements.semi_major_axis * (1.0 - self.elements.eccentricity)
    }

    /// `None` on an escape, which has no far side.
    pub fn apoapsis_m(&self) -> Option<f64> {
        (self.elements.eccentricity < 1.0)
            .then_some(self.elements.semi_major_axis * (1.0 + self.elements.eccentricity))
    }

    pub fn period_s(&self) -> Option<f64> {
        (self.elements.eccentricity < 1.0 && self.elements.semi_major_axis > 0.0).then(|| {
            std::f64::consts::TAU * (self.elements.semi_major_axis.powi(3) / self.mu).sqrt()
        })
    }

    pub fn is_escaping(&self) -> bool {
        self.elements.eccentricity >= 1.0
    }
}

/// A [`Coast`] as a traveller, so the crossing search can be asked when it leaves.
///
/// Borrowed rather than owned: the arc is the truth and the system places its primary, and a
/// copy of either in another shape is a copy that can be stale.
pub struct ConicPath<'a> {
    coast: &'a Coast,
    system: &'a LocalSystem,
}

impl em_sim::crossing::Traveller for ConicPath<'_> {
    fn state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
        self.coast.sim_state_at(self.system, time.to_j2000_seconds())
    }

    fn local_state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
        self.coast.local_state_at(time.to_j2000_seconds())
    }

    fn period_at(&self, _time: Instant) -> Option<TimeDelta> {
        self.coast.period_s().map(TimeDelta::from_seconds)
    }

    fn timescale_at(&self, time: Instant) -> Option<TimeDelta> {
        if let Some(period) = self.period_at(time) {
            return Some(period);
        }
        // A hyperbolic arc has no period but has the same time constant, and the traverse of
        // a sphere is a fraction of it.
        let a = self.coast.elements.semi_major_axis.abs();
        let scale = std::f64::consts::TAU * (a * a * a / self.coast.mu).sqrt();
        scale.is_finite().then(|| TimeDelta::from_seconds(scale))
    }
}

/// An eccentricity off the parabolic boundary. See [`PARABOLIC_TOLERANCE`].
fn nudged(eccentricity: f64) -> f64 {
    match eccentricity {
        e if (e - 1.0).abs() >= PARABOLIC_TOLERANCE => e,
        e if e < 1.0 => 1.0 - PARABOLIC_TOLERANCE,
        _ => 1.0 + PARABOLIC_TOLERANCE,
    }
}

/// Velocity as a fraction of `c`, for the shader and the readout.
pub fn beta_of(velocity_m_s: DVec3) -> DVec3 {
    velocity_m_s / C_M_S
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::{Course, Plane};

    const AU: f64 = 1.495_978_707e11;

    fn sol() -> LocalSystem {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    /// The state the station hold puts a ship in: a circular orbit, at the speed that keeps it
    /// there. Canceling from that has to leave it exactly where it was.
    fn circular(system: &LocalSystem, body: &str, radii: f64) -> (DVec3, DVec3) {
        let index = system.body_named(body).unwrap();
        let radius = system.sim().radius(index) * (1.0 + radii);
        let mu = system.sim().gravitational_constant() * system.sim().mass(index);
        let pole = system.body_pole(index);
        let (u, v) = crate::navigation::basis(pole);
        let at = system.sim().position(index) + u * radius;
        let speed = (mu / radius).sqrt();
        (
            system.origin_ly + at / M_PER_LY,
            system.sim().velocity(index) + v * speed,
        )
    }

    /// Canceling in a circular orbit leaves a circular orbit. The eccentricity is the whole
    /// test: anything that loses the primary's own motion comes out at thirty kilometers a
    /// second relative to Earth, which is an escape.
    #[test]
    fn a_circular_state_solves_to_a_circular_orbit() {
        let system = sol();
        let (at, velocity) = circular(&system, "Earth", 2.0);
        let coast = Coast::from_state(&system, at, velocity, 0.0).expect("an arc");
        assert_eq!(coast.primary, "Earth", "the ship is inside Earth's influence");
        assert!(coast.elements.eccentricity < 1e-6, "e = {}", coast.elements.eccentricity);
        assert!(!coast.is_escaping());
        let radius = 6.371e6 * 3.0;
        assert!((coast.periapsis_m() / radius - 1.0).abs() < 1e-6);
        // Seven and a third hours, the same period the station readout gives.
        let period = coast.period_s().expect("a closed orbit has a period");
        assert!((period / 26_400.0 - 1.0).abs() < 0.05, "{period} seconds");
    }

    /// And propagating it goes round rather than drifting off: the arc is read from the
    /// elements, so a quarter period is a quarter turn however the clock is stepped.
    #[test]
    fn a_coast_holds_its_radius_all_the_way_round() {
        let mut system = sol();
        let (at, velocity) = circular(&system, "Earth", 2.0);
        let coast = Coast::from_state(&system, at, velocity, 0.0).expect("an arc");
        let period = coast.period_s().unwrap();
        let radius = 6.371e6 * 3.0;
        let mut seen = Vec::new();
        for step in 0..8 {
            let now = period * step as f64 / 8.0;
            system.advance_to(now);
            let (where_now, _) = coast.at(&system, now).expect("a place");
            let earth = system.body_position_ly("Earth").unwrap();
            let r = where_now.distance(earth) * M_PER_LY;
            assert!((r / radius - 1.0).abs() < 1e-3, "step {step}: {r:e} against {radius:e}");
            seen.push((where_now, earth));
        }
        // A full period comes back to the start -- relative to Earth, which has itself run
        // eight hundred thousand kilometers along its own orbit in the meantime.
        system.advance_to(period);
        let (closed, _) = coast.at(&system, period).unwrap();
        let earth_then = seen[0].1;
        let earth_now = system.body_position_ly("Earth").unwrap();
        let drift = (closed - earth_now).distance(seen[0].0 - earth_then) * M_PER_LY;
        assert!(drift < radius * 1e-3, "the orbit did not close: {drift:e}");
        // And the half-way point is across the orbit, not next to the start.
        let half = (seen[4].0 - seen[4].1).distance(seen[0].0 - earth_then) * M_PER_LY;
        assert!(half > radius, "it did not go anywhere");
    }

    /// Slower than circular is an ellipse that falls inward; faster than escape is a hyperbola.
    /// Both are things a player can end up on by canceling at the wrong moment, and both have
    /// to come out as what they are rather than as an error.
    #[test]
    fn a_slow_state_falls_and_a_fast_one_escapes() {
        let system = sol();
        let (at, velocity) = circular(&system, "Earth", 2.0);
        let earth_v = system.sim().velocity(system.body_named("Earth").unwrap());
        let scaled = |factor: f64| earth_v + (velocity - earth_v) * factor;

        let slow = Coast::from_state(&system, at, scaled(0.8), 0.0).expect("an arc");
        assert!(!slow.is_escaping());
        assert!(slow.periapsis_m() < 6.371e6 * 3.0, "a slow orbit falls inward");
        assert!(slow.apoapsis_m().unwrap() > slow.periapsis_m());

        // Above root two times circular is escape velocity, by definition.
        let fast = Coast::from_state(&system, at, scaled(1.5), 0.0).expect("an arc");
        assert!(fast.is_escaping(), "e = {}", fast.elements.eccentricity);
        assert!(fast.apoapsis_m().is_none() && fast.period_s().is_none());
        // And it still propagates: a hyperbola needs the other anomaly solver.
        assert!(fast.true_anomaly_at(3600.0).is_some());
    }

    /// The sphere of influence decides what the arc is about, and it is not always the star.
    #[test]
    fn the_arc_is_about_whatever_holds_the_ship() {
        let system = sol();
        let (near_earth, v) = circular(&system, "Earth", 2.0);
        assert_eq!(Coast::from_state(&system, near_earth, v, 0.0).unwrap().primary, "Earth");

        // Out between the planets, nothing but the star holds it.
        let star = system.star_position_ly();
        let between = star + (system.body_position_ly("Earth").unwrap() - star) * 0.5;
        let sideways = DVec3::new(0.0, 2.0e4, 0.0);
        let drifting = Coast::from_state(&system, between, sideways, 0.0).unwrap();
        assert_eq!(drifting.primary, system.sim().name(system.primary()));
    }

    /// A ship at rest relative to what holds it is falling straight down it, and a straight
    /// line through the focus is not a conic anyone can write elements for: the angular
    /// momentum is zero and the orbital plane is undefined.
    ///
    /// It refuses rather than inventing an orbit, and a refusal leaves the ship drifting at the
    /// velocity it has, which is none. Standing still when you cut the engine standing still is
    /// the wrong physics and the right behavior; falling into the star over the next two
    /// months is neither.
    #[test]
    fn a_radial_state_has_no_elements_and_says_so() {
        let system = sol();
        let star = system.star_position_ly();
        let between = star + (system.body_position_ly("Earth").unwrap() - star) * 0.5;
        assert!(Coast::from_state(&system, between, DVec3::ZERO, 0.0).is_none());
    }

    /// Crossing out of a body's influence re-solves the arc about the next one out. Until the
    /// crossing there is nothing to do, which is what `None` says.
    #[test]
    fn leaving_a_sphere_of_influence_repatches_the_arc() {
        let system = sol();
        let (at, velocity) = circular(&system, "Earth", 2.0);
        let coast = Coast::from_state(&system, at, velocity, 0.0).expect("an arc");
        assert!(coast.repatched(&system, at, velocity, 0.0).is_none(), "nothing has changed");

        let star = system.star_position_ly();
        let far = star + (system.body_position_ly("Earth").unwrap() - star) * 0.6;
        let moved = coast.repatched(&system, far, velocity, 0.0).expect("a new arc");
        assert_eq!(moved.primary, system.sim().name(system.primary()));
    }

    /// A station is a place the engine holds the ship; canceling there has to produce the
    /// orbit the ship was being held on, which for an orbit station is that orbit.
    #[test]
    fn canceling_a_held_orbit_leaves_that_orbit() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).unwrap();
        let at = waypoint.place_at(&system, 0.0).unwrap();
        let velocity = waypoint.velocity_at(&system, 0.0).expect("a station has a velocity");
        let coast = Coast::from_state(&system, at, velocity, system.time_s()).expect("an arc");
        assert_eq!(coast.primary, "Earth");
        assert!(coast.elements.eccentricity < 1e-3, "e = {}", coast.elements.eccentricity);
        let radius = 6.371e6 * 3.0;
        assert!((coast.periapsis_m() / radius - 1.0).abs() < 1e-3);
    }

    /// A state that lands on the parabolic boundary has to come out as something a solver can
    /// propagate rather than as a division by zero.
    #[test]
    fn the_parabolic_boundary_is_nudged_off() {
        assert_eq!(nudged(0.5), 0.5);
        assert_eq!(nudged(2.0), 2.0);
        assert!(nudged(1.0) > 1.0);
        assert!(nudged(1.0 - 1e-12) < 1.0);
        // Not `>=`: `1.0 + 1e-6 - 1.0` does not come back as `1e-6`, and the point is that the
        // solvers have something to divide by, not that the gap is exact.
        assert!((nudged(1.0) - 1.0).abs() > PARABOLIC_TOLERANCE * 0.5);
    }

    #[test]
    fn beta_is_a_fraction_of_light() {
        assert!((beta_of(DVec3::X * C_M_S).x - 1.0).abs() < 1e-12);
        assert!(beta_of(DVec3::X * 3.0e4).length() < 1.0e-3, "orbital speeds are nothing");
    }

    /// An astronomical unit is what it is; this is only here so the constant is used.
    #[test]
    fn an_au_is_an_au() {
        assert!((AU - crate::navigation::AU).abs() < 1.0);
    }
}
