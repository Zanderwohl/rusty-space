//! Proper time, and motion under constant proper acceleration.
//!
//! Onboard processes advance on `tau = integral dt / gamma`; the player's clock is always
//! coordinate time. Units are natural, so `alpha` is in inverse microseconds.

/// SI m/s^2 to inverse microseconds.
#[inline]
pub fn proper_acceleration_from_si(a_m_per_s2: f64) -> f64 {
    // a / c gives inverse seconds; divide by 1e6 microseconds per second.
    a_m_per_s2 / 299_792_458.0 / 1e6
}

/// Standard gravity, 9.80665 m/s^2, in natural units.
pub const ONE_GEE: f64 = 9.80665 / 299_792_458.0 / 1e6;

/// Lorentz factor from a scalar `beta`.
#[inline]
pub fn gamma_from_beta(beta: f64) -> f64 {
    debug_assert!(beta.abs() < 1.0);
    (1.0 - beta * beta).sqrt().recip()
}

/// The largest `f64` strictly below 1.
///
/// `at / sqrt(1 + at^2)` rounds to exactly 1.0 past `at` of about 1e8, because `1 + at^2`
/// loses the 1, and that makes `gamma` infinite. Saturating here keeps it finite.
pub const MAX_BETA: f64 = 1.0 - f64::EPSILON / 2.0;

/// Velocity from rest after coordinate time `t`. Asymptotes to `c`.
#[inline]
pub fn hyperbolic_velocity(alpha: f64, t: f64) -> f64 {
    let at = alpha * t;
    (at / (1.0 + at * at).sqrt()).clamp(-MAX_BETA, MAX_BETA)
}

/// Displacement from rest after coordinate time `t`.
#[inline]
pub fn hyperbolic_position(alpha: f64, t: f64) -> f64 {
    let at = alpha * t;
    // sqrt(1 + u^2) - 1 loses nine digits to cancellation for small u, and u is small for
    // most of any realistic burn. The identical u^2 / (sqrt(1 + u^2) + 1) does not.
    alpha * t * t / ((1.0 + at * at).sqrt() + 1.0)
}

/// Proper time elapsed aboard over coordinate time `t`, from rest. Always less than `t`.
#[inline]
pub fn hyperbolic_proper_time(alpha: f64, t: f64) -> f64 {
    (alpha * t).asinh() / alpha
}

/// Proper time elapsed over a coasting leg at constant `beta`.
#[inline]
pub fn coasting_proper_time(beta: f64, dt: f64) -> f64 {
    dt / gamma_from_beta(beta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperbolic_motion_reduces_to_newtonian_at_low_speed() {
        let alpha = ONE_GEE;
        let t = 1e9; // 1000 s, still deeply non-relativistic
        let newtonian = 0.5 * alpha * t * t;
        assert!((hyperbolic_position(alpha, t) - newtonian).abs() / newtonian < 1e-9);
        assert!((hyperbolic_velocity(alpha, t) - alpha * t).abs() / (alpha * t) < 1e-9);
        assert!((hyperbolic_proper_time(alpha, t) - t).abs() / t < 1e-9);
    }

    #[test]
    fn velocity_never_reaches_c() {
        // 1.15e18 us is the world horizon; the rest are past it, and must still behave.
        for t in [1e6, 1e12, 1.15e18, 1e24, 1e30] {
            let v = hyperbolic_velocity(ONE_GEE, t);
            assert!(v < 1.0, "beta {v} at t {t}");
            assert!(v > 0.0);
            assert!(gamma_from_beta(v).is_finite(), "gamma must stay finite at t {t}");
        }
    }

    #[test]
    fn position_is_stable_where_the_naive_form_cancels() {
        // At 1e9 us under one gee the relativistic correction to the Newtonian answer is
        // about (alpha t)^2 / 4 = 2.7e-10. The naive form's cancellation error is larger than
        // the physics it is supposed to be showing, which is the whole problem.
        let (alpha, t) = (ONE_GEE, 1e9);
        let newtonian = 0.5 * alpha * t * t;
        let naive = ((1.0 + (alpha * t).powi(2)).sqrt() - 1.0) / alpha;
        let stable = hyperbolic_position(alpha, t);

        let naive_err = (naive - newtonian).abs() / newtonian;
        let stable_err = (stable - newtonian).abs() / newtonian;

        assert!(stable_err < 1e-9, "stable form should sit on the real correction, got {stable_err}");
        assert!(naive_err > 1e-8, "naive form should be visibly wrong, got {naive_err}");
        assert!(naive_err > 100.0 * stable_err);
    }

    #[test]
    fn one_gee_reaches_half_c_in_about_204_days() {
        // beta = 0.5 needs alpha*t = 0.5/sqrt(0.75).
        let alpha = ONE_GEE;
        let at = 0.5 / 0.75f64.sqrt();
        let t_micros = at / alpha;
        let days = t_micros / 1e6 / 86_400.0;
        assert!((days - 204.3).abs() < 0.5, "got {days} days");
        assert!((hyperbolic_velocity(alpha, t_micros) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn proper_time_always_runs_slow() {
        let alpha = ONE_GEE;
        for t in [1e9, 1e12, 1e14, 1e15] {
            let tau = hyperbolic_proper_time(alpha, t);
            assert!(tau <= t, "proper time {tau} exceeded coordinate time {t}");
        }
        // And measurably so once relativistic: a year at 1g.
        let year = 3.15576e7 * 1e6;
        let tau = hyperbolic_proper_time(alpha, year);
        assert!(tau / year < 0.95, "ratio {}", tau / year);
    }

    #[test]
    fn coasting_at_gamma_two_halves_the_onboard_clock() {
        let beta = (3.0f64 / 4.0).sqrt(); // gamma = 2
        assert!((gamma_from_beta(beta) - 2.0).abs() < 1e-12);
        assert!((coasting_proper_time(beta, 100.0) - 50.0).abs() < 1e-12);
    }

    #[test]
    fn one_gee_is_what_it_says() {
        assert_eq!(ONE_GEE, proper_acceleration_from_si(9.80665));
    }
}
