//! Telescopes: what they can see, and how well.

use em_spectra::{Band, BandMask, blackbody};
use serde::{Deserialize, Serialize};

/// Planck's constant times c, for photon energy.
const HC: f64 = blackbody::H * em_spectra::bands::C;

/// An instrument's band coverage is set by detector physics, not by tier. Silicon's 1.12 eV
/// bandgap covers B, V, R and I in one device; K needs a cooled narrow-gap detector, 10 um a
/// cryogenic bolometer, and 21 cm an antenna.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Instrument {
    pub aperture_m2: f64,
    /// Fraction of arriving photons counted.
    pub throughput: f64,
    pub bands: BandMask,
    /// Physical temperature. A warm instrument is a bright source in its own thermal band.
    pub temperature_k: f64,
    /// How well the optics radiate; 0 is a perfect mirror.
    pub emissivity: f64,
}

impl Instrument {
    /// A one square metre silicon camera at room temperature: the cheap baseline.
    pub const BASELINE: Self = Self {
        aperture_m2: 1.0,
        throughput: 0.5,
        bands: BandMask::SILICON,
        temperature_k: 290.0,
        emissivity: 0.05,
    };

    /// What a crewed ship carries: four square metres, cooled, across every band.
    pub const SHIP: Self = Self {
        aperture_m2: 4.0,
        throughput: 0.6,
        bands: BandMask::ALL,
        temperature_k: 45.0,
        emissivity: 0.05,
    };

    /// What fits on something small enough to throw somewhere. A quarter of a square metre
    /// and warmer, so it sees an order of magnitude less far than a ship does.
    pub const PROBE: Self = Self {
        aperture_m2: 0.25,
        throughput: 0.5,
        bands: BandMask::ALL,
        temperature_k: 120.0,
        emissivity: 0.08,
    };

    pub fn with_bands(mut self, bands: BandMask) -> Self {
        self.bands = bands;
        self
    }

    pub fn with_aperture(mut self, aperture_m2: f64) -> Self {
        self.aperture_m2 = aperture_m2;
        self
    }

    pub fn cooled_to(mut self, temperature_k: f64) -> Self {
        self.temperature_k = temperature_k;
        self
    }

    #[inline]
    pub fn sees(&self, band: Band) -> bool {
        self.bands.contains(band)
    }

    /// Photons counted from an incident flux, over an exposure.
    pub fn counts_from_flux(&self, band: Band, flux_w_per_m2: f64, exposure_s: f64) -> f64 {
        if flux_w_per_m2 <= 0.0 || exposure_s <= 0.0 {
            return 0.0;
        }
        flux_w_per_m2 / (HC / band.centre_m()) * self.aperture_m2 * self.throughput * exposure_s
    }

    /// Photons the instrument's own thermal emission contributes.
    ///
    /// A 300 K body peaks at 9.66 um, which is the thermal band almost exactly, so a warm
    /// telescope is a brighter source than anything it is pointed at. This is why that band
    /// wants to be beyond about 50 AU, or actively cooled.
    pub fn self_emission_counts(&self, band: Band, exposure_s: f64) -> f64 {
        let radiance = blackbody::band_radiance(band, self.temperature_k);
        self.counts_from_flux(band, std::f64::consts::PI * radiance * self.emissivity, exposure_s)
    }
}

/// What a telescope is committed to for a stretch of time.
///
/// Depth goes as the square root of exposure, so covering `n` times as many targets costs a
/// factor of `sqrt(n)` in the smallest depth detectable. Ending a commitment early does not
/// cancel it; it leaves the measurement with wider error bars.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurveyRegime {
    Stare,
    Field { targets: u32 },
    AllSky { targets: u32 },
}

impl SurveyRegime {
    pub fn targets(&self) -> u32 {
        match self {
            Self::Stare => 1,
            Self::Field { targets } | Self::AllSky { targets } => (*targets).max(1),
        }
    }

    pub fn exposure_per_target(&self, total_s: f64) -> f64 {
        total_s / self.targets() as f64
    }

    /// Relative depth sensitivity against a stare of the same total time.
    pub fn depth_penalty(&self) -> f64 {
        (self.targets() as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silicon_camera_sees_four_bands_and_no_more() {
        let i = Instrument::BASELINE;
        for b in Band::SILICON {
            assert!(i.sees(b));
        }
        for b in [Band::K, Band::ThermalIr, Band::Radio] {
            assert!(!i.sees(b));
        }
    }

    #[test]
    fn counts_scale_with_aperture_throughput_and_time() {
        let i = Instrument::BASELINE;
        let base = i.counts_from_flux(Band::V, 1e-12, 100.0);
        assert!((i.with_aperture(4.0).counts_from_flux(Band::V, 1e-12, 100.0) / base - 4.0).abs() < 1e-9);
        assert!((i.counts_from_flux(Band::V, 1e-12, 400.0) / base - 4.0).abs() < 1e-9);
        assert!((i.counts_from_flux(Band::V, 4e-12, 100.0) / base - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_warm_telescope_blinds_itself_in_the_thermal_band_and_not_elsewhere() {
        let warm = Instrument::BASELINE.with_bands(BandMask::ALL);
        let cold = warm.cooled_to(40.0);
        let thermal_warm = warm.self_emission_counts(Band::ThermalIr, 1.0);
        let thermal_cold = cold.self_emission_counts(Band::ThermalIr, 1.0);
        assert!(thermal_warm / thermal_cold > 1e6, "cooling must matter: {thermal_warm} vs {thermal_cold}");
        // The optical bands are untouched: at 290 K the Wien tail at 551 nm is thirty-three
        // orders of magnitude down on the thermal band, and under 1e-13 photons a second.
        let optical = warm.self_emission_counts(Band::V, 1.0);
        assert!(thermal_warm / optical > 1e30, "ratio {}", thermal_warm / optical);
        assert!(optical * 3.156e7 < 1.0, "{} photons a year in V", optical * 3.156e7);
    }

    #[test]
    fn passive_cooling_puts_a_thermal_observatory_past_fifty_au() {
        // T_eq = 278.6 / sqrt(a_AU) for a black sphere around a Sun-like star.
        let at = |au: f64| 278.6 / au.sqrt();
        assert!((at(1.0) - 278.6).abs() < 1.0);
        assert!(at(50.0) < 40.0 && at(30.0) > 40.0, "the 40 K line falls between 30 and 50 AU");
        let near = Instrument::BASELINE.cooled_to(at(1.0)).self_emission_counts(Band::ThermalIr, 1.0);
        let far = Instrument::BASELINE.cooled_to(at(50.0)).self_emission_counts(Band::ThermalIr, 1.0);
        assert!(near / far > 1e6, "distance buys darkness: {near} vs {far}");
    }

    #[test]
    fn survey_depth_goes_as_the_square_root_of_targets() {
        let stare = SurveyRegime::Stare;
        let survey = SurveyRegime::AllSky { targets: 10_000 };
        assert_eq!(stare.exposure_per_target(1e6), 1e6);
        assert_eq!(survey.exposure_per_target(1e6), 100.0);
        assert!((survey.depth_penalty() - 100.0).abs() < 1e-9);
        assert_eq!(stare.depth_penalty(), 1.0);
    }
}
