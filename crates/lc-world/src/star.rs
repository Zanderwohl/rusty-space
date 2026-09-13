//! The central body of a system, as photometry sees it.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub radius_m: f64,
    pub teff_k: f64,
    /// Gravitational parameter, m^3/s^2. Mu rather than mass, as `em-foundations` has it.
    pub mu: f64,
    /// Quadratic limb-darkening coefficients `(u1, u2)`.
    pub limb_darkening: (f64, f64),
}

impl Star {
    /// A Sun-like reference, for tests and defaults.
    pub const SOL: Self = Self {
        radius_m: 6.957e8,
        teff_k: 5772.0,
        mu: 1.327_124_400_18e20,
        limb_darkening: (0.4, 0.26),
    };

    #[inline]
    pub fn luminosity(&self) -> f64 {
        em_spectra::blackbody::luminosity(self.radius_m, self.teff_k)
    }

    #[inline]
    pub fn disc_area(&self) -> f64 {
        std::f64::consts::PI * self.radius_m * self.radius_m
    }

    /// Circular orbital speed at radius `r`, m/s.
    #[inline]
    pub fn orbital_speed(&self, r: f64) -> f64 {
        (self.mu / r).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sol_reproduces_known_values() {
        assert!((Star::SOL.luminosity() / 3.828e26 - 1.0).abs() < 0.01);
        assert!((Star::SOL.mu / 1.327e20 - 1.0).abs() < 1e-3);
        // Earth's orbital speed, 29.78 km/s.
        assert!((Star::SOL.orbital_speed(1.496e11) - 29_780.0).abs() < 50.0);
    }
}
