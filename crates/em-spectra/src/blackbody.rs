//! Planck radiation and the quantities derived from it.

use crate::bands::{Band, C};

/// Planck constant, J s. Exact since the 2019 SI redefinition.
pub const H: f64 = 6.626_070_15e-34;
/// Boltzmann constant, J/K. Exact.
pub const K_B: f64 = 1.380_649e-23;
/// Stefan-Boltzmann constant, W m^-2 K^-4.
pub const SIGMA: f64 = 5.670_374_419e-8;
/// Wien displacement constant, m K.
pub const WIEN_B: f64 = 2.897_771_955e-3;

const HC_OVER_K: f64 = H * C / K_B;
const TWO_H_C2: f64 = 2.0 * H * C * C;

/// Spectral radiance per unit wavelength, W m^-3 sr^-1.
pub fn spectral_radiance(wavelength_m: f64, temperature_k: f64) -> f64 {
    if wavelength_m <= 0.0 || temperature_k <= 0.0 {
        return 0.0;
    }
    let x = HC_OVER_K / (wavelength_m * temperature_k);
    // exp overflows f64 near 709, and 2hc^2/lambda^5 can overflow for tiny wavelengths, so
    // inf/inf would give NaN. The tail is zero to any precision that matters.
    if x > 700.0 {
        return 0.0;
    }
    // expm1 rather than exp - 1: the radio bands sit deep in the Rayleigh-Jeans regime where
    // x is around 1e-5 and the subtraction would lose eleven digits.
    TWO_H_C2 / wavelength_m.powi(5) / x.exp_m1()
}

/// Wavelength of peak spectral radiance, m.
#[inline]
pub fn wien_peak_m(temperature_k: f64) -> f64 {
    WIEN_B / temperature_k
}

/// Radiant exitance from a black surface, W/m^2.
#[inline]
pub fn radiant_exitance(temperature_k: f64) -> f64 {
    SIGMA * temperature_k.powi(4)
}

/// Bolometric luminosity of a sphere, W.
#[inline]
pub fn luminosity(radius_m: f64, temperature_k: f64) -> f64 {
    4.0 * std::f64::consts::PI * radius_m * radius_m * radiant_exitance(temperature_k)
}

/// Radiance integrated over a band's nominal top-hat, W m^-2 sr^-1.
///
/// A top-hat, not a real filter response. Adequate for colours and ratios; a photometric
/// zero-point calibration would need the response curve.
pub fn band_radiance(band: Band, temperature_k: f64) -> f64 {
    let (lo, hi) = band.limits_m();
    simpson(lo, hi, 32, |l| spectral_radiance(l, temperature_k))
}

/// Composite Simpson over an even number of intervals.
fn simpson(lo: f64, hi: f64, intervals: usize, f: impl Fn(f64) -> f64) -> f64 {
    debug_assert!(intervals % 2 == 0 && intervals > 0);
    let h = (hi - lo) / intervals as f64;
    let mut sum = f(lo) + f(hi);
    for i in 1..intervals {
        sum += f(lo + i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    sum * h / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Integrating Planck over all wavelengths must give sigma T^4 / pi.
    fn integrate_all(t: f64) -> f64 {
        simpson(1e-8, 300e-6, 200_000, |l| spectral_radiance(l, t))
    }

    #[test]
    fn planck_integrates_to_stefan_boltzmann() {
        for t in [3000.0, 5772.0, 10000.0] {
            let got = integrate_all(t);
            let want = radiant_exitance(t) / PI;
            assert!((got / want - 1.0).abs() < 1e-5, "T={t}: {got} vs {want}");
        }
    }

    #[test]
    fn wien_locates_the_peak() {
        for t in [1000.0, 5772.0, 30000.0] {
            let peak = wien_peak_m(t);
            let at_peak = spectral_radiance(peak, t);
            for factor in [0.5, 0.8, 1.25, 2.0] {
                assert!(spectral_radiance(peak * factor, t) < at_peak);
            }
        }
        // The Sun peaks at 502 nm; a 300 K instrument peaks at 9.66 um, in the thermal band.
        assert!((wien_peak_m(5772.0) - 502e-9).abs() < 1e-9);
        let warm = wien_peak_m(300.0);
        assert!((warm - 9.66e-6).abs() < 0.02e-6, "{warm}");
        let (lo, hi) = Band::ThermalIr.limits_m();
        assert!(warm > lo && warm < hi, "a warm instrument emits into its own thermal band");
    }

    #[test]
    fn the_radio_band_stays_precise_in_the_rayleigh_jeans_tail() {
        // x = hc/(lambda k T) is about 1.2e-5 here, where exp(x) - 1 would lose eleven
        // digits. Rayleigh-Jeans gives 2 c k T / lambda^4 as the limit.
        let (lambda, t) = (Band::Radio.centre_m(), 5772.0);
        let exact = spectral_radiance(lambda, t);
        let rj = 2.0 * C * K_B * t / lambda.powi(4);
        assert!((exact / rj - 1.0).abs() < 1e-4, "{exact} vs Rayleigh-Jeans {rj}");
    }

    #[test]
    fn extreme_arguments_give_zero_rather_than_nan() {
        for (l, t) in [(1e-12, 3.0), (1e-300, 5772.0), (0.0, 5772.0), (500e-9, 0.0)] {
            let v = spectral_radiance(l, t);
            assert!(v.is_finite(), "lambda={l} T={t} gave {v}");
            assert!(v >= 0.0);
        }
    }

    #[test]
    fn band_radiance_is_converged() {
        let (lo, hi) = Band::V.limits_m();
        let fine = simpson(lo, hi, 4096, |l| spectral_radiance(l, 5772.0));
        let got = band_radiance(Band::V, 5772.0);
        assert!((got / fine - 1.0).abs() < 1e-9, "{got} vs {fine}");
    }

    #[test]
    fn a_hotter_star_is_bluer_in_band_ratios() {
        let cool = band_radiance(Band::B, 3500.0) / band_radiance(Band::I, 3500.0);
        let hot = band_radiance(Band::B, 12000.0) / band_radiance(Band::I, 12000.0);
        assert!(hot > cool, "B/I must rise with temperature: {hot} vs {cool}");
    }

    #[test]
    fn solar_luminosity_comes_out_right() {
        let l = luminosity(6.957e8, 5772.0);
        assert!((l / 3.828e26 - 1.0).abs() < 0.01, "{l} W");
    }
}
