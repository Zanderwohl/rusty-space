//! Mapping band radiance to the three channels a display has.

use serde::{Deserialize, Serialize};

use crate::bands::{BANDS, Band, BandMask, PerBand};

/// A band-to-display matrix, plus which bands the viewing instrument actually has.
///
/// The player configures this. A ship has no eyes: it is looking at the output of its own
/// processing pipeline, and choosing that mapping is the same thing observational astronomy
/// does when it publishes a three-filter image.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BandMapping {
    /// Rows are display R, G, B; columns are bands.
    pub matrix: [[f32; BANDS]; 3],
    /// Bands the instrument cannot sense contribute nothing. Kept separate from the matrix
    /// so a UI can distinguish "dark" from "uninstrumented".
    pub available: BandMask,
    pub bloom_band: Option<Band>,
    pub bloom_gain: f32,
}

impl BandMapping {
    /// Build from one band per display channel, weight 1.
    pub fn direct(r: Band, g: Band, b: Band) -> Self {
        let mut matrix = [[0.0f32; BANDS]; 3];
        matrix[0][r.index()] = 1.0;
        matrix[1][g.index()] = 1.0;
        matrix[2][b.index()] = 1.0;
        Self {
            matrix,
            available: BandMask::ALL,
            bloom_band: None,
            bloom_gain: 0.0,
        }
    }

    #[must_use]
    pub fn with_available(mut self, available: BandMask) -> Self {
        self.available = available;
        self
    }

    #[must_use]
    pub fn with_bloom(mut self, band: Band, gain: f32) -> Self {
        self.bloom_band = Some(band);
        self.bloom_gain = gain;
        self
    }

    /// Which bands this mapping both reads and can actually get.
    pub fn effective(&self) -> BandMask {
        let used = Band::ALL
            .iter()
            .filter(|b| (0..3).any(|c| self.matrix[c][b.index()] != 0.0))
            .fold(BandMask::EMPTY, |m, b| m.with(*b));
        used.intersection(self.available)
    }

    /// Bands the mapping wants that the instrument does not have.
    pub fn missing(&self) -> BandMask {
        let used = Band::ALL
            .iter()
            .filter(|b| (0..3).any(|c| self.matrix[c][b.index()] != 0.0))
            .fold(BandMask::EMPTY, |m, b| m.with(*b));
        (0..BANDS)
            .map(|i| Band::ALL[i])
            .filter(|b| used.contains(*b) && !self.available.contains(*b))
            .fold(BandMask::EMPTY, |m, b| m.with(b))
    }

    pub fn apply(&self, radiance: &PerBand<f32>) -> [f32; 3] {
        std::array::from_fn(|c| {
            self.available
                .iter()
                .map(|b| self.matrix[c][b.index()] * radiance[b])
                .sum()
        })
    }

    /// Glow contribution: the dynamic range the three channels cannot carry.
    pub fn bloom(&self, radiance: &PerBand<f32>) -> f32 {
        match self.bloom_band {
            Some(b) if self.available.contains(b) => self.bloom_gain * radiance[b],
            _ => 0.0,
        }
    }
}

/// Named mappings. `NATURAL` is the default and exists for the player rather than for the
/// science: B, V and R are close enough to the display primaries that a direct assignment
/// works. It oversaturates near white — see the tests — and the CIE route in
/// [`crate::cie`] is the accurate one where that matters.
pub mod presets {
    use super::*;

    pub fn natural() -> BandMapping {
        BandMapping::direct(Band::R, Band::V, Band::B)
    }

    /// Natural, with I folded into red so M dwarfs appear at their real brightness.
    pub fn deep_natural() -> BandMapping {
        let mut m = natural();
        m.matrix[0][Band::I.index()] = 0.6;
        m
    }

    /// Industry and waste heat.
    pub fn thermal() -> BandMapping {
        BandMapping::direct(Band::ThermalIr, Band::K, Band::V).with_bloom(Band::ThermalIr, 1.0)
    }

    /// Through clouds that are opaque in the optical.
    pub fn dust_penetration() -> BandMapping {
        BandMapping::direct(Band::Radio, Band::ThermalIr, Band::K)
    }

    /// Grey versus reddening, made visible: dust reads orange, a swarm reads neutral.
    pub fn composition() -> BandMapping {
        BandMapping::direct(Band::K, Band::V, Band::B)
    }

    /// A monochrome sky in which only excess heat is coloured.
    pub fn survey() -> BandMapping {
        let mut m = BandMapping::direct(Band::V, Band::V, Band::V);
        m.matrix[0][Band::ThermalIr.index()] = 1.0;
        m
    }

    pub fn all() -> [(&'static str, BandMapping); 6] {
        [
            ("natural", natural()),
            ("deep natural", deep_natural()),
            ("thermal", thermal()),
            ("dust penetration", dust_penetration()),
            ("composition", composition()),
            ("survey", survey()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::Band;
    use crate::blackbody::band_radiance;
    use crate::cie;

    fn radiance_at(t: f64) -> PerBand<f32> {
        let mut p = PerBand::splat(0.0f32);
        for b in Band::ALL {
            p[b] = band_radiance(b, t) as f32;
        }
        p
    }

    #[test]
    fn a_direct_mapping_moves_each_band_to_its_channel() {
        let m = presets::natural();
        let mut r = PerBand::splat(0.0f32);
        r[Band::R] = 3.0;
        r[Band::V] = 2.0;
        r[Band::B] = 1.0;
        assert_eq!(m.apply(&r), [3.0, 2.0, 1.0]);
    }

    #[test]
    fn an_unavailable_band_contributes_nothing_and_is_reported() {
        let m = presets::composition().with_available(BandMask::SILICON);
        let mut r = PerBand::splat(1.0f32);
        r[Band::K] = 100.0;
        assert_eq!(m.apply(&r)[0], 0.0, "K is not instrumented, so red is dark");
        assert_eq!(m.missing(), BandMask::of(&[Band::K]));
        assert_eq!(m.effective(), BandMask::of(&[Band::B, Band::V]));
    }

    #[test]
    fn every_preset_reads_at_least_one_band_per_channel() {
        for (name, m) in presets::all() {
            for c in 0..3 {
                let total: f32 = m.matrix[c].iter().sum();
                assert!(total > 0.0, "{name} channel {c} reads nothing");
            }
        }
    }

    #[test]
    fn bloom_needs_the_band_to_exist() {
        let m = presets::thermal();
        let mut r = PerBand::splat(0.0f32);
        r[Band::ThermalIr] = 4.0;
        assert_eq!(m.bloom(&r), 4.0);
        assert_eq!(m.with_available(BandMask::SILICON).bloom(&r), 0.0);
        assert_eq!(presets::natural().bloom(&r), 0.0);
    }

    /// The natural preset is for the player, and it is not colorimetrically exact. Direct
    /// assignment fails to converge to neutral near white, where a colour cast is most
    /// visible; at the extremes the two agree.
    #[test]
    fn direct_assignment_oversaturates_near_white() {
        let natural = presets::natural();
        for t in [4500.0, 5772.0, 7000.0] {
            let direct = natural.apply(&radiance_at(t)).map(|v| v as f64);
            let direct = cie::normalise_to_max(direct);
            let exact = cie::normalise_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(t)));
            assert!(
                cie::chroma(direct) > cie::chroma(exact) + 0.02,
                "T={t}: direct {:.3} should exceed CIE {:.3}",
                cie::chroma(direct),
                cie::chroma(exact)
            );
        }
        // At 5772 K the excess is about half again as much colour as there should be.
        let d = cie::normalise_to_max(natural.apply(&radiance_at(5772.0)).map(|v| v as f64));
        let e = cie::normalise_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(5772.0)));
        assert!((cie::chroma(d) / cie::chroma(e) - 1.5).abs() < 0.2);
    }

    #[test]
    fn both_routes_agree_on_strongly_coloured_stars() {
        for t in [2500.0, 20000.0] {
            let direct = cie::normalise_to_max(presets::natural().apply(&radiance_at(t)).map(|v| v as f64));
            let exact = cie::normalise_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(t)));
            assert!((cie::chroma(direct) - cie::chroma(exact)).abs() < 0.03, "T={t}");
        }
    }

    #[test]
    fn natural_makes_a_cool_star_red_and_a_hot_one_blue() {
        let cool = presets::natural().apply(&radiance_at(3000.0));
        assert!(cool[0] > cool[2], "3000 K must be red-dominant");
        let hot = presets::natural().apply(&radiance_at(20000.0));
        assert!(hot[2] > hot[0], "20000 K must be blue-dominant");
    }
}
