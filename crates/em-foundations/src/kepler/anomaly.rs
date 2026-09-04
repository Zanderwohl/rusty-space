//! Converting between mean, eccentric and true anomaly.
//!
//! All angles are radians.
//!
//! Kepler's equation `M = E - e sin E` has no closed-form inverse, so getting from mean
//! anomaly to position needs either an iterative solve or a truncated series. Both are
//! exported: prefer [`eccentric_from_mean_halley`] for anything that matters, and reach
//! for the series only when you specifically want the classical expansion.
//!
//! # Why the series are not the default
//!
//! [`true_from_mean_bessel`] is a Fourier expansion in Bessel functions. It diverges past
//! the Laplace limit `e ≈ 0.6627`, and no iteration count rescues it. Even below that it
//! converges slowly. Measured against a converged solve, its worst-case true-anomaly
//! error is roughly 0.96° at Earth's eccentricity, 5.4° at Mars', 12.4° at Mercury's and
//! 31° at Eris'. An along-track angular error of `d` radians at radius `r` displaces a
//! body by `r*d`, which made it the dominant error term in this crate.
//!
//! [`eccentric_from_mean_halley`] converges cubically — two or three iterations to
//! machine precision at planetary eccentricities — and costs less than the ten Bessel
//! evaluations it replaces.

use std::f64::consts::{PI, TAU};

use scilib::math::bessel;

use crate::common::unit_circle_xy;

/// Convergence tolerance in radians. Near f64 resolution for angles of order 1.
pub const DEFAULT_TOLERANCE: f64 = 1e-13;

/// Iteration ceiling. Halley reaches `DEFAULT_TOLERANCE` in 2-4 steps for `e < 0.99`;
/// this only exists so a pathological input terminates.
pub const DEFAULT_MAX_ITERATIONS: u32 = 64;

/// Wrap an angle into `[0, 2π)`.
#[inline]
pub fn wrap_tau(angle: f64) -> f64 {
    angle.rem_euclid(TAU)
}

/// Wrap an angle into `(-π, π]`.
#[inline]
pub fn wrap_pi(angle: f64) -> f64 {
    let a = angle.rem_euclid(TAU);
    if a > PI { a - TAU } else { a }
}

// ---------------------------------------------------------------------------
// Elliptical: solving M = E - e sin E
// ---------------------------------------------------------------------------

/// Starting guess for the elliptical solvers.
///
/// `E ≈ M + e sin M` is the first term of the series and is excellent for small `e`.
/// Above `e = 0.8` it can land in a region where convergence is slow, so we use Danby's
/// starter there instead.
#[inline]
fn elliptical_seed(mean_anomaly: f64, eccentricity: f64) -> f64 {
    if eccentricity < 0.8 {
        mean_anomaly + eccentricity * mean_anomaly.sin()
    } else {
        let s = mean_anomaly.sin();
        mean_anomaly + 0.85 * eccentricity * if s >= 0.0 { 1.0 } else { -1.0 }
    }
}

/// Eccentric anomaly from mean anomaly by Newton's method. Quadratic convergence.
///
/// `eccentricity` must be in `[0, 1)`; see [`hyperbolic_from_mean_newton`] for `e > 1`.
pub fn eccentric_from_mean_newton(
    mean_anomaly: f64,
    eccentricity: f64,
    tolerance: f64,
    max_iterations: u32,
) -> f64 {
    let m = wrap_tau(mean_anomaly);
    let mut ea = elliptical_seed(m, eccentricity);
    for _ in 0..max_iterations {
        let f = ea - eccentricity * ea.sin() - m;
        let fp = 1.0 - eccentricity * ea.cos();
        let step = f / fp;
        ea -= step;
        if step.abs() < tolerance {
            break;
        }
    }
    ea
}

/// Eccentric anomaly from mean anomaly by Halley's method. Cubic convergence.
///
/// This is the solver the simulation uses.
///
/// `eccentricity` must be in `[0, 1)`; see [`hyperbolic_from_mean_newton`] for `e > 1`.
pub fn eccentric_from_mean_halley(
    mean_anomaly: f64,
    eccentricity: f64,
    tolerance: f64,
    max_iterations: u32,
) -> f64 {
    let m = wrap_tau(mean_anomaly);
    let mut ea = elliptical_seed(m, eccentricity);
    for _ in 0..max_iterations {
        let (sin_e, cos_e) = ea.sin_cos();
        let f = ea - eccentricity * sin_e - m;
        let fp = 1.0 - eccentricity * cos_e;
        let fpp = eccentricity * sin_e;
        // Halley: E -= 2 f f' / (2 f'^2 - f f'')
        let denominator = 2.0 * fp * fp - f * fpp;
        let step = if denominator == 0.0 { f / fp } else { 2.0 * f * fp / denominator };
        ea -= step;
        if step.abs() < tolerance {
            break;
        }
    }
    ea
}

/// Eccentric anomaly from mean anomaly by the classical series, to `e^3`:
///
/// `E ≈ M + e sin M + (e²/2) sin 2M + (e³/8)(3 sin 3M − sin M)`
///
/// Kept because it is the textbook expansion, and because it is closed-form. Truncated,
/// so accurate only for small `e` — prefer [`eccentric_from_mean_halley`].
pub fn eccentric_from_mean_series(mean_anomaly: f64, eccentricity: f64) -> f64 {
    let m = mean_anomaly;
    let e = eccentricity;
    m + e * m.sin()
        + (e * e / 2.0) * (2.0 * m).sin()
        + (e * e * e / 8.0) * (3.0 * (3.0 * m).sin() - m.sin())
}

// ---------------------------------------------------------------------------
// Hyperbolic: solving M = e sinh H - H
// ---------------------------------------------------------------------------

/// Hyperbolic anomaly from hyperbolic mean anomaly, by Newton's method.
///
/// `eccentricity` must be `> 1`. Unlike the elliptical case the mean anomaly is not
/// periodic, so it is used unwrapped.
pub fn hyperbolic_from_mean_newton(
    mean_anomaly: f64,
    eccentricity: f64,
    tolerance: f64,
    max_iterations: u32,
) -> f64 {
    // Seed: asinh for small M, the logarithmic form once M/e grows.
    let mut h = if mean_anomaly.abs() > 4.0 * eccentricity {
        let sign = if mean_anomaly >= 0.0 { 1.0 } else { -1.0 };
        sign * (2.0 * mean_anomaly.abs() / eccentricity + 1.8).ln()
    } else {
        (mean_anomaly / eccentricity).asinh()
    };

    for _ in 0..max_iterations {
        let f = eccentricity * h.sinh() - h - mean_anomaly;
        let fp = eccentricity * h.cosh() - 1.0;
        let step = f / fp;
        h -= step;
        if step.abs() < tolerance {
            break;
        }
    }
    h
}

// ---------------------------------------------------------------------------
// Anomaly conversions
// ---------------------------------------------------------------------------

/// True anomaly from eccentric anomaly, for `e < 1`.
///
/// Uses the half-angle form, which is numerically better behaved near apoapsis than
/// going through `cos ν` and keeps the quadrant without a sign fix-up.
#[inline]
pub fn true_from_eccentric(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    let half = eccentric_anomaly / 2.0;
    2.0 * f64::atan2(
        (1.0 + eccentricity).sqrt() * half.sin(),
        (1.0 - eccentricity).sqrt() * half.cos(),
    )
}

/// Eccentric anomaly from true anomaly, for `e < 1`. Inverse of [`true_from_eccentric`].
#[inline]
pub fn eccentric_from_true(true_anomaly: f64, eccentricity: f64) -> f64 {
    let half = true_anomaly / 2.0;
    2.0 * f64::atan2(
        (1.0 - eccentricity).sqrt() * half.sin(),
        (1.0 + eccentricity).sqrt() * half.cos(),
    )
}

/// True anomaly from hyperbolic anomaly, for `e > 1`.
#[inline]
pub fn true_from_hyperbolic(hyperbolic_anomaly: f64, eccentricity: f64) -> f64 {
    let half = hyperbolic_anomaly / 2.0;
    2.0 * f64::atan2(
        (eccentricity + 1.0).sqrt() * half.tanh(),
        (eccentricity - 1.0).sqrt(),
    )
}

/// Hyperbolic anomaly from true anomaly, for `e > 1`. Inverse of [`true_from_hyperbolic`].
///
/// Only true anomalies inside the asymptotes are on the trajectory: as `nu` approaches
/// `±acos(-1/e)` the body is escaping to infinity and `H` diverges. Beyond them there is
/// no orbit to speak of, so this returns `None` rather than a number that looks usable.
#[inline]
pub fn hyperbolic_from_true(true_anomaly: f64, eccentricity: f64) -> Option<f64> {
    let half = (true_anomaly / 2.0).tan();
    let ratio = ((eccentricity - 1.0) / (eccentricity + 1.0)).sqrt();
    let x = ratio * half;
    // `atanh` is defined on (-1, 1); |x| >= 1 is the asymptote and past it.
    if !x.is_finite() || x.abs() >= 1.0 {
        return None;
    }
    Some(2.0 * x.atanh())
}

// ---------------------------------------------------------------------------
// Series routes from mean straight to true anomaly
// ---------------------------------------------------------------------------

/// The equation of the centre, expanded to `e^3`.
///
/// Truncated series; see [`super::true_anomaly::from_mean_anomaly`], which this
/// delegates to.
#[inline]
pub fn true_from_mean_series(mean_anomaly: f64, eccentricity: f64) -> f64 {
    super::true_anomaly::from_mean_anomaly(mean_anomaly, eccentricity)
}

/// Fourier expansion of the true anomaly in Bessel functions.
///
/// **Diverges for `e > 0.6627`** (the Laplace limit) no matter how many terms are used,
/// and converges slowly below it. Retained because it is a classical result and useful
/// for comparison; [`true_from_mean`] is what you want in practice.
pub fn true_from_mean_bessel(mean_anomaly: f64, eccentricity: f64, terms: usize) -> f64 {
    let mut true_anomaly = mean_anomaly;
    for k in 1..=terms {
        let order = k as i32;
        let k = k as f64;
        true_anomaly += (2.0 / k) * bessel::j_n(order, eccentricity) * f64::sin(k * mean_anomaly);
    }
    true_anomaly
}

// ---------------------------------------------------------------------------
// The one to reach for
// ---------------------------------------------------------------------------

/// True anomaly from mean anomaly, solved to [`DEFAULT_TOLERANCE`].
///
/// Dispatches on eccentricity: Halley for closed orbits, Newton on the hyperbolic form
/// for `e > 1`. Parabolic (`e == 1`) has no mean anomaly in this parameterisation and
/// returns `None`.
pub fn true_from_mean(mean_anomaly: f64, eccentricity: f64) -> Option<f64> {
    if eccentricity < 1.0 {
        let ea = eccentric_from_mean_halley(
            mean_anomaly,
            eccentricity,
            DEFAULT_TOLERANCE,
            DEFAULT_MAX_ITERATIONS,
        );
        Some(true_from_eccentric(ea, eccentricity))
    } else if eccentricity > 1.0 {
        let h = hyperbolic_from_mean_newton(
            mean_anomaly,
            eccentricity,
            DEFAULT_TOLERANCE,
            DEFAULT_MAX_ITERATIONS,
        );
        Some(true_from_hyperbolic(h, eccentricity))
    } else {
        None
    }
}

/// Mean anomaly from eccentric anomaly — Kepler's equation itself, `M = E - e sin E`.
#[inline]
pub fn mean_from_eccentric(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
    eccentric_anomaly - eccentricity * eccentric_anomaly.sin()
}

/// Mean anomaly from hyperbolic anomaly, `M = e sinh H - H`.
#[inline]
pub fn mean_from_hyperbolic(hyperbolic_anomaly: f64, eccentricity: f64) -> f64 {
    eccentricity * hyperbolic_anomaly.sinh() - hyperbolic_anomaly
}

/// `sqrt(1 - e^2)`, the factor relating the semi-minor to the semi-major axis.
/// Re-exported here because the anomaly conversions are its main consumer.
#[inline]
pub fn eccentricity_factor(eccentricity: f64) -> f64 {
    unit_circle_xy(eccentricity)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eccentricities() -> Vec<f64> {
        vec![0.0, 0.0067, 0.0167, 0.0934, 0.2056, 0.2488, 0.436, 0.6, 0.8, 0.9, 0.95, 0.99]
    }

    fn angles(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * TAU / n as f64).collect()
    }

    /// The defining property: solving Kepler's equation and substituting back must
    /// reproduce the mean anomaly.
    #[test]
    fn halley_inverts_keplers_equation() {
        for e in eccentricities() {
            for m in angles(180) {
                let ea = eccentric_from_mean_halley(m, e, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
                let back = mean_from_eccentric(ea, e);
                let err = wrap_pi(back - m).abs();
                assert!(err < 1e-11, "e={e} M={m}: residual {err:e}");
            }
        }
    }

    #[test]
    fn newton_and_halley_agree() {
        for e in eccentricities() {
            for m in angles(90) {
                let n = eccentric_from_mean_newton(m, e, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
                let h = eccentric_from_mean_halley(m, e, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
                assert!(wrap_pi(n - h).abs() < 1e-10, "e={e} M={m}: {n} vs {h}");
            }
        }
    }

    /// Halley should need very few steps. If the iteration count is capped low and the
    /// answer still holds, convergence is genuinely cubic.
    #[test]
    fn halley_converges_in_a_handful_of_iterations() {
        for e in [0.0167, 0.2056, 0.6, 0.9] {
            for m in angles(60) {
                let few = eccentric_from_mean_halley(m, e, DEFAULT_TOLERANCE, 5);
                let many = eccentric_from_mean_halley(m, e, DEFAULT_TOLERANCE, 64);
                assert!(wrap_pi(few - many).abs() < 1e-11, "e={e} M={m}: 5 iterations was not enough");
            }
        }
    }

    #[test]
    fn true_and_eccentric_anomaly_round_trip() {
        for e in eccentricities() {
            for ea in angles(90) {
                let nu = true_from_eccentric(ea, e);
                let back = eccentric_from_true(nu, e);
                assert!(wrap_pi(back - ea).abs() < 1e-10, "e={e} E={ea}");
            }
        }
    }

    /// The new half-angle form must agree with the existing `atan2` form in the
    /// `eccentric_anomaly` module.
    #[test]
    fn agrees_with_the_existing_eccentric_anomaly_conversion() {
        for e in eccentricities() {
            for nu in angles(90) {
                let a = eccentric_from_true(nu, e);
                let b = super::super::eccentric_anomaly::from_true_anomaly(e, nu);
                assert!(wrap_pi(a - b).abs() < 1e-10, "e={e} nu={nu}: {a} vs {b}");
            }
        }
    }

    /// And with `true_anomaly::at_time`, the beta-parameter form.
    #[test]
    fn agrees_with_the_existing_true_anomaly_conversion() {
        for e in eccentricities() {
            for ea in angles(90) {
                let a = true_from_eccentric(ea, e);
                let b = super::super::true_anomaly::at_time(ea, e);
                assert!(wrap_pi(a - b).abs() < 1e-9, "e={e} E={ea}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn hyperbolic_inverts_its_kepler_equation() {
        for e in [1.05, 1.5, 3.0, 10.0] {
            for m in [-50.0, -5.0, -0.5, 0.0, 0.5, 5.0, 50.0, 500.0] {
                let h = hyperbolic_from_mean_newton(m, e, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
                let back = mean_from_hyperbolic(h, e);
                let scale = m.abs().max(1.0);
                assert!((back - m).abs() / scale < 1e-10, "e={e} M={m}: got {back}");
            }
        }
    }

    #[test]
    fn true_from_mean_dispatches_on_eccentricity() {
        assert!(true_from_mean(1.0, 0.5).is_some(), "closed orbit");
        assert!(true_from_mean(1.0, 2.0).is_some(), "hyperbolic orbit");
        assert!(true_from_mean(1.0, 1.0).is_none(), "parabolic has no mean anomaly here");
    }

    /// A circular orbit has E = nu = M everywhere.
    #[test]
    fn circular_orbit_is_degenerate() {
        for m in angles(60) {
            let ea = eccentric_from_mean_halley(m, 0.0, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
            assert!(wrap_pi(ea - m).abs() < 1e-12);
            assert!(wrap_pi(true_from_eccentric(ea, 0.0) - m).abs() < 1e-12);
        }
    }

    /// Quantifies why the series are not the default. The Bessel expansion is only
    /// acceptable at small eccentricity, and is hopeless past the Laplace limit.
    #[test]
    fn series_error_grows_with_eccentricity() {
        let worst = |e: f64, f: &dyn Fn(f64, f64) -> f64| {
            angles(360)
                .into_iter()
                .map(|m| wrap_pi(f(m, e) - true_from_mean(m, e).unwrap()).abs())
                .fold(0.0, f64::max)
        };

        let bessel = |m, e| true_from_mean_bessel(m, e, 10);
        assert!(worst(0.0167, &bessel) < 0.02, "Earth should be within ~1 degree");
        assert!(worst(0.2056, &bessel) > 0.1, "Mercury should be visibly wrong");
        assert!(worst(0.8, &bessel) > 1.0, "past the Laplace limit it should be hopeless");

        let series = |m, e| true_from_mean_series(m, e);
        assert!(worst(0.0167, &series) < 1e-4, "equation of the centre is fine at low e");
        assert!(worst(0.5, &series) > 0.05, "and poor at high e");
    }

    #[test]
    fn eccentric_series_matches_the_solver_at_low_eccentricity() {
        for e in [0.0, 0.0167, 0.05] {
            for m in angles(60) {
                let s = eccentric_from_mean_series(m, e);
                let exact = eccentric_from_mean_halley(m, e, DEFAULT_TOLERANCE, DEFAULT_MAX_ITERATIONS);
                let tol = 60.0 * e.powi(4) + 1e-12;
                assert!(wrap_pi(s - exact).abs() < tol, "e={e} M={m}");
            }
        }
    }

    #[test]
    fn wrapping_helpers() {
        assert!((wrap_tau(-0.5) - (TAU - 0.5)).abs() < 1e-12);
        assert!((wrap_pi(TAU - 0.5) - -0.5).abs() < 1e-12);
        assert!((wrap_pi(PI) - PI).abs() < 1e-12);
    }
}

#[cfg(test)]
mod hyperbolic_true_tests {
    use super::*;

    /// `H -> nu -> H` must come back where it started, across the open regime.
    #[test]
    fn hyperbolic_and_true_anomaly_round_trip() {
        for &e in &[1.05, 1.5, 2.0, 5.0] {
            for &h in &[-2.0, -0.5, 0.0, 0.5, 2.0] {
                let nu = true_from_hyperbolic(h, e);
                let back = hyperbolic_from_true(nu, e).expect("inside the asymptotes");
                assert!(
                    (back - h).abs() < 1e-12,
                    "e={e} H={h}: came back {back} via nu={nu}"
                );
            }
        }
    }

    /// Mean anomaly must survive the same trip, which is what the epoch conversion needs.
    #[test]
    fn mean_anomaly_survives_the_true_anomaly_round_trip() {
        let e = 1.4;
        for &h in &[-1.5, -0.25, 0.0, 0.25, 1.5] {
            let expected = mean_from_hyperbolic(h, e);
            let nu = true_from_hyperbolic(h, e);
            let got = mean_from_hyperbolic(hyperbolic_from_true(nu, e).unwrap(), e);
            assert!((got - expected).abs() < 1e-11, "e={e} H={h}: {got} vs {expected}");
        }
    }

    /// At and beyond the asymptote there is no trajectory, and it must say so rather
    /// than returning a plausible-looking number.
    #[test]
    fn past_the_asymptote_is_none() {
        let e = 2.0;
        let asymptote = (-1.0f64 / e).acos();
        assert!(hyperbolic_from_true(asymptote, e).is_none(), "at the asymptote");
        assert!(hyperbolic_from_true(asymptote + 0.1, e).is_none(), "past it");
        assert!(hyperbolic_from_true(asymptote - 0.05, e).is_some(), "just inside it");
    }
}
