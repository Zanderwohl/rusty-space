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

    /// One band per channel, each weighted so a blackbody at `reference_k` comes out neutral.
    ///
    /// Required whenever the three bands are far apart in wavelength, and the reason is
    /// arithmetic rather than taste: a sun-like star delivers 88 times more band-integrated
    /// radiance in V than at ten microns. Unweighted, that renders every ordinary star blue and
    /// leaves any swarm below about half coverage invisible, because its thermal excess has to
    /// beat the star's own visible light before it shows at all. Making the *baseline* neutral
    /// is what turns an excess in one band into a color.
    ///
    /// This is what a false-color astronomical image does, and it is why they are readable.
    /// [`presets::natural`] does not need it: B, V and R are close enough together that a
    /// blackbody is already nearly neutral across them.
    pub fn direct_normalized(r: Band, g: Band, b: Band, reference_k: f64) -> Self {
        let mut mapping = Self::direct(r, g, b);
        for (channel, band) in [r, g, b].into_iter().enumerate() {
            let at_reference = crate::blackbody::band_radiance(band, reference_k);
            mapping.matrix[channel][band.index()] =
                if at_reference > 0.0 { (1.0 / at_reference) as f32 } else { 0.0 };
        }
        mapping
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

    /// The star a normalized preset is neutral against. Sun-like, because that is the star a
    /// player has an intuition for.
    pub const REFERENCE_K: f64 = 5772.0;

    /// Industry and waste heat. A sun-like star is white; excess at ten microns is red.
    pub fn thermal() -> BandMapping {
        BandMapping::direct_normalized(Band::ThermalIr, Band::K, Band::V, REFERENCE_K)
            .with_bloom(Band::ThermalIr, 1.0)
    }

    /// Through clouds that are opaque in the optical.
    pub fn dust_penetration() -> BandMapping {
        BandMapping::direct_normalized(Band::Radio, Band::ThermalIr, Band::K, REFERENCE_K)
    }

    /// Gray versus reddening, made visible: dust reads orange, a swarm reads neutral.
    pub fn composition() -> BandMapping {
        BandMapping::direct_normalized(Band::K, Band::V, Band::B, REFERENCE_K)
    }

    /// A monochrome sky in which only excess heat is colored.
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
    /// assignment fails to converge to neutral near white, where a color cast is most
    /// visible; at the extremes the two agree.
    #[test]
    fn direct_assignment_oversaturates_near_white() {
        let natural = presets::natural();
        for t in [4500.0, 5772.0, 7000.0] {
            let direct = natural.apply(&radiance_at(t)).map(|v| v as f64);
            let direct = cie::normalize_to_max(direct);
            let exact = cie::normalize_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(t)));
            assert!(
                cie::chroma(direct) > cie::chroma(exact) + 0.02,
                "T={t}: direct {:.3} should exceed CIE {:.3}",
                cie::chroma(direct),
                cie::chroma(exact)
            );
        }
        // At 5772 K the excess is about half again as much color as there should be.
        let d = cie::normalize_to_max(natural.apply(&radiance_at(5772.0)).map(|v| v as f64));
        let e = cie::normalize_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(5772.0)));
        assert!((cie::chroma(d) / cie::chroma(e) - 1.5).abs() < 0.2);
    }

    #[test]
    fn both_routes_agree_on_strongly_colored_stars() {
        for t in [2500.0, 20000.0] {
            let direct = cie::normalize_to_max(presets::natural().apply(&radiance_at(t)).map(|v| v as f64));
            let exact = cie::normalize_to_max(cie::linear_srgb_from_xyz(cie::xyz_from_blackbody(t)));
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

    /// The bug this exists for: an unweighted thermal mapping renders every star blue.
    ///
    /// A sun-like star delivers 88 times more band-integrated radiance in V than at ten
    /// microns, so V wins the display and every swarm under about half coverage is invisible.
    /// The physics was right and the mapping could not show it.
    #[test]
    fn a_wide_mapping_must_be_normalized_or_one_band_wins_outright() {
        let solar = PerBand::new(std::array::from_fn(|i| {
            crate::blackbody::band_radiance(Band::ALL[i], presets::REFERENCE_K) as f32
        }));

        let naive = BandMapping::direct(Band::ThermalIr, Band::K, Band::V).apply(&solar);
        let ratio = naive[2] / naive[0];
        assert!(ratio > 50.0, "V should swamp the thermal band, got {ratio}");

        let fixed = presets::thermal().apply(&solar);
        for c in fixed {
            assert!((c - 1.0).abs() < 1e-4, "the reference star must be neutral: {fixed:?}");
        }
    }

    /// What the preset is for: waste heat, as a color.
    #[test]
    fn a_thermal_excess_reads_as_red_once_the_baseline_is_neutral() {
        let mut radiance = PerBand::new(std::array::from_fn(|i| {
            crate::blackbody::band_radiance(Band::ALL[i], presets::REFERENCE_K) as f32
        }));
        // A swarm covering half the sphere: ten microns up by two orders, the visible halved.
        radiance[Band::ThermalIr] *= 135.0;
        radiance[Band::V] *= 0.5;

        let rgb = presets::thermal().apply(&radiance);
        assert!(rgb[0] > 100.0, "red should carry the excess, got {rgb:?}");
        assert!(rgb[0] > 50.0 * rgb[2], "and dominate the visible channel, got {rgb:?}");
    }

    /// Normalizing must not disturb the preset that was already right. B, V and R sit close
    /// enough together that a blackbody is nearly neutral across them without any weighting.
    #[test]
    fn the_natural_preset_is_left_alone() {
        let m = presets::natural();
        for channel in 0..3 {
            assert!(m.matrix[channel].iter().any(|w| (*w - 1.0).abs() < 1e-9), "weights changed");
        }
    }

    #[test]
    fn every_normalized_preset_is_neutral_on_the_reference_star() {
        let solar = PerBand::new(std::array::from_fn(|i| {
            crate::blackbody::band_radiance(Band::ALL[i], presets::REFERENCE_K) as f32
        }));
        for mapping in [presets::thermal(), presets::dust_penetration(), presets::composition()] {
            let rgb = mapping.apply(&solar);
            for c in rgb {
                assert!((c - 1.0).abs() < 1e-3, "{rgb:?}");
            }
        }
    }
}
