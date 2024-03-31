pub mod orbital_period {
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
