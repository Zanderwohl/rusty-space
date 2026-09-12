//! The equatorial frame, and the one rotation that reaches simulation space.
//!
//! Catalogues and IAU pole tables are published in the **equatorial** frame of J2000
//! (ICRF): +X toward the vernal equinox, +Z toward the celestial pole. Simulation space
//! shares +X but tilts +Z to the *ecliptic* pole, so the two differ by a single rotation
//! about +X through the obliquity. Skipping it leaves everything 23.4° out — visible as a
//! sky whose constellations sit at the wrong angle to the planets.
//!
//! Radians, as everywhere in this crate. Catalogues that store right ascension in hours
//! convert on the way in.

use glam::DVec3;

/// Obliquity of the ecliptic at J2000, in radians (23.4392911°).
///
/// The IAU 1976 value at J2000.0. It drifts by about 47 arcseconds per century; over the
/// span a catalogue is useful that is far below the precision of the catalogue itself, so
/// this is a constant rather than a function of epoch.
pub const OBLIQUITY_J2000: f64 = 23.439_291_1 * std::f64::consts::PI / 180.0;

/// Unit vector in the equatorial frame from right ascension and declination.
///
/// `right_ascension` measures east from the vernal equinox; `declination` measures north
/// from the celestial equator. Both radians.
#[inline]
pub fn direction(right_ascension: f64, declination: f64) -> DVec3 {
    let (sin_ra, cos_ra) = right_ascension.sin_cos();
    let (sin_dec, cos_dec) = declination.sin_cos();
    DVec3::new(cos_dec * cos_ra, cos_dec * sin_ra, sin_dec)
}

/// Rotate an equatorial vector into simulation space (ecliptic of J2000, Z-up).
///
/// A rotation about the shared +X axis by −[`OBLIQUITY_J2000`]. Length is preserved, so
/// this serves positions and directions alike.
#[inline]
pub fn to_ecliptic(v: DVec3) -> DVec3 {
    let (sin_obl, cos_obl) = OBLIQUITY_J2000.sin_cos();
    DVec3::new(
        v.x,
        v.y * cos_obl + v.z * sin_obl,
        -v.y * sin_obl + v.z * cos_obl,
    )
}

/// Rotate a simulation-space vector back into the equatorial frame. The inverse of
/// [`to_ecliptic`].
#[inline]
pub fn from_ecliptic(v: DVec3) -> DVec3 {
    let (sin_obl, cos_obl) = OBLIQUITY_J2000.sin_cos();
    DVec3::new(
        v.x,
        v.y * cos_obl - v.z * sin_obl,
        v.y * sin_obl + v.z * cos_obl,
    )
}

/// Simulation-space unit vector from equatorial right ascension and declination, radians.
///
/// [`direction`] followed by [`to_ecliptic`], which is what a catalogue import wants.
#[inline]
pub fn ecliptic_direction(right_ascension: f64, declination: f64) -> DVec3 {
    to_ecliptic(direction(right_ascension, declination))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn close(a: DVec3, b: DVec3, tol: f64) -> bool {
        (a - b).length() < tol
    }

    /// The vernal equinox is the shared axis: RA 0h, dec 0° is +X in both frames.
    #[test]
    fn the_vernal_equinox_is_untouched() {
        let v = ecliptic_direction(0.0, 0.0);
        assert!(close(v, DVec3::X, 1e-12), "expected +X, got {v:?}");
    }

    /// The ecliptic pole sits at RA 18h, dec 90° − ε in the equatorial frame. If the
    /// obliquity is dropped or signed the wrong way, this lands 23.4° off +Z — the exact
    /// failure this module exists to prevent.
    #[test]
    fn the_ecliptic_pole_becomes_plus_z() {
        let ra = 18.0 * PI / 12.0; // 18 hours
        let dec = FRAC_PI_2 - OBLIQUITY_J2000;
        let v = ecliptic_direction(ra, dec);
        assert!(close(v, DVec3::Z, 1e-12), "expected +Z, got {v:?}");
    }

    /// And the celestial pole, which is +Z in the equatorial frame, tilts by exactly the
    /// obliquity.
    #[test]
    fn the_celestial_pole_tilts_by_the_obliquity() {
        let v = to_ecliptic(DVec3::Z);
        let angle = v.dot(DVec3::Z).clamp(-1.0, 1.0).acos();
        assert!((angle - OBLIQUITY_J2000).abs() < 1e-12, "tilted {angle} rad");
        assert!(v.y > 0.0, "the celestial pole leans toward +Y, got {v:?}");
    }

    /// Summer solstice: RA 6h, dec +ε is the ecliptic's northernmost point, so it must
    /// land in the ecliptic plane at +Y.
    #[test]
    fn the_solstice_lands_in_the_ecliptic_plane() {
        let ra = 6.0 * PI / 12.0; // 6 hours
        let v = ecliptic_direction(ra, OBLIQUITY_J2000);
        assert!(close(v, DVec3::Y, 1e-12), "expected +Y, got {v:?}");
    }

    #[test]
    fn the_rotation_round_trips() {
        let samples = [
            DVec3::X, DVec3::Y, DVec3::Z,
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(-4.0, 0.5, -7.25),
        ];
        for v in samples {
            assert!(close(from_ecliptic(to_ecliptic(v)), v, 1e-12), "{v:?}");
            assert!(close(to_ecliptic(from_ecliptic(v)), v, 1e-12), "{v:?}");
        }
    }

    /// A rotation, not a scaling or a mirror: lengths and handedness survive.
    #[test]
    fn it_is_a_proper_rotation() {
        let (x, y, z) = (to_ecliptic(DVec3::X), to_ecliptic(DVec3::Y), to_ecliptic(DVec3::Z));
        assert!((x.dot(y.cross(z)) - 1.0).abs() < 1e-12, "determinant should be +1");
        for v in [DVec3::new(3.0, -1.0, 2.0), DVec3::new(0.0, 5.0, 0.0)] {
            assert!((to_ecliptic(v).length() - v.length()).abs() < 1e-12);
        }
    }

    #[test]
    fn declination_is_measured_from_the_equator() {
        assert!(close(direction(0.0, FRAC_PI_2), DVec3::Z, 1e-12));
        assert!(close(direction(FRAC_PI_2, 0.0), DVec3::Y, 1e-12));
    }
}
