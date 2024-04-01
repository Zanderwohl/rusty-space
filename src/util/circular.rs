pub mod period {
    pub fn definition(radius: f64, mu: f64) -> f64 {
        let frac = (radius * radius * radius) / mu;
        2.0 * std::f64::consts::PI * f64::sqrt(frac)
    }
}

pub mod radius {
    pub fn from_period(period: f64, mu: f64) -> f64 {
        let a = period / (2.0 * std::f64::consts::PI);
        let a_sq = a * a;
        let b = a_sq * mu;
        f64::powf(b, 1.0 / 3.0)
    }
}

pub mod mu {
    pub fn from_params(period: f64, radius: f64) -> f64 {
        let a = period / (2.0 * std::f64::consts::PI);
        (radius * radius * radius) / (a * a)
    }
}

pub mod true_anomaly {
    use crate::util::circular::period;

    pub fn at_time(time: f64, radius: f64, mu: f64) -> f64 {
        let period = period::definition(radius, mu);
        let time = time % period;
        time / period
    }
}

pub mod position {
    use glam::DVec3;

    pub fn from_true_anomaly(radius: f64, true_anomaly: f64) -> DVec3 {
        DVec3::new(f64::sin(true_anomaly) * radius, 0.0, f64::cos(true_anomaly) * radius)
    }
}
