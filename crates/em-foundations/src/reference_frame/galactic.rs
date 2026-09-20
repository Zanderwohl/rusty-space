//! The galactic frame: the plane the Milky Way's disc lies in.
//!
//! A map can show what is above and below the *local* plane — the ecliptic — or above and
//! below the disc of the galaxy, and the two are 60° apart. Simulation space is the ecliptic
//! of J2000, so the ecliptic pole is `+Z` by construction and needs nothing; the galactic pole
//! is a published equatorial direction that reaches simulation space through
//! [`super::equatorial::to_ecliptic`], the same one rotation a catalogue import takes.
//!
//! Directions only. Where the galactic plane's own zero point sits is 26 000 light-years away
//! and irrelevant to anything drawn about a star: a map anchors its plane at what it is looking
//! at, and carrying that offset would only spend precision.
//!
//! Radians, as everywhere in this crate.

use glam::DVec3;

use super::equatorial;

const DEG: f64 = std::f64::consts::PI / 180.0;

/// Right ascension of the north galactic pole, J2000 (192.85948°).
///
/// The IAU 1958 pole as re-expressed in the ICRS by Hipparcos. Galactic coordinates are
/// *defined* by this direction rather than fitted to the visible disc, so it is exact by
/// convention and does not drift.
pub const NORTH_POLE_RA: f64 = 192.859_48 * DEG;

/// Declination of the north galactic pole, J2000 (27.12825°).
pub const NORTH_POLE_DEC: f64 = 27.128_25 * DEG;

/// Right ascension of the galactic center, J2000 (266.40510°).
pub const CENTER_RA: f64 = 266.405_10 * DEG;

/// Declination of the galactic center, J2000 (−28.93617°).
pub const CENTER_DEC: f64 = -28.936_17 * DEG;

/// The north galactic pole, as a unit vector in simulation space.
#[inline]
pub fn north_pole() -> DVec3 {
    equatorial::ecliptic_direction(NORTH_POLE_RA, NORTH_POLE_DEC)
}

/// The direction of the galactic center, as a unit vector in simulation space.
#[inline]
pub fn center() -> DVec3 {
    equatorial::ecliptic_direction(CENTER_RA, CENTER_DEC)
}

/// A right-handed orthonormal basis for the galactic frame, in simulation space: `(u, v, n)`.
///
/// `n` is the north pole and `u` points at the galactic center. The two published directions
/// are perpendicular to within a ten-thousandth of a degree and not exactly, so `u` is the
/// center with its `n` component removed rather than the center itself — otherwise the basis
/// would be very slightly skewed, and a basis is either orthonormal or it is a source of drift.
pub fn basis() -> (DVec3, DVec3, DVec3) {
    let n = north_pole();
    let u = (center() - n * center().dot(n)).normalize();
    (u, n.cross(u), n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    /// The galactic plane is inclined to the ecliptic by 60.2°.
    ///
    /// Derived here from the pole and the obliquity; 60.19° is the independently published
    /// figure, which is the point — a test that recovered the constant it was given would pass
    /// with the obliquity dropped, and dropping it is this module's one real failure mode. Done
    /// that way it lands at 62.87° instead, the equatorial answer.
    #[test]
    fn the_galactic_plane_is_sixty_degrees_from_the_ecliptic() {
        let tilt = north_pole().dot(DVec3::Z).clamp(-1.0, 1.0).acos();
        assert!(
            (tilt / DEG - 60.19).abs() < 0.01,
            "inclination {:.4}°, expected 60.19°",
            tilt / DEG
        );
    }

    /// And 62.87° from the *celestial* pole, which is 90° minus the pole's own declination and
    /// so tests the right ascension and declination went in the right way round.
    #[test]
    fn the_pole_sits_at_its_own_declination() {
        let equatorial_pole = equatorial::to_ecliptic(DVec3::Z);
        let angle = north_pole().dot(equatorial_pole).clamp(-1.0, 1.0).acos();
        assert!(
            (angle - (FRAC_PI_2 - NORTH_POLE_DEC)).abs() < 1e-9,
            "{:.5}°, expected {:.5}°",
            angle / DEG,
            (FRAC_PI_2 - NORTH_POLE_DEC) / DEG
        );
    }

    /// The galactic center lies in the galactic plane, which is what makes it usable as the
    /// basis's first axis. Not exactly: the two published directions disagree by under a
    /// ten-thousandth of a degree, which is the convention's own rounding.
    #[test]
    fn the_center_is_in_the_plane() {
        let out_of_plane = center().dot(north_pole()).abs();
        assert!(out_of_plane < 1e-5, "center is {out_of_plane:.2e} out of its own plane");
    }

    /// Seen from here, the galactic center is a few degrees below the ecliptic at a longitude
    /// of about 267° — a figure any ephemeris will give, and one no arrangement of the wrong
    /// axes reproduces.
    #[test]
    fn the_center_is_where_the_summer_sky_has_it() {
        let c = center();
        let longitude = c.y.atan2(c.x).rem_euclid(std::f64::consts::TAU) / DEG;
        let latitude = c.z.asin() / DEG;
        assert!((longitude - 266.84).abs() < 0.01, "longitude {longitude:.3}°");
        assert!((latitude + 5.54).abs() < 0.01, "latitude {latitude:.3}°");
    }

    /// Orthonormal and right-handed, or everything built on it shears.
    #[test]
    fn the_basis_is_a_proper_frame() {
        let (u, v, n) = basis();
        for (name, a) in [("u", u), ("v", v), ("n", n)] {
            assert!((a.length() - 1.0).abs() < 1e-12, "{name} is not unit: {a:?}");
        }
        assert!(u.dot(v).abs() < 1e-12 && v.dot(n).abs() < 1e-12 && n.dot(u).abs() < 1e-12);
        assert!((u.cross(v).dot(n) - 1.0).abs() < 1e-12, "left-handed");
    }
}
