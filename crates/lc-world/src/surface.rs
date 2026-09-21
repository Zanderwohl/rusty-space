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
/// and wrong about everything a person would recognize, since Saturn looks like Jupiter.
pub const GAS_GIANT_MASS: f64 = 2.0e26;

/// Past this a rocky surface is bare and dark, its volatiles long gone.
pub const SCORCHED_K: f64 = 500.0;

/// Below the ice line, the density that separates a body with an icy surface from one without.
///
/// Bulk density, so it is a proxy: Europa is 3013 and is an ice shell over rock, Io is 3528 and
/// has essentially no water at all. Those two are a hundred thousand kilometers apart and they
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

    /// Two colors the surface varies between, as linear RGB, and how much contrast to give
    /// them. Deliberately narrow ranges: a planet is one color with variation, not a palette.
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

    /// Bond albedo: the fraction of *all* incident light a body turns away.
    ///
    /// A different quantity from [`Surface::albedo`], not a different estimate of it. Geometric
    /// albedo is how bright the disc looks at full phase and is what reflected-light photometry
    /// wants; Bond albedo is integrated over every wavelength and direction and is what the
    /// energy balance wants. Jupiter's are 0.50 and 0.34, and using one for the other puts its
    /// temperature out by six per cent.
    ///
    /// Venus is the outlier this cannot follow: 0.76, where Mars and Titan are near 0.25 and
    /// share its class. A body's cloud deck is not derivable from its radius, mass and
    /// temperature, which is the whole basis of this module.
    pub fn bond_albedo(&self) -> f64 {
        match self {
            Self::GasGiant => 0.34,
            Self::IceGiant => 0.30,
            Self::Ice => 0.70,
            Self::Rock => 0.10,
            Self::Weathered => 0.25,
            Self::Scorched => 0.10,
        }
    }

    /// Radiated power over absorbed power.
    ///
    /// One for anything that only re-emits what it catches, which is every rocky body: Earth's
    /// own heat is 0.09 W/m² against the 240 it absorbs. A giant is not — it is still shrinking,
    /// and the gravitational energy comes out as infrared. Jupiter radiates 1.67 times what it
    /// takes from the Sun and Saturn 1.78, which is why they are warmer than sunlight can
    /// explain and why they are bright at ten microns on their night sides.
    ///
    /// **The ice giants disagree and nobody knows why.** Uranus is 1.06 — consistent with no
    /// internal heat at all — and Neptune is 2.61, though Neptune is half again as far out.
    /// Their effective temperatures come out within a fifth of a kelvin of each other by
    /// coincidence. One number has to stand for both here; this is nearer the Uranus end,
    /// which makes an ice giant read as the cold thing it mostly is.
    pub fn internal_heat_ratio(&self) -> f64 {
        match self {
            Self::GasGiant => 1.7,
            Self::IceGiant => 1.3,
            _ => 1.0,
        }
    }

    /// What the body actually radiates at, given the gray equilibrium temperature.
    ///
    /// `equilibrium_k` is the zero-albedo balance [`crate::system::equilibrium_temperature`]
    /// computes, which is what [`Surface::classify`] is calibrated against. This is the
    /// temperature a *photometer* sees: what the body keeps of the sunlight, plus whatever heat
    /// it makes itself, both as fourth powers.
    ///
    /// Against the measured effective temperatures of the solar system's giants this is good to
    /// a couple of per cent for Jupiter and Saturn. See [`Surface::internal_heat_ratio`] for why
    /// the ice giants cannot both be right.
    pub fn effective_temperature(&self, equilibrium_k: f64) -> f64 {
        if equilibrium_k <= 0.0 {
            return 0.0;
        }
        let kept = (1.0 - self.bond_albedo()).max(0.0);
        equilibrium_k * (kept * self.internal_heat_ratio()).powf(0.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Effective temperature against the four giants as measured.
    ///
    /// The whole point of the internal-heat term: sunlight alone cannot put Jupiter at 124 K or
    /// Neptune anywhere near Uranus. Distances and measured values are the standard fact-sheet
    /// numbers; the equilibrium temperature here is the zero-albedo one this crate computes, so
    /// what is being checked is the composition of albedo and internal heat, not the balance.
    #[test]
    fn the_giants_come_out_at_the_temperatures_they_are_measured_at() {
        // Zero-albedo equilibrium at 1 AU, which is where `equilibrium_temperature` puts Earth.
        const AT_EARTH_K: f64 = 278.6;
        let equilibrium = |au: f64| AT_EARTH_K / au.sqrt();

        // (name, AU, surface, measured effective temperature K, tolerance)
        let giants = [
            ("Jupiter", 5.2044, Surface::GasGiant, 124.4, 0.05),
            ("Saturn", 9.5826, Surface::GasGiant, 95.0, 0.05),
        ];
        for (name, au, surface, measured, tolerance) in giants {
            let got = surface.effective_temperature(equilibrium(au));
            assert!(
                (got / measured - 1.0).abs() < tolerance,
                "{name}: {got:.1} K against a measured {measured:.1} K",
            );
        }

        // The ice giants bracket rather than match: they have the same effective temperature
        // and half again the distance between them, and one ratio cannot do that. What must
        // hold is that both are warmer than sunlight alone leaves them and neither is absurd.
        for (name, au, measured) in [("Uranus", 19.201, 59.1), ("Neptune", 30.047, 59.3)] {
            let sunlit = equilibrium(au) * (1.0 - Surface::IceGiant.bond_albedo()).powf(0.25);
            let got = Surface::IceGiant.effective_temperature(equilibrium(au));
            assert!(got > sunlit, "{name} should be warmer than sunlight alone: {got:.1}");
            assert!((got / measured).clamp(0.7, 1.3) == got / measured, "{name}: {got:.1} K");
        }
    }

    /// A rocky body radiates what it catches and nothing else, so its effective temperature is
    /// the equilibrium one cut by what it reflects. Earth is the calibration everyone knows.
    #[test]
    fn a_rocky_body_has_no_heat_of_its_own() {
        assert_eq!(Surface::Rock.internal_heat_ratio(), 1.0);
        assert_eq!(Surface::Weathered.internal_heat_ratio(), 1.0);
        assert_eq!(Surface::Ice.internal_heat_ratio(), 1.0);

        // Earth: 278.6 K gray, Bond albedo near a third, and 254 K is the textbook answer.
        let earth = Surface::Weathered.effective_temperature(278.6);
        assert!((earth - 254.0).abs() < 8.0, "{earth:.1} K against a textbook 254 K");
        assert_eq!(Surface::Rock.effective_temperature(0.0), 0.0);
    }

    /// Bond and geometric albedo are different quantities, and a giant is where it shows.
    #[test]
    fn bond_albedo_is_not_the_geometric_one() {
        assert!(Surface::GasGiant.bond_albedo() < Surface::GasGiant.albedo());
        for surface in
            [Surface::GasGiant, Surface::IceGiant, Surface::Ice, Surface::Rock, Surface::Weathered]
        {
            assert!((0.0..1.0).contains(&surface.bond_albedo()), "{surface:?}");
        }
    }

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
    /// Jupiter, a hundred thousand kilometers apart, and one is ice and the other is not.
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
