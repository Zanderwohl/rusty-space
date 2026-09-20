//! The surface everything is measured above and below.

use em_foundations::reference_frame::galactic;
use glam::DVec3;

use crate::snapshot::M_PER_LY;

/// Which plane the map lays its rings in.
///
/// Two, because there are two questions. Where a moon sits relative to its system is a
/// question about the ecliptic; where a system sits relative to everything else is a question
/// about the disc of the galaxy. Drawing either one against the other's plane answers neither.
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

    /// The plane's normal, a unit vector in simulation space.
    ///
    /// The ecliptic's is `+Z` by construction rather than by coincidence: simulation space
    /// *is* the ecliptic of J2000. That is worth saying, because it looks like a shortcut.
    pub fn normal(self) -> DVec3 {
        match self {
            Self::Ecliptic => DVec3::Z,
            Self::Galactic => galactic::north_pole(),
        }
    }

    /// A right-handed orthonormal basis `(u, v, n)`, with `n` the normal.
    ///
    /// `u` is where a ring's zero bearing points and where the camera's azimuth is measured
    /// from, so it has to be a *fixed* direction rather than anything derived from the view —
    /// otherwise turning the camera would turn the thing it is measured against.
    pub fn basis(self) -> (DVec3, DVec3, DVec3) {
        match self {
            Self::Ecliptic => (DVec3::X, DVec3::Y, DVec3::Z),
            Self::Galactic => galactic::basis(),
        }
    }

    /// How far `at_ly` stands above the plane through `origin_ly`, meters. Signed.
    ///
    /// The plane is anchored at whatever the map is looking at, not at a galaxy's own zero
    /// point. At every scale this draws, where that zero sits makes no visible difference, and
    /// carrying a 26 000-light-year offset would spend precision on nothing.
    pub fn height_m(self, at_ly: DVec3, origin_ly: DVec3) -> f64 {
        (at_ly - origin_ly).dot(self.normal()) * M_PER_LY
    }

    /// Where a ray meets the plane, or `None` when it runs along it or points away.
    ///
    /// What "zoom toward what the cursor is over" is built from: the cursor names a ray, the
    /// ray names a place on the plane, and the place is held still while the camera comes in.
    pub fn intersect(self, from_ly: DVec3, direction: DVec3, origin_ly: DVec3) -> Option<DVec3> {
        let n = self.normal();
        let along = direction.dot(n);
        // A ray within a thousandth of parallel names a point so far away that holding it still
        // would throw the camera across the system. Edge-on, there is nothing under the cursor.
        if along.abs() < 1.0e-3 {
            return None;
        }
        let t = (origin_ly - from_ly).dot(n) / along;
        (t > 0.0 && t.is_finite()).then(|| from_ly + direction * t)
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

    /// What a light-year's worth of `f64` is worth, in meters.
    ///
    /// The mantissa is 53 bits, so a position held in light-years resolves about a meter per
    /// light-year of magnitude. A drop-line's foot is therefore "in the plane" to a few meters
    /// and never to less — asserting meters flat passes near the origin and fails at Alpha
    /// Centauri, which reads as a bug in the plane and is the representation.
    fn floor_m(magnitude_ly: f64) -> f64 {
        (magnitude_ly.abs().max(1.0) * M_PER_LY * 4.0 * f64::EPSILON).max(1.0)
    }

    /// Both planes, and both signs. A galactic drop-line that used `+Z` would land in the
    /// ecliptic instead and look entirely plausible from most angles.
    #[test]
    fn a_drop_line_ends_in_the_plane_it_was_dropped_to() {
        let origin = DVec3::new(4.2, -1.0, 0.7);
        for plane in [Plane::Ecliptic, Plane::Galactic] {
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
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let at = DVec3::new(0.3, -0.2, 0.5);
            let drop = plane.foot_ly(at, origin) - at;
            assert!(drop.cross(plane.normal()).length() < 1e-12, "{plane:?}: not vertical");
        }
    }

    /// Something already in the plane has nowhere to fall, and the host must draw no line.
    #[test]
    fn something_in_the_plane_has_no_drop_at_all() {
        let origin = DVec3::new(1.0, 2.0, 3.0);
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let (u, v, _) = plane.basis();
            let at = origin + u * 0.4 - v * 0.9;
            assert!(plane.height_m(at, origin).abs() < floor_m(at.length()));
            assert!((plane.foot_ly(at, origin) - at).length() < 1e-12);
        }
    }

    /// Height is signed, so a body below the plane drops upward.
    #[test]
    fn below_the_plane_is_negative() {
        let plane = Plane::Ecliptic;
        assert!(plane.height_m(DVec3::Z, DVec3::ZERO) > 0.0);
        assert!(plane.height_m(-DVec3::Z, DVec3::ZERO) < 0.0);
    }

    /// The two planes are genuinely different, which is the whole point of the toggle. A
    /// `Galactic` wired to `+Z` passes every other test in this file.
    #[test]
    fn the_two_planes_disagree() {
        let tilt = Plane::Ecliptic.normal().dot(Plane::Galactic.normal()).acos().to_degrees();
        assert!((tilt - 60.19).abs() < 0.01, "{tilt}°");

        let at = DVec3::new(0.0, 0.0, 1.0);
        let a = Plane::Ecliptic.height_m(at, DVec3::ZERO);
        let b = Plane::Galactic.height_m(at, DVec3::ZERO);
        assert!((a - b).abs() > 0.4 * M_PER_LY, "a point reads the same height in both");
    }

    #[test]
    fn a_basis_is_orthonormal_and_right_handed() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let (u, v, n) = plane.basis();
            assert!((u.cross(v).dot(n) - 1.0).abs() < 1e-12, "{plane:?} is left-handed");
            assert!((n - plane.normal()).length() < 1e-12, "{plane:?}: basis and normal differ");
        }
    }
}
