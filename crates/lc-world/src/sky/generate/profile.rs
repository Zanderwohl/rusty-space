//! A star's system profile: the authored, region-level shifts to the generator's tuning, quantized
//! as a published record stores them. See `lightcone/docs/35-system-contents.md`.

use serde::{Deserialize, Serialize};

use super::tuning::Tuning;

/// `ocean` and `life` are stored in sixteenths.
const FRACTION_STEPS: f64 = 16.0;
/// The dex offsets are stored in tenths of a dex, within this many either side of zero. Read back
/// by dividing, so a stored 3 is the same double as the literal 0.3.
const TENTHS: f64 = 10.0;
pub const DEX_LIMIT: f64 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SystemProfile {
    pub ocean: u8,
    pub life: u8,
    pub solids: i8,
    pub belts: i8,
    pub cloud: i8,
}

impl Default for SystemProfile {
    fn default() -> Self {
        Self::quantize(0.0, super::tuning::LIFE_FRACTION, 0.0, 0.0, 0.0)
    }
}

impl SystemProfile {
    /// The nearest profile to these values, each clamped to its range.
    pub fn quantize(ocean: f64, life: f64, solids_dex: f64, belts_dex: f64, cloud_dex: f64) -> Self {
        let fraction = |v: f64| (v.clamp(0.0, 1.0) * FRACTION_STEPS).round() as u8;
        let dex = |v: f64| (v.clamp(-DEX_LIMIT, DEX_LIMIT) * TENTHS).round() as i8;
        Self { ocean: fraction(ocean), life: fraction(life), solids: dex(solids_dex), belts: dex(belts_dex), cloud: dex(cloud_dex) }
    }

    pub fn ocean(&self) -> f64 {
        self.ocean as f64 / FRACTION_STEPS
    }

    pub fn life(&self) -> f64 {
        self.life as f64 / FRACTION_STEPS
    }

    pub fn solids_dex(&self) -> f64 {
        self.solids as f64 / TENTHS
    }

    pub fn belts_dex(&self) -> f64 {
        self.belts as f64 / TENTHS
    }

    pub fn cloud_dex(&self) -> f64 {
        self.cloud as f64 / TENTHS
    }

    /// The default tuning with this profile's shifts applied.
    pub fn tuning(&self) -> Tuning {
        let mut t = Tuning::default();
        let scale = |dex: f64| 10f64.powf(dex);
        t.disc.solid_earths *= scale(self.solids_dex());
        // Shares of a mass, so capped at all of it.
        t.belts.belt_survival = (t.belts.belt_survival * scale(self.belts_dex())).min(1.0);
        t.belts.kuiper_survival = (t.belts.kuiper_survival * scale(self.belts_dex())).min(1.0);
        t.belts.oort_efficiency = (t.belts.oort_efficiency * scale(self.cloud_dex())).min(1.0);
        t.world.life_fraction = self.life();
        t.world.ocean_first = self.ocean();
        t
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_profile_is_the_default_tuning() {
        assert_eq!(SystemProfile::default().tuning(), Tuning::default());
        let p = SystemProfile::quantize(0.5, 0.25, 0.3, -0.2, 9.0);
        assert_eq!((p.ocean(), p.life(), p.cloud_dex()), (0.5, 0.25, DEX_LIMIT));
        assert!((p.tuning().disc.solid_earths / Tuning::default().disc.solid_earths - 10f64.powf(0.3)).abs() < 1e-12);
    }
}
