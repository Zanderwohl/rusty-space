//! Keplerian orbital mechanics. **Every angle is in radians**, without exception.

pub mod anomaly;
pub mod state;

/// Mean anomaly, in **radians**.
pub mod mean_anomaly {

    /// Mean anomaly at `current_time`, propagated from the epoch value.
    ///
    /// Angles in radians, μ in m³/s², `a` in metres, both times in seconds since J2000.
    pub fn definition(mean_anomaly_at_epoch: f64,
                      gravitational_parameter: f64,
                      semi_major_axis: f64,
                      epoch_time: f64,
                      current_time: f64) -> f64 {
        let x = gravitational_parameter / (semi_major_axis * semi_major_axis * semi_major_axis);
        mean_anomaly_at_epoch + f64::sqrt(x) * (current_time - epoch_time)
    }

    /// Kepler's equation, `M = E - e sin E`.
    pub fn kepler(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
        eccentric_anomaly - eccentricity * f64::sin(eccentric_anomaly)
    }
}

pub mod angular_motion {
    /// Rate the mean anomaly advances, rad/s: `n = sqrt(mu / |a|^3)`.
    ///
    /// The magnitude of `a` is deliberate. A hyperbolic orbit has a negative semi-major
    /// axis, and `sqrt(mu / a^3)` on it is NaN — which would then propagate silently
    /// through the anomaly to the position, even though the solver in
    /// [`anomaly::true_from_mean`](super::anomaly::true_from_mean) handles `e > 1`
    /// perfectly well. Every capture into a sphere of influence is a hyperbola, so this is
    /// the common case, not the exotic one.
    pub fn mean(gravitational_parameter: f64, semi_major_axis: f64) -> f64 {
        let a = semi_major_axis.abs();
        f64::sqrt(gravitational_parameter / (a * a * a))
    }
}


pub mod local {
    pub mod angular_momentum {
        use glam::DVec3;

        pub fn specific(displacement: DVec3, velocity: DVec3) -> DVec3 {
            displacement.cross(velocity)
        }
    }

    pub mod radius {
        pub fn from_focal_parameter(focal_parameter: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            let numerator = focal_parameter * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            numerator / denominator
        }

        pub fn from_semi_major_axis(semi_major_axis: f64, eccentricity: f64, true_anomaly: f64) -> Option<f64> {
            let numerator = 1.0 - eccentricity * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            if denominator == 0.0 {
                None
            } else {
                Some(semi_major_axis * (numerator / denominator))
            }
        }

        pub fn from_semi_major_axis_infallible(semi_major_axis: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            from_semi_major_axis(semi_major_axis, eccentricity, true_anomaly).unwrap_or(f64::INFINITY)
        }

        pub fn from_eccentric_anomaly(semi_major_axis: f64, eccentricity: f64, eccentric_anomaly: f64) -> f64 {
            semi_major_axis * (1.0 - eccentricity * f64::cos(eccentric_anomaly))
        }
    }
}

mod third_law {
    pub(crate) const FOUR_PI_SQUARED: f64 = 4.0 * std::f64::consts::PI * std::f64::consts::PI;

    pub(crate) fn reused_term(semi_major_axis: f64) -> f64 {
        FOUR_PI_SQUARED * semi_major_axis * semi_major_axis * semi_major_axis
    }
}

/// Semi-major axis, from whichever pair of quantities is to hand.
///
/// Named for their inputs: the previous `conic_definition1/2/3` names let an inverted
/// form (`sqrt(1-e²)/b` for `b/sqrt(1-e²)`) be picked at a call site unnoticed.
pub mod semi_major_axis {
    use crate::common;
    use crate::kepler::third_law;

    pub fn third_law(gravitational_parameter: f64, period: f64) -> f64 {
        let x = (period * period * gravitational_parameter) / third_law::FOUR_PI_SQUARED;
        x.cbrt()
    }

    /// `a = b / sqrt(1 - e^2)`, the inverse of [`super::semi_minor_axis::conic_definition`].
    pub fn from_semi_minor_and_eccentricity(semi_minor_axis: f64, eccentricity: f64) -> f64 {
        semi_minor_axis / common::unit_circle_xy(eccentricity)
    }

    pub fn from_eccentricity_and_semi_latus_rectum(eccentricity: f64, semi_latus_rectum: f64) -> f64 {
        semi_latus_rectum / (1.0 - eccentricity * eccentricity)
    }

    pub fn from_focal_parameter_and_eccentricity(focal_parameter: f64, eccentricity: f64) -> f64 {
        (focal_parameter * eccentricity) / (1.0 - eccentricity * eccentricity)
    }

    pub fn radii(periapsis: f64, apoapsis: f64) -> f64 {
        (periapsis + apoapsis) / 2.0
    }
}

pub mod semi_latus_rectum {
    pub fn conic_definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        if eccentricity == 1.0 {
            return 2.0 * semi_major_axis
        }
        semi_major_axis * (1.0 - eccentricity * eccentricity)
    }
}

pub mod semi_minor_axis {
    use crate::common;
    pub fn conic_definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        semi_major_axis * common::unit_circle_xy(eccentricity)
    }
}

pub mod eccentricity {
    use crate::common;

    pub fn from_axes(semi_major_axis: f64, semi_minor_axis: f64) -> f64 {
        common::unit_circle_xy(semi_minor_axis / semi_major_axis)
    }

    pub fn conic_definition(semi_major_axis: f64, semi_latus_rectum: f64) -> f64 {
        f64::sqrt(1.0 - semi_latus_rectum / semi_major_axis)
    }

    pub fn radii(periapsis: f64, apoapsis: f64) -> f64 {
        (apoapsis - periapsis) / (apoapsis + periapsis)
    }

    pub mod vector {
        use glam::DVec3;
        use crate::kepler::local;

        pub fn definition(local_position: DVec3, local_velocity: DVec3, gravitational_parameter: f64) -> DVec3 {
            let term1 = local_velocity.cross(local::angular_momentum::specific(local_position, local_velocity)) / gravitational_parameter;
            let term2 = local_position.normalize();
            term1 - term2
        }
    }
}

pub mod semi_parameter {
    /// Semi-parameter (semi-latus rectum) `p = a(1 - e^2)`.
    ///
    /// Same quantity as [`super::semi_latus_rectum::conic_definition`]. NOT
    /// `a*sqrt(1 - e^2)`, which is the semi-minor axis.
    pub fn definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        semi_major_axis * (1.0 - eccentricity * eccentricity)
    }
}

pub mod periapsis {
    use crate::kepler::semi_parameter;

    pub fn definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        semi_parameter::definition(semi_major_axis, eccentricity) / (1.0 + eccentricity)
    }
}

pub mod apoapsis {
    use crate::kepler::semi_parameter;

    pub fn definition(semi_major_axis: f64, eccentricity: f64) -> Option<f64> {
        if eccentricity >= 1.0 { return None; }
        Some(semi_parameter::definition(semi_major_axis, eccentricity) / (1.0 - eccentricity))
    }
}

pub mod eccentric_anomaly {
    use crate::common::unit_circle_xy;

    /// `E = atan2(sqrt(1 - e^2) sin v, e + cos v)`, radians.
    ///
    /// `atan2` rather than `atan` of the ratio, for the correct quadrant across the orbit.
    pub fn from_true_anomaly(eccentricity: f64, true_anomaly: f64) -> f64 {
        let numerator = unit_circle_xy(eccentricity) * f64::sin(true_anomaly);
        let denominator = eccentricity + f64::cos(true_anomaly);
        f64::atan2(numerator, denominator)
    }
}

pub mod true_anomaly {
    use glam::DVec3;
    use crate::common::{unit_circle_xy};
    use scilib::math::bessel;

    pub fn at_time(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
        let beta = eccentricity / (1.0 + unit_circle_xy(eccentricity));
        let (sin_ea, cos_ea) = eccentric_anomaly.sin_cos();
        let numerator = beta * sin_ea;
        let denominator = 1.0 - beta * cos_ea;
        eccentric_anomaly + 2.0 * f64::atan(numerator / denominator)
    }

    /// True anomaly from a state vector, radians on `[0, tau)`.
    ///
    /// Measured from periapsis, which a circular orbit lacks; substitutes, matching the
    /// conventions and tolerances of [`super::state::from_state`]:
    ///
    /// - **Circular, inclined**: argument of latitude, from the ascending node.
    /// - **Circular, equatorial**: true longitude, from +X.
    ///
    /// `acos` arguments are clamped: near-parallel vectors give ratios a few ulps
    /// outside `[-1, 1]`, hence NaN.
    pub fn from_state_vectors(local_position: DVec3, local_velocity: DVec3, eccentricity_vector: DVec3) -> f64 {
        use crate::kepler::anomaly::wrap_tau;
        use crate::kepler::state::{CIRCULAR_TOLERANCE, EQUATORIAL_TOLERANCE};

        let r = local_position.length();
        let e = eccentricity_vector.length();
        if r == 0.0 {
            return 0.0;
        }

        if e >= CIRCULAR_TOLERANCE {
            let mut nu = (eccentricity_vector.dot(local_position) / (e * r))
                .clamp(-1.0, 1.0)
                .acos();
            // Outbound over the first half of the orbit; past apoapsis `r` falls.
            if local_position.dot(local_velocity) < 0.0 {
                nu = -nu;
            }
            return wrap_tau(nu);
        }

        let h = local_position.cross(local_velocity);
        let h_len = h.length();
        if h_len == 0.0 {
            // Radial: no orbital plane, so no angle within one.
            return 0.0;
        }
        let inclination = (h.z / h_len).clamp(-1.0, 1.0).acos();
        let equatorial = inclination < EQUATORIAL_TOLERANCE
            || (std::f64::consts::PI - inclination) < EQUATORIAL_TOLERANCE;

        if equatorial {
            // True longitude from +X; reversed for a retrograde orbit.
            let mut lon = (local_position.x / r).clamp(-1.0, 1.0).acos();
            if local_position.y < 0.0 {
                lon = -lon;
            }
            if h.z < 0.0 {
                lon = -lon;
            }
            wrap_tau(lon)
        } else {
            // Argument of latitude, from the ascending node z_hat x h.
            let node = DVec3::new(-h.y, h.x, 0.0);
            let mut u = (node.dot(local_position) / (node.length() * r))
                .clamp(-1.0, 1.0)
                .acos();
            if local_position.z < 0.0 {
                u = -u;
            }
            wrap_tau(u)
        }
    }

    /// Equation of the centre to `e^3`:
    ///
    /// `v ~= M + (2e - e^3/4) sin M + (5/4)e^2 sin 2M + (13/12)e^3 sin 3M`
    ///
    /// Truncated: accurate only for small `e`. Prefer solving Kepler's equation.
    pub fn from_mean_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
        let e = eccentricity;
        let first_term = mean_anomaly;
        let second_term = (2.0 * e - (1.0 / 4.0) * e * e * e) * f64::sin(mean_anomaly);
        let third_term = (5.0 / 4.0) * e * e * f64::sin(2.0 * mean_anomaly);
        let fourth_term = (13.0 / 12.0) * e * e * e * f64::sin(3.0 * mean_anomaly);
        first_term + second_term + third_term + fourth_term
    }

    pub fn fourier_expansion(mean_anomaly: f64, eccentricity: f64, iterations: usize) -> f64 {
        let mut true_anomaly = mean_anomaly + eccentricity * mean_anomaly.sin(); // Mikkola's seed

        for k in 1..=iterations {
            let order = k  as i32;
            let k: f64 = k as f64;
            let term = (2.0 / k) * bessel::j_n(order, eccentricity) * f64::sin(k * mean_anomaly);
            true_anomaly += term;
        }

        true_anomaly
    }

    /// Coefficients `c_k = (2/k) J_k(e)` for `k = 1..=iterations`, for
    /// [`fourier_expansion_with_precompute`] where eccentricity is loop-constant.
    pub fn precompute_coefficients(eccentricity: f64, iterations: usize) -> Vec<f64> {
        (1..=iterations)
            .map(|k| {
                let k_f64 = k as f64;
                (2.0 / k_f64) * bessel::j_n(k as i32, eccentricity)
            })
            .collect()
    }

    /// True anomaly from mean anomaly, using coefficients from
    /// [`precompute_coefficients`]. `eccentricity` is still needed for Mikkola's seed.
    #[inline]
    pub fn fourier_expansion_with_precompute(mean_anomaly: f64, eccentricity: f64, coefficients: &[f64]) -> f64 {
        let mut true_anomaly = mean_anomaly + eccentricity * mean_anomaly.sin(); // Mikkola's seed

        for (idx, &coeff) in coefficients.iter().enumerate() {
            let k = (idx + 1) as f64;
            true_anomaly += coeff * f64::sin(k * mean_anomaly);
        }

        true_anomaly
    }
}

pub mod apsides {
    pub mod periapsis {
        /// `r_p = p / (1 + e)`, `p = focal_parameter * e`. Equals `a(1 - e)`.
        pub fn definition(focal_parameter: f64, eccentricity: f64) -> f64 {
            (focal_parameter * eccentricity) / (1.0 + eccentricity)
        }

        pub fn from_parameters(semi_major_axis: f64, eccentricity: f64) -> f64 {
            semi_major_axis * (1.0 - eccentricity)
        }
    }

    pub mod apoapsis {
        /// `r_a = p / (1 - e)`, `p = focal_parameter * e`. Equals `a(1 + e)`; diverges as `e -> 1`.
        pub fn definition(focal_parameter: f64, eccentricity: f64) -> f64 {
            (focal_parameter * eccentricity) / (1.0 - eccentricity)
        }

        pub fn from_parameters(semi_major_axis: f64, eccentricity: f64) -> f64 {
            semi_major_axis * (1.0 + eccentricity)
        }
    }
}

pub mod period {
    use crate::kepler::third_law::reused_term;
    pub fn third_law(semi_major_axis: f64, gravitational_parameter: f64) -> f64 {
        let x = reused_term(semi_major_axis) / gravitational_parameter;
        x.sqrt()
    }
}

pub mod gravitational_parameter {
    use crate::kepler::third_law::reused_term;

    pub fn third_law(period: f64, semi_major_axis: f64) -> f64 {
        reused_term(semi_major_axis) / (period * period)
    }
}

pub mod eccentricity_vector {
    use glam::DVec3;

    /// `e_vec = (v x h) / mu - r_hat`, where `h = r x v`.
    ///
    /// Must agree with [`super::eccentricity::vector::definition`] (asserted in tests).
    pub fn definition(mu: f64, displacement: DVec3, velocity: DVec3) -> DVec3 {
        let specific_angular_momentum = displacement.cross(velocity);
        velocity.cross(specific_angular_momentum) / mu - displacement.normalize()
    }
}

pub mod energy {
    pub mod mechanical {
        use crate::kepler::energy::{kinetic, potential};

        /// Specific orbital energy `eps = v^2/2 - mu/r`.
        ///
        /// [`super::potential::specific`] is already negative, hence the addition.
        pub fn specific(velocity: f64, mu: f64, displacement: f64) -> f64 {
            kinetic::specific(velocity) + potential::specific(mu, displacement)
        }

        pub fn definition(mass: f64, velocity: f64, mu: f64, displacement: f64) -> f64 {
            mass * specific(velocity, mu, displacement)
        }
    }

    pub mod kinetic {
        pub fn specific(velocity: f64) -> f64 {
            (velocity * velocity) / 2.0
        }

        pub fn definition(mass: f64, velocity: f64) -> f64 {
            mass * specific(velocity)
        }
    }

    pub mod potential {
        pub fn specific(mu: f64, displacement: f64) -> f64 {
            -(mu / displacement)
        }

        pub fn definition(mass: f64, mu: f64, displacement: f64) -> f64 {
            mass * specific(mu, displacement)
        }
    }
}

#[cfg(test)]
mod hyperbolic_tests {
    use super::*;

    /// A flyby is a hyperbola, and it has to advance rather than turn into NaN.
    #[test]
    fn mean_motion_is_finite_on_a_hyperbola() {
        const MU: f64 = 4.9028695e12; // Luna
        let n = angular_motion::mean(MU, -6.75e6);
        assert!(n.is_finite() && n > 0.0, "hyperbolic mean motion was {n}");

        // Same magnitude of `a` gives the same rate either side of the parabolic limit.
        let closed = angular_motion::mean(MU, 6.75e6);
        assert!((n - closed).abs() < 1e-12 * closed);
    }

    /// And it must carry all the way to a true anomaly, which is where the NaN showed up.
    #[test]
    fn a_hyperbolic_arc_advances_to_a_real_anomaly() {
        const MU: f64 = 4.9028695e12;
        let (a, e) = (-6.75e6, 1.7879);
        let n = angular_motion::mean(MU, a);
        let mean = n * 3600.0; // an hour past periapsis
        let true_anomaly = anomaly::true_from_mean(mean, e).expect("e > 1 is solvable");
        assert!(true_anomaly.is_finite(), "true anomaly was {true_anomaly}");

        let radius = local::radius::from_semi_major_axis(a, e, true_anomaly)
            .expect("a hyperbola has a radius");
        assert!(radius.is_finite() && radius > 0.0, "radius was {radius}");
    }
}
