//! What each kind of ground on a rocky world reflects, band by band.
//!
//! A color map says what a world looks like to an eye, and nothing about the bands past it:
//! that forest is bright in I (the red edge), that water is black past the visible, that snow
//! goes dark in K. So the renderer mixes these by how much of each a texel is, and the color map
//! only carries the detail. See `lightcone/docs/07-rendering.md`, "Every band sees its own
//! ground".
//!
//! Hemispherical reflectance in the five reflected bands, shaped after laboratory spectra (USGS
//! and ASTER libraries) and read for their shape. Zero in the two emissive bands, where what
//! leaves a surface is its own heat, and each ground's emissivity and thermal inertia say how
//! much, and when.

use em_spectra::{BANDS, Band};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Water,
    Ice,
    Growth,
    Sand,
    Basalt,
    /// Oxidized: Mars's dust. What bare ground becomes under air.
    Rust,
    /// A water cloud's top.
    Cloud,
}

impl Ground {
    pub const ALL: [Ground; 7] =
        [Ground::Water, Ground::Ice, Ground::Growth, Ground::Sand, Ground::Basalt, Ground::Rust, Ground::Cloud];

    /// In [`Band`] order: B, V, R, I, K, then the two emissive bands.
    pub fn reflectance(self) -> [f32; BANDS] {
        match self {
            // Dark everywhere and darker toward the infrared, where water absorbs.
            Ground::Water => [0.07, 0.05, 0.03, 0.015, 0.005, 0.0, 0.0],
            // Snow and ice: the brightest thing in the visible, and dark by 2 microns.
            Ground::Ice => [0.92, 0.90, 0.87, 0.78, 0.15, 0.0, 0.0],
            // Chlorophyll takes blue and red, and the leaf's cells throw back the near
            // infrared: the red edge, which is why a forest is the brightest ground in I.
            Ground::Growth => [0.04, 0.09, 0.05, 0.45, 0.22, 0.0, 0.0],
            // Quartz sand rises steadily into the infrared.
            Ground::Sand => [0.25, 0.35, 0.45, 0.52, 0.55, 0.0, 0.0],
            Ground::Basalt => [0.06, 0.07, 0.08, 0.09, 0.10, 0.0, 0.0],
            // Ferric iron eats blue, and the red run carries on past the visible.
            Ground::Rust => [0.07, 0.15, 0.28, 0.33, 0.35, 0.0, 0.0],
            // Bright and gray, until water's own absorption at 2 microns.
            Ground::Cloud => [0.80, 0.80, 0.80, 0.78, 0.45, 0.0, 0.0],
        }
    }

    pub fn reflectance_in(self, band: Band) -> f32 {
        self.reflectance()[band.index()]
    }

    /// Emissivity in [`Band`] order. Kirchhoff's law where the ground reflects, so what does not
    /// reflect is what glows; measured values in the two emissive bands.
    ///
    /// A cloud's is its opacity instead: what share of the ground's own light it stops. Water
    /// cloud is opaque at ten microns and transparent at 21 cm, which is how radar and radio see
    /// through Venus's.
    pub fn emissivity(self) -> [f32; BANDS] {
        let (thermal, radio) = match self {
            Ground::Water => (0.98, 0.45),
            Ground::Ice => (0.97, 0.9),
            Ground::Growth => (0.97, 0.9),
            Ground::Sand => (0.90, 0.9),
            Ground::Basalt => (0.95, 0.9),
            Ground::Rust => (0.93, 0.9),
            Ground::Cloud => (0.95, 0.0),
        };
        let r = self.reflectance();
        std::array::from_fn(|k| match Band::ALL[k] {
            Band::ThermalIr => thermal,
            Band::Radio => radio,
            _ => 1.0 - r[k],
        })
    }

    /// Share of a day's swing in temperature the ground damps, `[0, 1]`. The sea's heat capacity
    /// is why it is as warm at night as by day; dry sand is the other end.
    pub fn inertia(self) -> f32 {
        match self {
            Ground::Water => 0.95,
            Ground::Ice => 0.6,
            Ground::Growth => 0.55,
            Ground::Sand => 0.15,
            Ground::Basalt => 0.3,
            Ground::Rust => 0.2,
            Ground::Cloud => 1.0,
        }
    }
}

/// Bare ground `rust` of the way from basalt to Mars, band by band, as rocky.tgraph mixes them.
pub fn rock(rust: f32) -> [f32; BANDS] {
    between(Ground::Basalt.reflectance(), Ground::Rust.reflectance(), rust)
}

/// [`rock`]'s emissivity and inertia.
pub fn rock_emissivity(rust: f32) -> [f32; BANDS] {
    between(Ground::Basalt.emissivity(), Ground::Rust.emissivity(), rust)
}

pub fn rock_inertia(rust: f32) -> f32 {
    between([Ground::Basalt.inertia()], [Ground::Rust.inertia()], rust)[0]
}

fn between<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facts the table exists for, each of which a color map has backwards or missing.
    #[test]
    fn each_ground_is_what_the_bands_past_the_eye_see() {
        let r = |g: Ground, b| g.reflectance_in(b);
        assert!(r(Ground::Growth, Band::I) > 4.0 * r(Ground::Growth, Band::R), "the red edge");
        assert!(r(Ground::Water, Band::K) < r(Ground::Water, Band::B) / 5.0, "water is black past the eye");
        assert!(r(Ground::Ice, Band::K) < r(Ground::Ice, Band::V) / 4.0, "snow goes dark in K");
        assert!(r(Ground::Rust, Band::R) > 3.0 * r(Ground::Rust, Band::B), "rust is red");
        for g in Ground::ALL {
            assert_eq!(r(g, Band::ThermalIr), 0.0);
            assert_eq!(r(g, Band::Radio), 0.0);
        }
    }

    /// The sea is the one ground that stays warm through the night, and a cloud hides the ground
    /// at ten microns and not at 21 cm.
    #[test]
    fn what_glows_and_when() {
        assert!(Ground::ALL.iter().all(|g| *g == Ground::Cloud || g.inertia() <= Ground::Water.inertia()));
        assert!(Ground::Sand.inertia() < Ground::Growth.inertia());
        let e = |g: Ground, b: Band| g.emissivity()[b.index()];
        assert!(e(Ground::Cloud, Band::ThermalIr) > 0.9 && e(Ground::Cloud, Band::Radio) == 0.0);
        assert!(e(Ground::Water, Band::Radio) < e(Ground::Sand, Band::Radio), "the sea is radio-cold");
        assert!((e(Ground::Water, Band::V) - 0.95).abs() < 1e-6, "Kirchhoff where it reflects");
    }

    #[test]
    fn rock_runs_from_basalt_to_rust() {
        assert_eq!(rock(0.0), Ground::Basalt.reflectance());
        assert_eq!(rock(1.0), Ground::Rust.reflectance());
    }
}
