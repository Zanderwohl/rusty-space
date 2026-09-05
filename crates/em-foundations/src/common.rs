use num_traits::Pow;

pub fn unit_circle_xy(x: f64) -> f64 {
    f64::sqrt(1.0 - (x * x))
}

/// Beta parameter of the Fourier expansion of true anomaly from mean anomaly.
#[inline]
pub fn beta(e: f64) -> f64 {
    (1.0 - f64::sqrt(1.0 - (e * e))) / e
}

/// Bessel function of the first kind, `J_a(x)`, by its defining series
/// `sum_m (-1)^m / (m! Γ(m + a + 1)) · (x/2)^(2m + a)`, truncated at `iterations` terms.
///
/// `a` must be non-negative. Successive terms are built by the recurrence
/// `t_m = -t_{m-1} · (x/2)² / (m (m + a))`, so neither `m!` nor `Γ(m + a + 1)` is ever
/// formed and there is no factorial overflow ceiling — only `Γ(a + 1)` is evaluated, and
/// for integral `a` that is `a!` exactly.
///
/// The series converges for all `x`, but the term magnitude peaks near `m ≈ x/2`, so
/// cancellation eats precision unless `iterations` comfortably exceeds that. Forty terms
/// hold full double precision out to about `x = 15`.
#[inline]
pub fn bessel_j(a: f64, x: f64, iterations: f64) -> f64 {
    let iterations = iterations.round() as i64;
    if iterations <= 0 {
        return 0.0;
    }
    let half_x = x / 2.0;
    // m = 0: (x/2)^a / Γ(a + 1).
    let mut term = half_x.pow(a) / gamma_plus_one(a);
    let mut sum = term;
    let half_x_squared = half_x * half_x;
    for m in 1..iterations {
        let m = m as f64;
        term *= -half_x_squared / (m * (m + a));
        sum += term;
    }
    sum
}

/// `Γ(a + 1)`, exact via `a!` when `a` is a non-negative integer within f64's factorial
/// range — which is how [`bessel_j`] is almost always called.
///
/// Off the integers it uses Stirling with its correction series, after lifting the
/// argument into the range where that is accurate via `Γ(z) = Γ(z + k) / (z … (z+k-1))`.
/// Note it does *not* delegate to `scilib::math::basic::gamma`, which diverges badly
/// above about `n = 5` (it returns 2.92e5 for `Γ(10) = 362880`).
fn gamma_plus_one(a: f64) -> f64 {
    if (0.0..=170.0).contains(&a) && a.fract() == 0.0 {
        return factorial(a);
    }
    // Stirling's relative error falls off as ~1/(12z); lift z here, divide the shift back
    // out below.
    const LIFT_TO: f64 = 64.0;
    let mut z = a + 1.0;
    let mut divisor = 1.0;
    while z < LIFT_TO {
        divisor *= z;
        z += 1.0;
    }
    stirling_corrected(z) / divisor
}

/// Stirling with the first four terms of its correction series. The first omitted term
/// is `163879 / (209018880 z^5)`, so this is accurate to ~1e-12 relative at `z >= 64`,
/// which is the only range [`gamma_plus_one`] calls it in.
fn stirling_corrected(z: f64) -> f64 {
    let base = (std::f64::consts::TAU / z).sqrt() * (z / std::f64::consts::E).powf(z);
    let zi = 1.0 / z;
    let series = 1.0
        + zi / 12.0
        + zi * zi / 288.0
        - 139.0 * zi * zi * zi / 51840.0
        - 571.0 * zi * zi * zi * zi / 2_488_320.0;
    base * series
}

/// `n!` for non-negative integral `n`, as f64. Overflows to infinity past `n = 170`.
fn factorial(n: f64) -> f64 {
    let mut product = 1.0;
    let n = n.round() as i64;
    for factor in 1..(n + 1) {
        product *= factor as f64
    }
    product
}

/// Stirling's approximation to the gamma function.
///
/// Asymptotic: it is only good for large `n`, and is ~8% low at `n = 1`. It is not the
/// gamma used by [`bessel_j`]; for a general Γ reach for `scilib::math::basic::gamma`.
pub fn gamma(n: f64) -> f64 {
    if n == 0.0 {
        return f64::INFINITY
    }
    let term1 = 2.0 * (std::f64::consts::PI / n);
    let term2 = n * (n / std::f64::consts::E).ln();
    term1.sqrt() * term2.exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from Abramowitz & Stegun table 9.1. The previous implementation
    /// used `(m + a + 1)` in place of `Γ(m + a + 1)` and missed every one of these.
    #[test]
    fn bessel_j_matches_reference_values() {
        let cases = [
            (0.0, 0.0, 1.0),
            (0.0, 1.0, 0.765_197_686_557_966),
            (0.0, 5.0, -0.177_596_771_314_338),
            (1.0, 1.0, 0.440_050_585_744_933),
            (1.0, 5.0, -0.327_579_137_591_465),
            (2.0, 1.0, 0.114_903_484_931_900),
            (2.0, 5.0, 0.046_565_116_277_752),
        ];
        for (a, x, expected) in cases {
            let got = bessel_j(a, x, 40.0);
            assert!((got - expected).abs() < 1e-12, "J_{a}({x}) = {got}, expected {expected}");
        }
    }

    /// `J_0` and `J_1` at the first zero of `J_0`, and the identity `J_1 = -J_0'`.
    #[test]
    fn bessel_j_zero_and_derivative_identity() {
        const FIRST_ZERO_J0: f64 = 2.404_825_557_695_773;
        assert!(bessel_j(0.0, FIRST_ZERO_J0, 40.0).abs() < 1e-12);

        let h = 1e-5;
        for x in [0.5, 1.0, 3.0, 6.0] {
            let numeric = -(bessel_j(0.0, x + h, 40.0) - bessel_j(0.0, x - h, 40.0)) / (2.0 * h);
            let analytic = bessel_j(1.0, x, 40.0);
            assert!((numeric - analytic).abs() < 1e-8, "at x={x}: {numeric} vs {analytic}");
        }
    }

    /// The public [`gamma`] is bare Stirling, whose relative error falls off as ~1/(12n):
    /// 8% low at n = 1, but under 1% by n = 10. It is documented as approximate, and this
    /// pins the shape of that approximation rather than a particular value.
    #[test]
    fn stirling_gamma_is_only_asymptotic() {
        assert!((gamma(1.0) / 1.0 - 1.0).abs() > 0.05, "8% low at n = 1");
        assert!((gamma(10.0) / 362_880.0 - 1.0).abs() < 0.01, "under 1% by n = 10");
        assert!(gamma(0.0).is_infinite());
    }

    /// Fractional order, where `Γ(a + 1)` is not a factorial. `J_{1/2}` has the closed
    /// form `sqrt(2 / (πx)) · sin x`, so it checks the gamma path end to end.
    #[test]
    fn bessel_j_at_half_integer_order_matches_its_closed_form() {
        for x in [0.5, 1.0, 3.0, 7.5] {
            let closed = (2.0 / (std::f64::consts::PI * x)).sqrt() * x.sin();
            let got = bessel_j(0.5, x, 60.0);
            assert!((got - closed).abs() < 1e-12, "J_0.5({x}) = {got}, expected {closed}");
        }
    }

    /// `Γ(1/2) = √π`, reached through the lift-and-correct path.
    #[test]
    fn gamma_plus_one_is_accurate_off_the_integers() {
        let got = gamma_plus_one(-0.5);
        let root_pi = std::f64::consts::PI.sqrt();
        assert!((got / root_pi - 1.0).abs() < 1e-11, "Γ(1/2) = {got}, want {root_pi}");
        // And still exact on them.
        assert_eq!(gamma_plus_one(5.0), 120.0);
    }
}
