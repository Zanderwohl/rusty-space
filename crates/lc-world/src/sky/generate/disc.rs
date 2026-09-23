//! The disc a system condenses out of: its edges, its snow line and where its solids are.
//!
//! One radius-temperature relation does all the geometry. The sublimation edge, the snow line
//! and both ends of the habitable zone are the same curve read at four temperatures, so a red
//! dwarf's system is scaled by its own light without a second formula anywhere.

use super::tuning::Tuning;
use crate::rng;
use crate::sky::CatalogueStar;
use crate::star::Star;

/// Earth masses.
pub const EARTH_MASS: f64 = 5.9722e24;
pub const EARTH_RADIUS: f64 = 6.371e6;
pub const SOLAR_MASS_KG: f64 = 1.988_41e30;
pub const JUPITER_EARTHS: f64 = 317.83;
pub const G: f64 = 6.674_301_5e-11;

/// Where a body sits, in the terms everything downstream is decided by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    /// Innermost radius holding solids, meters.
    pub inner_m: f64,
    /// Where water ice condenses, meters.
    pub snow_m: f64,
    /// Outermost radius holding solids, meters.
    pub outer_m: f64,
    /// Habitable zone, warm edge first, meters.
    pub habitable_m: (f64, f64),
    /// Solid mass available across the whole disc, Earth masses.
    pub solid_earths: f64,
    index: f64,
    ice_boost: f64,
}

impl Disc {
    /// The disc around one star.
    pub fn of(star: &CatalogueStar, tuning: &Tuning) -> Self {
        let t = &tuning.disc;
        let seed = star.seed();
        let at = |k: f64| radius_at(&star.star, k);

        let snow_m = at(t.snow_k);
        let jitter = rng::uniform_in(rng::hash(&[seed, 0xd15c, 1]), t.outer_jitter.0, t.outer_jitter.1);
        let spread = rng::gaussian(rng::hash(&[seed, 0xd15c, 2])) * t.solid_spread_dex;
        let solid_earths = t.solid_earths
            * star.mass_solar.max(0.05).powf(t.solids_per_stellar_mass)
            * crate::sky::metallicity::solid_mass_factor(star.metallicity)
            * 10f64.powf(spread);

        Self {
            inner_m: at(t.sublimation_k),
            snow_m,
            outer_m: snow_m * t.outer_over_snow * jitter,
            habitable_m: (at(t.habitable_k.0), at(t.habitable_k.1)),
            solid_earths,
            index: t.surface_index,
            ice_boost: t.ice_boost,
        }
    }

    /// Solid mass between two radii, Earth masses.
    ///
    /// The surface density is a power law with a step at the snow line, so the annulus
    /// integral is closed form and no rung has to be quadratured.
    pub fn solids_between(&self, a_m: f64, b_m: f64) -> f64 {
        let total = self.integral(self.inner_m, self.outer_m);
        if total <= 0.0 {
            return 0.0;
        }
        let (lo, hi) = (a_m.max(self.inner_m), b_m.min(self.outer_m));
        if hi <= lo {
            return 0.0;
        }
        self.solid_earths * self.integral(lo, hi) / total
    }

    /// Unnormalized `integral of Sigma * 2 pi r dr` between two radii.
    fn integral(&self, a_m: f64, b_m: f64) -> f64 {
        let w = |r: f64| {
            let p = 2.0 - self.index;
            if p.abs() < 1.0e-9 { r.max(1.0).ln() } else { r.powf(p) / p }
        };
        let rocky = w(b_m.min(self.snow_m)) - w(a_m.min(self.snow_m));
        let icy = w(b_m.max(self.snow_m)) - w(a_m.max(self.snow_m));
        rocky.max(0.0) + self.ice_boost * icy.max(0.0)
    }

    /// Whether a radius is past the snow line, and so forms with ices.
    pub fn icy(&self, a_m: f64) -> bool {
        a_m >= self.snow_m
    }

    /// Whether a radius is in the habitable zone.
    pub fn habitable(&self, a_m: f64) -> bool {
        a_m >= self.habitable_m.0 && a_m <= self.habitable_m.1
    }

    /// Total mass of ices beyond the snow line, Earth masses. What a system has to throw
    /// around: comets, the Kuiper analogue and the Oort cloud all come out of it.
    pub fn icy_reservoir(&self) -> f64 {
        self.solids_between(self.snow_m, self.outer_m)
    }
}

/// Radius at which a gray body reaches `temperature_k`, meters.
///
/// Zero albedo, which is the convention every other temperature here uses. Inverting
/// `T = T* sqrt(R*/2a)`.
pub fn radius_at(star: &Star, temperature_k: f64) -> f64 {
    let t = temperature_k.max(1.0);
    star.radius_m * star.teff_k * star.teff_k / (2.0 * t * t)
}

/// Equilibrium temperature at a radius, kelvin, zero albedo.
pub fn temperature_at(star: &Star, a_m: f64) -> f64 {
    star.teff_k * (star.radius_m / (2.0 * a_m.max(1.0))).sqrt()
}

/// Radius from mass, Earth units.
///
/// Piecewise power law anchored on Earth, Neptune and Jupiter, which is the shape the
/// exoplanet mass-radius relation actually has: rock compresses slowly, a volatile envelope
/// buys radius cheaply, and past a Jupiter mass degeneracy pressure takes the radius back
/// down. `icy` lifts a small body for a composition that is half water.
pub fn radius_earths(mass_earths: f64, icy: bool) -> f64 {
    const TERRAN_TOP: f64 = 2.04;
    const NEPTUNIAN_TOP: f64 = 132.0;
    let m = mass_earths.max(1.0e-6);
    let r = if m < TERRAN_TOP {
        m.powf(0.279)
    } else if m < NEPTUNIAN_TOP {
        1.2196 * (m / TERRAN_TOP).powf(0.541)
    } else {
        11.64 * (m / NEPTUNIAN_TOP).powf(-0.03)
    };
    // Ice is about half the density of rock, and only a body too small to hold an envelope
    // still shows it in its radius.
    if icy && m < TERRAN_TOP { r * 1.2 } else { r }
}

/// Escape velocity, m/s, from a mass and radius in Earth units.
pub fn escape_speed(mass_earths: f64, radius_earths: f64) -> f64 {
    (2.0 * G * mass_earths * EARTH_MASS / (radius_earths.max(1.0e-6) * EARTH_RADIUS)).sqrt()
}

/// Most probable speed of a molecule of `molar_g` at `temperature_k`, m/s.
pub fn thermal_speed(molar_g: f64, temperature_k: f64) -> f64 {
    const BOLTZMANN: f64 = 1.380_649e-23;
    const AMU: f64 = 1.660_539e-27;
    (2.0 * BOLTZMANN * temperature_k.max(1.0) / (molar_g * AMU)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{AuthoredStars, StarProvider};

    const AU: f64 = 1.495_978_707e11;

    fn sun_like() -> CatalogueStar {
        let mut s = AuthoredStars::sample().stars()[1].clone();
        s.star = Star::SOL;
        s.luminosity_solar = 1.0;
        s.mass_solar = 1.0;
        s.metallicity = 0.0;
        s
    }

    /// The one relation the whole geometry rests on, checked against the numbers it is
    /// supposed to reproduce: the snow line at 2.7 astronomical units and Earth at 278 K.
    #[test]
    fn the_temperature_curve_puts_the_snow_line_where_it_belongs() {
        assert!((radius_at(&Star::SOL, 170.0) / AU - 2.69).abs() < 0.05);
        assert!((temperature_at(&Star::SOL, AU) - 278.3).abs() < 0.5);
        // And the two are inverses.
        let r = radius_at(&Star::SOL, 300.0);
        assert!((temperature_at(&Star::SOL, r) - 300.0).abs() < 1.0e-9);
    }

    /// The optimistic habitable zone, which is what the default tuning asks for: recent Venus
    /// to early Mars, 0.75 to 1.77 astronomical units around the Sun.
    #[test]
    fn the_habitable_zone_lands_where_the_tuning_says() {
        let disc = Disc::of(&sun_like(), &Tuning::default());
        assert!((disc.habitable_m.0 / AU - 0.75).abs() < 0.05, "{}", disc.habitable_m.0 / AU);
        assert!((disc.habitable_m.1 / AU - 1.77).abs() < 0.08, "{}", disc.habitable_m.1 / AU);
        assert!(disc.habitable(AU) && !disc.habitable(5.0 * AU));
    }

    /// A dim star's disc is the Sun's scaled by its own light, everywhere at once. Nothing
    /// carries a separate red-dwarf case, and this is what says so.
    #[test]
    fn a_dim_star_gets_the_same_disc_pulled_inward() {
        let mut red = sun_like();
        red.star.radius_m = 0.15 * Star::SOL.radius_m;
        red.star.teff_k = 3200.0;
        red.mass_solar = 0.2;
        let (sun, dwarf) = (Disc::of(&sun_like(), &Tuning::default()), Disc::of(&red, &Tuning::default()));

        let ratio = dwarf.snow_m / sun.snow_m;
        assert!(ratio < 0.1, "a red dwarf's snow line is much closer in: {ratio}");
        for (a, b) in [
            (dwarf.inner_m, sun.inner_m),
            (dwarf.habitable_m.0, sun.habitable_m.0),
            (dwarf.outer_m, sun.outer_m),
        ] {
            // Every radius moves by the same factor, because they are one curve.
            assert!(((a / b) / ratio - 1.0).abs() < 1.0e-9, "{a} / {b} against {ratio}");
        }
    }

    /// Mass has to be conserved across the feeding zones, or a ladder invents or loses
    /// planets. Tiling the disc has to give the disc back.
    #[test]
    fn the_feeding_zones_tile_the_disc() {
        let disc = Disc::of(&sun_like(), &Tuning::default());
        let mut edge = disc.inner_m;
        let mut total = 0.0;
        while edge < disc.outer_m {
            let next = (edge * 1.3).min(disc.outer_m);
            total += disc.solids_between(edge, next);
            edge = next;
        }
        assert!((total / disc.solid_earths - 1.0).abs() < 1.0e-9, "{total} of {}", disc.solid_earths);
        assert_eq!(disc.solids_between(disc.outer_m * 2.0, disc.outer_m * 3.0), 0.0);
    }

    /// Most of the solid is past the snow line, which is the whole reason giants form out
    /// there and not in here.
    #[test]
    fn the_ice_step_puts_the_mass_outside() {
        let disc = Disc::of(&sun_like(), &Tuning::default());
        let inside = disc.solids_between(disc.inner_m, disc.snow_m);
        assert!(inside / disc.solid_earths < 0.15, "inner disc holds {inside} of {}", disc.solid_earths);
        assert!(disc.icy_reservoir() > 0.8 * disc.solid_earths);
    }

    /// Metals are the rock. A tenth of the metallicity is a tenth of the disc, and that is
    /// what makes a metal-rich star worth flying to.
    #[test]
    fn metallicity_sets_the_mass_budget() {
        let mut poor = sun_like();
        poor.metallicity = -1.0;
        // Same seed, so the lognormal scatter is identical and only the metals differ.
        let (rich, poor) = (Disc::of(&sun_like(), &Tuning::default()), Disc::of(&poor, &Tuning::default()));
        assert!((poor.solid_earths / rich.solid_earths - 0.1).abs() < 1.0e-9);
    }

    /// The mass-radius relation is anchored on three real planets, and this is the anchoring.
    #[test]
    fn mass_and_radius_agree_with_the_planets_they_were_fitted_to() {
        assert!((radius_earths(1.0, false) - 1.0).abs() < 1.0e-9, "Earth");
        assert!((radius_earths(17.15, false) - 3.86).abs() < 0.05, "Neptune");
        assert!((radius_earths(317.8, false) - 11.21).abs() < 0.2, "Jupiter");
        assert!((radius_earths(95.2, false) - 9.14).abs() < 0.7, "Saturn");
        // Monotonic up to the Jovian branch, and turning over past it.
        assert!(radius_earths(300.0, false) > radius_earths(1500.0, false));
        assert!(radius_earths(0.5, true) > radius_earths(0.5, false));
    }

    /// Earth's own numbers, which is what the retention test is calibrated against: it loses
    /// hydrogen and keeps nitrogen, and the same two lines have to say so.
    #[test]
    fn escape_and_thermal_speeds_put_hydrogen_off_earth_and_nitrogen_on_it() {
        let v_esc = escape_speed(1.0, 1.0);
        assert!((v_esc - 11_180.0).abs() < 50.0, "{v_esc} m/s");
        let exosphere = 278.3 * Tuning::default().world.exosphere_factor;
        let held = |molar: f64| v_esc > Tuning::default().world.retention * thermal_speed(molar, exosphere);
        assert!(!held(2.0), "Earth has no hydrogen envelope");
        assert!(held(28.0), "and does have nitrogen");
        assert!(held(18.0), "and water");
    }
}
