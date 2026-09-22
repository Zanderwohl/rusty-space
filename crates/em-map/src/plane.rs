//! The surface everything is measured above and below.

use em_foundations::reference_frame::galactic;
use glam::DVec3;

use crate::snapshot::M_PER_LY;

/// Which plane the map lays its rings in.
///
/// Two, because there are two questions: where a moon sits in its system is about the
/// ecliptic, and where a system sits among the rest is about the disc of the galaxy.
///
/// A selector and nothing more. The geometry it names is a [`Datum`], which needs to know the
/// system being looked at, because the ecliptic is a different plane in every one of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Plane {
    #[default]
    Ecliptic,
    Galactic,
}

impl Plane {
    pub fn other(self) -> Self {
        match self {
            Self::Ecliptic => Self::Galactic,
            Self::Galactic => Self::Ecliptic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ecliptic => "ecliptic",
            Self::Galactic => "galactic",
        }
    }

    /// Resolve against the pole of the system being looked at.
    ///
    /// `system_pole` is the normal of the plane that system's planets orbit in. It is ignored
    /// for [`Plane::Galactic`], whose frame is the same everywhere.
    pub fn about(self, system_pole: DVec3) -> Datum {
        let (u, v, n) = match self {
            Self::Galactic => galactic::basis(),
            Self::Ecliptic => {
                let n = system_pole.normalize_or(DVec3::Z);
                let u = zero_longitude(n);
                (u, n.cross(u), n)
            }
        };
        Datum { plane: self, u, v, n }
    }
}

/// Where a system's longitudes start: the ascending node of its plane on the galactic plane.
///
/// Computable by anyone from the plane alone, so two craft that solved the same system agree
/// on it, and it moves only with the plane's own error rather than with which planet was found
/// first. There is no vernal equinox to borrow. See
/// `lightcone/docs/25-system-knowledge.md#where-longitude-starts`.
///
/// `g.cross(n)` lies in both planes, and is the ascending rather than the descending node
/// because a body at it is moving north: `(n × u) · g = 1 - (n · g)²`, positive unless the two
/// planes coincide. Within about a degree of coinciding the cross product is too short to
/// normalize, and the zero falls back to the galactic center projected into the plane.
fn zero_longitude(n: DVec3) -> DVec3 {
    const COINCIDENT: f64 = 1.745e-2;
    let node = galactic::north_pole().cross(n);
    if node.length() > COINCIDENT {
        return node.normalize();
    }
    let center = galactic::center();
    (center - n * center.dot(n)).normalize_or(n.any_orthonormal_vector())
}

/// A [`Plane`] resolved against one system: the frame the map actually measures in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Datum {
    plane: Plane,
    u: DVec3,
    v: DVec3,
    n: DVec3,
}

impl Datum {
    /// Which of the two this resolved, for a label or a toggle.
    pub fn plane(self) -> Plane {
        self.plane
    }

    /// The plane's normal, a unit vector in simulation space.
    pub fn normal(self) -> DVec3 {
        self.n
    }

    /// A right-handed orthonormal basis `(u, v, n)`, with `n` the normal.
    ///
    /// `u` is where a ring's zero bearing points and where azimuth is measured from, so it
    /// must be fixed rather than derived from the view.
    pub fn basis(self) -> (DVec3, DVec3, DVec3) {
        (self.u, self.v, self.n)
    }

    /// How far `at_ly` stands above the plane through `origin_ly`, meters. Signed.
    ///
    /// Anchored at what the map is looking at rather than a galaxy's own zero point: at every
    /// scale this draws the difference is invisible, and the offset would cost precision.
    pub fn height_m(self, at_ly: DVec3, origin_ly: DVec3) -> f64 {
        (at_ly - origin_ly).dot(self.normal()) * M_PER_LY
    }

    /// Where a ray meets the plane, or `None` when it runs along it or points away. Zooming
    /// toward the cursor is built from this.
    pub fn intersect(self, from_ly: DVec3, direction: DVec3, origin_ly: DVec3) -> Option<DVec3> {
        let n = self.normal();
        let along = direction.dot(n);
        // A ray within a thousandth of parallel names a point far enough away that holding it
        // still would throw the camera across the system.
        if along.abs() < 1.0e-3 {
            return None;
        }
        let t = (origin_ly - from_ly).dot(n) / along;
        (t > 0.0 && t.is_finite()).then(|| from_ly + direction * t)
    }

    /// Which way `offset` points within the plane, radians, measured the way an [`crate::Orbit`]
    /// measures its azimuth.
    ///
    /// `None` for an offset along the normal, which points nowhere in the plane and would name
    /// an arbitrary bearing. The tolerance is relative: what matters is whether the in-plane
    /// part is a real fraction of the offset, not how long the offset is.
    pub fn bearing(self, offset: DVec3) -> Option<f64> {
        let (u, v, _) = self.basis();
        let (along_u, along_v) = (offset.dot(u), offset.dot(v));
        let flat = (along_u * along_u + along_v * along_v).sqrt();
        (flat > offset.length() * 1.0e-6).then(|| along_v.atan2(along_u))
    }

    /// Where a drop-line from `at_ly` meets the plane: the point straight below it.
    pub fn foot_ly(self, at_ly: DVec3, origin_ly: DVec3) -> DVec3 {
        let n = self.normal();
        at_ly - n * (at_ly - origin_ly).dot(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pole that is neither axis and neither plane's normal, so nothing here passes by
    /// accidentally agreeing with `+Z`.
    fn a_pole() -> DVec3 {
        DVec3::new(0.3, -0.5, 0.81).normalize()
    }

    /// Both planes, resolved against one system. Every geometric property below holds of any
    /// datum, which is the point of resolving one.
    fn datums() -> [Datum; 2] {
        [Plane::Ecliptic.about(a_pole()), Plane::Galactic.about(a_pole())]
    }

    /// A bearing is what the camera's azimuth is measured against, so the two have to agree:
    /// a camera at the bearing of a thing looks at it along the plane.
    #[test]
    fn a_bearing_is_the_azimuth_that_points_at_it() {
        for plane in datums() {
            let (u, v, n) = plane.basis();
            assert!(plane.bearing(u).unwrap().abs() < 1.0e-12, "the axis it is measured from");
            assert!((plane.bearing(v).unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1.0e-12);
            assert_eq!(plane.bearing(n), None, "straight up points nowhere in the plane");
            assert_eq!(plane.bearing(DVec3::ZERO), None);

            // The azimuth an orbit would use for the same direction, which is the claim.
            for turns in [0.1, 0.5, 2.5, -1.7] {
                let azimuth = turns;
                let mut orbit = crate::Orbit::default();
                orbit.azimuth = azimuth;
                orbit.elevation = 0.0;
                let toward = orbit.offset_direction(plane);
                let seen = plane.bearing(toward).expect("an in-plane direction");
                let apart = (seen - azimuth).sin().abs();
                assert!(apart < 1.0e-9, "{plane:?} at {azimuth}: read back {seen}");
                // And a long offset reads the same as a short one.
                let far = plane.bearing(toward * 1.0e12).expect("still in the plane");
                assert!((far - seen).abs() < 1.0e-9, "scale changed the bearing");
            }
        }
    }

    /// What a light-year's worth of `f64` is worth, in meters.
    ///
    /// The mantissa is 53 bits, so a position in light-years resolves about a meter per
    /// light-year of magnitude. A flat meter tolerance passes near the origin and fails at
    /// Alpha Centauri, which reads as a bug in the plane and is the representation.
    fn floor_m(magnitude_ly: f64) -> f64 {
        (magnitude_ly.abs().max(1.0) * M_PER_LY * 4.0 * f64::EPSILON).max(1.0)
    }

    /// Both planes and both signs. A galactic drop-line using `+Z` lands in the ecliptic and
    /// looks plausible from most angles.
    #[test]
    fn a_drop_line_ends_in_the_plane_it_was_dropped_to() {
        let origin = DVec3::new(4.2, -1.0, 0.7);
        for plane in datums() {
            for offset in [DVec3::new(0.1, 0.2, 0.3), DVec3::new(-0.4, 0.05, -0.9)] {
                let at = origin + offset;
                let foot = plane.foot_ly(at, origin);
                let left = plane.height_m(foot, origin);
                let floor = floor_m(at.length());
                assert!(left.abs() < floor, "{plane:?}: foot is {left} m off, floor is {floor}");
            }
        }
    }

    /// And the foot is directly below, not merely somewhere in the plane.
    #[test]
    fn the_foot_is_straight_below_what_it_hangs_from() {
        let origin = DVec3::ZERO;
        for plane in datums() {
            let at = DVec3::new(0.3, -0.2, 0.5);
            let drop = plane.foot_ly(at, origin) - at;
            assert!(drop.cross(plane.normal()).length() < 1e-12, "{plane:?}: not vertical");
        }
    }

    /// Something already in the plane has nowhere to fall, and the host must draw no line.
    #[test]
    fn something_in_the_plane_has_no_drop_at_all() {
        let origin = DVec3::new(1.0, 2.0, 3.0);
        for plane in datums() {
            let (u, v, _) = plane.basis();
            let at = origin + u * 0.4 - v * 0.9;
            assert!(plane.height_m(at, origin).abs() < floor_m(at.length()));
            assert!((plane.foot_ly(at, origin) - at).length() < 1e-12);
        }
    }

    /// Height is signed, so a body below the plane drops upward.
    #[test]
    fn below_the_plane_is_negative() {
        let plane = Plane::Ecliptic.about(DVec3::Z);
        assert!(plane.height_m(DVec3::Z, DVec3::ZERO) > 0.0);
        assert!(plane.height_m(-DVec3::Z, DVec3::ZERO) < 0.0);
    }

    /// The two planes differ, which is the point of the toggle. A `Galactic` wired to `+Z`
    /// passes every other test here. Measured against Sol, whose pole is `+Z`, because 60.19°
    /// is the real tilt between the ecliptic of J2000 and the galactic plane.
    #[test]
    fn the_two_planes_disagree() {
        let ecliptic = Plane::Ecliptic.about(DVec3::Z);
        let galactic = Plane::Galactic.about(DVec3::Z);
        let tilt = ecliptic.normal().dot(galactic.normal()).acos().to_degrees();
        assert!((tilt - 60.19).abs() < 0.01, "{tilt}°");

        let at = DVec3::new(0.0, 0.0, 1.0);
        let a = ecliptic.height_m(at, DVec3::ZERO);
        let b = galactic.height_m(at, DVec3::ZERO);
        assert!((a - b).abs() > 0.4 * M_PER_LY, "a point reads the same height in both");
    }

    /// The whole point of resolving: a system's ecliptic is its own planets' plane, and only
    /// Sol's is `+Z`. This is the bug doc 25 opens with.
    #[test]
    fn an_ecliptic_is_the_system_it_was_resolved_against() {
        let pole = a_pole();
        assert!((Plane::Ecliptic.about(pole).normal() - pole).length() < 1e-12);
        assert_eq!(Plane::Ecliptic.about(DVec3::Z).normal(), DVec3::Z);
        // And the galactic frame is the same in every system.
        assert_eq!(Plane::Galactic.about(pole), Plane::Galactic.about(DVec3::Z));
    }

    /// Zero longitude is the ascending node on the galactic plane: in both planes at once, and
    /// on the ascending side, so a body there is heading galactic north.
    #[test]
    fn longitude_starts_at_the_ascending_galactic_node() {
        let north = galactic::north_pole();
        for pole in [a_pole(), DVec3::Z, DVec3::X, -DVec3::Z, DVec3::new(-0.2, 0.9, 0.3).normalize()] {
            let datum = Plane::Ecliptic.about(pole.normalize());
            let (u, _, n) = datum.basis();
            assert!(u.dot(n).abs() < 1e-12, "{pole}: the zero is out of its own plane");
            assert!(u.dot(north).abs() < 1e-12, "{pole}: the zero is off the galactic plane");
            assert!(n.cross(u).dot(north) > 0.0, "{pole}: that is the descending node");
        }
    }

    /// A system whose plane is the galactic plane has no node to start from, and must still
    /// produce a usable frame rather than a zero vector.
    #[test]
    fn a_system_lying_in_the_galactic_plane_falls_back() {
        for pole in [galactic::north_pole(), -galactic::north_pole()] {
            let (u, v, n) = Plane::Ecliptic.about(pole).basis();
            assert!((u.length() - 1.0).abs() < 1e-12, "the zero is not a unit vector");
            assert!(u.dot(n).abs() < 1e-12, "the zero is out of the plane");
            assert!((u.cross(v).dot(n) - 1.0).abs() < 1e-12, "left-handed");
        }
    }

    #[test]
    fn a_basis_is_orthonormal_and_right_handed() {
        for plane in datums() {
            let (u, v, n) = plane.basis();
            assert!((u.cross(v).dot(n) - 1.0).abs() < 1e-12, "{plane:?} is left-handed");
            assert!((n - plane.normal()).length() < 1e-12, "{plane:?}: basis and normal differ");
        }
    }
}
