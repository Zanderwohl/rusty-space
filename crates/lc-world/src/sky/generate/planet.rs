//! What a rung turns into once it has an orbit, a spin and an age.
//!
//! Air, water and habitability are not drawn. They are one chain: a body's equilibrium
//! temperature sets its exosphere, the exosphere and the escape velocity decide which
//! molecules are still there, and what is still there decides whether anything could live on
//! it. Venus, Earth, Mars, Titan and Triton all fall out of the same three lines, which is the
//! test this file is calibrated against.

use super::architecture::{Architecture, Class, Rung};
use super::disc::{self, EARTH_MASS, EARTH_RADIUS};
use super::tuning::Tuning;
use crate::rng;
use crate::worlds::{Atmosphere, Top};

/// Equilibrium temperature at one astronomical unit from the Sun, zero albedo, kelvin. The
/// reference the exosphere heating is measured against.
pub const REFERENCE_K: f64 = 278.3;

/// Molar masses, g/mol. Hydrogen is what a body has to hold to be a giant; nitrogen is what it
/// has to hold to have air at all; water is what it has to hold to be worth landing on.
const HYDROGEN: f64 = 2.0;
const NITROGEN: f64 = 28.0;
const WATER: f64 = 18.0;

/// Above this equilibrium temperature a body's water is vapour and can be stripped; below it,
/// the water is ice and stays whatever the star does. Kelvin.
const WATER_MOBILE_K: f64 = 200.0;

/// Retention margin above which an atmosphere is opaque rather than a trace.
const THICK_MARGIN: f64 = 1.5;

/// Retention margin at which an unmagnetised body keeps its water anyway.
const UNSHIELDED_WATER_MARGIN: f64 = 2.0;

/// Smallest water fraction that counts as having any.
const WET: f64 = 1.0e-6;

/// One generated planet.
#[derive(Clone, Debug, PartialEq)]
pub struct Planet {
    pub name: String,
    pub semi_major_m: f64,
    pub eccentricity: f64,
    pub inclination_deg: f64,
    pub mean_anomaly_deg: f64,
    pub radius_m: f64,
    pub mass_kg: f64,
    /// How it formed, which is not the same as what it is now: a rocky planet that kept a
    /// hydrogen envelope is still a rocky planet with a rock under the cloud.
    pub class: Class,
    /// A giant that ended up far inside where it formed, dragging its retinue into a Hill
    /// sphere a tenth the size.
    pub migrated: bool,
    /// How long one turn takes, seconds. What a survey reads off the periodogram of a body's
    /// own flux. See `lightcone/docs/25-system-knowledge.md`.
    pub spin_s: f64,
    /// How far the spin axis leans from the system's pole, radians. Earth is 0.41 and Uranus
    /// is 1.71, so the distribution has to allow both.
    pub obliquity_rad: f64,
    /// Which way it leans, measured about the system pole, radians.
    pub spin_node_rad: f64,
    /// Zero-albedo equilibrium temperature, kelvin.
    pub equilibrium_k: f64,
    /// Whether a convecting core gives it a field. What decides if its water survives.
    pub magnetic: bool,
    pub atmosphere: Atmosphere,
    pub top: Top,
    /// Water as a share of the planet's mass. Earth's is 2.3e-4; an ice world's is near a half.
    pub water_fraction: f64,
    /// In the habitable zone, with a surface, air over it and water on it.
    pub habitable: bool,
    /// What orbits it. **A planet with no moon has no mass anybody can measure**: mass comes
    /// from a satellite's period through Kepler's third law, which is the only route a
    /// telescope has to it. See `lightcone/docs/25-system-knowledge.md`.
    pub moons: Vec<super::moon::Moon>,
}

impl Planet {
    pub fn mass_earths(&self) -> f64 {
        self.mass_kg / EARTH_MASS
    }
    pub fn radius_earths(&self) -> f64 {
        self.radius_m / EARTH_RADIUS
    }
}

/// Temperature of the exosphere, kelvin, where escape actually happens.
///
/// The equilibrium temperature plus stellar extreme-ultraviolet heating, which falls as the
/// inverse square of the distance while the equilibrium temperature falls as its square root.
/// Written in temperature alone -- `(T/T_ref)^4` is the inverse square of the radius -- so it
/// needs nothing about the star. Earth's 278 K equilibrium becomes the 1085 K exosphere that
/// is measured, and Titan's 90 K stays at 99 K, which is why Titan has air and Ganymede does
/// not.
pub fn exosphere_k(equilibrium_k: f64, tuning: &Tuning) -> f64 {
    let heating = (tuning.world.exosphere_factor - 1.0) * REFERENCE_K;
    equilibrium_k + heating * (equilibrium_k / REFERENCE_K).powi(4)
}

/// How far a body is from losing a gas: escape velocity over what it takes to hold one.
///
/// Above one the gas stays for the age of the system. The margin rather than the bare test,
/// because a trace atmosphere and an opaque one are the same answer to the test and different
/// answers to a telescope.
pub fn retention_margin(molar_g: f64, mass_earths: f64, radius_earths: f64, exosphere_k: f64, tuning: &Tuning) -> f64 {
    let escape = disc::escape_speed(mass_earths, radius_earths);
    let needed = tuning.world.retention * disc::thermal_speed(molar_g, exosphere_k);
    if needed <= 0.0 { f64::INFINITY } else { escape / needed }
}

/// What a body has over its surface, from what it can hold on to.
///
/// Three outcomes from one margin, because a telescope can tell a trace apart from an opaque
/// deck and cannot tell either apart from a number. Calibrated against the solar system: Venus
/// and Earth opaque, Mars and Triton traces, Mercury and Luna bare, Titan opaque although it
/// is smaller than Luna -- because it is cold, which is the whole point of putting the
/// exosphere in the middle of the chain.
pub fn air(mass_earths: f64, radius_earths: f64, equilibrium_k: f64, envelope: bool, tuning: &Tuning) -> Atmosphere {
    if envelope {
        return Atmosphere::Envelope;
    }
    let margin = retention_margin(NITROGEN, mass_earths, radius_earths, exosphere_k(equilibrium_k, tuning), tuning);
    match margin {
        m if m > THICK_MARGIN => Atmosphere::Thick,
        m if m > 1.0 => Atmosphere::Thin,
        _ => Atmosphere::None,
    }
}

/// Whether a body has a core convecting fast enough to run a dynamo.
///
/// Mass keeps the core molten and rotation organises the flow, so a small body has no field
/// and a slow one has no field however big it is. Venus is the case that fixes the rotation
/// term: it has Earth's mass, 243 days of rotation and no field at all.
pub fn dynamo(mass_earths: f64, spin_s: f64, h: u64, tuning: &Tuning) -> bool {
    let t = &tuning.world;
    let by_mass = (mass_earths / t.dynamo_mass_earths).powf(1.5).min(1.0);
    let by_spin = (t.dynamo_spin_s / spin_s.max(1.0)).min(1.0);
    rng::uniform(h) < (by_mass * by_spin).max(t.dynamo_floor)
}

/// The planets of one architecture, innermost first.
pub fn planets(system: &str, arch: &Architecture, star: &crate::sky::CatalogueStar, tuning: &Tuning) -> Vec<Planet> {
    let seed = star.seed();
    let giants = arch.giant_jupiters();
    arch.planets()
        .enumerate()
        .map(|(k, rung)| {
            let name = format!("{system} {}", (b'b' + k.min(24) as u8) as char);
            of(name, k, rung, arch, giants, seed, tuning)
        })
        .collect()
}

fn of(
    name: String,
    k: usize,
    rung: &Rung,
    arch: &Architecture,
    giant_jupiters: f64,
    seed: u64,
    tuning: &Tuning,
) -> Planet {
    let h = |tag: u64| rng::hash(&[seed, 0x91a4, k as u64, tag]);
    let t = &tuning.world;

    let spin_s = spin_of(h(7), rung.class.is_giant(), t);
    let mass_earths = rung.mass_earths;
    let magnetic = rung.class.is_giant() || dynamo(mass_earths, spin_s, h(13), tuning);
    let exosphere = exosphere_k(rung.equilibrium_k, tuning);
    let margin = |molar: f64, radius: f64| retention_margin(molar, mass_earths, radius, exosphere, tuning);

    // A rocky core heavy enough to have bound nebular gas, cold enough to still hold it, is a
    // sub-Neptune -- the commonest planet there is, and the reason a habitable world has to be
    // a small one.
    let envelope = rung.class.is_giant()
        || (rung.core_earths >= tuning.ladder.ice_giant_core_earths && margin(HYDROGEN, rung.radius_earths) > 1.0);
    let composition = 10f64.powf(rng::gaussian(h(16)) * t.radius_spread_dex);
    let radius_earths = composition
        * match (envelope, rung.class.is_giant()) {
            (true, false) => rung.radius_earths * rng::uniform_in(h(14), 1.4, 2.5),
            _ => rung.radius_earths,
        };

    let atmosphere = air(mass_earths, radius_earths, rung.equilibrium_k, envelope, tuning);

    let water_fraction = water(rung, arch, magnetic, margin(WATER, radius_earths), giant_jupiters, h(15), tuning);
    let top = top_of(rung.class, atmosphere, rung.equilibrium_k, water_fraction);
    let habitable = arch.disc.habitable(rung.semi_major_m)
        && atmosphere != Atmosphere::Envelope
        && atmosphere != Atmosphere::None
        && water_fraction > WET;

    Planet {
        name,
        semi_major_m: rung.semi_major_m,
        eccentricity: rng::uniform_in(h(4), 0.0, 0.12),
        inclination_deg: rng::gaussian(h(5)) * 2.0,
        mean_anomaly_deg: rng::uniform_in(h(6), 0.0, 360.0),
        radius_m: radius_earths * EARTH_RADIUS,
        mass_kg: mass_earths * EARTH_MASS,
        class: rung.class,
        migrated: rung.migrated,
        spin_s,
        obliquity_rad: obliquity_of(h(8), t),
        spin_node_rad: rng::uniform_in(h(9), 0.0, std::f64::consts::TAU),
        equilibrium_k: rung.equilibrium_k,
        magnetic,
        atmosphere,
        top,
        water_fraction,
        habitable,
        moons: Vec::new(),
    }
}

/// How much water a planet has, as a share of its mass.
///
/// Past the snow line water is what the body is made of. Inside it there was never any, so all
/// of it arrived: icy bodies thrown inward by whatever giants the system has, which is why a
/// system with no giant is a dry one.
///
/// Then it can be lost. An unmagnetised planet warm enough for its water to be vapour loses it
/// to the stellar wind unless its gravity is far above what thermal escape alone would need --
/// which is Venus, dry under an atmosphere it had no trouble keeping.
fn water(
    rung: &Rung,
    arch: &Architecture,
    magnetic: bool,
    margin: f64,
    giant_jupiters: f64,
    h: u64,
    tuning: &Tuning,
) -> f64 {
    let t = &tuning.world;
    let born_with = if rung.class.is_giant() {
        // A giant's ices are in its core, under an envelope that is neither ice nor rock, so
        // what it holds as water is a share of the core rather than of the planet.
        0.5 * rung.core_earths / rung.mass_earths.max(1.0e-9)
    } else if arch.disc.icy(rung.semi_major_m) {
        rng::uniform_in(rng::mix(h), 0.25, 0.5)
    } else {
        let delivery = 0.2 + 0.8 * (giant_jupiters / tuning.belts.oort_saturation_jupiters).min(1.0);
        rng::uniform_in(h, t.delivered_water.0.ln(), t.delivered_water.1.ln()).exp() * delivery
    };

    let mobile = rung.equilibrium_k > WATER_MOBILE_K;
    if margin <= 1.0 && mobile {
        return 0.0;
    }
    if mobile && !magnetic && margin < UNSHIELDED_WATER_MARGIN {
        return 0.0;
    }
    born_with
}

/// What is at the top, which is what reflects.
///
/// Water beats cloud where there is any: Earth has as thick an atmosphere as Venus and reads
/// blue, because the ocean is what a telescope sees. Cloud is what is left when a thick
/// atmosphere has no water under it to be seen through, which is Venus exactly.
fn top_of(class: Class, atmosphere: Atmosphere, equilibrium_k: f64, water_fraction: f64) -> Top {
    const FREEZING_K: f64 = 273.0;
    match atmosphere {
        Atmosphere::Envelope => Top::Cloud,
        _ if water_fraction > WET && equilibrium_k < FREEZING_K => Top::Ice,
        _ if water_fraction > WET => Top::Ocean,
        Atmosphere::Thick => Top::Cloud,
        _ if class == Class::Icy => Top::Ice,
        _ => Top::Rock,
    }
}

/// One turn, seconds, log-uniform inside the class's range.
///
/// Log rather than linear, because the range spans three orders of magnitude and a linear draw
/// would make almost every rocky planet a slow one.
pub fn spin_of(h: u64, giant: bool, t: &super::tuning::World) -> f64 {
    let (lo, hi) = if giant { t.giant_spin_s } else { t.rocky_spin_s };
    (rng::uniform_in(h, lo.ln(), hi.ln())).exp()
}

/// How far this one leans, radians, in `0..=PI`.
pub fn obliquity_of(h: u64, t: &super::tuning::World) -> f64 {
    let lean = if rng::uniform(rng::mix(h)) < t.tumbled_chance {
        // On its side or retrograde, as Uranus and Venus are.
        rng::uniform_in(h, std::f64::consts::FRAC_PI_2, std::f64::consts::PI)
    } else {
        (rng::gaussian(h) * t.typical_obliquity_rad).abs()
    };
    lean.clamp(0.0, std::f64::consts::PI)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::architecture::architecture;
    use crate::sky::{AuthoredStars, StarId, StarProvider};

    fn sun_like(key: u64) -> crate::sky::CatalogueStar {
        let mut s = AuthoredStars::sample().stars()[1].clone();
        s.id = StarId::synthesise("planet", key);
        s.star = crate::star::Star::SOL;
        s.luminosity_solar = 1.0;
        s.mass_solar = 1.0;
        s.metallicity = 0.0;
        s
    }

    fn census(count: u64) -> Vec<Planet> {
        let t = Tuning::default();
        (0..count)
            .flat_map(|k| {
                let s = sun_like(k);
                planets("S", &architecture(&s, &t), &s, &t)
            })
            .collect()
    }

    /// **The calibration.** Nine real bodies, each given only its mass, its radius and its
    /// distance from the Sun, all classified by the same three lines. The pairs are what
    /// matter: Venus and Mars differ only in size, Titan and Luna only in temperature, and the
    /// chain gets both the right way round.
    #[test]
    fn the_solar_system_comes_out_of_the_retention_chain() {
        let t = Tuning::default();
        // Mass and radius in Earth units, semi-major axis in astronomical units.
        let body = |mass: f64, radius: f64, au: f64| {
            air(mass, radius, REFERENCE_K / au.sqrt(), false, &t)
        };
        assert_eq!(body(0.0553, 0.383, 0.387), Atmosphere::None, "Mercury");
        assert_eq!(body(0.815, 0.949, 0.723), Atmosphere::Thick, "Venus");
        assert_eq!(body(1.0, 1.0, 1.0), Atmosphere::Thick, "Earth");
        assert_eq!(body(0.0123, 0.2727, 1.0), Atmosphere::None, "Luna");
        assert_eq!(body(0.107, 0.532, 1.524), Atmosphere::Thin, "Mars");
        assert_eq!(body(0.015, 0.2859, 5.2), Atmosphere::Thin, "Io");
        assert_eq!(body(0.008, 0.2450, 5.2), Atmosphere::Thin, "Europa");
        // Smaller than Luna and colder than Mars, and the only reason it has air is the cold.
        assert_eq!(body(0.0225, 0.404, 9.58), Atmosphere::Thick, "Titan");
        assert_eq!(body(0.00359, 0.2124, 30.07), Atmosphere::Thin, "Triton");
    }

    /// Titan against Luna is the whole argument for heating the exosphere rather than using
    /// the equilibrium temperature: Titan is the lighter of the two and has the atmosphere.
    #[test]
    fn a_cold_body_holds_air_a_warm_one_of_the_same_size_cannot() {
        let t = Tuning::default();
        assert!(exosphere_k(REFERENCE_K, &t) > 1000.0, "Earth's exosphere is measured at 1000 K");
        assert!(exosphere_k(90.0, &t) < 110.0, "and Titan's is barely above its equilibrium");
        let warm = air(0.0225, 0.404, REFERENCE_K, false, &t);
        let cold = air(0.0225, 0.404, 90.0, false, &t);
        assert_eq!((warm, cold), (Atmosphere::None, Atmosphere::Thick));
    }

    /// Venus and Earth: the same size, the same air, and one of them dry. A field is what
    /// keeps the water, and Venus does not have one.
    #[test]
    fn a_field_is_what_decides_whether_the_water_stays() {
        let t = Tuning::default();
        let wet = |magnetic: bool| {
            let margin = retention_margin(18.0, 1.0, 1.0, exosphere_k(REFERENCE_K, &t), &t);
            magnetic || margin >= UNSHIELDED_WATER_MARGIN
        };
        assert!(wet(true), "Earth keeps its oceans");
        assert!(!wet(false), "and would not without a field");

        // Venus's rotation is what costs it the field, not its mass.
        assert!(dynamo(1.0, 86_400.0, rng::hash(&[1]), &t), "Earth turns in a day");
        let venus = (0..200).filter(|k| dynamo(0.815, 243.0 * 86_400.0, rng::hash(&[*k]), &t)).count();
        assert!(venus < 30, "{venus} of 200 Venuses ran a dynamo at 243 days");
        let mars = (0..200).filter(|k| dynamo(0.107, 86_400.0, rng::hash(&[*k]), &t)).count();
        assert!(mars < 40, "{mars} of 200 Marses ran a dynamo at a tenth of an Earth mass");
    }

    /// A body cold enough that its water is ice keeps it whatever the star does, which is why
    /// the outer system is wet and the inner system is not.
    #[test]
    fn frozen_water_is_not_stripped() {
        let planets = census(120);
        let cold: Vec<&Planet> = planets.iter().filter(|p| p.equilibrium_k < 150.0 && !p.class.is_giant()).collect();
        assert!(cold.len() > 20, "only {} cold bodies", cold.len());
        assert!(cold.iter().all(|p| p.water_fraction > 0.1), "a cold body keeps its ice");

        // Rocky bodies only: a hot Jupiter's gravity holds water vapour at any temperature the
        // inner disc reaches, and what it holds is the ice in its core.
        let baked: Vec<&Planet> =
            planets.iter().filter(|p| p.equilibrium_k > 600.0 && !p.class.is_giant()).collect();
        assert!(baked.iter().all(|p| p.water_fraction == 0.0), "nothing holds water at 600 K");
        // Air is a different question: a heavy enough body keeps nitrogen at any temperature
        // the inner disc reaches, which is what a hot super-Earth is.
        assert!(baked.iter().filter(|p| p.mass_earths() < 3.0).all(|p| p.atmosphere == Atmosphere::None));
    }

    /// **What the generator is tuned for.** A habitable planet is a rocky one in the zone with
    /// air over a surface and water on it, and four independent rules have to agree before one
    /// appears -- so the rate is a real measurement of the tuning rather than a dial.
    #[test]
    fn about_one_star_in_two_gets_a_habitable_world() {
        let t = Tuning::default();
        let mut with = 0;
        let n = 300;
        for k in 0..n {
            let s = sun_like(k);
            let arch = architecture(&s, &t);
            if planets("S", &arch, &s, &t).iter().any(|p| p.habitable) {
                with += 1;
            }
        }
        let rate = with as f64 / n as f64;
        assert!((0.35..0.85).contains(&rate), "{with} of {n} stars have a habitable world");
    }

    /// A habitable planet has to be a small one: a heavy rocky core keeps the hydrogen it was
    /// born under, and what is under an envelope has no surface to stand on.
    #[test]
    fn a_habitable_planet_is_a_small_one() {
        for p in census(200).iter().filter(|p| p.habitable) {
            assert!(p.mass_earths() < 10.0, "{} is {} Earths", p.name, p.mass_earths());
            assert_ne!(p.atmosphere, Atmosphere::Envelope);
            assert!(matches!(p.top, Top::Ocean | Top::Rock | Top::Ice));
            assert!(p.water_fraction > 0.0);
        }
    }

    /// Everything a planet carries is finite and in range, because the plots and the renderer
    /// both read all of it.
    #[test]
    fn a_planet_is_always_well_formed() {
        for p in census(150) {
            assert!(p.radius_m > 0.0 && p.mass_kg > 0.0 && p.spin_s > 3600.0);
            assert!((0.0..=std::f64::consts::PI).contains(&p.obliquity_rad));
            assert!((0.0..=1.0).contains(&p.water_fraction), "{} holds {}", p.name, p.water_fraction);
            assert!(p.equilibrium_k.is_finite() && p.equilibrium_k > 0.0);
            if p.class.is_giant() {
                assert_eq!(p.atmosphere, Atmosphere::Envelope, "{} is a giant with no envelope", p.name);
            }
        }
    }
}
