/// Mean anomaly calculations.
///
/// All angle parameters and return values are in **radians**.
pub mod mean_anomaly {

    /// Calculate mean anomaly at the given time.
    ///
    /// # Parameters
    /// - `mean_anomaly_at_epoch`: Mean anomaly at epoch time, in **radians**
    /// - `gravitational_parameter`: μ = G × M, in m³/s²
    /// - `semi_major_axis`: in meters
    /// - `epoch_time`: epoch time in seconds since J2000
    /// - `current_time`: current time in seconds since J2000
    ///
    /// # Returns
    /// Mean anomaly at `current_time` in **radians**
    pub fn definition(mean_anomaly_at_epoch: f64,
                      gravitational_parameter: f64,
                      semi_major_axis: f64,
                      epoch_time: f64,
                      current_time: f64) -> f64 {
        let x = gravitational_parameter / (semi_major_axis * semi_major_axis * semi_major_axis);
        mean_anomaly_at_epoch + f64::sqrt(x) * (current_time - epoch_time)
    }

    /// Kepler's equation: M = E - e × sin(E)
    ///
    /// # Parameters
    /// - `eccentric_anomaly`: E in **radians**
    /// - `eccentricity`: dimensionless
    ///
    /// # Returns
    /// Mean anomaly M in **radians**
    pub fn kepler(eccentric_anomaly: f64, eccentricity: f64) -> f64 {
        eccentric_anomaly - eccentricity * f64::sin(eccentric_anomaly)
    }
}

pub mod angular_motion {
    pub fn mean(gravitational_parameter: f64, semi_major_axis: f64) -> f64 {
        f64::sqrt(gravitational_parameter / (semi_major_axis * semi_major_axis * semi_major_axis))
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
        pub fn from_elements1(focal_parameter: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            let numerator = focal_parameter * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            numerator / denominator
        }

        pub fn from_elements2(semi_major_axis: f64, eccentricity: f64, true_anomaly: f64) -> Option<f64> {
            let numerator = 1.0 - eccentricity * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            if denominator == 0.0 {
                None
            } else {
                Some(semi_major_axis * (numerator / denominator))
            }
        }

        pub fn from_elements2_infallible(semi_major_axis: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            from_elements2(semi_major_axis, eccentricity, true_anomaly).unwrap_or(f64::INFINITY)
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

pub mod semi_major_axis {
    use crate::common;
    use crate::kepler::third_law;

    pub fn third_law(gravitational_parameter: f64, period: f64) -> f64 {
        let x= (period * period * gravitational_parameter) / third_law::FOUR_PI_SQUARED;
        f64::powf(x, 1.0 / 3.0)
    }

    /// `a = b / sqrt(1 - e^2)`, the inverse of [`super::semi_minor_axis::conic_definition`].
    pub fn conic_definition1(semi_minor_axis: f64, eccentricity: f64) -> f64 {
        semi_minor_axis / common::unit_circle_xy(eccentricity)
    }

    pub fn conic_definition2(eccentricity: f64, semi_latus_rectum: f64) -> f64 {
        semi_latus_rectum / (1.0 - eccentricity * eccentricity)
    }

    pub fn conic_definition3(focal_parameter: f64, eccentricity: f64) -> f64 {
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
    /// The semi-parameter (a.k.a. semi-latus rectum) `p = a(1 - e^2)`.
    ///
    /// This is the same quantity as [`super::semi_latus_rectum::conic_definition`];
    /// both names appear in the literature. It is NOT `a*sqrt(1 - e^2)`, which is the
    /// semi-minor axis (see [`super::semi_minor_axis::conic_definition`]).
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

    /// `E = atan2(sqrt(1 - e^2) sin v, e + cos v)`, in radians.
    ///
    /// Uses `atan2` rather than `atan` of the ratio so the result lands in the correct
    /// quadrant across the whole orbit.
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
        let numerator = beta * f64::sin(eccentric_anomaly);
        let denominator = 1.0 - beta * f64::cos(eccentric_anomaly);
        eccentric_anomaly + 2.0 * f64::atan(numerator / denominator)
    }

    pub fn from_state_vectors(local_position: DVec3, local_velocity: DVec3, eccentricity_vector: DVec3) -> f64 {
        // TODO: I don't think this works for circular orbits.
        // or circular orbits with zero inclination?
        let numerator = eccentricity_vector.dot(local_position);
        let denominator = eccentricity_vector.length() * local_position.length();
        let answer = f64::acos(numerator / denominator);
        if local_position.dot(local_velocity) < 0.0 {
            return (2.0 * std::f64::consts::PI) - answer;
        }
        answer
    }

    /// The equation of the centre, expanded to `e^3`:
    ///
    /// `v ~= M + (2e - e^3/4) sin M + (5/4)e^2 sin 2M + (13/12)e^3 sin 3M`
    ///
    /// Truncated series: accurate only for small `e`. Prefer solving Kepler's equation.
    pub fn from_mean_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
        let e = eccentricity;
        let first_term = mean_anomaly;
        let second_term = (2.0 * e - (1.0 / 4.0) * e * e * e) * f64::sin(mean_anomaly);
        let third_term = (5.0 / 4.0) * e * e * f64::sin(2.0 * mean_anomaly);
        let fourth_term = (13.0 / 12.0) * e * e * e * f64::sin(3.0 * mean_anomaly);
        first_term + second_term + third_term + fourth_term
    }

    pub fn fourier_expansion(mean_anomaly: f64, eccentricity: f64, iterations: usize) -> f64 {
        let mut true_anomaly = mean_anomaly;

        for k in 1..=iterations {
            let order = k  as i32;
            let k: f64 = k as f64;
            let term = (2.0 / k) * bessel::j_n(order, eccentricity) * f64::sin(k * mean_anomaly);
            true_anomaly += term;
        }

        true_anomaly
    }
}

pub mod apsides {
    pub mod periapsis {
        /// `r_p = p / (1 + e)` where `p = focal_parameter * e` is the semi-latus rectum.
        /// Equals `a(1 - e)`.
        pub fn definition(focal_parameter: f64, eccentricity: f64) -> f64 {
            (focal_parameter * eccentricity) / (1.0 + eccentricity)
        }

        pub fn from_parameters(semi_major_axis: f64, eccentricity: f64) -> f64 {
            semi_major_axis * (1.0 - eccentricity)
        }
    }

    pub mod apoapsis {
        /// `r_a = p / (1 - e)` where `p = focal_parameter * e` is the semi-latus rectum.
        /// Equals `a(1 + e)`. Diverges as `e -> 1`.
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
    /// Equivalent to [`super::eccentricity::vector::definition`]; both are kept because
    /// both names are in use. They must agree (asserted in tests).
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
        /// Note [`super::potential::specific`] already returns a negative value, so these
        /// are *added*.
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
