//! What an airless rocky world looks like from orbit, from what it is.
//!
//! The counterpart of [`crate::climate`] for bodies with nothing to weather them. Such a
//! surface is a record of impacts, so what varies is how many, how long ago, and how much of
//! the oldest record lava has since buried. Three series of craters stand for that history:
//! an ancient one that saturates any old surface, a later one that lands on the lava too, and a
//! fresh one whose ejecta has not yet darkened. The bodies anybody has been to are measured
//! instead, for the reason [`crate::worlds`] gives: no rule reaches Io.
//!
//! Display quantities, as a climate's are: what they have to get right is the ordering -- a
//! bigger body kept its heat longer and flooded more, a hotter one is darker -- and the look.

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

/// An airless rocky body's paint: measured where anybody has been, derived otherwise. `None`
/// for anything with air or with ice on top, which [`crate::climate`] or the body's class
/// paints instead.
pub fn of(id: &str, world: &World, surface: Surface, radius_m: f64) -> Option<Airless> {
    if world.atmosphere != Atmosphere::None || world.top != Top::Rock {
        return None;
    }
    if matches!(surface, Surface::GasGiant | Surface::IceGiant) {
        return None;
    }
    measured(id).or_else(|| Some(derived(surface, radius_m, crate::climate::variety(id))))
}

/// Share of the surface flooded, from the radius: lava needs heat, and a small body lost its
/// own before the ancient bombardment ended. Nothing under 800 km; the Moon's 1737 km gives it
/// a few to twenty per cent, and a Mercury up to a third.
fn maria_share(radius_m: f64, draw: f32) -> f32 {
    let size = ((radius_m / 1.0e3 - 800.0) / 2000.0).clamp(0.0, 1.0) as f32;
    size * (0.08 + 0.3 * draw)
}

pub fn derived(surface: Surface, radius_m: f64, variety: crate::climate::Variety) -> Airless {
    let [a, b, c, d] = variety.0;
    // Close to a star, rock is baked dark and flat, and ejecta darkens quickly.
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
        // Resurfaced faster than anything can crater it: sulfur plains and dark volcanic
        // centers, and not a crater on it.
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
        assert!(of("Luna", world("Luna"), Surface::Weathered, 1.737e6).is_some());
        assert!(of("Mercury", world("Mercury"), Surface::Scorched, 2.44e6).is_some());
        assert!(of("Mars", world("Mars"), Surface::Weathered, 3.39e6).is_none(), "Mars has air");
        assert!(of("Europa", world("Europa"), Surface::Ice, 1.56e6).is_none(), "Europa is ice");
        let generated = crate::worlds::of("Kettle b", Surface::Rock, &[]);
        assert!(of("Kettle b", &generated, Surface::Rock, 5.0e5).is_some());
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
            let p = of(id, &crate::worlds::of(id, Surface::Rock, &[]), Surface::Rock, 2.0e6).unwrap();
            assert!(p.mare[0] < p.highland[0] && p.highland[0] < p.ejecta[0], "{id}: {p:?}");
            assert!(p.craters.windows(2).all(|w| w[1] <= w[0]), "{id}: the old series is the densest");
        }
    }

    #[test]
    fn a_scorched_world_is_darker() {
        let v = crate::climate::variety("x");
        assert!(derived(Surface::Scorched, 2.0e6, v).highland[0] < derived(Surface::Rock, 2.0e6, v).highland[0]);
    }
}
