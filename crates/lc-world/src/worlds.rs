//! What a body is made of and wrapped in, as measured.
//!
//! [`crate::surface::Surface`] derives a class from radius, mass and temperature, and says
//! plainly where that stops: "a body's cloud deck is not derivable from its radius, mass and
//! temperature." Venus is the case it names. It shares a class with Mars and Titan and has
//! nearly three times their albedo, and the survey in
//! `lightcone/docs/25-system-knowledge.md` turns on telling them apart. So the bodies that have
//! been visited are authored, and everything else falls back to rules on its class.
//!
//! **What the reflectances are for is the ordering and the color**, exactly as the optical
//! depths in [`crate::rings`] are for the ordering of ring systems. Venus is bright and nearly
//! gray; Earth is darker and blue; Mars is darker still and red. Those three statements are what
//! a survey reads, and they are right here. The third digit of any one number is not.
//!
//! Reflectance is meaningless in the two emissive bands, where what leaves a body is its own
//! heat rather than reflected starlight, so those entries are zero and
//! [`World::emissivity`] answers for them instead.
//!
//! Rotation is not here. `em-sim`'s presets carry every Sol body's IAU rotation, and a second
//! copy of a spin is a second chance to have it wrong — the same reason poles are not in
//! [`crate::rings`].

use em_spectra::{BANDS, Band};

use crate::surface::Surface;

/// How much atmosphere a body has, in the terms a survey can distinguish.
///
/// Four, because that is how many a telescope can tell apart at a distance: nothing, a trace
/// that barely reddens a surface, a deck that hides one, and a body that is atmosphere all the
/// way down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atmosphere {
    None,
    /// Thin enough to see the ground through. Mars, Triton.
    Thin,
    /// Opaque: what is seen is the top of the air, not the body. Venus, Titan.
    Thick,
    /// No surface to have an atmosphere above. The giants.
    Envelope,
}

impl Atmosphere {
    /// The tag spelling, which is not the label: one is data and the other is prose.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Thin => "Thin",
            Self::Thick => "Thick",
            Self::Envelope => "Envelope",
        }
    }

    pub fn named(text: &str) -> Option<Self> {
        [Self::None, Self::Thin, Self::Thick, Self::Envelope].into_iter().find(|a| a.name() == text)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "airless",
            Self::Thin => "a thin atmosphere",
            Self::Thick => "a thick atmosphere",
            Self::Envelope => "an envelope",
        }
    }

    /// Whether radio can reach a surface under it.
    ///
    /// The point of the radio band: Venus's cloud deck hides a 737 K surface from every optical
    /// band and none of it from radio, which is how the real planet was mapped.
    pub fn hides_a_surface(self) -> bool {
        self == Self::Thick
    }
}

/// What is at the top of a body, which is what reflects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Top {
    Rock,
    Ice,
    Ocean,
    /// A cloud deck, whether or not there is anything under it.
    Cloud,
}

impl Top {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rock => "Rock",
            Self::Ice => "Ice",
            Self::Ocean => "Ocean",
            Self::Cloud => "Cloud",
        }
    }

    pub fn named(text: &str) -> Option<Self> {
        [Self::Rock, Self::Ice, Self::Ocean, Self::Cloud].into_iter().find(|t| t.name() == text)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rock => "rock",
            Self::Ice => "ice",
            Self::Ocean => "ocean",
            Self::Cloud => "cloud",
        }
    }
}

/// One body's measured surface and air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    /// The `em-sim` body id this describes.
    pub body_id: &'static str,
    /// Geometric albedo per band, in [`Band`] order. Zero in the emissive bands.
    pub reflectance: [f64; BANDS],
    pub atmosphere: Atmosphere,
    pub top: Top,
    /// What it radiates over what it absorbs. One for a body with no heat of its own.
    ///
    /// Jupiter is the measurement this exists for: it emits about 1.67 times what it takes from
    /// the Sun, which is why its temperature does not follow from its distance.
    pub heat_ratio: f64,
}

impl World {
    pub fn reflectance_in(&self, band: Band) -> f64 {
        self.reflectance.get(band.index()).copied().unwrap_or(0.0)
    }

    /// Emissivity in the two bands a body radiates in rather than reflects.
    ///
    /// A rock is near a blackbody in the thermal infrared and poor in radio; a thick atmosphere
    /// is opaque to the first and transparent to the second, which is the asymmetry that lets a
    /// surface be found under cloud. One number each rather than a spectrum, because that is as
    /// much as the two bands can be asked.
    pub fn emissivity(&self, band: Band) -> f64 {
        match (band, self.atmosphere) {
            (Band::ThermalIr, Atmosphere::Thick) => 0.99,
            (Band::ThermalIr, _) => 0.95,
            (Band::Radio, Atmosphere::Thick) => 0.9,
            (Band::Radio, Atmosphere::Envelope) => 0.2,
            (Band::Radio, _) => 0.8,
            _ => 0.0,
        }
    }

    /// Geometric albedo averaged over the four bands one silicon detector sees.
    ///
    /// What a single-number albedo meant before there were seven of them, so a caller that only
    /// wants "how bright" keeps getting an answer.
    pub fn gray_albedo(&self) -> f64 {
        let sum: f64 = Band::SILICON.iter().map(|b| self.reflectance_in(*b)).sum();
        sum / Band::SILICON.len() as f64
    }
}

/// Reflectance in the four optical bands, with the emissive two left at zero.
const fn optical(b: f64, v: f64, r: f64, i: f64, k: f64) -> [f64; BANDS] {
    [b, v, r, i, k, 0.0, 0.0]
}

/// The bodies anybody has been to.
///
/// Geometric albedos are the published visual values, with the optical run shaped to the color
/// each body is actually seen to be. Heat ratios are measured for the giants and one for
/// everything else.
pub const ALL: &[World] = &[
    // Bare, dark, airless rock. Nearly gray with a slight reddening.
    World {
        body_id: "Mercury",
        reflectance: optical(0.10, 0.14, 0.16, 0.17, 0.17),
        atmosphere: Atmosphere::None,
        top: Top::Rock,
        heat_ratio: 1.0,
    },
    // **The case this table exists for.** Bright and nearly gray, over a surface no optical
    // band ever sees. Its class would have given it 0.20.
    World {
        body_id: "Venus",
        reflectance: optical(0.60, 0.67, 0.70, 0.72, 0.60),
        atmosphere: Atmosphere::Thick,
        top: Top::Cloud,
        heat_ratio: 1.0,
    },
    // Blue: Rayleigh scattering and ocean, so B is the brightest band and R the dimmest of the
    // three. The only ocean in the table.
    World {
        body_id: "Earth",
        reflectance: optical(0.43, 0.37, 0.33, 0.31, 0.29),
        atmosphere: Atmosphere::Thick,
        top: Top::Ocean,
        heat_ratio: 1.0,
    },
    World {
        body_id: "Luna",
        reflectance: optical(0.09, 0.12, 0.14, 0.15, 0.16),
        atmosphere: Atmosphere::None,
        top: Top::Rock,
        heat_ratio: 1.0,
    },
    // Red, and the strongest color in the table: iron oxide takes B down and lets I through.
    World {
        body_id: "Mars",
        reflectance: optical(0.09, 0.17, 0.25, 0.29, 0.30),
        atmosphere: Atmosphere::Thin,
        top: Top::Rock,
        heat_ratio: 1.0,
    },
    // Banded, and warmer than the Sun alone would leave it.
    World {
        body_id: "Jupiter",
        reflectance: optical(0.49, 0.52, 0.55, 0.55, 0.28),
        atmosphere: Atmosphere::Envelope,
        top: Top::Cloud,
        heat_ratio: 1.67,
    },
    World {
        body_id: "Saturn",
        reflectance: optical(0.45, 0.47, 0.51, 0.52, 0.25),
        atmosphere: Atmosphere::Envelope,
        top: Top::Cloud,
        heat_ratio: 1.78,
    },
    // Featureless: methane absorbs the red end, which is why it reads blue-green and why I and
    // K are so much darker than V.
    World {
        body_id: "Uranus",
        reflectance: optical(0.54, 0.51, 0.41, 0.26, 0.05),
        atmosphere: Atmosphere::Envelope,
        top: Top::Cloud,
        heat_ratio: 1.06,
    },
    World {
        body_id: "Neptune",
        reflectance: optical(0.50, 0.41, 0.34, 0.22, 0.05),
        atmosphere: Atmosphere::Envelope,
        top: Top::Cloud,
        heat_ratio: 2.61,
    },
    // Volcanic sulfur: dark in B, bright and yellow through R and I.
    World {
        body_id: "Io",
        reflectance: optical(0.42, 0.63, 0.75, 0.80, 0.80),
        atmosphere: Atmosphere::None,
        top: Top::Rock,
        heat_ratio: 1.0,
    },
    // The brightest thing in the table, and ice rather than cloud.
    World {
        body_id: "Europa",
        reflectance: optical(0.70, 0.67, 0.65, 0.63, 0.55),
        atmosphere: Atmosphere::None,
        top: Top::Ice,
        heat_ratio: 1.0,
    },
    World {
        body_id: "Ganymede",
        reflectance: optical(0.40, 0.43, 0.44, 0.44, 0.40),
        atmosphere: Atmosphere::None,
        top: Top::Ice,
        heat_ratio: 1.0,
    },
    World {
        body_id: "Callisto",
        reflectance: optical(0.15, 0.22, 0.25, 0.26, 0.26),
        atmosphere: Atmosphere::None,
        top: Top::Ice,
        heat_ratio: 1.0,
    },
    // A moon with a thick atmosphere, which is why it is here: orange haze over a surface no
    // optical band reaches, and the only body other than Venus that hides one.
    World {
        body_id: "Titan",
        reflectance: optical(0.12, 0.22, 0.29, 0.33, 0.20),
        atmosphere: Atmosphere::Thick,
        top: Top::Cloud,
        heat_ratio: 1.0,
    },
    World {
        body_id: "Enceladus",
        reflectance: optical(1.00, 1.00, 0.99, 0.95, 0.85),
        atmosphere: Atmosphere::None,
        top: Top::Ice,
        heat_ratio: 1.0,
    },
    World {
        body_id: "Triton",
        reflectance: optical(0.72, 0.72, 0.72, 0.70, 0.60),
        atmosphere: Atmosphere::Thin,
        top: Top::Ice,
        heat_ratio: 1.0,
    },
];

/// One body's measured world, by its `em-sim` id.
pub fn for_body(id: &str) -> Option<&'static World> {
    ALL.iter().find(|w| w.body_id == id)
}

/// What the generator stated about a body, as `em-sim` tags carry it.
///
/// Tags rather than a second lookup, for the reason [`crate::navigation::Kind`] uses them: the
/// data says what a body is and nothing downstream re-derives it. A body with no tags -- every
/// body of the solar-system preset, and anything authored -- falls back to its class.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stated {
    pub atmosphere: Option<Atmosphere>,
    pub top: Option<Top>,
    /// Whether the body's envelope is over a core that reached runaway, which is the
    /// difference between a banded giant and a methane-blue one.
    pub gas_giant: bool,
}

impl Stated {
    pub const AIR: &'static str = "Air:";
    pub const TOP: &'static str = "Top:";
    pub const GAS_GIANT: &'static str = "GasGiant";

    pub fn from_tags(tags: &[String]) -> Self {
        let mut out = Self::default();
        for tag in tags {
            if let Some(rest) = tag.strip_prefix(Self::AIR) {
                out.atmosphere = Atmosphere::named(rest);
            } else if let Some(rest) = tag.strip_prefix(Self::TOP) {
                out.top = Top::named(rest);
            } else if tag == Self::GAS_GIANT {
                out.gas_giant = true;
            }
        }
        out
    }

    /// The tags that state this, for the generator to write.
    pub fn tags(atmosphere: Atmosphere, top: Top, gas_giant: bool) -> Vec<String> {
        let mut out = vec![format!("{}{}", Self::AIR, atmosphere.name()), format!("{}{}", Self::TOP, top.name())];
        if gas_giant {
            out.push(Self::GAS_GIANT.to_string());
        }
        out
    }
}

/// What a body is: measured where anybody has been, stated by the generator where it made one,
/// and derived from its class otherwise.
///
/// The measured table wins because Venus is in it and no rule reaches Venus. Below that, a
/// generated body says what it is rather than having it guessed from radius, mass and
/// temperature -- which is what makes a generated ocean read blue and a generated ice world
/// read bright, and so what gives a type hypothesis anything to work on.
pub fn of(id: &str, surface: Surface, tags: &[String]) -> World {
    if let Some(known) = for_body(id) {
        return *known;
    }
    let stated = Stated::from_tags(tags);
    let atmosphere = stated.atmosphere.unwrap_or_else(|| derived_atmosphere(surface));
    let top = stated.top.unwrap_or_else(|| derived_top(surface));
    World {
        body_id: "",
        reflectance: reflectance_of(top, atmosphere, surface, stated.gas_giant, varied(id)),
        atmosphere,
        top,
        heat_ratio: surface.internal_heat_ratio(),
    }
}

/// How far this particular body departs from the type's curve: a brightness factor and a tilt
/// across the optical run.
///
/// **Not decoration.** Without it the reflectance is a pure function of the type, so measuring
/// a color would identify the type exactly and a hypothesis would never have more than one
/// entry in it. Two ocean worlds are not the same color, and the spread is what makes the
/// classification an inference rather than a lookup. Keyed by the body's id, so it does not
/// shimmer between frames.
fn varied(id: &str) -> (f64, f64) {
    // FNV-1a: the ids are short and this needs no allocation in a per-frame loop.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    let brightness = crate::rng::uniform_in(crate::rng::hash(&[h, 1]), 0.85, 1.15);
    let tilt = crate::rng::uniform_in(crate::rng::hash(&[h, 2]), -0.08, 0.08);
    (brightness, tilt)
}

/// The reflectance a type has, before this body's own variation.
///
/// Every row is one of the measured bodies above, which is where its shape comes from: a
/// generated ocean is Earth's curve, a generated ice world is between Europa's and Callisto's,
/// a generated gas giant is Jupiter's and an ice giant is Uranus's. The third digit is not the
/// point; the ordering and the color are.
fn reflectance_of(top: Top, atmosphere: Atmosphere, surface: Surface, gas_giant: bool, varied: (f64, f64)) -> [f64; BANDS] {
    let base = match (top, atmosphere) {
        // A giant's deck. Jupiter is banded and warm; Uranus is methane-blue and eats the red
        // end, which is the one color difference that tells the two kinds of giant apart.
        (Top::Cloud, Atmosphere::Envelope) if gas_giant => optical(0.47, 0.50, 0.53, 0.54, 0.27),
        (Top::Cloud, Atmosphere::Envelope) => optical(0.52, 0.46, 0.38, 0.24, 0.05),
        // A rocky body's deck: Venus where it is warm, Titan's orange haze where it is not.
        (Top::Cloud, _) if surface == Surface::Ice => optical(0.12, 0.22, 0.29, 0.33, 0.20),
        (Top::Cloud, _) => optical(0.60, 0.67, 0.70, 0.72, 0.60),
        (Top::Ocean, _) => optical(0.43, 0.37, 0.33, 0.31, 0.29),
        (Top::Ice, _) => optical(0.60, 0.60, 0.58, 0.55, 0.48),
        // Scorched rock is darker and flatter than weathered rock, which is red because it is
        // oxidized, which needs air.
        (Top::Rock, _) if surface == Surface::Scorched => optical(0.06, 0.08, 0.10, 0.11, 0.11),
        (Top::Rock, Atmosphere::None) => optical(0.09, 0.13, 0.15, 0.16, 0.16),
        (Top::Rock, _) => optical(0.09, 0.17, 0.25, 0.29, 0.30),
    };
    let (brightness, tilt) = varied;
    // Every band the row states, not only the four a silicon detector sees: K is reflected
    // light too, and it is where methane tells the two kinds of giant apart.
    let optical = base.iter().filter(|r| **r > 0.0).count().max(2);
    let mut out = [0.0; BANDS];
    for (k, value) in base.iter().enumerate().filter(|(_, r)| **r > 0.0) {
        // Tilt about the middle of the optical run, so it reddens or blues without changing
        // how bright the body is overall.
        let across = k as f64 / (optical - 1) as f64 - 0.5;
        out[k] = (value * brightness * (1.0 + tilt * 2.0 * across)).clamp(0.0, 1.0);
    }
    out
}

/// The air a class implies, where the generator stated nothing.
fn derived_atmosphere(surface: Surface) -> Atmosphere {
    match surface {
        Surface::GasGiant | Surface::IceGiant => Atmosphere::Envelope,
        // Warm enough to hold weather and not to have been stripped.
        Surface::Weathered => Atmosphere::Thin,
        Surface::Rock | Surface::Ice | Surface::Scorched => Atmosphere::None,
    }
}

fn derived_top(surface: Surface) -> Top {
    match surface {
        Surface::GasGiant | Surface::IceGiant => Top::Cloud,
        Surface::Ice => Top::Ice,
        Surface::Rock | Surface::Weathered | Surface::Scorched => Top::Rock,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(id: &str) -> &'static World {
        for_body(id).unwrap_or_else(|| panic!("{id} is not in the table"))
    }

    /// **The three statements a survey actually reads.** Venus is bright and nearly gray, Earth
    /// is darker and blue, Mars is darker still and red. If the table ever stops saying that, it
    /// has stopped being worth having.
    #[test]
    fn the_inner_planets_are_ordered_and_colored_as_they_are_seen() {
        let (venus, earth, mars) = (world("Venus"), world("Earth"), world("Mars"));

        assert!(venus.gray_albedo() > earth.gray_albedo(), "Venus is the bright one");
        assert!(earth.gray_albedo() > mars.gray_albedo(), "Mars is the dark one");

        // Color is the slope across the optical run, which is what a per-band reading measures.
        let slope = |w: &World| w.reflectance_in(Band::R) - w.reflectance_in(Band::B);
        assert!(slope(earth) < 0.0, "Earth is blue: B brighter than R");
        assert!(slope(mars) > 0.1, "Mars is strongly red");
        assert!(slope(venus).abs() < 0.15, "Venus is nearly gray");
    }

    /// The whole reason this file exists: Venus's class would have made it Mars.
    #[test]
    fn venus_is_what_its_class_could_never_have_said() {
        let venus = world("Venus");
        let classed = of("some generated rock", Surface::Weathered, &[]);
        assert!(
            venus.gray_albedo() > classed.gray_albedo() * 2.0,
            "authored {} against derived {}",
            venus.gray_albedo(),
            classed.gray_albedo(),
        );
        assert_eq!(venus.atmosphere, Atmosphere::Thick);
        assert!(venus.atmosphere.hides_a_surface(), "a 737 K surface no optical band sees");
        assert!(!classed.atmosphere.hides_a_surface(), "a class cannot claim a cloud deck");
    }

    /// Radio is what reaches a surface under cloud, and the emissivities have to say so or the
    /// survey's Venus reading has nothing behind it.
    #[test]
    fn radio_gets_through_a_deck_that_stops_the_infrared() {
        let venus = world("Venus");
        assert!(venus.emissivity(Band::Radio) > 0.5, "radio is how a hidden surface is found");
        assert!(venus.emissivity(Band::ThermalIr) > 0.9, "the deck itself radiates");
        // A giant is the other way about: nothing solid under it to answer in radio.
        assert!(world("Jupiter").emissivity(Band::Radio) < 0.5);
        // And reflectance means nothing in an emissive band.
        assert_eq!(venus.reflectance_in(Band::ThermalIr), 0.0);
    }

    /// A giant is warmer than its distance allows, and that is measured rather than classed.
    #[test]
    fn the_giants_carry_their_own_heat() {
        assert!(world("Jupiter").heat_ratio > 1.5, "Jupiter emits more than it absorbs");
        assert!(world("Neptune").heat_ratio > 2.0, "and Neptune most of all");
        assert_eq!(world("Earth").heat_ratio, 1.0, "a rock radiates what it is given");
    }

    /// Methane absorbs the red end, which is why the ice giants read blue-green. Ordering
    /// again: it is the shape across the bands that matters, not any one number.
    #[test]
    fn the_ice_giants_darken_toward_the_infrared() {
        for id in ["Uranus", "Neptune"] {
            let w = world(id);
            assert!(w.reflectance_in(Band::B) > w.reflectance_in(Band::I) * 1.5, "{id}");
            assert!(w.reflectance_in(Band::K) < 0.1, "{id}: methane eats K");
        }
    }

    /// Every authored body is a real one, and no id is in twice.
    #[test]
    fn the_table_is_well_formed() {
        for w in ALL {
            assert_eq!(ALL.iter().filter(|o| o.body_id == w.body_id).count(), 1, "{}", w.body_id);
            for band in Band::SILICON {
                let r = w.reflectance_in(band);
                assert!((0.0..=1.0).contains(&r), "{}: reflectance {r} in {band:?}", w.body_id);
            }
            assert!(w.heat_ratio >= 1.0, "{}: a body cannot emit less than it absorbs", w.body_id);
        }
    }

    /// **An authored body keyed by a name nothing has is silently dead**, which is the one way
    /// this table can fail without anybody noticing. Checked against the ids the lookup
    /// actually uses: `em_sim::System::name` is the `id` field, not `info.name`, and the two
    /// differ — Luna's id is lowercase.
    #[test]
    fn every_authored_body_is_one_the_preset_has() {
        let sim = em_sim::system::System::from_contents(&em_sim::presets::solar_system())
            .expect("the solar system preset must load");
        let ids: Vec<&str> = sim.indices().map(|i| sim.name(i)).collect();
        for w in ALL {
            assert!(ids.contains(&w.body_id), "{} is in no preset body's id", w.body_id);
        }
        // And the bodies the survey's own table names are all covered.
        for id in ["Venus", "Earth", "Mars", "Jupiter", "Saturn"] {
            assert!(for_body(id).is_some(), "{id} is what phase 6 measures");
        }
    }

    /// A body nobody has been to still answers, from its class, and an ice giant's derived
    /// world is an envelope rather than bare rock.
    #[test]
    fn an_unvisited_body_falls_back_to_its_class() {
        let giant = of("generated-3", Surface::IceGiant, &[]);
        assert_eq!((giant.atmosphere, giant.top), (Atmosphere::Envelope, Top::Cloud));
        assert!(giant.heat_ratio >= 1.0);

        let ice = of("generated-4", Surface::Ice, &[]);
        assert_eq!((ice.atmosphere, ice.top), (Atmosphere::None, Top::Ice));
        assert!(ice.gray_albedo() > giant.gray_albedo() * 0.8, "ice is bright");
    }

    /// **What a type hypothesis has to work on.** A generated body says what it is through its
    /// tags, and what it says decides its color: an ocean is blue, an ice world is bright, a
    /// deck is bright and flat, and bare rock is dark. Without this every generated body is
    /// flat at its class albedo and a color measures nothing.
    #[test]
    fn a_generated_body_is_the_colour_of_what_it_is_made_of() {
        let made = |top: Top, air: Atmosphere, surface| {
            of("generated-body", surface, &Stated::tags(air, top, false))
        };
        let slope = |w: &World| w.reflectance_in(Band::R) - w.reflectance_in(Band::B);

        let ocean = made(Top::Ocean, Atmosphere::Thick, Surface::Weathered);
        let deck = made(Top::Cloud, Atmosphere::Thick, Surface::Weathered);
        let ice = made(Top::Ice, Atmosphere::None, Surface::Ice);
        let bare = made(Top::Rock, Atmosphere::None, Surface::Rock);
        let dusty = made(Top::Rock, Atmosphere::Thin, Surface::Weathered);

        assert!(slope(&ocean) < 0.0, "an ocean is blue");
        assert!(slope(&dusty) > 0.1, "weathered rock is red");
        assert!(slope(&bare).abs() < 0.1, "bare rock is nearly gray");
        assert!(deck.gray_albedo() > ocean.gray_albedo(), "a deck is brighter than an ocean");
        assert!(ice.gray_albedo() > ocean.gray_albedo(), "and so is ice");
        assert!(bare.gray_albedo() < 0.25, "bare rock is dark");

        // And the two kinds of giant differ where methane does.
        let gas = of("g", Surface::GasGiant, &Stated::tags(Atmosphere::Envelope, Top::Cloud, true));
        let icy = of("g", Surface::IceGiant, &Stated::tags(Atmosphere::Envelope, Top::Cloud, false));
        assert!(gas.reflectance_in(Band::K) > icy.reflectance_in(Band::K) * 3.0, "methane eats K");
    }

    /// Two ocean worlds are not the same color. Without the spread a color would identify a
    /// type exactly and a hypothesis would never hold more than one entry.
    #[test]
    fn two_bodies_of_a_type_are_not_the_same_body() {
        let tags = Stated::tags(Atmosphere::Thick, Top::Ocean, false);
        let (a, b) = (of("Kettle e", Surface::Weathered, &tags), of("Kettle f", Surface::Weathered, &tags));
        assert_ne!(a.reflectance, b.reflectance);
        // Same every time it is asked, or a body would shimmer between frames.
        assert_eq!(a.reflectance, of("Kettle e", Surface::Weathered, &tags).reflectance);
        // And still recognizably its type.
        assert!((a.gray_albedo() / b.gray_albedo()).ln().abs() < 0.4);
    }

    /// Tags are the channel, so they have to survive the round trip.
    #[test]
    fn what_the_generator_states_is_what_is_read_back() {
        for air in [Atmosphere::None, Atmosphere::Thin, Atmosphere::Thick, Atmosphere::Envelope] {
            for top in [Top::Rock, Top::Ice, Top::Ocean, Top::Cloud] {
                let stated = Stated::from_tags(&Stated::tags(air, top, true));
                assert_eq!((stated.atmosphere, stated.top, stated.gas_giant), (Some(air), Some(top), true));
            }
        }
        // Tags that say nothing leave it to the class, which is every body of the preset.
        let bare = Stated::from_tags(&["Planet".to_string(), "Moon".to_string()]);
        assert_eq!((bare.atmosphere, bare.top, bare.gas_giant), (None, None, false));
    }
}
