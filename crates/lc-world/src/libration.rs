//! Hanging about at a libration point.
//!
//! L1 and L2 are unstable equilibria: a craft placed exactly on one falls off it in weeks. No
//! real mission tries. They fly a *libration orbit* about the point instead — SOHO round
//! Sun-Earth L1, JWST round L2 — which costs a few meters a second a year to hold and, unlike
//! the point itself, is somewhere you can see out from.
//!
//! [`em_foundations::lagrange`] has the mechanics. This is the curve those numbers describe,
//! placed in a real system: the linearised Lissajous of the circular restricted three-body
//! problem, written in the rotating frame of the two bodies and read back out into simulation
//! space.
//!
//! # What this is not
//!
//! The linear solution, not a halo. A halo is the non-linear orbit you get by choosing the
//! amplitudes so the two frequencies resonate, and it closes; a Lissajous does not, because
//! the planar and vertical frequencies are never equal. The difference is
//! visible over years and is not what a player is looking at.

use em_foundations::lagrange;
use glam::DVec3;

use crate::navigation::LagrangePoint;
use crate::system::{LocalSystem, M_PER_LY};

/// Radial amplitude as a fraction of the point's own distance from the body.
///
/// A fifth puts a Sun-Earth libration orbit about three hundred thousand kilometers across the
/// radial direction and nine hundred thousand along track, which is the size JWST's actually
/// is. Large enough to be a place rather than a dot; small enough that the linearisation the
/// whole thing rests on still holds.
pub const DEFAULT_AMPLITUDE: f64 = 0.2;

/// A libration orbit about a collinear point, as the numbers that fix one.
///
/// Everything needed to evaluate it at an arbitrary time, so it is a worldline like any other
/// waypoint. The rates are frozen at [`epoch_s`](Self::epoch_s) rather than recomputed: the
/// three-body problem this comes from assumes a circular orbit of the two primaries, and
/// re-deriving the mean motion every frame would make the curve depend on where the eccentric
/// orbit happens to be rather than on the pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Libration {
    /// The smaller of the two primaries — the body the point belongs to.
    pub body: String,
    pub point: LagrangePoint,
    /// Distance from the body to the point, meters.
    pub standoff_m: f64,
    /// Half-width in the radial direction, meters. Along track it is
    /// [`amplitude_ratio`](Self::amplitude_ratio) times this.
    pub radial_m: f64,
    /// Half-height out of the orbital plane, meters.
    pub vertical_m: f64,
    /// Radians a second, in plane and out of it.
    pub planar_rate: f64,
    pub vertical_rate: f64,
    /// Along-track amplitude over radial. About 3.2 for a Sun-planet pair.
    pub amplitude_ratio: f64,
    pub phase_rad: f64,
    pub vertical_phase_rad: f64,
    pub epoch_s: f64,
}

impl Libration {
    /// Work out the orbit a craft would fly about `body`'s L1 or L2.
    ///
    /// `None` when the body has no parent — there is no second mass and so no libration
    /// point — or when the pair's geometry is degenerate.
    pub fn about(
        system: &LocalSystem,
        body: &str,
        point: LagrangePoint,
        now_s: f64,
    ) -> Option<Self> {
        let index = system.body_named(body)?;
        let parent = system.sim().parent(index)?;
        let (separation, relative) = pair(system, index, parent, now_s)?;

        let mass = system.sim().mass(index);
        let ratio = mass / (mass + system.sim().mass(parent));
        let (gamma, frequencies) = lagrange::about(ratio, point.collinear())?;

        let distance = separation.length();
        // Instantaneous angular rate of the pair, from `r x v`. The mean motion of a circular
        // orbit, and the closest thing an eccentric one has to it at this instant.
        let n = separation.cross(relative).length() / (distance * distance);
        if !(n > 0.0) || !n.is_finite() {
            return None;
        }

        let standoff_m = gamma * distance;
        Some(Self {
            body: body.to_string(),
            point,
            standoff_m,
            radial_m: standoff_m * DEFAULT_AMPLITUDE,
            vertical_m: standoff_m * DEFAULT_AMPLITUDE,
            planar_rate: frequencies.planar * n,
            vertical_rate: frequencies.vertical * n,
            amplitude_ratio: frequencies.amplitude_ratio,
            phase_rad: 0.0,
            vertical_phase_rad: 0.0,
            epoch_s: now_s,
        })
    }

    /// Where the craft is at a coordinate time, light-years from the world origin.
    ///
    /// `None` when the body it names has gone, which happens when the ship leaves the system.
    pub fn at(&self, system: &LocalSystem, now_s: f64) -> Option<DVec3> {
        let index = system.body_named(&self.body)?;
        let parent = system.sim().parent(index)?;
        let (separation, relative) = pair(system, index, parent, now_s)?;
        let (body_at, _) = system.body_state_at(index, now_s)?;

        // The rotating frame of the pair: outward from the parent, along track, and the
        // orbital normal.
        let out = separation.normalize_or_zero();
        let normal = separation.cross(relative).normalize_or_zero();
        let along = normal.cross(out);
        if out == DVec3::ZERO || normal == DVec3::ZERO {
            return None;
        }

        let center = body_at + out * self.standoff_m * self.point.outward_sign();

        let offset = self.offset_m(now_s);
        let displaced = out * offset.x + along * offset.y + normal * offset.z;
        Some(system.origin_ly + (center + displaced) / M_PER_LY)
    }

    /// Where the craft is relative to the point, in the rotating frame of the pair: meters
    /// outward, along track, and out of plane.
    ///
    /// This, not [`at`](Self::at), is where the orbit is a closed curve. Seen from the body it
    /// is not one — the frame turns a half circle in a libration period, so a craft that has
    /// come all the way round is on the far side of the planet from where it started.
    pub fn offset_m(&self, now_s: f64) -> DVec3 {
        let tau = now_s - self.epoch_s;
        let planar = self.planar_rate * tau + self.phase_rad;
        let vertical = self.vertical_rate * tau + self.vertical_phase_rad;
        DVec3::new(
            -(self.radial_m * planar.cos()),
            self.amplitude_ratio * self.radial_m * planar.sin(),
            self.vertical_m * vertical.sin(),
        )
    }

    /// One turn of the in-plane motion, seconds. Five to six months at a Sun-planet pair.
    pub fn period_s(&self) -> f64 {
        std::f64::consts::TAU / self.planar_rate
    }

    /// Longest dimension of the orbit, meters: the along-track axis.
    pub fn extent_m(&self) -> f64 {
        2.0 * self.amplitude_ratio * self.radial_m
    }
}

/// Separation and relative velocity of a body from its parent, at a time. Meters.
fn pair(
    system: &LocalSystem,
    body: em_sim::id::BodyIndex,
    parent: em_sim::id::BodyIndex,
    now_s: f64,
) -> Option<(DVec3, DVec3)> {
    let (at_body, v_body) = system.body_state_at(body, now_s)?;
    let (at_parent, v_parent) = system.body_state_at(parent, now_s)?;
    let separation = at_body - at_parent;
    (separation.length() > 0.0).then_some((separation, v_body - v_parent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{CatalogStar, StarProvider};

    fn sol() -> Option<LocalSystem> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun: CatalogStar =
            provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some("Sol"))?.clone();
        let mut system = LocalSystem::for_star(&sun)?;
        system.advance_to(0.0);
        Some(system)
    }

    const KM: f64 = 1_000.0;

    /// The numbers JWST flies: about a million and a half kilometers out from Earth, and an
    /// orbit some nine hundred thousand kilometers along track.
    #[test]
    fn earths_l2_is_where_the_telescopes_are() {
        let Some(system) = sol() else { return };
        let orbit = Libration::about(&system, "Earth", LagrangePoint::L2, 0.0).expect("L2");

        // Scaled to the *live* separation, so it breathes with Earth's year: 1.476 million km
        // at J2000, which is close to perihelion, against 1.501 at the mean distance.
        assert!(
            (1_470_000.0..1_530_000.0).contains(&(orbit.standoff_m / KM)),
            "{} km out",
            orbit.standoff_m / KM,
        );
        // Three times longer than it is wide, which is what the amplitude ratio means.
        assert!((orbit.amplitude_ratio - 3.19).abs() < 0.05, "{}", orbit.amplitude_ratio);
        assert!(
            (orbit.extent_m() / KM - 1_883_000.0).abs() < 60_000.0,
            "{} km along track",
            orbit.extent_m() / KM,
        );

        // Five to six months a turn, against Earth's twelve.
        let months = orbit.period_s() / (86_400.0 * 30.44);
        assert!((months - 5.8).abs() < 0.3, "{months} months");
    }

    /// L1 is on the other side, and nearer.
    #[test]
    fn l1_is_sunward_and_closer_than_l2() {
        let Some(system) = sol() else { return };
        let one = Libration::about(&system, "Earth", LagrangePoint::L1, 0.0).expect("L1");
        let two = Libration::about(&system, "Earth", LagrangePoint::L2, 0.0).expect("L2");
        assert!(one.standoff_m < two.standoff_m, "{} against {}", one.standoff_m, two.standoff_m);

        let sun = system.star_position_ly();
        let earth = system.body_position_ly("Earth").expect("Earth");
        let here = one.at(&system, 0.0).expect("a place");
        let there = two.at(&system, 0.0).expect("a place");
        assert!(
            here.distance(sun) < earth.distance(sun),
            "L1 is between Earth and the Sun",
        );
        assert!(there.distance(sun) > earth.distance(sun), "L2 is behind Earth");
    }

    /// It is an orbit, not a point: over a turn it goes round and comes back.
    ///
    /// Measured in the rotating frame, which is the only frame this closes in. Seen from Earth
    /// it does not — the frame turns a half circle in a libration period, so a craft that has
    /// come all the way round is three million kilometers from where it started, on the far
    /// side of the planet. That is real, and it is not the orbit failing to close.
    #[test]
    fn the_craft_goes_round_the_point_and_returns() {
        let Some(system) = sol() else { return };
        let orbit = Libration::about(&system, "Earth", LagrangePoint::L2, 0.0).expect("L2");
        let period = orbit.period_s();

        let start = orbit.offset_m(0.0);
        let half = orbit.offset_m(period / 2.0);
        let full = orbit.offset_m(period);

        // Half a turn is across the orbit: the radial component has flipped sign.
        assert!(start.x * half.x < 0.0, "{} and {}", start.x, half.x);
        assert!((start.x + half.x).abs() < orbit.radial_m * 1.0e-6, "it is not a half turn");

        // A full turn of the in-plane motion comes back in plane, exactly.
        assert!((full.x - start.x).abs() < orbit.radial_m * 1.0e-6);
        assert!((full.y - start.y).abs() < orbit.extent_m() * 1.0e-6);
        // But not out of it, because the vertical rate differs -- which is the whole reason a
        // Lissajous does not close and a halo has to be forced to.
        assert!(
            (full.z - start.z).abs() > orbit.vertical_m * 0.01,
            "a Lissajous that closed exactly would be a halo: {} m", (full.z - start.z).abs(),
        );

        // And it stays in the neighborhood of the point all the way round.
        for step in 0..16 {
            let at = orbit.offset_m(period * step as f64 / 16.0).length();
            assert!(at < orbit.extent_m(), "step {step}: {at:e} m from the point");
        }

        // The along-track axis really is the long one, by the amplitude ratio.
        let widest = (0..64)
            .map(|i| orbit.offset_m(period * i as f64 / 64.0).y.abs())
            .fold(0.0, f64::max);
        assert!(
            (widest / orbit.radial_m - orbit.amplitude_ratio).abs() < 0.01,
            "{widest:e} m against a radial {:e}",
            orbit.radial_m,
        );
    }

    /// A worldline like any other: the answer at `t` does not depend on where the system's
    /// own clock is sitting.
    #[test]
    fn it_is_read_at_the_time_asked_for() {
        let Some(system) = sol() else { return };
        let orbit = Libration::about(&system, "Earth", LagrangePoint::L2, 0.0).expect("L2");
        let ahead = orbit.period_s() / 3.0;
        let now = orbit.at(&system, ahead).expect("a place");
        for present in [-1.0e7, ahead, 5.0e7] {
            let stale = system.propagated_to(present);
            assert_eq!(orbit.at(&stale, ahead), Some(now), "the clock at {present} changed it");
        }
    }

    /// Why a mission flies a libration orbit and does not park on the point.
    ///
    /// A craft *at* L2 is on the Sun-Earth line by definition, so Earth eclipses the Sun there
    /// permanently — no sunlight, ever. The Lissajous swings a third of a right angle off the
    /// line and is in the shadow only where it crosses. That is not a quirk of this model; it
    /// is the reason halo orbits exist.
    #[test]
    fn the_point_is_permanently_eclipsed_and_the_orbit_about_it_is_not() {
        let Some(system) = sol() else { return };
        let sun = system.primary();
        let earth = system.body_named("Earth").unwrap();
        let r_earth = system.sim().radius(earth);
        let r_sun = system.sim().radius(sun);

        // How much of a year is spent with Earth over any part of the Sun, and how far off the
        // line the craft ever gets.
        let survey = |place: &dyn Fn(f64) -> Option<DVec3>| {
            let (mut overlapped, mut widest) = (0, 0.0f64);
            for step in 0..64 {
                let t = 365.25 * 86_400.0 * step as f64 / 64.0;
                let at = place(t).expect("a place");
                let to_earth = (system.body_position_at("Earth", t).unwrap() - at).normalize();
                let to_sun = (system.star_position_at(t).unwrap() - at).normalize();
                let separation = to_earth.dot(to_sun).clamp(-1.0, 1.0).acos();
                let earth_range =
                    (system.body_position_at("Earth", t).unwrap() - at).length() * M_PER_LY;
                let sun_range = (system.star_position_at(t).unwrap() - at).length() * M_PER_LY;
                if separation < (r_earth / earth_range).asin() + (r_sun / sun_range).asin() {
                    overlapped += 1;
                }
                widest = widest.max(separation.to_degrees());
            }
            (overlapped, widest)
        };

        let point =
            crate::navigation::Waypoint::Lagrange { body: "Earth".into(), point: LagrangePoint::L2 };
        let (at_point, widest_at_point) = survey(&|t| point.place_at(&system, t));
        assert_eq!(at_point, 64, "the point is on the line at every instant, by construction");
        // Not zero: the direction is normalized out of positions of order 1e11 meters, and a
        // millionth of a degree at this range is four centimeters.
        assert!(widest_at_point < 1.0e-4, "{widest_at_point} degrees off the line");

        let orbit = Libration::about(&system, "Earth", LagrangePoint::L2, 0.0).expect("L2");
        let (in_orbit, widest_in_orbit) = survey(&|t| orbit.at(&system, t));
        assert!(in_orbit <= 4, "{in_orbit} of 64 samples in the shadow");
        assert!(widest_in_orbit > 25.0, "only {widest_in_orbit} degrees off the line");
    }

    /// The companion and the hangout measure the same standoff, because they now solve the
    /// same quintic. The Hill radius is not that number, and is the *same* number for L1 and
    /// L2 — which put them symmetrically either side of Earth. They are not symmetric.
    #[test]
    fn the_point_and_the_orbit_about_it_agree_on_where_it_is() {
        let Some(system) = sol() else { return };
        for point in [LagrangePoint::L1, LagrangePoint::L2] {
            let orbit = Libration::about(&system, "Earth", point, 0.0).expect("a point");
            let standoff = point.standoff_m(&system, "Earth", 0.0).expect("a point");
            assert!(
                (standoff / orbit.standoff_m - 1.0).abs() < 1.0e-12,
                "{standoff} against {}",
                orbit.standoff_m,
            );
        }

        let one = LagrangePoint::L1.standoff_m(&system, "Earth", 0.0).unwrap();
        let two = LagrangePoint::L2.standoff_m(&system, "Earth", 0.0).unwrap();
        assert!(two > one, "L2 is the further out: {two} against {one}");
        // Some seven thousand kilometers apart, which the Hill radius collapsed to zero.
        assert!((two - one) / KM > 5_000.0, "{} km apart", (two - one) / KM);
    }

    /// The Sun has no parent, so it has no libration points, and that is said rather than
    /// guessed at.
    #[test]
    fn a_body_with_no_parent_has_no_points() {
        let Some(system) = sol() else { return };
        let star = system.sim().name(system.primary()).to_string();
        assert!(Libration::about(&system, &star, LagrangePoint::L1, 0.0).is_none());
        assert!(Libration::about(&system, "nowhere", LagrangePoint::L2, 0.0).is_none());
    }
}
