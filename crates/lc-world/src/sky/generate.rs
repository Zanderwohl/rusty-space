//! Procedural systems, deterministic from a star's seed.
//!
//! Nothing here is stored. A world of a hundred thousand systems costs nothing until someone
//! looks at one, and a seed is eight bytes; what a player *changes* becomes an event, and
//! replaying those over this baseline reconstructs the system exactly.

use em_sim::appearance::{Appearance, DebugBall, AppearanceColor};
use em_sim::body::BodyInfo;
use em_sim::motive::kepler::{
    EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerRotation,
    MeanAnomalyAtJ2000,
};
use em_sim::universe::{
    FixedEntry, KeplerEntry, SomeBody, UniverseFileContents, UniverseFileTime, UniversePhysics,
    ViewSettings,
};
use em_spectra::{PerBand, extinction};
use glam::DVec3;

use super::{CatalogueStar, metallicity};
use crate::distribution::{Distribution, Inclination};
use crate::population::Population;
use crate::rng;
use crate::star::Star;

pub const AU: f64 = 1.495_978_707e11;
const EARTH_MASS: f64 = 5.9722e24;
const EARTH_RADIUS: f64 = 6.371e6;

#[derive(Clone, Debug, PartialEq)]
pub struct Planet {
    pub name: String,
    pub semi_major_m: f64,
    pub eccentricity: f64,
    pub inclination_deg: f64,
    pub mean_anomaly_deg: f64,
    pub radius_m: f64,
    pub mass_kg: f64,
    /// How long one turn takes, seconds. What a survey reads off the periodogram of a body's
    /// own flux, so a generated planet without one is a planet whose rotation can never be
    /// measured. See `lightcone/docs/25-system-knowledge.md`.
    pub spin_s: f64,
    /// How far the spin axis leans from the system's pole, radians. Earth is 0.41 and Uranus
    /// is 1.71, so the distribution has to allow both.
    pub obliquity_rad: f64,
    /// Which way it leans, measured about the system pole, radians.
    pub spin_node_rad: f64,
}

/// One star or a barycenter with two, plus everything orbiting it.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedSystem {
    pub name: String,
    /// The primary, and a companion when this is a multiple.
    pub stars: Vec<(String, Star, f64)>,
    /// Separation of the two stars, meters. Zero for a single.
    pub separation_m: f64,
    pub planets: Vec<Planet>,
    pub populations: Vec<Population>,
    /// The normal of the plane every planet and belt orbits in: see [`pole_for`].
    pub pole: DVec3,
}

impl GeneratedSystem {
    pub fn is_multiple(&self) -> bool {
        self.stars.len() > 1
    }

    /// Total mass, in solar masses.
    pub fn mass_solar(&self) -> f64 {
        self.stars.iter().map(|(_, _, m)| m).sum()
    }
}

/// The normal of a system's orbital plane: a direction uniform over the sky, from its seed.
///
/// Every planet and belt shares it, so from most directions nothing transits and from a few
/// the whole system does — the orientation the photometry's priors integrate over. See
/// `lightcone/docs/24-standing-instruments.md`.
pub fn pole_for(seed: u64) -> DVec3 {
    let h = rng::hash(&[seed, 0x9013]);
    let z = rng::uniform(h) * 2.0 - 1.0;
    let phi = rng::uniform(rng::mix(h)) * std::f64::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    DVec3::new(r * phi.cos(), r * phi.sin(), z)
}

/// How long a generated body takes to turn once, seconds.
///
/// Log-uniform between these, by class. A giant is fast because it kept its accretion angular
/// momentum -- Jupiter is 9.9 hours and Saturn 10.6 -- and a rocky planet is anywhere from
/// hours to months, with Venus's 243 days the slow end of what the solar system offers.
const GIANT_SPIN_S: (f64, f64) = (7.0 * 3600.0, 20.0 * 3600.0);
const ROCKY_SPIN_S: (f64, f64) = (4.0 * 3600.0, 250.0 * 86_400.0);

/// How far a planet's spin axis leans from its system's pole, radians.
///
/// Most lean a little and a few lean absurdly: the solar system has six planets under 30
/// degrees and Uranus at 98. Drawn as a gaussian with a heavy tail rather than uniformly,
/// because a system where every planet is on its side would be as wrong as one where none is.
const TYPICAL_OBLIQUITY_RAD: f64 = 0.35;
const TUMBLED_CHANCE: f64 = 0.1;

/// How far a star's spin axis can lie from the plane its planets orbit in.
///
/// The Sun's is 7.25 degrees off the ecliptic, so a star spinning exactly with its planets
/// would be the odd one out. Not a measured distribution: one solar system's worth of evidence
/// does not have a spread in it.
pub const SPIN_TILT_MAX_RAD: f64 = 12.0 * std::f64::consts::PI / 180.0;

/// A star's own spin axis: its system's pole, tilted by a few degrees.
///
/// Deterministic from the seed, because this is where a star's surface features sit and a spin
/// axis that moved between loads would take its sunspots with it.
pub fn spin_axis_for(system_pole: DVec3, seed: u64) -> DVec3 {
    let h = rng::hash(&[seed, 0x5891]);
    // Uniform over the cap rather than over the angle: the latter crowds the pole.
    let cos_max = SPIN_TILT_MAX_RAD.cos();
    let cos_tilt = cos_max + rng::uniform(h) * (1.0 - cos_max);
    let sin_tilt = (1.0 - cos_tilt * cos_tilt).max(0.0).sqrt();
    let phi = rng::uniform(rng::mix(h)) * std::f64::consts::TAU;
    let (u, v) = system_pole.any_orthonormal_pair();
    (system_pole * cos_tilt + (u * phi.cos() + v * phi.sin()) * sin_tilt).normalize_or(system_pole)
}

/// Generate the system around one catalogue star.
pub fn system_for(star: &CatalogueStar) -> GeneratedSystem {
    let seed = star.seed();
    let name = star.provenance.name.clone().unwrap_or_else(|| format!("Star {:016x}", star.id.get()));
    let mut system = GeneratedSystem {
        stars: vec![(name.clone(), star.star, star.mass_solar)],
        separation_m: 0.0,
        planets: planets(seed, star),
        populations: Vec::new(),
        pole: pole_for(seed),
        name,
    };
    system.populations = populations(seed, star, &system.planets);
    system
}

/// A multiple, as a barycenter with two children.
///
/// Hierarchical two-body decomposition, which `em-sim` propagates unmodified. Contact and
/// near-contact systems are excluded rather than modeled: they need more than two-body
/// Keplerian motion.
pub fn binary_for(primary: &CatalogueStar, secondary: &CatalogueStar) -> GeneratedSystem {
    let mut system = system_for(primary);
    let secondary_name =
        secondary.provenance.name.clone().unwrap_or_else(|| format!("Star {:016x}", secondary.id.get()));
    let seed = rng::hash(&[primary.seed(), secondary.id.get()]);

    // Wide enough that neither star fills its Roche lobe, and wide enough not to scatter the
    // planets generated above out of the system.
    let widest_planet = system.planets.last().map(|p| p.semi_major_m).unwrap_or(AU);
    let floor = 20.0 * (primary.star.radius_m + secondary.star.radius_m);
    system.separation_m = (widest_planet * 4.0).max(floor).max(20.0 * AU)
        * rng::uniform_in(rng::hash(&[seed, 1]), 1.0, 6.0);
    system.stars.push((secondary_name, secondary.star, secondary.mass_solar));
    system
}

/// One planet's place in a system, before it is given an orbit's other elements. What the
/// photometry's priors are measured from, so they are the generator's own distribution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rung {
    pub semi_major_m: f64,
    pub rocky: bool,
    pub mass_earths: f64,
    pub radius_earths: f64,
}

/// The planets a star with this seed, luminosity and metallicity is given, innermost first.
pub fn ladder(seed: u64, luminosity_solar: f64, metallicity: f64) -> Vec<Rung> {
    let count = (rng::uniform(rng::hash(&[seed, 0x9001])) * 9.0) as usize;
    let factor = metallicity::solid_mass_factor(metallicity);
    // The habitable-ish scale moves out with luminosity, so hotter stars get wider systems.
    let scale = AU * luminosity_solar.max(1e-4).sqrt();

    let mut out = Vec::with_capacity(count);
    let mut a = scale * rng::uniform_in(rng::hash(&[seed, 0x9002]), 0.2, 0.6);
    for k in 0..count {
        let h = |tag: u64| rng::hash(&[seed, 0x91a4, k as u64, tag]);
        // Geometric spacing, jittered: a Titius-Bode-like ladder without the numerology.
        a *= rng::uniform_in(h(1), 1.4, 2.3);
        let rocky = a < 2.5 * scale;
        let mass_earths = if rocky {
            rng::uniform_in(h(2), 0.02, 6.0) * factor
        } else {
            rng::uniform_in(h(3), 5.0, 400.0) * factor
        };
        // Rocky bodies scale as M^0.27, gas giants barely at all.
        let radius_earths =
            if rocky { mass_earths.powf(0.27) } else { 4.0 * mass_earths.powf(0.08) };
        out.push(Rung { semi_major_m: a, rocky, mass_earths, radius_earths });
    }
    out
}

fn planets(seed: u64, star: &CatalogueStar) -> Vec<Planet> {
    ladder(seed, star.luminosity_solar, star.metallicity)
        .into_iter()
        .enumerate()
        .map(|(k, rung)| {
            let h = |tag: u64| rng::hash(&[seed, 0x91a4, k as u64, tag]);
            Planet {
                name: format!("{} {}", star.provenance.name.as_deref().unwrap_or("b"), (b'b' + k as u8) as char),
                semi_major_m: rung.semi_major_m,
                eccentricity: rng::uniform_in(h(4), 0.0, 0.12),
                inclination_deg: rng::gaussian(h(5)) * 2.0,
                mean_anomaly_deg: rng::uniform_in(h(6), 0.0, 360.0),
                radius_m: rung.radius_earths * EARTH_RADIUS,
                mass_kg: rung.mass_earths * EARTH_MASS,
                spin_s: spin_of(h(7), rung.rocky),
                obliquity_rad: obliquity_of(h(8)),
                spin_node_rad: rng::uniform_in(h(9), 0.0, std::f64::consts::TAU),
            }
        })
        .collect()
}

/// A planet's rotation, as `em-sim` states one.
///
/// The axis is the system's pole leaned by the planet's own obliquity, so a system's planets
/// mostly spin near their orbital plane and occasionally do not. Built as the quaternion that
/// carries `+Z` onto that axis, because [`crate::system::pole_of`] reads the axis back out as
/// `orientation * DVec3::Z` and the two have to agree.
fn spin_of_planet(system_pole: DVec3, planet: &Planet) -> em_sim::body::BodyRotation {
    use em_sim::body::{BodyRotation, RotationEpoch};
    let pole = system_pole.normalize_or(DVec3::Z);
    let (u, v) = pole.any_orthonormal_pair();
    let lean = u * planet.spin_node_rad.cos() + v * planet.spin_node_rad.sin();
    let axis = (pole * planet.obliquity_rad.cos() + lean * planet.obliquity_rad.sin())
        .normalize_or(pole);
    let orientation = glam::DQuat::from_rotation_arc(DVec3::Z, axis);
    // Radians per second. Always positive: which way it turns is the axis's own sign, and an
    // obliquity past a right angle is what retrograde means here.
    let rate = std::f64::consts::TAU / planet.spin_s.max(1.0);
    BodyRotation::spinning(orientation, rate, RotationEpoch::J2000)
}

/// One turn, seconds, log-uniform inside the class's range.
///
/// Log rather than linear, because the range spans three orders of magnitude and a linear draw
/// would make almost every rocky planet a slow one.
fn spin_of(h: u64, rocky: bool) -> f64 {
    let (lo, hi) = if rocky { ROCKY_SPIN_S } else { GIANT_SPIN_S };
    (rng::uniform_in(h, lo.ln(), hi.ln())).exp()
}

/// How far this one leans, radians, in `0..=PI`.
fn obliquity_of(h: u64) -> f64 {
    let tumbled = rng::uniform(rng::mix(h)) < TUMBLED_CHANCE;
    let lean = match tumbled {
        // On its side or retrograde, as Uranus and Venus are.
        true => rng::uniform_in(h, std::f64::consts::FRAC_PI_2, std::f64::consts::PI),
        false => (rng::gaussian(h) * TYPICAL_OBLIQUITY_RAD).abs(),
    };
    lean.clamp(0.0, std::f64::consts::PI)
}

/// The belt, Kuiper analogue and Oort cloud every system gets.
///
/// Masses scale with metallicity: a tenth of the metals is a tenth of the rock. The Oort
/// cloud is photometrically invisible and earns its record by defining the shell radius and
/// holding the volatiles.
fn populations(seed: u64, star: &CatalogueStar, planets: &[Planet]) -> Vec<Population> {
    let pole = pole_for(seed);
    let factor = metallicity::solid_mass_factor(star.metallicity);
    let scale = AU * star.luminosity_solar.max(1e-4).sqrt();
    let outer = planets.last().map(|p| p.semi_major_m).unwrap_or(5.0 * scale);

    let mut dust_response = PerBand::splat(0.0f32);
    for b in em_spectra::Band::ALL {
        dust_response[b] = extinction::RATIO[b] as f32;
    }

    vec![
        // Asteroid belt: narrow, low inclination, mildly eccentric.
        Population {
            pole,
            semi_major: Distribution::normal(outer * 0.4, outer * 0.08, 9),
            eccentricity: Distribution::uniform(0.0, 0.25, 5),
            inclination: Inclination::uniform_angle(0.0, 0.2, 12),
            count: 1e6 * factor,
            cross_section: 3.0e6,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        },
        // Kuiper analogue: wide, cold, many small bodies.
        Population {
            pole,
            semi_major: Distribution::uniform(outer * 1.2, outer * 3.0, 9),
            eccentricity: Distribution::uniform(0.0, 0.2, 5),
            inclination: Inclination::uniform_angle(0.0, 0.35, 12),
            count: 1e9 * factor,
            cross_section: 7.8e9,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        },
        // Oort cloud: isotropic, very wide, nearly parabolic. Invisible, and the reason the
        // shell radius is where it is.
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::uniform(2_000.0 * AU, 100_000.0 * AU, 9),
            eccentricity: Distribution::uniform(0.6, 0.95, 5),
            inclination: Inclination::isotropic(),
            count: 1e12 * factor,
            cross_section: 3.1e6,
            band_response: dust_response,
            radiating_ratio: Population::SPHERICAL,
        },
    ]
    .into_iter()
    .map(|mut p| {
        p.count *= rng::uniform_in(rng::hash(&[seed, 0xc10d, p.count.to_bits()]), 0.5, 2.0);
        p
    })
    // After the jitter: a swarm's coverage is drawn deliberately and is not a natural
    // population with an uncertain mass.
    .chain(swarm(seed, star))
    .collect()
}

/// Fraction of systems carrying an engineered swarm.
///
/// Low on purpose. What makes a technosignature worth anything is that most stars do not have
/// one; a sky where every third star is engineered is a sky nobody searches.
pub const SWARM_FRACTION: f64 = 0.03;

/// An engineered swarm, if this star has one. See [`swarm_for`] for the public entry.
///
/// Coverage is log-uniform from a thousandth to nine tenths, which is the range that makes the
/// instrument worth having. At the bottom it is a few tenths of a percent of gray deficit and a
/// thermal excess that needs integrating to see at all. At the top the star is most of a
/// magnitude down in V and brighter at ten microns than in the visible.
///
/// Isotropic, circular and gray. Those three together are the signature, and no natural
/// population has all three: an isotropic natural population is an Oort cloud, which is
/// eccentric and made of dust, and dust reddens where panels do not.
/// Whether a star has a swarm, without generating its whole system.
///
/// The renderer needs this for every star in the sky and a full system for almost none of them,
/// so the draw is separable: one hash per star rather than a planet set and three populations.
pub fn swarm_for(star: &CatalogueStar) -> Option<Population> {
    swarm(star.seed(), star)
}

fn swarm(seed: u64, star: &CatalogueStar) -> Option<Population> {
    if rng::uniform(rng::hash(&[seed, 0x5761_726d])) > SWARM_FRACTION {
        return None;
    }
    let u = rng::uniform(rng::hash(&[seed, 0x436f_7665]));
    let coverage = 1.0e-3f64.powf(1.0 - u) * 0.9f64.powf(u);

    // Where the light is: the radius at which a collector sees about what Earth sees.
    let radius = AU * star.luminosity_solar.max(1e-4).sqrt();
    // A square kilometer apiece, which is a size the moment inversion can recover.
    let element = 1.0e6;

    Some(Population {
        pole: DVec3::Z,
        semi_major: Distribution::normal(radius, radius * 0.05, 9),
        eccentricity: Distribution::uniform(0.0, 0.02, 3),
        inclination: Inclination::isotropic(),
        count: coverage * 4.0 * std::f64::consts::PI * radius * radius / element,
        cross_section: element,
        band_response: PerBand::splat(1.0),
        radiating_ratio: Population::PANEL,
    })
}

fn debug_ball(radius: f64, rgb: (u16, u16, u16)) -> Appearance {
    Appearance::DebugBall(DebugBall {
        radius,
        color: AppearanceColor { r: rgb.0, g: rgb.1, b: rgb.2 },
        highlight_latitudes: Vec::new(),
    })
}

fn info(id: &str, mass: f64, major: bool, tags: &[&str]) -> BodyInfo {
    BodyInfo {
        name: Some(id.to_string()),
        id: id.to_string(),
        mass,
        major,
        designation: None,
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

fn kepler(primary: &str, a: f64, e: f64, inc_deg: f64, node_deg: f64, anomaly_deg: f64, mu: Option<f64>) -> KeplerMotive {
    KeplerMotive {
        primary_id: primary.to_string(),
        shape: KeplerShapeEcc(a, e),
        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
            inclination: inc_deg,
            longitude_of_ascending_node: node_deg,
            argument_of_periapsis: 0.0,
        }),
        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly: anomaly_deg }),
        anomalistic_period: None,
        gravitational_parameter: mu,
    }
}

#[allow(non_snake_case)]
fn KeplerShapeEcc(a: f64, e: f64) -> em_sim::motive::kepler::KeplerShape {
    em_sim::motive::kepler::KeplerShape::EccentricitySMA(EccentricitySMA {
        eccentricity: e,
        semi_major_axis: a,
    })
}

impl GeneratedSystem {
    /// Convert to the form `em_sim::system::System::from_contents` consumes.
    ///
    /// A multiple becomes a fixed barycenter with two Keplerian children, which is the
    /// hierarchical decomposition `em-sim` already propagates. Planets orbit the barycenter
    /// in that case and the primary otherwise.
    pub fn to_universe(&self) -> UniverseFileContents {
        const SOLAR_MASS: f64 = 1.988_41e30;
        let mut bodies = Vec::with_capacity(self.planets.len() + self.stars.len() + 1);

        let center = if self.is_multiple() {
            let name = format!("{} Barycenter", self.name);
            bodies.push(SomeBody::FixedEntry(FixedEntry {
                info: info(&name, 0.0, false, &["Barycenter"]),
                position: DVec3::ZERO,
                appearance: Appearance::Empty,
                rotation: None,
            }));
            let total: f64 = self.mass_solar();
            for (k, (star_name, star, mass_solar)) in self.stars.iter().enumerate() {
                // Each star orbits the barycenter at a radius set by the *other* star's share
                // of the mass, and both must share one period. Matching
                // 2 pi sqrt(r^3 / mu) to 2 pi sqrt(d^3 / (G M)) gives mu = G M share^3 --
                // not G times either star's own mass, which is why em-sim lets this be
                // stated explicitly.
                let share = (total - mass_solar) / total;
                let g_per_solar_mass = star.mu / mass_solar;
                bodies.push(SomeBody::KeplerEntry(KeplerEntry {
                    info: info(star_name, mass_solar * SOLAR_MASS, true, &["Star"]),
                    params: kepler(
                        &name,
                        self.separation_m * share,
                        0.0,
                        0.0,
                        0.0,
                        180.0 * k as f64,
                        Some(g_per_solar_mass * total * share.powi(3)),
                    ),
                    appearance: debug_ball(star.radius_m, (255, 240, 200)),
                    rotation: None,
                }));
            }
            name
        } else {
            let (star_name, star, mass_solar) = &self.stars[0];
            bodies.push(SomeBody::FixedEntry(FixedEntry {
                info: info(star_name, mass_solar * SOLAR_MASS, true, &["Star"]),
                position: DVec3::ZERO,
                appearance: debug_ball(star.radius_m, (255, 240, 200)),
                rotation: None,
            }));
            star_name.clone()
        };

        // The system's plane as Euler angles: a normal (sin i sin O, -sin i cos O, cos i).
        let tilt_deg = self.pole.z.clamp(-1.0, 1.0).acos().to_degrees();
        let node_deg = self.pole.x.atan2(-self.pole.y).to_degrees();
        for p in &self.planets {
            bodies.push(SomeBody::KeplerEntry(KeplerEntry {
                info: info(&p.name, p.mass_kg, false, &["Planet"]),
                params: kepler(
                    &center,
                    p.semi_major_m,
                    p.eccentricity,
                    tilt_deg + p.inclination_deg,
                    node_deg,
                    p.mean_anomaly_deg,
                    None,
                ),
                appearance: debug_ball(p.radius_m, (140, 140, 160)),
                rotation: Some(spin_of_planet(self.pole, p)),
            }));
        }

        UniverseFileContents {
            version: "1".to_string(),
            time: UniverseFileTime {
                time_julian_days: 2_451_545.0,
                step: 0.1,
                gui_speed: 1.0,
                max_frame_time: 0.016,
            },
            view: ViewSettings::default(),
            physics: UniversePhysics::default(),
            bodies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{AuthoredStars, StarProvider};
    use em_foundations::time::Instant;
    use em_sim::system::System;

    fn sun_like() -> CatalogueStar {
        AuthoredStars::sample().stars()[1].clone()
    }

    fn build(system: &GeneratedSystem) -> System {
        System::from_contents(&system.to_universe()).expect("generated system must load")
    }

    /// **A generated planet with no rotation is one whose spin can never be measured**, which
    /// is what phase 6 reads off the periodogram of its flux. Every one has a spin now, and the
    /// axis reads back out the way `system::pole_of` extracts it.
    #[test]
    fn every_generated_planet_spins_about_a_readable_axis() {
        let star = sun_like();
        let system = system_for(&star);
        let sim = build(&system);
        assert!(!system.planets.is_empty(), "nothing to spin");

        let mut leaning = 0;
        for planet in &system.planets {
            assert!(planet.spin_s > 3600.0, "{}: {} s is not a rotation", planet.name, planet.spin_s);
            assert!(planet.spin_s < 300.0 * 86_400.0, "{}: slower than any planet", planet.name);
            if planet.obliquity_rad > 0.05 {
                leaning += 1;
            }
        }
        assert!(leaning > 0, "no planet leans at all");

        // The axis em-sim hands back is the one the obliquity describes.
        for i in sim.indices() {
            if crate::navigation::Kind::of(&sim.info(i).tags) != crate::navigation::Kind::Planet {
                continue;
            }
            let rotation = sim.rotation(i).expect("a generated planet has a rotation");
            let axis = crate::system::pole_of(rotation).expect("and a readable axis");
            let planet = system
                .planets
                .iter()
                .find(|p| p.name == sim.name(i))
                .expect("every sim planet came from a generated one");
            let lean = axis.dot(system.pole).clamp(-1.0, 1.0).acos();
            assert!(
                (lean - planet.obliquity_rad).abs() < 1.0e-9,
                "{}: leans {lean} against {}",
                planet.name,
                planet.obliquity_rad,
            );
        }
    }

    /// Spins are log-uniform over three orders of magnitude, so a sample has to hold both fast
    /// and slow ones -- a linear draw would make almost everything slow.
    #[test]
    fn generated_spins_span_hours_to_months() {
        let mut fast = 0;
        let mut slow = 0;
        for key in 0..400u64 {
            if spin_of(rng::hash(&[key, 7]), true) < 12.0 * 3600.0 {
                fast += 1;
            }
            if spin_of(rng::hash(&[key, 7]), true) > 30.0 * 86_400.0 {
                slow += 1;
            }
        }
        assert!(fast > 20, "only {fast} of 400 turn in under half a day");
        assert!(slow > 20, "only {slow} of 400 take over a month");
    }

    /// Sol's planets are fitted against JPL in the ecliptic of J2000, so its plane is `+Z` and
    /// not the pole its seed would have given it.
    #[test]
    fn sol_orbits_the_ecliptic_and_everything_else_orbits_its_own_pole() {
        let mut sol = sun_like();
        sol.provenance.name = Some(crate::system::SOL.to_string());
        assert_eq!(sol.system_pole(), DVec3::Z);

        let other = sun_like();
        assert_eq!(other.system_pole(), pole_for(other.seed()));
        assert!(other.system_pole().dot(DVec3::Z).abs() < 0.999, "a generated pole that is +Z");
    }

    /// A star spins near its planets' plane but not exactly in it, and the same star always
    /// spins the same way -- its surface features are pinned to this.
    #[test]
    fn a_star_spins_near_its_planets_plane_but_not_in_it() {
        let mut tilted = 0;
        for key in 0..200u64 {
            let pole = pole_for(key);
            let axis = spin_axis_for(pole, key);
            assert!((axis.length() - 1.0).abs() < 1.0e-12, "{key}: not a unit vector");
            let tilt = axis.dot(pole).clamp(-1.0, 1.0).acos();
            assert!(tilt <= SPIN_TILT_MAX_RAD + 1.0e-12, "{key}: {}° off", tilt.to_degrees());
            if tilt > 1.0e-6 {
                tilted += 1;
            }
            assert_eq!(axis, spin_axis_for(pole, key), "{key}: not reproducible");
        }
        assert!(tilted > 190, "only {tilted} of 200 stars are tilted at all");
    }

    #[test]
    fn the_same_seed_gives_the_same_system() {
        let star = sun_like();
        assert_eq!(system_for(&star), system_for(&star));
        // And a different star gives a different one.
        let other = AuthoredStars::sample().stars()[2].clone();
        assert_ne!(system_for(&star).planets, system_for(&other).planets);
    }

    #[test]
    fn planets_are_ordered_outward_and_physically_plausible() {
        for s in AuthoredStars::sample().stars() {
            let sys = system_for(s);
            let mut prev = 0.0;
            for p in &sys.planets {
                assert!(p.semi_major_m > prev, "{} is not outside its neighbor", p.name);
                prev = p.semi_major_m;
                assert!((0.0..0.2).contains(&p.eccentricity));
                assert!(p.radius_m > 0.0 && p.mass_kg > 0.0);
                assert!(p.radius_m < 2e9, "{} is larger than a star", p.name);
            }
        }
    }

    #[test]
    fn every_system_gets_a_belt_a_kuiper_analogue_and_an_oort_cloud() {
        let sys = system_for(&sun_like());
        assert_eq!(sys.populations.len(), 3);
        let oort = sys.populations.last().unwrap();
        assert!(oort.semi_major.mean() > 1000.0 * AU, "the Oort cloud must be far out");
        // Invisible, which is the correct answer and costs one record to say.
        let deficit = oort.mean_deficit(DVec3::X, &sys.stars[0].1);
        assert!(deficit < 1e-10, "an Oort cloud should not be detectable: {deficit}");
    }

    #[test]
    fn metallicity_drives_how_much_rock_there_is() {
        let mut rich = sun_like();
        let mut poor = rich.clone();
        rich.metallicity = 0.3;
        poor.metallicity = -1.5;
        let (r, p) = (system_for(&rich), system_for(&poor));
        let mass = |s: &GeneratedSystem| s.populations.iter().map(|x| x.count * x.cross_section).sum::<f64>();
        assert!(mass(&r) / mass(&p) > 20.0, "metal-poor systems must be thin on solids");
    }

    #[test]
    fn a_single_system_loads_and_propagates() {
        let sys = system_for(&sun_like());
        let mut sim = build(&sys);
        assert_eq!(sim.len(), sys.planets.len() + 1);
        for days in [0.0, 100.0, 3650.0] {
            em_sim::propagate::evaluate_at(&mut sim, Instant::from_seconds_since_j2000(days * 86_400.0));
            for i in sim.indices() {
                assert!(sim.position(i).is_finite(), "a body left the universe at day {days}");
            }
        }
    }

    /// The phase's structural check: a multiple is a barycenter with two children, and
    /// `em-sim` propagates it unmodified.
    #[test]
    fn a_binary_orbits_its_barycenter() {
        let stars = AuthoredStars::sample();
        let sys = binary_for(&stars.stars()[1], &stars.stars()[2]);
        assert!(sys.is_multiple());
        assert!(sys.separation_m > 0.0);

        let mut sim = build(&sys);
        let barycenter = sim.by_name(&format!("{} Barycenter", sys.name)).expect("barycenter");
        let a = sim.by_name(&sys.stars[0].0).expect("primary");
        let b = sim.by_name(&sys.stars[1].0).expect("secondary");

        let mut separations = Vec::new();
        for days in [0.0, 2_000.0, 9_000.0, 40_000.0] {
            let t = Instant::from_seconds_since_j2000(days * 86_400.0);
            em_sim::propagate::evaluate_at(&mut sim, t);
            let (pa, pb, pc) = (sim.position(a), sim.position(b), sim.position(barycenter));
            assert!(pa.is_finite() && pb.is_finite());
            assert_eq!(pc, DVec3::ZERO, "the barycenter is the frame");
            // Both stars are always on opposite sides of it.
            assert!(pa.dot(pb) < 0.0, "stars must stay opposed at day {days}");
            separations.push(pa.distance(pb));
        }
        // Circular, so the separation holds.
        let first = separations[0];
        for s in &separations {
            assert!((s / first - 1.0).abs() < 1e-6, "separation drifted: {separations:?}");
        }
        // The heavier star sits closer in.
        em_sim::propagate::evaluate_at(&mut sim, Instant::J2000);
        let heavier_first = sys.stars[0].2 > sys.stars[1].2;
        assert_eq!(heavier_first, sim.position(a).length() < sim.position(b).length());
    }

    #[test]
    fn a_binary_keeps_its_planets_well_inside_the_pair() {
        let stars = AuthoredStars::sample();
        let sys = binary_for(&stars.stars()[1], &stars.stars()[2]);
        for p in &sys.planets {
            assert!(p.semi_major_m * 3.0 < sys.separation_m, "{} is not safely inside", p.name);
        }
    }
}
