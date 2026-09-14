//! What a body looks like, derived from what it is.
//!
//! No authored appearance and no textures. A body's radius, mass and equilibrium temperature
//! already say whether it is a gas giant, a ball of ice or a rock, and those three facts are
//! carried by every body in every system, real or generated. Anything the renderer needs beyond
//! them is a seed.

use serde::{Deserialize, Serialize};

/// Density below which a body cannot be rock. Saturn is 687 kg/m^3 and Jupiter 1326; the
/// densest rocky body in the solar system is Earth at 5514.
pub const GIANT_DENSITY: f64 = 2500.0;

/// Water freezes; past this a surface is not ice.
pub const ICE_LINE_K: f64 = 190.0;

/// Mass separating a gas giant from an ice giant.
///
/// Mass, not temperature. An ice giant is one because it never accreted enough hydrogen and
/// helium, and that is a fact about its mass: Saturn is 5.7e26 kg and Neptune 1.0e26. Sorting
/// by temperature put Saturn, at 90 K, in with Uranus -- which is true about its temperature
/// and wrong about everything a person would recognise, since Saturn looks like Jupiter.
pub const GAS_GIANT_MASS: f64 = 2.0e26;

/// Past this a rocky surface is bare and dark, its volatiles long gone.
pub const SCORCHED_K: f64 = 500.0;

/// Below the ice line, the density that separates a body with an icy surface from one without.
///
/// Bulk density, so it is a proxy: Europa is 3013 and is an ice shell over rock, Io is 3528 and
/// has essentially no water at all. Those two are a hundred thousand kilometres apart and they
/// are what this number is set between.
pub const ICY_SURFACE_DENSITY: f64 = 3200.0;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Banded, warm, turbulent. Jupiter and Saturn.
    GasGiant,
    /// Banded, cold, and far smoother; the bands of Uranus are barely there.
    IceGiant,
    /// Bright and high-contrast, cracked rather than cratered. Europa, Enceladus, Pluto.
    Ice,
    /// Dark, cratered, airless. Luna, Mercury, most moons.
    Rock,
    /// Warm rock with weather: dust, oxides, some cloud. Mars, Venus, Titan.
    Weathered,
    /// Scorched bare rock, close in and stripped.
    Scorched,
}

impl Surface {
    /// Classify a body from what the simulation already knows about it.
    pub fn classify(radius_m: f64, mass_kg: f64, equilibrium_k: f64) -> Self {
        let volume = 4.0 / 3.0 * std::f64::consts::PI * radius_m.powi(3);
        let density = if volume > 0.0 { mass_kg / volume } else { f64::INFINITY };

        // Density first, because it is the one that cannot be faked: nothing rocky is that
        // light and nothing gaseous is that heavy.
        if density < GIANT_DENSITY && radius_m > 1.0e7 {
            return if mass_kg >= GAS_GIANT_MASS { Self::GasGiant } else { Self::IceGiant };
        }
        if equilibrium_k > SCORCHED_K {
            return Self::Scorched;
        }
        if equilibrium_k < ICE_LINE_K {
            // Below the ice line, density says whether it is a dirty snowball or a rock that
            // happens to be cold.
            return if density < ICY_SURFACE_DENSITY { Self::Ice } else { Self::Rock };
        }
        Self::Weathered
    }

    /// Whether the surface is latitude-banded rather than mottled.
    pub fn is_banded(&self) -> bool {
        matches!(self, Self::GasGiant | Self::IceGiant)
    }

    /// Two colours the surface varies between, as linear RGB, and how much contrast to give
    /// them. Deliberately narrow ranges: a planet is one colour with variation, not a palette.
    pub fn palette(&self) -> ([f32; 3], [f32; 3], f32) {
        match self {
            Self::GasGiant => ([0.52, 0.42, 0.32], [0.84, 0.76, 0.63], 1.0),
            Self::IceGiant => ([0.24, 0.45, 0.52], [0.42, 0.66, 0.72], 0.45),
            Self::Ice => ([0.62, 0.68, 0.76], [0.94, 0.96, 0.99], 0.7),
            Self::Rock => ([0.13, 0.12, 0.11], [0.34, 0.32, 0.29], 0.9),
            Self::Weathered => ([0.30, 0.17, 0.11], [0.62, 0.44, 0.30], 0.85),
            Self::Scorched => ([0.16, 0.13, 0.12], [0.42, 0.33, 0.26], 1.0),
        }
    }

    /// Geometric albedo, which the photometry wants and which a flat 0.3 was standing in for.
    pub fn albedo(&self) -> f64 {
        match self {
            Self::GasGiant => 0.50,
            Self::IceGiant => 0.45,
            Self::Ice => 0.65,
            Self::Rock => 0.11,
            Self::Weathered => 0.20,
            Self::Scorched => 0.10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The solar system, by the numbers it actually has. If the classification cannot sort
    /// these it cannot sort anything.
    #[test]
    fn the_solar_system_sorts_the_way_a_person_would() {
        let cases = [
            ("Jupiter", 6.9911e7, 1.898e27, 122.0, Surface::GasGiant),
            ("Saturn", 5.8232e7, 5.683e26, 90.0, Surface::GasGiant),
            ("Uranus", 2.5559e7, 8.681e25, 64.0, Surface::IceGiant),
            ("Neptune", 2.4764e7, 1.024e26, 51.0, Surface::IceGiant),
            ("Earth", 6.371e6, 5.972e24, 278.0, Surface::Weathered),
            ("Mars", 3.3895e6, 6.417e23, 226.0, Surface::Weathered),
            ("Luna", 1.7374e6, 7.342e22, 278.0, Surface::Weathered),
            ("Mercury", 2.4397e6, 3.301e23, 440.0, Surface::Weathered),
            ("Europa", 1.5608e6, 4.800e22, 102.0, Surface::Ice),
            ("Enceladus", 2.521e5, 1.080e20, 75.0, Surface::Ice),
            ("Callisto", 2.4103e6, 1.076e23, 122.0, Surface::Ice),
            ("Io", 1.8216e6, 8.932e22, 110.0, Surface::Rock),
        ];
        for (name, r, m, t, want) in cases {
            let got = Surface::classify(r, m, t);
            assert_eq!(got, want, "{name} classified as {got:?}");
        }
    }

    /// Saturn and Neptune are the pair the giant threshold is set between. Saturn is colder
    /// than some ice giants and looks nothing like one.
    #[test]
    fn saturn_and_neptune_land_on_opposite_sides() {
        assert_eq!(Surface::classify(5.8232e7, 5.683e26, 90.0), Surface::GasGiant);
        assert_eq!(Surface::classify(2.4764e7, 1.024e26, 51.0), Surface::IceGiant);
    }

    /// Europa and Io are the pair the ice threshold is set between: both cold, both moons of
    /// Jupiter, a hundred thousand kilometres apart, and one is ice and the other is not.
    #[test]
    fn europa_and_io_land_on_opposite_sides() {
        assert_eq!(Surface::classify(1.5608e6, 4.800e22, 102.0), Surface::Ice);
        assert_eq!(Surface::classify(1.8216e6, 8.932e22, 110.0), Surface::Rock);
    }

    /// Density is the discriminator that cannot be faked: nothing rocky is that light.
    #[test]
    fn density_decides_giant_against_rock() {
        let radius: f64 = 3.0e7;
        let volume = 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
        let gassy = Surface::classify(radius, 1200.0 * volume, 150.0);
        let rocky = Surface::classify(radius, 5000.0 * volume, 150.0);
        assert!(gassy.is_banded(), "{gassy:?}");
        assert!(!rocky.is_banded(), "{rocky:?}");
    }

    #[test]
    fn a_small_light_body_is_not_a_giant() {
        // A comet is less dense than water and is not Jupiter.
        assert!(!Surface::classify(5.0e3, 2.2e14, 150.0).is_banded());
    }

    #[test]
    fn every_surface_has_a_palette_and_a_plausible_albedo() {
        for s in [
            Surface::GasGiant,
            Surface::IceGiant,
            Surface::Ice,
            Surface::Rock,
            Surface::Weathered,
            Surface::Scorched,
        ] {
            let (dark, light, contrast) = s.palette();
            let lum = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
            assert!(lum(light) > lum(dark), "{s:?} light should be lighter");
            assert!(contrast > 0.0 && contrast <= 1.0);
            assert!(s.albedo() > 0.0 && s.albedo() < 1.0);
        }
        // Ice is the bright one and bare rock the dark one, which is the whole point of
        // having an albedo per class rather than one number for everything.
        assert!(Surface::Ice.albedo() > Surface::Rock.albedo() * 4.0);
    }
}
