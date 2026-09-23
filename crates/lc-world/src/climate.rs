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
    /// Share of the wet land that is alive, `[0, 1]`.
    pub life: f32,
    /// What colour the living land is.
    pub foliage: Foliage,
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

/// Oklch `(L, C, hue in degrees)`: lowland growth, and the sparser growth higher up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foliage {
    pub low: [f32; 3],
    pub high: [f32; 3],
}

/// The colour of what grows under a star of `teff_k`.
///
/// A pigment is worth making where the photons are, so what a plant reflects is what its star
/// sends least usefully -- the argument of Kiang et al. (2007), taken for its ordering rather
/// than its spectra. Under a hot white star growth takes the blue and returns gold; under the
/// Sun it is green; under a cool orange star it darkens toward crimson; under a red
/// dwarf, which gives little of anything, it absorbs everything it can and is nearly black. So a
/// dimmer star makes darker growth. Anchors lerp in Oklab rather than round the hue circle, which
/// would put a blue forest between green and red.
pub fn foliage(teff_k: f64) -> Foliage {
    const ANCHORS: [(f64, Foliage); 4] = [
        // Hues kept off rust's 40 degrees at both ends, or growth reads as bare red ground.
        (3000.0, Foliage { low: [0.24, 0.04, 330.0], high: [0.30, 0.04, 335.0] }),
        (4300.0, Foliage { low: [0.38, 0.11, 355.0], high: [0.43, 0.09, 350.0] }),
        // Forest rather than meadow: the deep green of a Blue Marble photograph.
        (5772.0, Foliage { low: [0.40, 0.12, 145.0], high: [0.43, 0.10, 130.0] }),
        (7000.0, Foliage { low: [0.70, 0.13, 95.0], high: [0.66, 0.11, 85.0] }),
    ];
    let i = ANCHORS.windows(2).position(|w| teff_k <= w[1].0).unwrap_or(ANCHORS.len() - 2);
    let [(ta, a), (tb, b)] = [ANCHORS[i], ANCHORS[i + 1]];
    let t = ((teff_k - ta) / (tb - ta)).clamp(0.0, 1.0) as f32;
    Foliage { low: oklab_lerp(a.low, b.low, t), high: oklab_lerp(a.high, b.high, t) }
}

fn oklab_lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let ab = |c: [f32; 3]| {
        let h = c[2].to_radians();
        [c[0], c[1] * h.cos(), c[1] * h.sin()]
    };
    let (a, b) = (ab(a), ab(b));
    let m: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
    [m[0], m[1].hypot(m[2]), m[2].atan2(m[1]).to_degrees().rem_euclid(360.0)]
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
    /// How far the air evens out day and night, `[0, 1]`: carrying heat round to the night side
    /// and holding it there. Venus's night is as hot as its day; Mars's air does almost nothing.
    pub evens: f32,
}

/// Vertical optical depth of an Earth's worth of nitrogen, per channel at 680, 550 and 440 nm:
/// half the true figures. The scale height is exaggerated, so the full depth washed the disc
/// out to a pastel; at half the limb still shows, because a tangent ray crosses sixteen times
/// the vertical depth.
const EARTH_GAS: [f32; 3] = [0.022, 0.05, 0.12];

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

/// The generator's statement of how much of a world is alive.
pub const LIFE: &str = "Life:";

pub fn life_tag(life: f64) -> String {
    format!("{LIFE}{life}")
}

pub fn life_of(tags: &[String]) -> Option<f64> {
    tags.iter().find_map(|t| t.strip_prefix(LIFE)?.parse().ok())
}

/// What a climate is derived from: what the generator decided, and the star it decided it
/// under. What is `None` was not stated, and the rules stand in for it.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub atmosphere: Atmosphere,
    pub top: Top,
    pub equilibrium_k: f64,
    pub water_fraction: Option<f64>,
    /// How much of the wet land is alive. Unstated, a temperate sea is taken to be.
    pub life: Option<f64>,
    pub star_teff_k: f64,
}

impl Inputs {
    pub fn from_tags(world: &World, equilibrium_k: f64, star_teff_k: f64, tags: &[String]) -> Self {
        Self {
            atmosphere: world.atmosphere,
            top: world.top,
            equilibrium_k,
            water_fraction: water_of(tags),
            life: life_of(tags),
            star_teff_k,
        }
    }
}

/// A rocky world's paint: measured where anybody has been, derived otherwise. `None` for a body
/// without air or without a surface, which keeps its class's pattern.
pub fn of(id: &str, world: &World, equilibrium_k: f64, star_teff_k: f64, tags: &[String]) -> Option<Climate> {
    if let Some(known) = measured(id) {
        return Some(known);
    }
    derived(&Inputs::from_tags(world, equilibrium_k, star_teff_k, tags), variety(id))
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
pub fn derived(inputs: &Inputs, variety: Variety) -> Option<Climate> {
    let Inputs { atmosphere, top, equilibrium_k, water_fraction, life, star_teff_k } = *inputs;
    let foliage = foliage(star_teff_k);
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
            foliage,
            rust: 0.5,
            sand: 0.0,
            aridity: 0.3,
            dark: 0.0,
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint },
            // The haze above the deck, not the deck: the deck is the ground here, and a haze
            // as deep as the whole cloud would put out the light before the terminator.
            air: Air { gas, haze: 0.25 + 0.5 * b, haze_albedo, height: 0.035, evens: 1.0 },
        });
    }

    let ocean = sea_share(water) as f32;
    let ice = cold_share(t).min(ice_capacity(water)).max(f64::from(top == Top::Ice) * 0.6) as f32;
    let ice = if top == Top::Ice && t < 230.0 { 1.0 } else { ice };
    // Green wherever there is sea to rain on the land and warmth enough to grow, most at 290 K.
    let temperate = (-((t - 290.0) / 20.0).powi(2)).exp() as f32;
    let life = match life {
        Some(stated) => stated as f32,
        None if top == Top::Ocean => temperate * (0.45 + 0.55 * b),
        None => 0.0,
    };
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
        foliage,
        rust,
        sand,
        aridity,
        dark,
        clouds,
        air: Air {
            gas,
            haze,
            haze_albedo,
            height: if thick { 0.025 } else { 0.018 },
            evens: if thick { 0.5 } else { 0.05 },
        },
    })
}

/// A solar-system body under a thick deck, which the measured ones start from.
fn deck(equilibrium_k: f64) -> Inputs {
    Inputs {
        atmosphere: Atmosphere::Thick,
        top: Top::Cloud,
        equilibrium_k,
        water_fraction: Some(0.0),
        life: Some(0.0),
        star_teff_k: 5772.0,
    }
}

/// Earth and Mars, which the rules were drawn to reach but are not trusted to, plus the two
/// bodies with decks the table in [`crate::worlds`] has.
fn measured(id: &str) -> Option<Climate> {
    Some(match id {
        "Earth" => Climate {
            ocean: 0.71,
            ice: 0.07,
            life: 0.95,
            foliage: foliage(5772.0),
            rust: 0.2,
            sand: 0.45,
            aridity: 0.0,
            dark: 0.0,
            clouds: Clouds { cover: 0.0, opacity: 1.0, tint: [1.0; 3] },
            air: Air { gas: EARTH_GAS, haze: 0.01, haze_albedo: [0.9, 0.9, 0.9], height: 0.025, evens: 0.5 },
        },
        // Its clouds are water ice, faint and sparse; exaggerated a little so the wisps show.
        "Mars" => Climate {
            ocean: 0.0,
            ice: 0.025,
            life: 0.0,
            foliage: foliage(5772.0),
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
                evens: 0.05,
            },
        },
        "Venus" => Climate {
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint: [1.0, 0.93, 0.7] },
            air: Air { gas: EARTH_GAS.map(|g| g * 1.2), haze: 0.35, haze_albedo: [0.98, 0.93, 0.74], height: 0.03, evens: 1.0 },
            ..derived(&deck(328.0), variety(id))?
        },
        "Titan" => Climate {
            clouds: Clouds { cover: 1.0, opacity: 1.1, tint: [0.68, 0.42, 0.16] },
            air: Air { gas: [0.0; 3], haze: 1.0, haze_albedo: [0.95, 0.6, 0.26], height: 0.035, evens: 1.0 },
            ..derived(&deck(90.0), variety(id))?
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(atmosphere: Atmosphere, top: Top, k: f64, water: Option<f64>) -> Inputs {
        Inputs { atmosphere, top, equilibrium_k: k, water_fraction: water, life: None, star_teff_k: 5772.0 }
    }

    fn made(atmosphere: Atmosphere, top: Top, k: f64, water: Option<f64>) -> Climate {
        derived(&inputs(atmosphere, top, k, water), variety("test")).expect("has air")
    }

    /// A stated life is the life drawn, dead or alive, and unstated the rules stand in.
    #[test]
    fn life_is_what_the_generator_says() {
        let at = |life| derived(&Inputs { life, ..inputs(Atmosphere::Thick, Top::Ocean, 278.0, None) }, variety("x")).unwrap().life;
        assert_eq!(at(Some(0.0)), 0.0, "a habitable world nothing lives on is bare");
        assert_eq!(at(Some(0.8)), 0.8);
        assert!(at(None) > 0.3);
        assert_eq!(life_of(&[life_tag(0.37)]), Some(0.37));
    }

    /// Gold under a hot star, green under the Sun, dark under a red dwarf, and darker all the
    /// way down.
    #[test]
    fn growth_is_the_colour_its_star_leaves() {
        let sun = foliage(5772.0);
        let off = sun.low.iter().zip([0.40, 0.12, 145.0]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        // rocky.tgraph's defaults, which are the Sun's.
        assert!(off < 1e-3, "the Sun's is the graph's own green: {:?}", sun.low);
        let hot = foliage(7000.0).low;
        assert!((40.0..100.0).contains(&hot[2]), "hot-star growth is gold: hue {}", hot[2]);
        let cool = foliage(4300.0).low;
        assert!(cool[2] < 40.0 || cool[2] > 330.0, "cool-star growth is red: hue {}", cool[2]);
        let lightness: Vec<f32> = [2800.0, 3500.0, 4300.0, 5000.0, 5772.0, 6500.0].map(|t| foliage(t).low[0]).to_vec();
        assert!(lightness.windows(2).all(|w| w[1] >= w[0]), "{lightness:?}");
        assert!(foliage(2800.0).low[0] < 0.3, "a red dwarf's growth is nearly black");
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
        assert_eq!(derived(&inputs(Atmosphere::None, Top::Rock, 400.0, None), variety("x")), None);
        assert_eq!(derived(&inputs(Atmosphere::Envelope, Top::Cloud, 100.0, None), variety("x")), None);
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
            assert!(of(w.body_id, w, 200.0, 5772.0, &[]).is_some(), "{}", w.body_id);
        }
    }
}
