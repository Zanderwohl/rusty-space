//! What kind of world a body is, as a posterior over what the generator makes.
//!
//! A survey measures a radius, a density, a temperature and a color. None of those is a type,
//! and no threshold on any of them is one either: a 1.4-Earth-radius body is a super-Earth or a
//! small sub-Neptune depending on its density, an ocean and an ice world differ only in how
//! bright and how blue they are, and every one of those numbers has an error bar. So the answer
//! is a list of types with probabilities, and the prior is the generator itself -- the same
//! code that made the body, sampled over a neighborhood of stars.
//!
//! Same shape as [`super::prior::Prior::rocky_given`] and for the same reason: the *spread* is
//! modeled, not just the measurement. Two ocean worlds are not the same color, and a
//! classification that assumed they were would be certain and wrong.

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use crate::sky::CatalogStar;
use crate::sky::generate::architecture::Class;
use crate::sky::generate::{disc, planets_of};
use crate::worlds::{Atmosphere, Top, World};

/// Stars sampled to build the prior. Beyond this the answer stops moving and the cost does not.
const SORT_STARS: usize = 600;

/// Floors on the width each observable is compared at, in log units.
///
/// Not error bars: these are how much two bodies of one type differ from each other. A
/// measurement tighter than the type's own spread cannot be more certain than the spread, and
/// pretending otherwise is what turns a classification into a lookup.
const RADIUS_WIDTH: f64 = 0.04;
const DENSITY_WIDTH: f64 = 0.10;
const TEMPERATURE_WIDTH: f64 = 0.03;
/// Wider than the rest, because [`crate::worlds`] varies a body's brightness by fifteen percent
/// about its type's curve and that is the whole reason this is an inference.
const ALBEDO_WIDTH: f64 = 0.18;
const COLOR_WIDTH: f64 = 0.07;

/// What kind of world, in the terms a survey can tell apart.
///
/// Derived from what the generator decided and nothing else, so no threshold is applied twice.
/// See `lightcone/docs/26-system-generation.md`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sort {
    /// A core that reached runaway and took all the hydrogen it could. Banded.
    GasGiant,
    /// A core that took a little. Methane-blue, and darker in the near infrared for it.
    IceGiant,
    /// A rocky or icy core that kept the hydrogen it was born under. The commonest planet
    /// there is.
    Subneptune,
    /// Liquid water at the top.
    Ocean,
    /// An opaque deck with nothing wet under it. Venus.
    Greenhouse,
    /// A trace of air over dry ground. Mars.
    Desert,
    /// Ice at the top, whatever is under it.
    IceWorld,
    /// Airless rock.
    Barren,
    /// Airless rock hot enough to run.
    Molten,
}

impl Sort {
    pub const ALL: [Sort; 9] = [
        Sort::GasGiant,
        Sort::IceGiant,
        Sort::Subneptune,
        Sort::Ocean,
        Sort::Greenhouse,
        Sort::Desert,
        Sort::IceWorld,
        Sort::Barren,
        Sort::Molten,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sort::GasGiant => "a gas giant",
            Sort::IceGiant => "an ice giant",
            Sort::Subneptune => "a sub-Neptune",
            Sort::Ocean => "an ocean world",
            Sort::Greenhouse => "a greenhouse world",
            Sort::Desert => "a desert world",
            Sort::IceWorld => "an ice world",
            Sort::Barren => "a barren world",
            Sort::Molten => "a molten world",
        }
    }

    /// Whether a ship could stand on it.
    pub fn has_a_surface(self) -> bool {
        !matches!(self, Sort::GasGiant | Sort::IceGiant | Sort::Subneptune)
    }

    /// What kind of world the generator made, from what it decided about it.
    pub fn of(class: Class, atmosphere: Atmosphere, top: Top, equilibrium_k: f64) -> Sort {
        if atmosphere == Atmosphere::Envelope {
            return match class {
                Class::GasGiant => Sort::GasGiant,
                Class::IceGiant => Sort::IceGiant,
                _ => Sort::Subneptune,
            };
        }
        match top {
            Top::Ocean => Sort::Ocean,
            Top::Cloud => Sort::Greenhouse,
            Top::Ice => Sort::IceWorld,
            // The surface classification's own threshold, so hot is decided in one place.
            Top::Rock if equilibrium_k > crate::surface::SCORCHED_K => Sort::Molten,
            Top::Rock if atmosphere == Atmosphere::None => Sort::Barren,
            Top::Rock => Sort::Desert,
        }
    }
}

/// What a craft has measured about a body. Every field is optional, because a survey gets them
/// one at a time and an answer from three of them is worth having.
///
/// Each is a value and one sigma. Colors are *reflectance* ratios: a craft measures a flux
/// ratio and divides out the star it already measured, so what is left is the body.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    pub radius_earths: Option<(f64, f64)>,
    pub density_kg_m3: Option<(f64, f64)>,
    /// Zero-albedo equilibrium temperature, kelvin. From the orbit and the star, or from the
    /// body's own thermal infrared.
    pub equilibrium_k: Option<(f64, f64)>,
    /// Geometric albedo over the optical run.
    pub albedo: Option<(f64, f64)>,
    /// Reflectance in R over reflectance in B: how red it is.
    pub red: Option<(f64, f64)>,
    /// Reflectance in K over reflectance in R: how far the near infrared falls away, which is
    /// what methane does to an ice giant.
    pub methane: Option<(f64, f64)>,
}

impl Measured {
    /// Whether anything has been measured at all.
    pub fn is_empty(&self) -> bool {
        self.observables().all(|o| o.is_none())
    }

    fn observables(&self) -> impl Iterator<Item = Option<(f64, f64)>> {
        [self.radius_earths, self.density_kg_m3, self.equilibrium_k, self.albedo, self.red, self.methane]
            .into_iter()
    }

    /// What a craft's own file on a body says, read as the six numbers.
    ///
    /// Nothing here is a new measurement: every number is already in the file with a witness on
    /// it, so a type needs no record of its own and moves when a better measurement arrives.
    ///
    /// Colors divide the star out: the range to the body and the star's output both cancel in a
    /// flux ratio, which is why a color is what a distant craft reads cleanly.
    ///
    /// The albedo is left unmeasured. It needs the body's distance from its star *and* the
    /// range to the craft, and a belief carries neither.
    pub fn from_belief(belief: &super::BodyBelief, star: &crate::star::Star) -> Self {
        let radius_earths = belief.radius_m.map(|(r, s)| (r / disc::EARTH_RADIUS, s / disc::EARTH_RADIUS));
        let density = match (belief.radius_m, belief.mass_kg) {
            (Some((r, rs)), Some((m, ms))) if r > 0.0 && m > 0.0 => {
                let volume = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
                let value = m / volume;
                // Three radii in the volume, so its fractional error counts three times.
                let fraction = ((ms / m).powi(2) + 9.0 * (rs / r).powi(2)).sqrt();
                Some((value, value * fraction))
            }
            _ => None,
        };
        let equilibrium = belief.semi_major_au.map(|(a, sigma)| {
            let value = equilibrium_at(star, a * crate::sky::generate::AU);
            // Temperature goes as the inverse square root of the radius.
            (value, value * 0.5 * (sigma / a.max(f64::MIN_POSITIVE)).abs())
        });
        let color = |over: Band, under: Band| {
            let (ratio, sigma) = belief.colors.as_ref()?.color(over, under)?;
            let sun = em_spectra::blackbody::band_radiance(over, star.teff_k)
                / em_spectra::blackbody::band_radiance(under, star.teff_k);
            (sun > 0.0 && sun.is_finite()).then_some((ratio / sun, sigma / sun))
        };
        Self {
            radius_earths,
            density_kg_m3: density,
            equilibrium_k: equilibrium,
            albedo: None,
            red: color(Band::R, Band::B),
            methane: color(Band::K, Band::R),
        }
    }

    /// What a survey reads off a body it has been close to.
    ///
    /// Radius and density come from proximity, temperature from the orbit and the star, colors
    /// from the per-band photometry with the star divided out.
    pub fn of(world: &World, radius_earths: f64, density_kg_m3: f64, equilibrium_k: f64) -> Self {
        let reflect = |b: Band| world.reflectance_in(b);
        Self {
            radius_earths: Some((radius_earths, 0.0)),
            density_kg_m3: Some((density_kg_m3, 0.0)),
            equilibrium_k: Some((equilibrium_k, 0.0)),
            albedo: Some((world.gray_albedo(), 0.0)),
            red: ratio(reflect(Band::R), reflect(Band::B)),
            methane: ratio(reflect(Band::K), reflect(Band::R)),
        }
    }
}

fn ratio(over: f64, under: f64) -> Option<(f64, f64)> {
    (over > 0.0 && under > 0.0).then_some((over / under, 0.0))
}

/// One body the generator made, as the six numbers a survey would read off it.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    sort: Sort,
    radius_earths: f64,
    density_kg_m3: f64,
    equilibrium_k: f64,
    albedo: f64,
    red: Option<f64>,
    methane: Option<f64>,
}

/// The generator's own population of worlds, as a prior over what a body might be.
#[derive(Clone, Debug, Default)]
pub struct Sorts {
    drawn: Vec<Drawn>,
}

impl Sorts {
    /// Sample the generator over the stars a craft cannot tell this one apart from.
    pub fn measure<'a>(stars: impl IntoIterator<Item = &'a CatalogStar>) -> Self {
        let stars: Vec<&CatalogStar> = stars.into_iter().collect();
        let stride = stars.len().div_ceil(SORT_STARS).max(1);
        let mut drawn = Vec::new();
        for star in stars.iter().step_by(stride) {
            for planet in planets_of(star) {
                drawn.push(Drawn::of(&planet, star));
            }
        }
        Self { drawn }
    }

    pub fn is_empty(&self) -> bool {
        self.drawn.is_empty()
    }

    /// What kind of world this is, most probable first.
    ///
    /// Every drawn body is weighted by how well it matches on every observable there is and
    /// the weights are summed by type. Nothing when nothing has been measured, and nothing when
    /// the generator makes nothing like this -- which is an answer, not an even split.
    pub fn given(&self, measured: &Measured) -> Vec<(Sort, f64)> {
        if measured.is_empty() || self.drawn.is_empty() {
            return Vec::new();
        }
        let mut weight = [0.0f64; Sort::ALL.len()];
        let mut total = 0.0;
        for body in &self.drawn {
            let w = body.likelihood(measured);
            if w > 0.0 && let Some(slot) = weight.get_mut(body.sort as usize) {
                *slot += w;
                total += w;
            }
        }
        if !(total > 0.0) {
            return Vec::new();
        }
        let mut out: Vec<(Sort, f64)> = Sort::ALL
            .iter()
            .zip(weight)
            .filter(|(_, w)| *w > 0.0)
            .map(|(sort, w)| (*sort, w / total))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    /// The most probable type, when one stands far enough out to be worth stating.
    pub fn leading(&self, measured: &Measured, settled: f64) -> Option<Sort> {
        self.given(measured).first().filter(|(_, p)| *p >= settled).map(|(s, _)| *s)
    }
}

impl Drawn {
    fn of(planet: &crate::sky::generate::Planet, star: &CatalogStar) -> Self {
        let radius_earths = planet.radius_earths();
        let volume = 4.0 / 3.0 * std::f64::consts::PI * planet.radius_m.powi(3);
        let world = planet.world(star);
        Self {
            sort: Sort::of(planet.class, planet.atmosphere, planet.top, planet.equilibrium_k),
            radius_earths,
            density_kg_m3: if volume > 0.0 { planet.mass_kg / volume } else { 0.0 },
            equilibrium_k: planet.equilibrium_k,
            albedo: world.gray_albedo(),
            red: ratio(world.reflectance_in(Band::R), world.reflectance_in(Band::B)).map(|(r, _)| r),
            methane: ratio(world.reflectance_in(Band::K), world.reflectance_in(Band::R)).map(|(r, _)| r),
        }
    }

    /// How well this body matches what was measured, as a product of gaussians in log.
    ///
    /// Log because each is positive, spans decades, and is delivered as a fractional error.
    fn likelihood(&self, measured: &Measured) -> f64 {
        let mut chi2 = 0.0;
        let mut compare = |seen: Option<(f64, f64)>, drawn: Option<f64>, floor: f64| {
            let (Some((value, sigma)), Some(drawn)) = (seen, drawn) else { return };
            if !(value > 0.0 && drawn > 0.0) {
                return;
            }
            let width = floor.max(sigma / value);
            let miss = (drawn.ln() - value.ln()) / width;
            chi2 += miss * miss;
        };
        compare(measured.radius_earths, Some(self.radius_earths), RADIUS_WIDTH);
        compare(measured.density_kg_m3, Some(self.density_kg_m3), DENSITY_WIDTH);
        compare(measured.equilibrium_k, Some(self.equilibrium_k), TEMPERATURE_WIDTH);
        compare(measured.albedo, Some(self.albedo), ALBEDO_WIDTH);
        compare(measured.red, self.red, COLOR_WIDTH);
        compare(measured.methane, self.methane, COLOR_WIDTH);
        (-0.5 * chi2).exp()
    }
}

/// Zero-albedo equilibrium temperature of a body at this distance from a star, kelvin.
///
/// Here so a caller assembling a [`Measured`] from an orbit does not open-code it.
pub fn equilibrium_at(star: &crate::star::Star, semi_major_m: f64) -> f64 {
    disc::temperature_at(star, semi_major_m)
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::sky::{AuthoredStars, StarId, StarProvider};

    /// A main-sequence star of this luminosity, with the columns a catalog would give it.
    pub fn star_of(key: u64, luminosity: f64) -> CatalogStar {
        let teff = 5772.0 * luminosity.powf(0.13);
        let mut s = AuthoredStars::sample().stars()[1].clone();
        s.id = StarId::synthesize("sorts", key);
        s.luminosity_solar = luminosity;
        s.star.teff_k = teff;
        s.star.radius_m = em_spectra::stellar::radius_from_luminosity(
            luminosity * em_spectra::stellar::SOLAR_LUMINOSITY,
            teff,
        );
        s.mass_solar = em_spectra::stellar::main_sequence_mass_solar(luminosity);
        s.star.mu = em_spectra::stellar::mu_from_mass_solar(s.mass_solar);
        s.metallicity = 0.0;
        s
    }

    pub fn neighborhood(keys: std::ops::Range<u64>) -> Vec<CatalogStar> {
        keys.map(|k| {
            let u = crate::rng::uniform(crate::rng::hash(&[k, 0x1u64]));
            star_of(k, 10f64.powf(-2.0 + 3.0 * u * u))
        })
        .collect()
    }

    /// Truth and a perfect reading of it, for every planet of these stars.
    pub fn truths(stars: &[CatalogStar]) -> Vec<(Sort, Measured)> {
        stars
            .iter()
            .flat_map(|s| {
                planets_of(s).into_iter().map(move |p| {
                    let drawn = Drawn::of(&p, s);
                    let volume = 4.0 / 3.0 * std::f64::consts::PI * p.radius_m.powi(3);
                    let world = p.world(s);
                    (
                        drawn.sort,
                        Measured::of(&world, p.radius_earths(), p.mass_kg / volume, p.equilibrium_k),
                    )
                })
            })
            .collect()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use super::tests_support::*;

    /// The test this module exists for: build the prior from one set of stars, then classify
    /// the planets of stars it has never seen.
    #[test]
    fn a_measured_world_is_recognized_for_what_it_is() {
        let prior = Sorts::measure(&neighborhood(0..400));
        assert!(!prior.is_empty());

        let cases = truths(&neighborhood(10_000..10_120));
        assert!(cases.len() > 800, "only {} bodies to classify", cases.len());
        let right = cases
            .iter()
            .filter(|(truth, measured)| prior.given(measured).first().is_some_and(|(s, _)| s == truth))
            .count();
        let rate = right as f64 / cases.len() as f64;
        assert!(rate > 0.97, "only {right} of {} classified correctly ({rate:.2})", cases.len());

        // And the probability it states is honest: what it calls settled is almost always so.
        let (mut claimed, mut correct) = (0, 0);
        for (truth, measured) in &cases {
            if let Some(sort) = prior.leading(measured, 0.9) {
                claimed += 1;
                correct += usize::from(sort == *truth);
            }
        }
        assert!(claimed > cases.len() / 3, "only {claimed} settled calls out of {}", cases.len());
        assert!(
            correct as f64 / claimed as f64 > 0.95,
            "{correct} of {claimed} settled calls were right"
        );
    }

    /// A posterior is a distribution: it sums to one, it runs most probable first, and it names
    /// no type twice.
    #[test]
    fn what_comes_back_is_a_distribution() {
        let prior = Sorts::measure(&neighborhood(0..200));
        for (_, measured) in truths(&neighborhood(20_000..20_020)) {
            let out = prior.given(&measured);
            assert!(!out.is_empty());
            let total: f64 = out.iter().map(|(_, p)| p).sum();
            assert!((total - 1.0).abs() < 1.0e-9, "{total}");
            for pair in out.windows(2) {
                assert!(pair[0].1 >= pair[1].1, "not ordered: {out:?}");
            }
            let mut seen: Vec<Sort> = out.iter().map(|(s, _)| *s).collect();
            seen.sort_by_key(|s| *s as usize);
            seen.dedup();
            assert_eq!(seen.len(), out.len(), "a type named twice");
        }
    }

    /// Where the color earns its place, and it is one place.
    ///
    /// Radius, density and temperature get nineteen in twenty right on their own, because the
    /// retention chain is a function of exactly those. The exception is Venus against Earth:
    /// same size, same density, nearly the same temperature, separated by whether there is
    /// water under the air, which a magnetic field decides and none of the three shows. A
    /// color takes that error from one in five to one in two hundred.
    #[test]
    fn color_is_what_tells_an_ocean_from_a_deck() {
        let prior = Sorts::measure(&neighborhood(0..400));
        let cases: Vec<(Sort, Measured)> = truths(&neighborhood(30_000..30_120))
            .into_iter()
            .filter(|(s, _)| matches!(s, Sort::Ocean | Sort::Greenhouse))
            .collect();
        assert!(cases.len() > 120, "only {} oceans and decks", cases.len());

        let score = |strip: fn(&Measured) -> Measured| {
            cases
                .iter()
                .filter(|(truth, m)| prior.given(&strip(m)).first().is_some_and(|(s, _)| s == truth))
                .count()
        };
        let blind = score(|m| Measured { albedo: None, red: None, methane: None, ..*m });
        let seeing = score(|m| *m);
        assert!(blind * 10 < cases.len() * 9, "bulk properties alone should miss some: {blind}");
        assert!(
            seeing * 100 > cases.len() * 97,
            "color should settle it: {seeing} against {blind} of {}",
            cases.len()
        );
    }

    /// A poor measurement gives a broad answer, which is what stops the panel naming a type
    /// off one distant glance.
    #[test]
    fn a_loose_measurement_settles_nothing() {
        let prior = Sorts::measure(&neighborhood(0..300));
        let cases = truths(&neighborhood(40_000..40_040));
        let loosen = |m: &Measured| Measured {
            radius_earths: m.radius_earths.map(|(v, _)| (v, v * 0.8)),
            density_kg_m3: m.density_kg_m3.map(|(v, _)| (v, v * 0.8)),
            equilibrium_k: m.equilibrium_k.map(|(v, _)| (v, v * 0.5)),
            albedo: m.albedo.map(|(v, _)| (v, v * 0.9)),
            red: m.red.map(|(v, _)| (v, v * 0.9)),
            methane: m.methane.map(|(v, _)| (v, v * 0.9)),
        };
        let settled = |f: &dyn Fn(&Measured) -> Measured| {
            cases.iter().filter(|(_, m)| prior.leading(&f(m), 0.9).is_some()).count()
        };
        let tight = settled(&|m| *m);
        let loose = settled(&loosen);
        assert!(loose * 3 < tight, "loose {loose} against tight {tight} of {}", cases.len());
    }

    /// **The regime a craft is actually in, and what it costs.** A transit gives a radius and
    /// an orbit gives a temperature, and that is everything until somebody goes there: no
    /// mass, so no density, and nothing resolved, so no color.
    ///
    /// Two numbers go a long way. They settle whether a body has a surface nearly always, and
    /// they name it outright more often than not -- a small hot body is molten and an
    /// eleven-Earth-radius body is a giant, and neither needs a second opinion.
    ///
    /// What they cannot do is tell a habitable world from a dead one. Ocean, greenhouse and
    /// desert are the same size at the same distance, and from across the system they stay a
    /// three-way split. **That is the reason to fly there**, and it is a consequence of the
    /// chain rather than a rule written to produce it.
    #[test]
    fn a_transit_and_an_orbit_cannot_tell_an_ocean_from_a_desert() {
        let prior = Sorts::measure(&neighborhood(0..400));
        let cases = truths(&neighborhood(50_000..50_060));
        let distant = |m: &Measured| Measured {
            radius_earths: m.radius_earths.map(|(v, _)| (v, v * 0.08)),
            equilibrium_k: m.equilibrium_k.map(|(v, _)| (v, v * 0.05)),
            ..Default::default()
        };

        let (mut standing_right, mut n) = (0, 0);
        let (mut warm_named, mut warm) = (0, 0);
        for (truth, m) in &cases {
            let out = prior.given(&distant(m));
            assert!(!out.is_empty(), "a radius and a temperature are worth something");
            n += 1;
            let standing: f64 =
                out.iter().filter(|(s, _)| s.has_a_surface() == truth.has_a_surface()).map(|(_, p)| p).sum();
            standing_right += usize::from(standing > 0.9);

            if matches!(truth, Sort::Ocean | Sort::Greenhouse | Sort::Desert) {
                warm += 1;
                warm_named += usize::from(out.first().is_some_and(|(s, p)| s == truth && *p > 0.9));
            }
        }
        assert!(standing_right * 10 > n * 8, "only {standing_right} of {n} settled whether one could land");
        assert!(warm > 60, "only {warm} bodies with air over ground");
        assert!(
            warm_named * 3 < warm,
            "{warm_named} of {warm} habitable-or-not calls settled from across the system"
        );

        // And going there settles them.
        let close = cases
            .iter()
            .filter(|(t, _)| matches!(t, Sort::Ocean | Sort::Greenhouse | Sort::Desert))
            .filter(|(truth, m)| prior.given(m).first().is_some_and(|(s, p)| s == truth && *p > 0.9))
            .count();
        assert!(close * 10 > warm * 9, "a visit should settle them: {close} of {warm}");
    }

    /// Nothing measured is not an even split over the types; it is nothing said.
    #[test]
    fn nothing_measured_says_nothing() {
        let prior = Sorts::measure(&neighborhood(0..100));
        assert!(prior.given(&Measured::default()).is_empty());
        assert!(Sorts::default().given(&Measured::default()).is_empty());
        // And a body unlike anything the generator makes gets no answer either.
        let absurd = Measured { radius_earths: Some((400.0, 1.0)), ..Default::default() };
        assert!(prior.given(&absurd).is_empty());
    }

    /// Every type the generator can make has one definition, and the giants are the ones with
    /// no surface under them.
    #[test]
    fn a_type_is_derived_in_one_place() {
        let of = |class, air, top| Sort::of(class, air, top, 250.0);
        assert_eq!(of(Class::GasGiant, Atmosphere::Envelope, Top::Cloud), Sort::GasGiant);
        assert_eq!(of(Class::IceGiant, Atmosphere::Envelope, Top::Cloud), Sort::IceGiant);
        assert_eq!(of(Class::Rocky, Atmosphere::Envelope, Top::Cloud), Sort::Subneptune);
        assert_eq!(of(Class::Rocky, Atmosphere::Thick, Top::Ocean), Sort::Ocean);
        assert_eq!(of(Class::Rocky, Atmosphere::Thick, Top::Cloud), Sort::Greenhouse);
        assert_eq!(of(Class::Rocky, Atmosphere::Thin, Top::Rock), Sort::Desert);
        assert_eq!(of(Class::Rocky, Atmosphere::None, Top::Rock), Sort::Barren);
        assert_eq!(of(Class::Icy, Atmosphere::None, Top::Ice), Sort::IceWorld);
        // Hot enough and bare rock runs, whatever it formed as.
        assert_eq!(
            Sort::of(Class::Rocky, Atmosphere::None, Top::Rock, crate::surface::SCORCHED_K + 1.0),
            Sort::Molten
        );
        for sort in Sort::ALL {
            assert!(!sort.label().is_empty());
        }
        assert!(!Sort::GasGiant.has_a_surface() && Sort::Ocean.has_a_surface());
    }
}

#[cfg(test)]
mod from_a_file {
    use super::*;
    use super::tests_support::*;
    use crate::knowledge::{BodyId, Knowledge, Witness};

    /// **What a craft's own file reads as.** The chain has to survive the round trip: a body is
    /// visited, its measurements go into the file with a witness on each, and reading them back
    /// out has to give the same answer as reading the body directly.
    #[test]
    fn a_file_reads_as_the_body_it_is_about() {
        let star = star_of(7, 1.0);
        let prior = Sorts::measure(&neighborhood(0..400));
        let planets = planets_of(&star);
        let mut checked = 0;

        for planet in planets.iter().filter(|p| p.radius_earths() < 6.0) {
            let truth = Sort::of(planet.class, planet.atmosphere, planet.top, planet.equilibrium_k);
            let world = planet.world(&star);

            // What a file would hold: a radius, a mass, an orbit and per-band photometry.
            let mut colors = crate::knowledge::Colors::new(Witness(1));
            for visit in 0..8 {
                let flux = em_spectra::PerBand::new(std::array::from_fn(|i| {
                    let band = Band::ALL[i];
                    let reflect = world.reflectance_in(band);
                    (reflect > 0.0).then(|| {
                        let arriving =
                            reflect * em_spectra::blackbody::band_radiance(band, star.star.teff_k);
                        (arriving, arriving * 1.0e-3)
                    })
                }));
                colors.fold(visit as f64, &flux);
            }
            let volume = 4.0 / 3.0 * std::f64::consts::PI * planet.radius_m.powi(3);
            let belief = crate::knowledge::BodyBelief {
                radius_m: Some((planet.radius_m, planet.radius_m * 0.01)),
                mass_kg: Some((planet.mass_kg, planet.mass_kg * 0.03)),
                semi_major_au: Some((planet.semi_major_m / crate::sky::generate::AU, 0.001)),
                colors: Some(colors),
                ..empty_belief(star.id)
            };

            let measured = Measured::from_belief(&belief, &star.star);
            // The same numbers, by a different route.
            let direct = Measured::of(
                &world,
                planet.radius_earths(),
                planet.mass_kg / volume,
                planet.equilibrium_k,
            );
            for (a, b) in [
                (measured.radius_earths, direct.radius_earths),
                (measured.density_kg_m3, direct.density_kg_m3),
                (measured.equilibrium_k, direct.equilibrium_k),
                (measured.red, direct.red),
                (measured.methane, direct.methane),
            ] {
                let (Some((a, _)), Some((b, _))) = (a, b) else { panic!("{} lost a reading", planet.name) };
                assert!((a / b - 1.0).abs() < 0.02, "{}: {a} against {b}", planet.name);
            }
            // Leading, or a serious share of the posterior. A body sitting on a threshold --
            // 500 K is where bare rock starts to run -- is genuinely two things at once, and
            // the answer says so rather than picking.
            let out = prior.given(&measured);
            let held = out.iter().find(|(s, _)| *s == truth).map_or(0.0, |(_, p)| *p);
            assert!(held > 0.25, "{}: {truth:?} holds only {held:.2} of {out:?}", planet.name);
            checked += 1;
        }
        assert!(checked > 3, "only {checked} bodies");
    }

    /// A file with nothing in it says nothing, and a file with only an orbit says only what an
    /// orbit is worth.
    #[test]
    fn an_empty_file_is_not_a_guess() {
        let star = star_of(7, 1.0);
        let prior = Sorts::measure(&neighborhood(0..200));
        let bare = empty_belief(star.id);
        assert!(Measured::from_belief(&bare, &star.star).is_empty());
        assert!(prior.given(&Measured::from_belief(&bare, &star.star)).is_empty());

        let orbit_only = crate::knowledge::BodyBelief { semi_major_au: Some((1.0, 0.01)), ..bare };
        let measured = Measured::from_belief(&orbit_only, &star.star);
        assert!(measured.equilibrium_k.is_some() && measured.radius_earths.is_none());
        let out = prior.given(&measured);
        assert!(out.len() > 3, "a distance alone should rule almost nothing out: {out:?}");
    }

    fn empty_belief(star: crate::sky::StarId) -> crate::knowledge::BodyBelief {
        let body = BodyId::of(star, "one");
        let k = Knowledge::new(Witness(1));
        let _ = &k;
        crate::knowledge::BodyBelief {
            subject: crate::knowledge::Subject::Body { star, body },
            body,
            given: None,
            designation: None,
            kind: Vec::new(),
            period_s: None,
            semi_major_au: None,
            orientation: crate::knowledge::Orientation::Unknown,
            method: None,
            position_now: crate::knowledge::Placed::Unknown,
            radius_m: None,
            spin_s: None,
            velocity_m_s: None,
            about: None,
            colors: None,
            mass_kg: None,
            stated_by: None,
            hops: 0,
        }
    }
}
