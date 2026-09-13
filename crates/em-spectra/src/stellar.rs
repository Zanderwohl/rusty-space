//! Main-sequence relations between luminosity, radius and mass.

use crate::blackbody::SIGMA;

pub const SOLAR_LUMINOSITY: f64 = 3.828e26;
pub const SOLAR_RADIUS: f64 = 6.957e8;
/// Gravitational parameter of the Sun, m^3/s^2. Known far better than `G * M` separately, so
/// masses are carried as solar masses and converted through this rather than through `G`.
pub const SOLAR_MU: f64 = 1.327_124_400_18e20;
pub const SOLAR_TEFF: f64 = 5772.0;

/// Radius of a sphere with this luminosity and effective temperature.
pub fn radius_from_luminosity(luminosity_w: f64, teff_k: f64) -> f64 {
    if luminosity_w <= 0.0 || teff_k <= 0.0 {
        return 0.0;
    }
    (luminosity_w / (4.0 * std::f64::consts::PI * SIGMA * teff_k.powi(4))).sqrt()
}

/// Mass in solar masses from luminosity in solar luminosities, by `L ~ M^3.5`.
///
/// Main sequence only. A giant has the luminosity of a much heavier star and this will say
/// so, which is why anything using it should check the luminosity class first.
pub fn main_sequence_mass_solar(luminosity_solar: f64) -> f64 {
    if luminosity_solar <= 0.0 {
        return 0.0;
    }
    luminosity_solar.powf(1.0 / 3.5)
}

#[inline]
pub fn mu_from_mass_solar(mass_solar: f64) -> f64 {
    SOLAR_MU * mass_solar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_returns_its_own_radius_and_mass() {
        let r = radius_from_luminosity(SOLAR_LUMINOSITY, SOLAR_TEFF);
        assert!((r / SOLAR_RADIUS - 1.0).abs() < 0.01, "{r}");
        assert!((main_sequence_mass_solar(1.0) - 1.0).abs() < 1e-12);
        assert!((mu_from_mass_solar(1.0) - SOLAR_MU).abs() < 1e-6);
    }

    #[test]
    fn the_mass_luminosity_relation_runs_the_right_way() {
        assert!(main_sequence_mass_solar(100.0) > 3.0);
        assert!(main_sequence_mass_solar(0.01) < 0.4);
        assert_eq!(main_sequence_mass_solar(0.0), 0.0);
    }

    #[test]
    fn a_hotter_star_of_equal_luminosity_is_smaller() {
        let cool = radius_from_luminosity(SOLAR_LUMINOSITY, 3000.0);
        let hot = radius_from_luminosity(SOLAR_LUMINOSITY, 12000.0);
        assert!(cool > hot * 10.0, "radius goes as T^-2");
    }
}
