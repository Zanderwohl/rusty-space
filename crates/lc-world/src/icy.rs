//! What an airless icy world looks like from orbit, from what it is.
//!
//! The counterpart of [`crate::airless`] for ice: three series of craters, a dark lag of rock and
//! organics on the old ground, young plains where tidal heat resurfaced it, and lineae along the
//! cracks. Measured where anybody has been.
//!
//! Display quantities, like [`crate::airless`]: what has to be right is the ordering -- a hotter
//! moon is younger and more cracked, a rockier one darker, a colder one redder -- and the look.

use crate::surface::Surface;
use crate::worlds::{Atmosphere, Top, World};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icy {
    /// How densely each series cratered the surface, oldest first, `[0, 1]`.
    pub craters: [f32; 3],
    /// Share of the surface that is young plains, `[0, 1]`. The oldest series is erased there.
    pub resurfaced: f32,
    /// Share of the old ground under the dark lag, `[0, 1]`.
    pub lag: f32,
    /// How strongly the lineae show, `[0, 1]`.
    pub fractures: f32,
    /// How bright fresh ejecta is over the ground it lands on, `[0, 1]`.
    pub rays: f32,
    /// Oklch `(L, C, hue in degrees)` of clean old ice.
    pub frost: [f32; 3],
    /// Of the lag.
    pub dark: [f32; 3],
    /// Of the young plains.
    pub plains: [f32; 3],
    /// Of the lineae.
    pub lineae: [f32; 3],
    /// Of fresh ejecta.
    pub ejecta: [f32; 3],
}

/// The generator's statement of how hard tides heat a moon, `[0, 1]`: see [`tidal_heat`].
pub const HEAT: &str = "Heat:";

pub fn heat_tag(heat: f64) -> String {
    format!("{HEAT}{heat:.3}")
}

pub fn heat_of(tags: &[String]) -> Option<f64> {
    tags.iter().find_map(|t| t.strip_prefix(HEAT)?.parse().ok())
}

/// Tidal heat flow through a moon's surface, on a log scale from nothing to Io's, `[0, 1]`.
///
/// Flux scales as `M^2.5 R^3 e^2 / a^7.5` (dissipated power over area, with the mean motion
/// written out). Io is 1, Europa about 0.66, Enceladus 0.37 and Ganymede 0: three decades
/// below Io is nothing that shows.
pub fn tidal_heat(planet_mass_kg: f64, radius_m: f64, eccentricity: f64, semi_major_m: f64) -> f64 {
    let flux = |m: f64, r: f64, e: f64, a: f64| m.powf(2.5) * r.powi(3) * e * e / a.powf(7.5);
    let io = flux(1.898e27, 1.8216e6, 0.0041, 4.217e8);
    let this = flux(planet_mass_kg, radius_m, eccentricity, semi_major_m);
    if this <= 0.0 || !this.is_finite() {
        return 0.0;
    }
    (1.0 + (this / io).log10() / 3.0).clamp(0.0, 1.0)
}

/// `None` for anything with air or without ice on top.
pub fn of(id: &str, world: &World, surface: Surface, radius_m: f64, mass_kg: f64, equilibrium_k: f64, tags: &[String]) -> Option<Icy> {
    if world.atmosphere != Atmosphere::None || world.top != Top::Ice || surface.is_banded() {
        return None;
    }
    let volume = 4.0 / 3.0 * std::f64::consts::PI * radius_m.powi(3);
    let density = if volume > 0.0 { mass_kg / volume } else { 0.0 };
    let heat = heat_of(tags).unwrap_or(0.0);
    measured(id).or_else(|| Some(derived(radius_m, density, equilibrium_k, heat, crate::climate::variety(id))))
}

pub fn derived(radius_m: f64, density: f64, equilibrium_k: f64, heat: f64, variety: crate::climate::Variety) -> Icy {
    let [a, b, c, d] = variety.0;
    let km = (radius_m / 1.0e3) as f32;
    // A small moon froze through before anything could well up.
    let size = ((km - 150.0) / 450.0).clamp(0.0, 1.0);
    // A Ganymede kept enough of its early heat to resurface without tides.
    let big = ((km - 1000.0) / 1000.0).clamp(0.0, 1.0);
    let heat = heat as f32;
    let tidal = ((heat - 0.2) / 0.4).clamp(0.0, 1.0);
    let resurfaced = (size * tidal).max(0.7 * big * a).min(1.0);
    let fractures = (size * ((heat - 0.3) / 0.35).clamp(0.0, 1.0)).max(0.4 * big * a);
    let rock = (((density - 1200.0) / 1000.0) as f32).clamp(0.0, 1.0);
    let lag = rock * (0.4 + 0.6 * b);
    // Radiation turns methane frost red-brown, which needs it cold enough to keep methane.
    let tholin = (((70.0 - equilibrium_k) / 40.0) as f32).clamp(0.0, 1.0) * (0.4 + 0.6 * c);
    let frost_l = 0.84 + 0.08 * (1.0 - rock) - 0.08 * tholin;
    let (chroma, hue) = if tholin > 0.15 { (0.01 + 0.06 * tholin, 40.0 + 15.0 * d) } else { (0.008 + 0.01 * d, 210.0 + 30.0 * d) };
    Icy {
        craters: [0.9 + 0.1 * b, (0.25 + 0.3 * c) * (1.0 - 0.6 * tidal), 0.08 + 0.2 * d],
        resurfaced,
        lag,
        fractures,
        rays: 0.5 + 0.3 * c,
        frost: [frost_l, chroma, hue],
        dark: [0.42 + 0.06 * d, 0.02 + 0.03 * tholin, 55.0 + 20.0 * c],
        plains: [(frost_l + 0.02).min(0.97), 0.8 * chroma, hue - 10.0],
        lineae: [frost_l - 0.28, 0.05, 40.0 + 10.0 * d],
        ejecta: [(frost_l + 0.05).min(0.97), 0.5 * chroma, hue],
    }
}

fn measured(id: &str) -> Option<Icy> {
    Some(match id {
        // Old dark terrain over most of it, cut by bright grooved terrain.
        "Ganymede" => Icy {
            craters: [1.0, 0.4, 0.2],
            resurfaced: 0.6,
            lag: 0.95,
            fractures: 0.35,
            rays: 0.7,
            frost: [0.82, 0.012, 230.0],
            dark: [0.50, 0.02, 65.0],
            plains: [0.80, 0.010, 220.0],
            lineae: [0.70, 0.015, 220.0],
            ejecta: [0.92, 0.008, 230.0],
        },
        // Saturated with craters and dark between them; the bright spots are their ejecta.
        "Callisto" => Icy {
            craters: [1.0, 0.5, 0.25],
            resurfaced: 0.0,
            lag: 0.85,
            fractures: 0.0,
            rays: 0.6,
            frost: [0.80, 0.010, 230.0],
            dark: [0.42, 0.02, 60.0],
            plains: [0.80, 0.010, 230.0],
            lineae: [0.80, 0.010, 230.0],
            ejecta: [0.90, 0.008, 230.0],
        },
        // Young everywhere, with almost no craters, and crossed by red-brown lineae.
        "Europa" => Icy {
            craters: [0.0, 0.05, 0.05],
            resurfaced: 1.0,
            lag: 0.0,
            fractures: 1.0,
            rays: 0.3,
            frost: [0.90, 0.015, 80.0],
            dark: [0.60, 0.04, 50.0],
            plains: [0.90, 0.012, 85.0],
            lineae: [0.58, 0.06, 45.0],
            ejecta: [0.94, 0.01, 85.0],
        },
        // The brightest thing in the solar system: cratered in the north, smooth and striped in
        // the south.
        "Enceladus" => Icy {
            craters: [0.6, 0.2, 0.05],
            resurfaced: 0.5,
            lag: 0.0,
            fractures: 0.7,
            rays: 0.2,
            frost: [0.97, 0.005, 230.0],
            dark: [0.80, 0.01, 230.0],
            plains: [0.98, 0.005, 220.0],
            lineae: [0.85, 0.02, 230.0],
            ejecta: [0.98, 0.004, 230.0],
        },
        // Nitrogen frost over young terrain, pink, with dark streaks.
        "Triton" => Icy {
            craters: [0.0, 0.1, 0.05],
            resurfaced: 0.9,
            lag: 0.2,
            fractures: 0.3,
            rays: 0.3,
            frost: [0.86, 0.03, 40.0],
            dark: [0.55, 0.04, 50.0],
            plains: [0.88, 0.025, 30.0],
            lineae: [0.70, 0.04, 40.0],
            ejecta: [0.90, 0.02, 40.0],
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate::variety;

    #[test]
    fn io_is_one_and_europa_and_ganymede_fall_in_order() {
        let jupiter = 1.898e27;
        let io = tidal_heat(jupiter, 1.8216e6, 0.0041, 4.217e8);
        let europa = tidal_heat(jupiter, 1.5608e6, 0.009, 6.709e8);
        let ganymede = tidal_heat(jupiter, 2.634e6, 0.0013, 1.0704e9);
        assert!((io - 1.0).abs() < 0.01, "{io}");
        assert!((0.55..0.75).contains(&europa), "{europa}");
        assert!(ganymede < 0.05, "{ganymede}");
        assert_eq!(tidal_heat(jupiter, 1.0e6, 0.0, 1.0e9), 0.0);
        assert_eq!(heat_of(&[heat_tag(europa)]).map(|h| (h * 1000.0).round()), Some((europa * 1000.0).round()));
    }

    #[test]
    fn only_bare_ice_is_icy() {
        let europa = crate::worlds::for_body("Europa").unwrap();
        assert!(of("Europa", europa, Surface::Ice, 1.56e6, 4.8e22, 102.0, &[]).is_some());
        let luna = crate::worlds::for_body("Luna").unwrap();
        assert!(of("Luna", luna, Surface::Weathered, 1.737e6, 7.35e22, 270.0, &[]).is_none());
        let generated = crate::worlds::of("Kettle b I", Surface::Ice, &[], None);
        assert!(of("Kettle b I", &generated, Surface::Ice, 6.0e5, 1.7e21, 90.0, &[]).is_some());
    }

    #[test]
    fn tides_resurface_and_crack_a_moon_large_enough() {
        for id in ["a", "b", "c", "d", "e"] {
            let v = variety(id);
            let cold = derived(8.0e5, 1600.0, 100.0, 0.0, v);
            let hot = derived(8.0e5, 1600.0, 100.0, 0.7, v);
            assert!(hot.resurfaced > cold.resurfaced + 0.5, "{id}: {hot:?}");
            assert!(hot.fractures > cold.fractures + 0.5, "{id}");
            assert!(hot.craters[1] < cold.craters[1], "{id}");
            let small = derived(1.0e5, 1600.0, 100.0, 0.7, v);
            assert_eq!((small.resurfaced, small.fractures), (0.0, 0.0), "{id}: too small to well up");
        }
    }

    #[test]
    fn rockier_is_darker_and_colder_is_redder() {
        for id in ["a", "b", "c", "d", "e"] {
            let v = variety(id);
            let clean = derived(5.0e5, 1100.0, 100.0, 0.0, v);
            let dirty = derived(5.0e5, 2200.0, 100.0, 0.0, v);
            assert!(dirty.lag > clean.lag && dirty.frost[0] < clean.frost[0], "{id}");
            let far = derived(5.0e5, 1600.0, 35.0, 0.0, v);
            assert!(far.frost[1] > clean.frost[1] && far.frost[2] < 90.0, "{id}: {far:?}");
        }
    }

    #[test]
    fn the_lag_is_darker_than_ice_and_ejecta_brighter() {
        for id in ["a", "b", "c", "d", "e", "Europa", "Ganymede", "Callisto", "Enceladus", "Triton"] {
            let p = measured(id).unwrap_or_else(|| derived(1.0e6, 1800.0, 90.0, 0.3, variety(id)));
            assert!(p.dark[0] < p.frost[0] && p.frost[0] <= p.ejecta[0], "{id}: {p:?}");
            assert!(p.fractures == 0.0 || p.lineae[0] < p.plains[0], "{id}");
        }
    }
}
