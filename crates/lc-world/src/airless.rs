//! What an airless rocky world looks like from orbit, from what it is.
//!
//! The counterpart of [`crate::climate`] for bodies without air: three series of craters, oldest
//! first, with lava flooding the oldest. Measured where anybody has been; no rule reaches Io.
//!
//! Display quantities: what has to be right is the ordering -- a bigger body flooded more, a
//! hotter one is darker -- and the look.

use crate::surface::Surface;
use crate::worlds::{Atmosphere, Top, World};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Airless {
    /// How densely each series cratered the surface, oldest first, `[0, 1]`: one saturates it.
    pub craters: [f32; 3],
    /// Share of the surface lava has flooded, `[0, 1]`, between the ancient series and the later.
    pub maria: f32,
    /// How bright fresh ejecta is over the ground it lands on, `[0, 1]`.
    pub rays: f32,
    /// Oklch `(L, C, hue in degrees)` of the old cratered ground at its brightest.
    pub highland: [f32; 3],
    /// Of the lava plains.
    pub mare: [f32; 3],
    /// Of fresh ejecta.
    pub ejecta: [f32; 3],
}

/// `None` for anything with air or with ice on top.
pub fn of(id: &str, world: &World, surface: Surface, radius_m: f64, equilibrium_k: f64, tags: &[String]) -> Option<Airless> {
    if world.atmosphere != Atmosphere::None || world.top != Top::Rock {
        return None;
    }
    if matches!(surface, Surface::GasGiant | Surface::IceGiant) {
        return None;
    }
    let variety = crate::climate::variety(id);
    if crate::worlds::Stated::captured(tags) {
        return measured(id).or(Some(rubble(variety)));
    }
    measured(id).or_else(|| Some(molten(derived(surface, radius_m, variety), equilibrium_k)))
}

/// Where rock starts to melt at the surface, kelvin: basalt's solidus, a little over 1,000 K,
/// reached across the day side somewhat below it.
const MELT_K: f64 = 900.0;
/// Where the day side is all melt.
const MOLTEN_K: f64 = 1500.0;

/// The same world close enough to its star that its surface melts: lava spreads over the
/// craters until, past [`MOLTEN_K`], nothing is left but dark glassy rock and melt. Its glow is
/// the body's own blackbody light, which the client already draws.
pub fn molten(a: Airless, equilibrium_k: f64) -> Airless {
    let melt = ((equilibrium_k - MELT_K) / (MOLTEN_K - MELT_K)).clamp(0.0, 1.0) as f32;
    if melt <= 0.0 {
        return a;
    }
    let [ancient, later, fresh] = a.craters;
    let keep = 1.0 - 0.85 * melt;
    let mix = |x: f32, y: f32| x + (y - x) * melt;
    Airless {
        craters: [ancient, later * keep, fresh * keep],
        maria: mix(a.maria, 1.0),
        rays: a.rays * keep,
        mare: [mix(a.mare[0], 0.30), mix(a.mare[1], 0.012), mix(a.mare[2], 35.0)],
        ..a
    }
}

/// What a planet catches: carbon-rich rubble, cratered at every scale and never flooded.
///
/// L 0.48 against Luna's 0.66 is the ratio of their albedos, 0.05 to 0.12, taken in linear light.
pub fn rubble(variety: crate::climate::Variety) -> Airless {
    let [a, b, c, d] = variety.0;
    let l = 0.48 + 0.06 * (b - 0.5);
    // Neutral gray C-type to the redder D-type.
    let (chroma, hue) = (0.005 + 0.012 * a, 60.0 + 25.0 * a);
    // Ejecta stays bright where an impact digs through the dark lag to cleaner material below,
    // which is most of what shows Phoebe's craters at all.
    Airless {
        craters: [1.0, 0.6 + 0.2 * c, 0.3 + 0.2 * d],
        maria: 0.0,
        rays: 0.3 + 0.15 * c,
        highland: [l, chroma, hue],
        mare: [l - 0.06, 0.004, hue],
        ejecta: [l + 0.16, 0.5 * chroma, hue],
    }
}

/// Lava needs heat, which a small body lost early: nothing under 800 km, a few to twenty per cent
/// at the Moon's 1737 km, up to a third at Mercury's.
fn maria_share(radius_m: f64, draw: f32) -> f32 {
    let size = ((radius_m / 1.0e3 - 800.0) / 2000.0).clamp(0.0, 1.0) as f32;
    size * (0.08 + 0.3 * draw)
}

pub fn derived(surface: Surface, radius_m: f64, variety: crate::climate::Variety) -> Airless {
    let [a, b, c, d] = variety.0;
    // Close to a star rock is darker, and ejecta darkens faster.
    let scorched = surface == Surface::Scorched;
    let highland_l = if scorched { 0.55 } else { 0.62 } + 0.08 * (b - 0.5);
    let hue = 72.0 + 24.0 * (d - 0.5);
    // Titanium-rich lava is bluish, iron-rich brownish.
    let mare_hue = if a < 0.5 { 250.0 } else { 60.0 };
    Airless {
        craters: [0.8 + 0.2 * b, 0.2 + 0.3 * c, 0.08 + 0.2 * d],
        maria: maria_share(radius_m, a),
        rays: if scorched { 0.35 } else { 0.5 + 0.4 * c },
        highland: [highland_l, 0.008 + 0.012 * c, hue],
        mare: [highland_l - 0.14 - 0.06 * a, 0.007, mare_hue],
        ejecta: [(highland_l + 0.2).min(0.9), 0.006, hue],
    }
}

fn measured(id: &str) -> Option<Airless> {
    Some(match id {
        // Maria are a sixth of the surface, nearly all of it on the near side.
        "Luna" => Airless {
            craters: [1.0, 0.35, 0.2],
            maria: 0.16,
            rays: 0.75,
            highland: [0.66, 0.014, 75.0],
            mare: [0.47, 0.007, 255.0],
            ejecta: [0.86, 0.008, 80.0],
        },
        // The smooth plains are more than a quarter of it and barely darker than the rest.
        "Mercury" => Airless {
            craters: [1.0, 0.45, 0.2],
            maria: 0.27,
            rays: 0.6,
            highland: [0.58, 0.012, 70.0],
            mare: [0.52, 0.010, 65.0],
            ejecta: [0.80, 0.008, 75.0],
        },
        // Resurfaced faster than anything craters it.
        "Io" => Airless {
            craters: [0.0, 0.0, 0.0],
            maria: 0.25,
            rays: 0.0,
            highland: [0.84, 0.10, 95.0],
            mare: [0.50, 0.06, 45.0],
            ejecta: [0.90, 0.04, 90.0],
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(id: &str) -> &'static World {
        crate::worlds::for_body(id).unwrap()
    }

    #[test]
    fn only_bare_rock_is_airless() {
        assert!(of("Luna", world("Luna"), Surface::Weathered, 1.737e6, 250.0, &[]).is_some());
        assert!(of("Mercury", world("Mercury"), Surface::Scorched, 2.44e6, 250.0, &[]).is_some());
        assert!(of("Mars", world("Mars"), Surface::Weathered, 3.39e6, 250.0, &[]).is_none(), "Mars has air");
        assert!(of("Europa", world("Europa"), Surface::Ice, 1.56e6, 250.0, &[]).is_none(), "Europa is ice");
        let generated = crate::worlds::of("Kettle b", Surface::Rock, &[], None);
        assert!(of("Kettle b", &generated, Surface::Rock, 5.0e5, 250.0, &[]).is_some());
    }

    #[test]
    fn a_small_body_never_flooded_and_a_large_one_did() {
        for id in ["a", "b", "c", "d", "e"] {
            let v = crate::climate::variety(id);
            assert_eq!(derived(Surface::Rock, 4.0e5, v).maria, 0.0, "{id}");
            assert!(derived(Surface::Rock, 3.0e6, v).maria > derived(Surface::Rock, 1.7e6, v).maria, "{id}");
        }
    }

    #[test]
    fn maria_are_darker_than_highland_and_ejecta_brighter() {
        for id in ["a", "b", "c", "d", "e", "Luna", "Mercury"] {
            let p = of(id, &crate::worlds::of(id, Surface::Rock, &[], None), Surface::Rock, 2.0e6, 250.0, &[]).unwrap();
            assert!(p.mare[0] < p.highland[0] && p.highland[0] < p.ejecta[0], "{id}: {p:?}");
            assert!(p.craters.windows(2).all(|w| w[1] <= w[0]), "{id}: the old series is the densest");
        }
    }

    #[test]
    fn captured_rubble_is_darker_than_any_rock() {
        use crate::worlds::{Atmosphere, Stated, Top};
        let mut tags = Stated::tags(Atmosphere::None, Top::Rock, false);
        tags.push(Stated::CAPTURED.to_string());
        let luna = of("Luna", world("Luna"), Surface::Weathered, 1.737e6, 250.0, &[]).unwrap();
        for id in ["a", "b", "c", "d", "e"] {
            let caught = crate::worlds::of(id, Surface::Ice, &tags, None);
            let grown = crate::worlds::of(id, Surface::Ice, &tags[..2], None);
            assert!(caught.gray_albedo() < 0.08, "{id}: {}", caught.gray_albedo());
            assert!(caught.gray_albedo() < 0.6 * grown.gray_albedo(), "{id}");
            let rubble = of(id, &caught, Surface::Ice, 3.0e4, 80.0, &tags).expect("caught moons are airless");
            assert_eq!(rubble.maria, 0.0);
            assert!(rubble.highland[0] < luna.highland[0] - 0.1, "{id}: {rubble:?}");
            assert!(rubble.mare[0] < rubble.highland[0] && rubble.highland[0] < rubble.ejecta[0]);
        }
    }

    #[test]
    fn heat_floods_the_craters_with_dark_lava() {
        for id in ["a", "b", "c", "d", "e"] {
            let rock = crate::worlds::of(id, Surface::Scorched, &[], None);
            let at = |k| of(id, &rock, Surface::Scorched, 6.0e6, k, &[]).unwrap();
            let (cool, warm, hot) = (at(600.0), at(1200.0), at(1600.0));
            assert_eq!(cool, derived(Surface::Scorched, 6.0e6, crate::climate::variety(id)), "{id}: under the melt");
            assert!(cool.maria < warm.maria && warm.maria < hot.maria, "{id}");
            assert_eq!(hot.maria, 1.0, "{id}: all melt");
            assert!(hot.craters[2] < 0.3 * cool.craters[2], "{id}");
            assert!(hot.mare[0] < hot.highland[0], "{id}");
        }
        let io = crate::worlds::for_body("Io").unwrap();
        assert_eq!(of("Io", io, Surface::Rock, 1.82e6, 1600.0, &[]), measured("Io"), "measured wins");
    }

    #[test]
    fn a_scorched_world_is_darker() {
        let v = crate::climate::variety("x");
        assert!(derived(Surface::Scorched, 2.0e6, v).highland[0] < derived(Surface::Rock, 2.0e6, v).highland[0]);
    }
}
