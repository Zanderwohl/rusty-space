//! The seven bands, and containers keyed by them.

use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

pub const BANDS: usize = 7;

/// Speed of light in vacuum, m/s. Exact by definition.
pub const C: f64 = 299_792_458.0;

/// The rest frequency of the neutral hydrogen hyperfine transition, Hz.
pub const HI_LINE_HZ: f64 = 1_420_405_751.0;

/// B, V, R and I are the Johnson-Cousins optical run and share one silicon detector, whose
/// 1.12 eV bandgap stops responding past about 1100 nm. K, thermal and radio each need
/// different hardware. See `lightcone/docs/05-observation.md`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Band {
    B = 0,
    V = 1,
    R = 2,
    I = 3,
    K = 4,
    ThermalIr = 5,
    Radio = 6,
}

pub struct BandSpec {
    pub name: &'static str,
    pub centre_m: f64,
    /// Nominal FWHM. The radio entry is a receiver bandwidth, not a filter width.
    pub width_m: f64,
}

const SPECS: [BandSpec; BANDS] = [
    BandSpec { name: "B", centre_m: 445e-9, width_m: 94e-9 },
    BandSpec { name: "V", centre_m: 551e-9, width_m: 88e-9 },
    BandSpec { name: "R", centre_m: 658e-9, width_m: 138e-9 },
    BandSpec { name: "I", centre_m: 806e-9, width_m: 149e-9 },
    BandSpec { name: "K", centre_m: 2190e-9, width_m: 390e-9 },
    BandSpec { name: "10um", centre_m: 10e-6, width_m: 5e-6 },
    // Derived from HI_LINE_HZ rather than transcribed: a nine-digit literal is 2.3 Hz off.
    // Width is a nominal 1 MHz receiver bandwidth as d(lambda) = lambda^2 dnu / c.
    BandSpec { name: "21cm", centre_m: C / HI_LINE_HZ, width_m: 1.486e-4 },
];

impl Band {
    pub const ALL: [Band; BANDS] =
        [Band::B, Band::V, Band::R, Band::I, Band::K, Band::ThermalIr, Band::Radio];

    /// The four bands one silicon detector covers.
    pub const SILICON: [Band; 4] = [Band::B, Band::V, Band::R, Band::I];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn spec(self) -> &'static BandSpec {
        &SPECS[self as usize]
    }

    #[inline]
    pub const fn name(self) -> &'static str {
        self.spec().name
    }

    #[inline]
    pub const fn centre_m(self) -> f64 {
        self.spec().centre_m
    }

    #[inline]
    pub const fn width_m(self) -> f64 {
        self.spec().width_m
    }

    #[inline]
    pub fn centre_hz(self) -> f64 {
        C / self.centre_m()
    }

    /// Inclusive wavelength limits of the nominal top-hat.
    #[inline]
    pub fn limits_m(self) -> (f64, f64) {
        let (c, w) = (self.centre_m(), self.width_m());
        (c - w / 2.0, c + w / 2.0)
    }
}

/// A set of bands. Seven fit in a `u8`, so this is a hand-rolled bitset rather than a
/// dependency.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BandMask(u8);

impl BandMask {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self((1 << BANDS) - 1);
    /// What one silicon camera sees.
    pub const SILICON: Self = Self(0b0000_1111);

    pub fn of(bands: &[Band]) -> Self {
        let mut m = 0u8;
        for b in bands {
            m |= 1 << b.index();
        }
        Self(m)
    }

    #[inline]
    pub const fn contains(self, b: Band) -> bool {
        self.0 & (1 << b as u8) != 0
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    #[must_use]
    pub const fn with(self, b: Band) -> Self {
        Self(self.0 | (1 << b as u8))
    }

    #[must_use]
    pub const fn without(self, b: Band) -> Self {
        Self(self.0 & !(1 << b as u8))
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn iter(self) -> impl Iterator<Item = Band> {
        Band::ALL.into_iter().filter(move |b| self.contains(*b))
    }
}

/// A value per band, indexed by [`Band`] rather than by `usize`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PerBand<T>([T; BANDS]);

impl<T> PerBand<T> {
    pub const fn new(values: [T; BANDS]) -> Self {
        Self(values)
    }

    pub fn as_array(&self) -> &[T; BANDS] {
        &self.0
    }

    pub fn iter(&self) -> impl Iterator<Item = (Band, &T)> {
        Band::ALL.into_iter().map(move |b| (b, &self.0[b.index()]))
    }

    pub fn map<U>(&self, mut f: impl FnMut(Band, &T) -> U) -> PerBand<U> {
        PerBand(std::array::from_fn(|i| {
            let b = Band::ALL[i];
            f(b, &self.0[i])
        }))
    }
}

impl<T: Copy> PerBand<T> {
    pub const fn splat(v: T) -> Self {
        Self([v; BANDS])
    }
}

impl<T: Default + Copy> Default for PerBand<T> {
    fn default() -> Self {
        Self([T::default(); BANDS])
    }
}

impl<T> Index<Band> for PerBand<T> {
    type Output = T;
    #[inline]
    fn index(&self, b: Band) -> &T {
        &self.0[b.index()]
    }
}

impl<T> IndexMut<Band> for PerBand<T> {
    #[inline]
    fn index_mut(&mut self, b: Band) -> &mut T {
        &mut self.0[b.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_indices_match_their_position_in_all() {
        for (i, b) in Band::ALL.iter().enumerate() {
            assert_eq!(b.index(), i);
            assert_eq!(b.spec().name, b.name());
        }
    }

    #[test]
    fn the_radio_band_sits_on_the_hydrogen_line() {
        let f = Band::Radio.centre_hz();
        assert!((f - HI_LINE_HZ).abs() < 1.0, "{f} Hz should be the HI line");
    }

    #[test]
    fn the_optical_run_is_inside_silicons_response() {
        // Silicon's 1.12 eV bandgap cuts off near 1100 nm; K is well past it.
        for b in Band::SILICON {
            assert!(b.limits_m().1 < 1100e-9, "{} should be a silicon band", b.name());
        }
        assert!(Band::K.limits_m().0 > 1100e-9);
        assert_eq!(BandMask::SILICON, BandMask::of(&Band::SILICON));
    }

    #[test]
    fn bands_are_ordered_by_increasing_wavelength() {
        for pair in Band::ALL.windows(2) {
            assert!(pair[0].centre_m() < pair[1].centre_m());
        }
    }

    #[test]
    fn masks_hold_and_iterate_the_bands_put_in_them() {
        let m = BandMask::of(&[Band::V, Band::Radio]);
        assert!(m.contains(Band::V) && m.contains(Band::Radio));
        assert!(!m.contains(Band::B));
        assert_eq!(m.len(), 2);
        assert_eq!(m.iter().collect::<Vec<_>>(), vec![Band::V, Band::Radio]);
        assert_eq!(BandMask::ALL.len(), BANDS as u32);
        assert!(BandMask::EMPTY.is_empty());
        assert_eq!(m.with(Band::B).without(Band::B), m);
        assert_eq!(m.intersection(BandMask::SILICON), BandMask::of(&[Band::V]));
    }

    #[test]
    fn per_band_indexes_by_band_not_by_number() {
        let mut p = PerBand::splat(0.0f32);
        p[Band::K] = 2.5;
        assert_eq!(p[Band::K], 2.5);
        assert_eq!(p[Band::B], 0.0);
        assert_eq!(p.as_array()[Band::K.index()], 2.5);
        let doubled = p.map(|_, v| v * 2.0);
        assert_eq!(doubled[Band::K], 5.0);
    }
}
