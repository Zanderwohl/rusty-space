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

/// What a body is, measured where anybody has been and derived from its class everywhere else.
///
/// The derived answer is deliberately duller than any authored one: flat reflectance at the
/// class's own albedo, and an atmosphere read off the class. A generated planet cannot be a
/// Venus, because nothing about its radius, mass and temperature says it is one — which is the
/// honest position rather than a shortcoming. What it can be is the thing its class describes.
pub fn of(id: &str, surface: Surface) -> World {
    if let Some(known) = for_body(id) {
        return *known;
    }
    let flat = surface.albedo();
    World {
        body_id: "",
        reflectance: optical(flat, flat, flat, flat, flat),
        atmosphere: derived_atmosphere(surface),
        top: derived_top(surface),
        heat_ratio: surface.internal_heat_ratio(),
    }
}

/// The air a class implies, where nothing has been measured.
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
        let classed = of("some generated rock", Surface::Weathered);
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
        let giant = of("generated-3", Surface::IceGiant);
        assert_eq!((giant.atmosphere, giant.top), (Atmosphere::Envelope, Top::Cloud));
        assert!(giant.heat_ratio >= 1.0);

        let ice = of("generated-4", Surface::Ice);
        assert_eq!((ice.atmosphere, ice.top), (Atmosphere::None, Top::Ice));
        // Flat, because a class says how bright and nothing about color.
        let r = |b| ice.reflectance_in(b);
        assert_eq!(r(Band::B), r(Band::R));
    }
}
