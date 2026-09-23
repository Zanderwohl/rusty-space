//! What a rocky world with air looks like from orbit, from what it is.
//!
//! Earth and Mars are two outputs of one function rather than two portraits. How much of the
//! surface is sea follows from how much water the world has, how much is ice from how cold it
//! is and how much water there is to freeze, how green it is from whether it is habitable, and
//! its clouds and sky from all three. The bodies anybody has been to are measured instead, for
//! the reason [`crate::worlds`] gives: no rule reaches Venus.
//!
//! These are display quantities. What they have to get right is the ordering -- a wetter world
//! has more sea, a colder one more ice, a thin atmosphere a fainter limb -- and the look; an
//! atmosphere's scale height is exaggerated about twentyfold so a limb shows at the distances a
//! ship sees planets from. Nothing a survey measures reads them.

use crate::worlds::{Atmosphere, Top, World};

/// Where the paint of a rocky world with air comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    /// Share of the surface under sea, `[0, 1]`, before any of it freezes.
    pub ocean: f32,
    /// Share of the surface under ice, `[0, 1]`, counted in from the poles.
    pub ice: f32,
    /// Share of the wet land that is green, `[0, 1]`.
    pub life: f32,
    /// How oxidized bare ground is: gray basalt at 0, Mars at 1.
    pub rust: f32,
    /// Share of the desert that is pale sorted sand, `[0, 1]`; it takes wind and water to make.
    pub sand: f32,
    /// Widens the desert belts, `[-0.3, 0.3]`.
    pub aridity: f32,
    /// More dark basaltic provinces when positive, `[-0.3, 0.3]`.
    pub dark: f32,
    pub clouds: Clouds,
    pub air: Air,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    /// Added to the deck's drive. Zero is Earth's cover, a fifth below is scattered wisps, and
    /// one is a deck with no break in it.
    pub cover: f32,
    /// What share of the deck's alpha is drawn. Above one closes the gaps.
    pub opacity: f32,
    /// Multiplies the deck's gray, linear: white for water cloud.
    pub tint: [f32; 3],
}

/// Single scattering, in two parts: gas, which scatters as the inverse fourth power of the
/// wavelength and so is blue, and haze, which scatters forward in whatever color it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    /// Vertical optical depth of the gas, in (R, G, B).
    pub gas: [f32; 3],
    /// Vertical optical depth of the haze, the same in every channel.
    pub haze: f32,
    /// What share of the light the haze meets it scatters rather than absorbs, per channel:
    /// the haze's color. Mars's dust eats blue.
    pub haze_albedo: [f32; 3],
    /// Scale height as a share of the radius. See the module doc.
    pub height: f32,
}

/// Vertical optical depth of an Earth's worth of nitrogen, per channel at 680, 550 and 440 nm.
const EARTH_GAS: [f32; 3] = [0.045, 0.1, 0.24];

/// The water tag the generator writes, since a share of the mass is not something a class says.
pub const WATER: &str = "Water:";

/// Earth's water, as a share of its mass: the calibration for how much of a surface is sea.
pub const EARTH_WATER: f64 = 2.3e-4;

/// The generator's statement of a world's water, as a tag.
pub fn water_tag(fraction: f64) -> String {
    format!("{WATER}{fraction:e}")
}

pub fn water_of(tags: &[String]) -> Option<f64> {
    tags.iter().find_map(|t| t.strip_prefix(WATER)?.parse().ok())
}

/// A rocky world's paint: measured where anybody has been, derived otherwise. `None` for a body
/// without air or without a surface, which keeps its class's pattern.
pub fn of(id: &str, world: &World, equilibrium_k: f64, tags: &[String]) -> Option<Climate> {
    if let Some(known) = measured(id) {
        return Some(known);
    }
    derived(world.atmosphere, world.top, equilibrium_k, water_of(tags), variety(id))
}

/// Per-body departures from the type, keyed by id so a world looks the same on every approach.
/// Each is uniform in `[0, 1]`.
#[derive(Clone, Copy, Debug)]
pub struct Variety([f32; 4]);

pub fn variety(id: &str) -> Variety {
    // FNV-1a, as `worlds::varied` uses, salted apart from it.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    Variety(std::array::from_fn(|k| crate::rng::uniform(crate::rng::hash(&[h, 0xc11, k as u64])) as f32))
}

/// Mean surface temperature, kelvin: zero-albedo equilibrium plus what the air adds. Earth's
/// 278 becomes 288, and a thin atmosphere over bright ground comes out below equilibrium.
pub fn surface_k(atmosphere: Atmosphere, equilibrium_k: f64) -> f64 {
    equilibrium_k
        + match atmosphere {
            Atmosphere::Thick => 10.0,
            Atmosphere::Thin => -15.0,
            Atmosphere::None | Atmosphere::Envelope => 0.0,
        }
}

fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Share of a surface the sea covers, from the water's share of the mass. Logistic in the
/// logarithm, through Earth's 71 per cent at Earth's water: a tenth of it is an archipelago
/// and ten times it drowns everything.
pub fn sea_share(water_fraction: f64) -> f64 {
    if water_fraction <= 0.0 {
        return 0.0;
    }
    logistic((water_fraction.log10() - EARTH_WATER.log10()) / 0.35 + 0.9)
}

/// The most of the surface its water could whiten if all of it froze. Ice spread as frost goes
/// further than the same water as sea, which is why Mars has caps and no sea.
fn ice_capacity(water_fraction: f64) -> f64 {
    if water_fraction <= 0.0 {
        return 0.0;
    }
    logistic((water_fraction.log10() + 4.3) / 0.35)
}

/// Share of the surface cold enough for ice to last, from the mean surface temperature. Earth's
/// 288 K gives about a tenth; below 250 K it reaches the equator.
fn cold_share(surface_k: f64) -> f64 {
    ((300.0 - surface_k) / 50.0).clamp(0.0, 1.0).powf(1.6)
}

/// From what the generator decided. The rules only have to reach the worlds it makes: a
/// habitable one is always a sea under thick or thin air.
pub fn derived(
    atmosphere: Atmosphere,
    top: Top,
    equilibrium_k: f64,
    water_fraction: Option<f64>,
    variety: Variety,
) -> Option<Climate> {
    if matches!(atmosphere, Atmosphere::None | Atmosphere::Envelope) {
        return None;
    }
    let Variety([a, b, c, d]) = variety;
    // A sea means the greenhouse is holding it liquid, whatever the equilibrium says: that is
    // what the generator's habitable zone stands for. Half as steep as the equilibrium, through
    // Earth, and never so cold the equator freezes.
    let t = if top == Top::Ocean {
        (288.0 + 0.5 * (equilibrium_k - 278.0)).max(262.0)
    } else {
        surface_k(atmosphere, equilibrium_k)
    };
    let water = water_fraction.unwrap_or(match top {
        Top::Ocean => EARTH_WATER,
        Top::Ice => 0.3,
        Top::Rock | Top::Cloud => 0.0,
    });
    let thick = atmosphere == Atmosphere::Thick;
    let air_scale = if thick { 0.6 + 0.8 * a } else { 0.08 + 0.12 * a };
    let gas = EARTH_GAS.map(|g| g * air_scale);

    if top == Top::Cloud {
        // Venus where it is warm and Titan's orange where it is not: one deck with no break.
        let warm = t > 200.0;
        let (tint, haze_albedo) = if warm {
            ([1.0, 0.92, 0.7], [0.98, 0.92, 0.72])
        } else {
            ([0.68, 0.42, 0.16], [0.95, 0.62, 0.28])
        };
        return Some(Climate {
            ocean: 0.0,
            ice: 0.0,
            life: 0.0,
            rust: 0.5,
            sand: 0.0,
            aridity: 0.3,
            dark: 0.0,
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint },
            // The haze above the deck, not the deck: the deck is the ground here, and a haze
            // as deep as the whole cloud would put out the light before the terminator.
            air: Air { gas, haze: 0.25 + 0.5 * b, haze_albedo, height: 0.035 },
        });
    }

    let ocean = sea_share(water) as f32;
    let ice = cold_share(t).min(ice_capacity(water)).max(f64::from(top == Top::Ice) * 0.6) as f32;
    let ice = if top == Top::Ice && t < 230.0 { 1.0 } else { ice };
    // Green wherever there is sea to rain on the land and warmth enough to grow, most at 290 K.
    let temperate = (-((t - 290.0) / 20.0).powi(2)).exp() as f32;
    let life = if top == Top::Ocean { temperate * (0.45 + 0.55 * b) } else { 0.0 };
    let wet = ocean > 0.05 && t > 250.0;
    // Oxidation wants water and air, and red ground wants it to have gone. A dry world with
    // thin air is Mars; with a sea, rust is what the deserts are.
    let rust = match (wet, top) {
        (true, _) => 0.1 + 0.3 * c,
        // What dusts an ice world's ice, which is mostly clean.
        (false, Top::Ice) => 0.1 + 0.5 * c,
        (false, _) => 0.55 + 0.45 * c,
    };
    let sand = if wet { 0.25 + 0.4 * d } else { 0.15 * d };
    let aridity = (0.3 * (0.6 - ocean) + 0.1 * (d - 0.5)).clamp(-0.3, 0.3);
    let dark = 0.3 * (a - 0.5);

    let clouds = if wet {
        Clouds { cover: 0.25 * (ocean - 0.71) + 0.06 * (b - 0.5), opacity: 1.0, tint: [1.0; 3] }
    } else if ocean > 0.0 || ice > 0.0 {
        // Frozen: what the ice sublimates.
        Clouds { cover: -0.13, opacity: 0.6, tint: [0.97, 0.98, 1.02] }
    } else {
        Clouds { cover: -0.18, opacity: 0.45, tint: [0.95, 0.97, 1.0] }
    };

    let (haze, haze_albedo) = if thick && t < 150.0 {
        // Thick and cold is Titan's chemistry, if not yet its deck: a tholin haze.
        (0.2 + 0.4 * c, [0.95, 0.66, 0.34])
    } else if wet || thick {
        (0.03, [0.9, 0.9, 0.9])
    } else {
        // Dust. It is why Mars's sky is butterscotch.
        (0.06 + 0.12 * c, [0.95, 0.66, 0.42])
    };
    Some(Climate {
        ocean,
        ice,
        life,
        rust,
        sand,
        aridity,
        dark,
        clouds,
        air: Air { gas, haze, haze_albedo, height: if thick { 0.025 } else { 0.018 } },
    })
}

/// Earth and Mars, which the rules were drawn to reach but are not trusted to, plus the two
/// bodies with decks the table in [`crate::worlds`] has.
fn measured(id: &str) -> Option<Climate> {
    Some(match id {
        "Earth" => Climate {
            ocean: 0.71,
            ice: 0.07,
            life: 0.95,
            rust: 0.2,
            sand: 0.45,
            aridity: 0.0,
            dark: 0.0,
            clouds: Clouds { cover: 0.0, opacity: 1.0, tint: [1.0; 3] },
            air: Air { gas: EARTH_GAS, haze: 0.03, haze_albedo: [0.9, 0.9, 0.9], height: 0.025 },
        },
        // Its clouds are water ice, faint and sparse; exaggerated a little so the wisps show.
        "Mars" => Climate {
            ocean: 0.0,
            ice: 0.025,
            life: 0.0,
            rust: 1.0,
            sand: 0.2,
            aridity: 0.2,
            dark: 0.15,
            clouds: Clouds { cover: -0.17, opacity: 0.5, tint: [0.95, 0.97, 1.03] },
            air: Air {
                gas: EARTH_GAS.map(|g| g * 0.1),
                haze: 0.12,
                haze_albedo: [0.95, 0.66, 0.42],
                height: 0.02,
            },
        },
        "Venus" => Climate {
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint: [1.0, 0.93, 0.7] },
            air: Air { gas: EARTH_GAS.map(|g| g * 1.2), haze: 0.35, haze_albedo: [0.98, 0.93, 0.74], height: 0.03 },
            ..derived(Atmosphere::Thick, Top::Cloud, 328.0, Some(0.0), variety(id))?
        },
        "Titan" => Climate {
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint: [0.68, 0.42, 0.16] },
            air: Air { gas: [0.0; 3], haze: 1.0, haze_albedo: [0.95, 0.6, 0.26], height: 0.035 },
            ..derived(Atmosphere::Thick, Top::Cloud, 90.0, Some(0.0), variety(id))?
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(atmosphere: Atmosphere, top: Top, k: f64, water: Option<f64>) -> Climate {
        derived(atmosphere, top, k, water, variety("test")).expect("has air")
    }

    #[test]
    fn earths_water_is_earths_sea() {
        assert!((sea_share(EARTH_WATER) - 0.71).abs() < 0.01, "{}", sea_share(EARTH_WATER));
        assert!(sea_share(EARTH_WATER / 10.0) < 0.2);
        assert!(sea_share(EARTH_WATER * 10.0) > 0.97);
        assert_eq!(sea_share(0.0), 0.0);
    }

    #[test]
    fn a_colder_world_has_more_ice_and_a_drier_one_less() {
        let at = |k| made(Atmosphere::Thick, Top::Ocean, k, Some(EARTH_WATER)).ice;
        assert!(at(278.0) < at(265.0) && at(265.0) < at(250.0));
        assert!(at(310.0) == 0.0);
        let temperate = at(278.0);
        assert!((0.04..0.2).contains(&temperate), "Earth's temperature gives {temperate}");
        // Mars's cold with little water: caps, not a snowball.
        let dry = made(Atmosphere::Thin, Top::Ice, 250.0, Some(2.0e-6)).ice;
        assert!(dry < made(Atmosphere::Thin, Top::Ice, 250.0, Some(EARTH_WATER)).ice);
    }

    #[test]
    fn only_a_temperate_sea_is_green() {
        assert!(made(Atmosphere::Thick, Top::Ocean, 278.0, None).life > 0.3);
        assert_eq!(made(Atmosphere::Thin, Top::Rock, 226.0, None).life, 0.0);
        assert!(made(Atmosphere::Thick, Top::Ocean, 360.0, None).life < 0.05);
    }

    #[test]
    fn a_dry_thin_world_is_red_and_hazy_with_sparse_cloud() {
        let mars = made(Atmosphere::Thin, Top::Rock, 226.0, None);
        let earth = made(Atmosphere::Thick, Top::Ocean, 278.0, None);
        assert!(mars.rust > earth.rust);
        assert!(mars.clouds.cover < earth.clouds.cover - 0.1);
        assert!(mars.air.haze > earth.air.haze);
        assert!(mars.air.gas[2] < earth.air.gas[2] / 3.0, "thin air has a faint limb");
    }

    #[test]
    fn a_deck_hides_everything_and_airless_bodies_have_no_climate() {
        let venus = made(Atmosphere::Thick, Top::Cloud, 328.0, None);
        assert_eq!(venus.clouds.cover, 1.0);
        let titan = made(Atmosphere::Thick, Top::Cloud, 90.0, None);
        assert!(titan.clouds.tint[2] < titan.clouds.tint[0] / 2.0, "Titan is orange");
        assert_eq!(derived(Atmosphere::None, Top::Rock, 400.0, None, variety("x")), None);
        assert_eq!(derived(Atmosphere::Envelope, Top::Cloud, 100.0, None, variety("x")), None);
    }

    #[test]
    fn the_water_tag_survives_the_round_trip() {
        for w in [0.0, 1.0e-7, EARTH_WATER, 0.45] {
            assert_eq!(water_of(&[water_tag(w)]), Some(w));
        }
        assert_eq!(water_of(&["Air:Thin".into()]), None);
    }

    /// The measured worlds are the rocky ones with air in the solar system, and every one
    /// of them is reached.
    #[test]
    fn every_measured_world_with_air_has_a_climate() {
        for w in crate::worlds::ALL.iter().filter(|w| matches!(w.atmosphere, Atmosphere::Thin | Atmosphere::Thick)) {
            assert!(of(w.body_id, w, 200.0, &[]).is_some(), "{}", w.body_id);
        }
    }
}
