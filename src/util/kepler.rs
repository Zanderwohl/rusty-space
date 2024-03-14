pub mod mean_anomaly {

    pub fn definition(mean_anomaly_at_epoch: f64,
                      gravitational_parameter: f64,
                      semi_major_axis: f64,
                      epoch_time: f64,
                      current_time: f64) -> f64 {
        let x = gravitational_parameter / (semi_major_axis * semi_major_axis * semi_major_axis);
        mean_anomaly_at_epoch + f64::sqrt(x) * (current_time - epoch_time)
    }

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

    pub mod distance {
        pub fn from_elements1(focal_parameter: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            let numerator = focal_parameter * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            numerator / denominator
        }

        pub fn from_elements2(semi_major_axis: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
            let numerator = 1.0 - eccentricity * eccentricity;
            let denominator = 1.0 + eccentricity * f64::cos(true_anomaly);
            semi_major_axis * (numerator / denominator)
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
    use crate::util::common;
    use crate::util::kepler::third_law;

    pub fn third_law(gravitational_parameter: f64, period: f64) -> f64 {
        let x= (period * period * gravitational_parameter) / third_law::FOUR_PI_SQUARED;
        f64::powf(x, 1.0 / 3.0)
    }

    pub fn conic_definition1(semi_minor_axis: f64, eccentricity: f64) -> f64 {
        common::unit_circle_xy(eccentricity) / semi_minor_axis
    }

    pub fn conic_definition2(eccentricity: f64, semi_latus_rectum: f64) -> f64 {
        semi_latus_rectum / (1.0 - eccentricity * eccentricity)
    }
}

pub mod semi_latus_rectum {
    pub fn conic_definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        semi_major_axis * (1.0 - eccentricity * eccentricity)
    }
}

pub mod semi_minor_axis {
    use crate::util::common;
    pub fn conic_definition(semi_major_axis: f64, eccentricity: f64) -> f64 {
        semi_major_axis * common::unit_circle_xy(eccentricity)
    }
}

pub mod eccentricity {
    use crate::util::common;

    pub fn from_axes(semi_major_axis: f64, semi_minor_axis: f64) -> f64 {
        common::unit_circle_xy(semi_minor_axis / semi_major_axis)
    }

    pub fn conic_definition(semi_major_axis: f64, semi_latus_rectum: f64) -> f64 {
        f64::sqrt(1.0 - semi_latus_rectum / semi_major_axis)
    }

    pub mod vector {
        use glam::DVec3;
        use crate::util::kepler::local;

        pub fn definition(local_position: DVec3, local_velocity: DVec3, gravitational_parameter: f64) -> DVec3 {
            let term1 = local_velocity.cross(local::angular_momentum::specific(local_position, local_velocity)) / gravitational_parameter;
            let term2 = local_position.normalize();
            term1 - term2
        }
    }
}

pub mod eccentric_anomaly {
    use crate::util::common::unit_circle_xy;

    pub fn from_true_anomaly(eccentricity: f64, true_anomaly: f64) -> f64 {
        let numerator = unit_circle_xy(eccentricity) * f64::sin(true_anomaly);
        let denominator = eccentricity + f64::cos(true_anomaly);
        let fraction = numerator / denominator;
        f64::atan(fraction)
    }
}

pub mod true_anomaly {
    use glam::DVec3;
    use crate::util::common::unit_circle_xy;

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

    /// This is the Fourier expansion up to e^3
    pub fn from_mean_anomaly(mean_anomaly: f64, eccentricity: f64) -> f64 {
        let first_term = mean_anomaly;
        let second_term = (2.0 * eccentricity - (1.0 / 4.0) * eccentricity * eccentricity * eccentricity) * f64::sin(mean_anomaly);
        let third_term = (5.0 / 4.0) * eccentricity * eccentricity * f64::sin(2.0 * mean_anomaly);
        let fourth_term = (13.0 / 12.0) * eccentricity * eccentricity * eccentricity * f64::sin(3.0 * mean_anomaly);
        first_term + second_term + third_term + fourth_term
    }
}

pub mod period {
    use crate::util::kepler::third_law::reused_term;
    pub fn third_law(semi_major_axis: f64, gravitational_parameter: f64) -> f64 {
        let x = reused_term(semi_major_axis) / gravitational_parameter;
        f64::sqrt(x)
    }
}

pub mod gravitational_parameter {
    use crate::util::kepler::third_law::reused_term;

    pub fn third_law(period: f64, semi_major_axis: f64) -> f64 {
        reused_term(semi_major_axis) / (period * period)
    }
}
