//! Doppler shift, aberration and relativistic beaming. Velocities are `beta`.

use glam::DVec3;

/// Lorentz factor. Panics in debug if `beta >= 1`.
#[inline]
pub fn gamma(beta: DVec3) -> f64 {
    let b2 = beta.length_squared();
    debug_assert!(b2 < 1.0, "gamma is undefined at or above c");
    (1.0 - b2).sqrt().recip()
}

/// Observed over emitted frequency, for an observer at `beta` looking along `to_source`.
///
/// Above 1 is a blueshift. Approaching gives `sqrt((1+b)/(1-b))`; moving across still gives
/// `gamma`, the observer's own clock running slow.
#[inline]
pub fn doppler_factor(to_source: DVec3, beta: DVec3) -> f64 {
    gamma(beta) * (1.0 + beta.dot(to_source))
}

/// How a direction of light *propagation* transforms into a moving observer's frame.
#[inline]
pub fn aberrate_propagation(n: DVec3, beta: DVec3) -> DVec3 {
    let g = gamma(beta);
    let ndb = n.dot(beta);
    let num = n + beta * (g * (g / (g + 1.0) * ndb - 1.0));
    (num / (g * (1.0 - ndb))).normalize()
}

/// Where a source actually along `to_source` appears to be.
///
/// The sky compresses forward: at `beta = 0.5` a source 90 degrees off the bow appears at 60.
#[inline]
pub fn apparent_source_direction(to_source: DVec3, beta: DVec3) -> DVec3 {
    -aberrate_propagation(-to_source, beta)
}

/// Bolometric intensity, as the fourth power of the Doppler factor.
#[inline]
pub fn beaming_factor(to_source: DVec3, beta: DVec3) -> f64 {
    doppler_factor(to_source, beta).powi(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    const X: DVec3 = DVec3::X;
    const Y: DVec3 = DVec3::Y;

    #[test]
    fn a_stationary_observer_sees_no_shift_and_no_aberration() {
        assert_eq!(doppler_factor(X, DVec3::ZERO), 1.0);
        assert!((apparent_source_direction(Y, DVec3::ZERO) - Y).length() < 1e-15);
    }

    #[test]
    fn head_on_and_receding_are_reciprocal() {
        let beta = X * 0.5;
        let toward = doppler_factor(X, beta);
        let away = doppler_factor(-X, beta);
        assert!((toward - 3f64.sqrt()).abs() < 1e-12, "approach gives sqrt(3), got {toward}");
        assert!((away - 1.0 / 3f64.sqrt()).abs() < 1e-12);
        assert!((toward * away - 1.0).abs() < 1e-12);
    }

    #[test]
    fn transverse_motion_still_blueshifts() {
        let beta = X * 0.5;
        let d = doppler_factor(Y, beta);
        assert!((d - gamma(beta)).abs() < 1e-12, "transverse factor is gamma, got {d}");
        assert!(d > 1.0);
    }

    #[test]
    fn a_source_at_90_degrees_appears_at_60_at_half_c() {
        let beta = X * 0.5;
        let apparent = apparent_source_direction(Y, beta);
        let angle = apparent.dot(X).acos().to_degrees();
        assert!((angle - 60.0).abs() < 1e-9, "expected 60 degrees, got {angle}");
        assert!((apparent.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn aberration_leaves_the_poles_of_motion_alone() {
        let beta = X * 0.75;
        assert!((apparent_source_direction(X, beta) - X).length() < 1e-12);
        assert!((apparent_source_direction(-X, beta) + X).length() < 1e-12);
    }

    #[test]
    fn aberration_is_invertible_by_reversing_beta() {
        let beta = DVec3::new(0.3, -0.2, 0.1);
        let s = DVec3::new(1.0, 2.0, -3.0).normalize();
        let there = apparent_source_direction(s, beta);
        let back = apparent_source_direction(there, -beta);
        assert!((back - s).length() < 1e-12, "{back} should be {s}");
    }

    #[test]
    fn a_600nm_source_leaves_the_visible_band_at_half_c() {
        // Head-on: 600 nm arrives at 347 nm, ultraviolet. Astern: 1039 nm, infrared.
        let beta = X * 0.5;
        let ahead = 600.0 / doppler_factor(X, beta);
        let astern = 600.0 / doppler_factor(-X, beta);
        assert!((ahead - 346.4).abs() < 0.1, "{ahead}");
        assert!((astern - 1039.2).abs() < 0.1, "{astern}");
    }

    #[test]
    fn forward_sources_beam_by_the_fourth_power() {
        let beta = X * 0.5;
        assert!((beaming_factor(X, beta) - 9.0).abs() < 1e-9);
    }
}
