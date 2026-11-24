//! The grid, the built id, and the 16 bytes a published star stores.
//!
//! What is stored is what cannot be recomputed: identity, position and the draws. State,
//! luminosity, temperature, radius and velocity follow from them on load.

use glam::DVec3;

use crate::sky::StarId;

/// Side of a grid cell in ly. Part of the format: the grid is anchored at the origin and never
/// moves.
pub const CELL_LY: f64 = 8.0;

/// Cells per axis either side of the origin: `2^13 · 8 ly` covers the ±36 500 ly box.
pub const CELL_HALF_RANGE: i32 = 1 << 13;

/// The provider tag in a built id's low bits.
const GALAXY_PROVIDER_TAG: u64 = 1;

/// The initial-mass range a record can hold, M☉.
pub const MASS_RANGE: (f64, f64) = (0.08, 150.0);
/// The age range a record can hold, Gyr.
pub const MAX_AGE_GYR: f64 = 13.8;
/// `[Fe/H]` is stored in steps of this, dex, as an `i8`: −2.56 to +2.54.
pub const FEH_STEP: f64 = 0.02;

pub fn cell_in_grid(cell: [i32; 3]) -> bool {
    cell.iter().all(|v| (-CELL_HALF_RANGE..CELL_HALF_RANGE).contains(v))
}

/// 14 bits per axis; `None` outside the grid, where two cells would share bits.
pub fn pack_cell(cell: [i32; 3]) -> Option<u64> {
    if !cell_in_grid(cell) {
        return None;
    }
    let axis = |v: i32| (v + CELL_HALF_RANGE) as u64;
    Some(axis(cell[0]) << 28 | axis(cell[1]) << 14 | axis(cell[2]))
}

/// `cell (42) | index (16) | tier (1) | provider (5)`: unique by construction, and independent
/// of how stars are grouped into files.
pub fn star_id(cell: [i32; 3], index: u16, tier_a: bool) -> Option<StarId> {
    let packed = pack_cell(cell)?;
    Some(StarId::from_raw(packed << 22 | (index as u64) << 6 | (tier_a as u64) << 5 | GALAXY_PROVIDER_TAG))
}

/// One star as published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GalaxyRecord {
    pub cell: [i32; 3],
    /// The candidate's index within its cell; with the cell, the id.
    pub index: u16,
    /// Position within the cell, `u16` per axis over [`CELL_LY`]: 7.7 AU steps.
    pub offset: [u16; 3],
    /// Logarithmic over [`MASS_RANGE`].
    pub mass: u16,
    /// Linear over `0..MAX_AGE_GYR`.
    pub age: u16,
    pub feh: i8,
    /// Low four bits: the population's index in its chunk. Bit 4: tier A.
    pub flags: u8,
}

const POPULATION_BITS: u8 = 0x0f;
const TIER_A: u8 = 0x10;

impl GalaxyRecord {
    /// The nearest record to these values. Values outside a field's range are clamped to it.
    pub fn quantize(
        cell: [i32; 3],
        index: u16,
        position_ly: DVec3,
        initial_mass_solar: f64,
        age_gyr: f64,
        feh: f64,
        population: u8,
        tier_a: bool,
    ) -> Self {
        let min = DVec3::new(cell[0] as f64, cell[1] as f64, cell[2] as f64) * CELL_LY;
        let q = |v: f64| ((v - 0.5).round().clamp(0.0, u16::MAX as f64)) as u16;
        let frac = (position_ly - min) / CELL_LY * 65536.0;
        let (lo, hi) = (MASS_RANGE.0.ln(), MASS_RANGE.1.ln());
        let mass = ((initial_mass_solar.clamp(MASS_RANGE.0, MASS_RANGE.1).ln() - lo) / (hi - lo) * u16::MAX as f64).round() as u16;
        let age = (age_gyr.clamp(0.0, MAX_AGE_GYR) / MAX_AGE_GYR * u16::MAX as f64).round() as u16;
        let feh = (feh / FEH_STEP).round().clamp(i8::MIN as f64, i8::MAX as f64) as i8;
        Self {
            cell,
            index,
            offset: [q(frac.x), q(frac.y), q(frac.z)],
            mass,
            age,
            feh,
            flags: (population & POPULATION_BITS) | if tier_a { TIER_A } else { 0 },
        }
    }

    /// The center of the stored position's step.
    pub fn position_ly(&self) -> DVec3 {
        let min = DVec3::new(self.cell[0] as f64, self.cell[1] as f64, self.cell[2] as f64) * CELL_LY;
        let o = |v: u16| (v as f64 + 0.5) / 65536.0 * CELL_LY;
        min + DVec3::new(o(self.offset[0]), o(self.offset[1]), o(self.offset[2]))
    }

    pub fn initial_mass_solar(&self) -> f64 {
        let (lo, hi) = (MASS_RANGE.0.ln(), MASS_RANGE.1.ln());
        libm::exp(lo + self.mass as f64 / u16::MAX as f64 * (hi - lo))
    }

    pub fn age_gyr(&self) -> f64 {
        self.age as f64 / u16::MAX as f64 * MAX_AGE_GYR
    }

    pub fn feh(&self) -> f64 {
        self.feh as f64 * FEH_STEP
    }

    pub fn population(&self) -> u8 {
        self.flags & POPULATION_BITS
    }

    pub fn tier_a(&self) -> bool {
        self.flags & TIER_A != 0
    }

    pub fn id(&self) -> Option<StarId> {
        star_id(self.cell, self.index, self.tier_a())
    }
}
