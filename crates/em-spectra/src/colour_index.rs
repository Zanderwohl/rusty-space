//! Colour index to effective temperature.

/// `B - V` outside this range puts Ballesteros' fit outside the data it was fitted to.
pub const BV_VALID: (f64, f64) = (-0.4, 2.0);

/// Effective temperature from `B - V`, by Ballesteros' two-blackbody fit.
///
/// Returns 5778 K at `B - V = 0.65`, against the Sun's actual 5772 K.
pub fn teff_from_bv(bv: f64) -> f64 {
    4600.0 * (1.0 / (0.92 * bv + 1.70) + 1.0 / (0.92 * bv + 0.62))
}

#[inline]
pub fn bv_is_valid(bv: f64) -> bool {
    bv >= BV_VALID.0 && bv <= BV_VALID.1
}

/// Inverse of [`teff_from_bv`], by bisection over [`BV_VALID`].
///
/// `None` when the temperature is outside what the fit spans, which is roughly 3000 K to
/// 20 000 K.
pub fn bv_from_teff(teff: f64) -> Option<f64> {
    let (mut lo, mut hi) = BV_VALID;
    // teff_from_bv decreases monotonically over the valid range.
    if teff > teff_from_bv(lo) || teff < teff_from_bv(hi) {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if teff_from_bv(mid) > teff { lo = mid } else { hi = mid }
    }
    Some(0.5 * (lo + hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ballesteros_reproduces_known_stars() {
        for (bv, want, who) in [
            (-0.33, 17833.0, "B0"),
            (0.00, 10125.0, "A0"),
            (0.31, 7399.0, "F0"),
            (0.65, 5778.0, "the Sun"),
            (0.81, 5251.0, "K0"),
            (1.40, 3950.0, "M0"),
        ] {
            let got = teff_from_bv(bv);
            assert!((got - want).abs() < 1.0, "{who}: got {got}, want {want}");
        }
    }

    #[test]
    fn the_sun_lands_within_ten_kelvin_of_its_real_temperature() {
        assert!((teff_from_bv(0.65) - 5772.0).abs() < 10.0);
    }

    #[test]
    fn temperature_falls_monotonically_with_colour() {
        let mut prev = f64::INFINITY;
        let mut bv = BV_VALID.0;
        while bv <= BV_VALID.1 {
            let t = teff_from_bv(bv);
            assert!(t < prev, "not monotonic at B-V={bv}");
            prev = t;
            bv += 0.01;
        }
    }

    #[test]
    fn the_inverse_round_trips() {
        for bv in [-0.3, 0.0, 0.31, 0.65, 1.0, 1.4, 1.9] {
            let t = teff_from_bv(bv);
            let back = bv_from_teff(t).expect("in range");
            assert!((back - bv).abs() < 1e-9, "{bv} -> {t} -> {back}");
        }
        assert!(bv_from_teff(50_000.0).is_none());
        assert!(bv_from_teff(1_000.0).is_none());
    }

    #[test]
    fn validity_is_reported_rather_than_clamped() {
        assert!(bv_is_valid(0.65));
        assert!(!bv_is_valid(3.0));
        // The fit still returns a number outside its range; callers must check.
        assert!(teff_from_bv(3.0).is_finite());
    }
}
