//! What the ship is doing when nothing is pushing it.
//!
//! Cancelling a manoeuvre does not stop the ship. It stops the *engine*, and the ship keeps
//! whatever velocity it had — which inside a system means it is now on a conic about whichever
//! body's sphere of influence it happens to be in. That can be a circular orbit, an ellipse
//! that grazes the atmosphere, or an escape.
//!
//! None of the mathematics is here. `em-foundations` turns a state vector into elements and
//! back, and `em-sim` finds the sphere of influence; this is the join, plus the patched-conic
//! rule that when the ship crosses into another body's influence the arc is re-solved about it.

use em_foundations::kepler::{anomaly, state::{self, Elements}};
use em_foundations::time::Instant;
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
    /// moving at thirty kilometres a second about the Sun, and only one of those is the orbit
    /// it is on.
    pub fn from_state(
        system: &LocalSystem,
        position_ly: DVec3,
        velocity_m_s: DVec3,
        now_s: f64,
    ) -> Option<Self> {
        let at_m = (position_ly - system.origin_ly) * M_PER_LY;
        let index = em_sim::influence::containing(
            system.sim(),
            at_m,
            Instant::from_seconds_since_j2000(now_s),
        )
        // Outside every sphere of influence the star still holds it: the system's own influence
        // has no outer edge until another star's begins.
        .unwrap_or_else(|| system.primary());

        let mu = system.sim().gravitational_constant() * system.sim().mass(index);
        if mu <= 0.0 || !mu.is_finite() {
            return None;
        }
        let local = at_m - system.sim().position(index);
        let relative = velocity_m_s - system.sim().velocity(index);
        let mut elements = state::from_state(mu, local, relative)?;
        elements.eccentricity = nudged(elements.eccentricity);
        Some(Self {
            primary: system.sim().name(index).to_string(),
            mu,
            elements,
            epoch_s: now_s,
        })
    }

    /// Where the ship is and how fast, at a coordinate time. Light-years and metres a second,
    /// world frame.
    ///
    /// `system` must be propagated to `now_s`: the arc is about a body that is itself moving,
    /// and the conic only gives the part relative to it.
    pub fn at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let index = system.body_named(&self.primary)?;
        let mut elements = self.elements;
        elements.true_anomaly = self.true_anomaly_at(now_s)?;
        let (local, relative) = state::to_state(self.mu, &elements)?;
        let at_m = system.sim().position(index) + local;
        Some((
            system.origin_ly + at_m / M_PER_LY,
            system.sim().velocity(index) + relative,
        ))
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
        let at_m = (position_ly - system.origin_ly) * M_PER_LY;
        let holding = em_sim::influence::containing(
            system.sim(),
            at_m,
            Instant::from_seconds_since_j2000(now_s),
        )
        .unwrap_or_else(|| system.primary());
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
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = lc_world::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    /// The state the station hold puts a ship in: a circular orbit, at the speed that keeps it
    /// there. Cancelling from that has to leave it exactly where it was.
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

    /// Cancelling in a circular orbit leaves a circular orbit. The eccentricity is the whole
    /// test: anything that loses the primary's own motion comes out at thirty kilometres a
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
        // eight hundred thousand kilometres along its own orbit in the meantime.
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
    /// Both are things a player can end up on by cancelling at the wrong moment, and both have
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
    /// the wrong physics and the right behaviour; falling into the star over the next two
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

    /// A station is a place the engine holds the ship; cancelling there has to produce the
    /// orbit the ship was being held on, which for an orbit station is that orbit.
    #[test]
    fn cancelling_a_held_orbit_leaves_that_orbit() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO).unwrap();
        let at = waypoint.place(&system).unwrap();
        let velocity = waypoint.velocity_at(&system).expect("a station has a velocity");
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

/// The whole of it through the session: fly a course, cut the engine partway, and end up
    /// on a real orbit that is then held without thrust.
    #[test]
    fn cancelling_a_crossing_leaves_the_ship_on_a_conic() {
        let provider =
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv");
        let Ok(provider) = provider else { return };
        let mut session = crate::session::Session::new(&provider, 64);
        session.sync_system();
        let course = Course::Orbit {
            body: "Earth".into(),
            altitude_radii: 2.0,
            plane: Plane::Equatorial,
        };
        session.set_course(&course).expect("a course");

        // Partway: far enough to be moving, not so far as to have arrived.
        for _ in 0..20 {
            session.advance(0.05);
        }
        assert!(session.cruise.is_some(), "still under way");
        let moving = session.velocity_m_s();
        assert!(moving.length() > 1.0e3, "only {} m/s", moving.length());

        let coast = session.cancel().expect("an arc");
        assert!(session.cruise.is_none() && session.station.is_none());
        eprintln!(
            "cut at {:.0} km/s -> {} (e {:.3})",
            moving.length() / 1.0e3,
            crate::hud::arc(&coast),
            coast.elements.eccentricity,
        );

        // And it keeps going, ballistically, without any of the three modes fighting.
        let before = session.position_ly;
        for _ in 0..20 {
            session.advance(0.05);
        }
        assert!(session.position_ly != before, "a coasting ship is not parked");
        assert!(session.coast.is_some(), "and it is still on an arc");
    }

    /// An astronomical unit is what it is; this is only here so the constant is used.
    #[test]
    fn an_au_is_an_au() {
        assert!((AU - crate::navigation::AU).abs() < 1.0);
    }
}
