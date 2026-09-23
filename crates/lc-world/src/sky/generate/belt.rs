//! Belts, the trans-planetary disc and the Oort cloud: what the ladder did not assemble.
//!
//! None of these is placed. A belt is a rung a giant stirred past accreting, the
//! trans-planetary disc is the outer rungs that ran out of time, and the Oort cloud is what
//! the giants threw. A system with no giant therefore has no belt and nearly no cloud, which
//! is a statement about where comets come from rather than a rule invented here.

use em_spectra::{PerBand, extinction};
use glam::DVec3;

use super::AU;
use super::architecture::Architecture;
use super::disc::EARTH_MASS;
use super::tuning::Tuning;
use crate::distribution::{Distribution, Inclination};
use crate::population::Population;
use crate::rng;

/// The populations of one system.
pub fn populations(arch: &Architecture, pole: DVec3, seed: u64, tuning: &Tuning) -> Vec<Population> {
    let t = &tuning.belts;
    let mut out = Vec::new();
    let mut thrown = 0.0;

    // Where the outer disc begins: past the growth radius nothing finished assembling. A disc
    // that ends inside its own growth radius still has an outer edge with debris at it, so the
    // outermost rung is always on the far side of this whatever the radius says.
    let outermost = arch.rungs.last().map(|r| r.semi_major_m).unwrap_or(f64::MAX);
    let edge = (tuning.ladder.growth_over_snow * arch.disc.snow_m).min(outermost);

    for (k, rung) in arch.belts().filter(|r| r.semi_major_m < edge).enumerate() {
        // Why it never grew decides how much of it is still there. A giant's resonances throw
        // a belt out over the age of the system; a rung that simply never had the mass to
        // assemble was left alone and still holds most of what it started with.
        let survival = if rung.stirred { t.belt_survival } else { t.kuiper_survival };
        let mass = rung.debris_earths * EARTH_MASS * survival;
        thrown += rung.debris_earths * (1.0 - survival);
        if let Some(p) = belt(
            pole,
            mass,
            (rung.zone_m.0, rung.zone_m.1),
            (0.0, 0.25),
            0.2,
            t.belt_area_per_kg,
            t.belt_element_m,
            PerBand::splat(1.0),
            rng::hash(&[seed, 0xbe17, k as u64]),
        ) {
            out.push(p);
        }
    }

    // Everything the outer disc never got round to, whether the rung is a belt or a planet
    // with leftovers around it. One population rather than one per rung: it is a single
    // continuous disc and nothing separates its parts.
    let outer: Vec<_> = arch.rungs.iter().filter(|r| r.semi_major_m >= edge).collect();
    let leftover: f64 = outer.iter().map(|r| r.debris_earths).sum();
    if let Some(inner) = outer.first() {
        thrown += leftover * (1.0 - t.kuiper_survival);
        if let Some(p) = belt(
            pole,
            leftover * EARTH_MASS * t.kuiper_survival,
            (inner.zone_m.0, arch.disc.outer_m),
            (0.0, 0.2),
            0.35,
            t.kuiper_area_per_kg,
            t.kuiper_element_m,
            PerBand::splat(1.0),
            rng::hash(&[seed, 0xc01d]),
        ) {
            out.push(p);
        }
    }

    // What the planets themselves left behind on the way. It is not a belt -- a rung that
    // assembled a planet swept most of its annulus -- but it is not nothing either, and a
    // ladder that conserves its disc cannot quietly drop it before the cloud is weighed.
    thrown += arch
        .planets()
        .filter(|r| r.semi_major_m < edge)
        .map(|r| r.debris_earths)
        .sum::<f64>();

    if let Some(cloud) = oort(arch, thrown, seed, tuning) {
        out.push(cloud);
    }
    out
}

/// The cloud the giants threw.
///
/// Scattering needs something massive on a wide orbit to do the scattering, so the share that
/// ends up bound at a hundred thousand astronomical units rather than on the star or out of
/// the system goes with how much giant the system has.
fn oort(arch: &Architecture, thrown_earths: f64, seed: u64, tuning: &Tuning) -> Option<Population> {
    let t = &tuning.belts;
    let scatterers = (arch.giant_jupiters() / t.oort_saturation_jupiters).min(1.0);
    let mass = thrown_earths * EARTH_MASS * t.oort_efficiency * scatterers;

    let mut dust = PerBand::splat(0.0f32);
    for b in em_spectra::Band::ALL {
        dust[b] = extinction::RATIO[b] as f32;
    }
    belt(
        // Isotropic, so it has no plane of its own to be tilted out of.
        DVec3::Z,
        mass,
        (t.oort_au.0 * AU, t.oort_au.1 * AU),
        t.oort_eccentricity,
        std::f64::consts::FRAC_PI_2,
        t.oort_area_per_kg,
        t.oort_element_m,
        dust,
        rng::hash(&[seed, 0x0027]),
    )
}

/// The solar system's own three, measured rather than generated.
///
/// Sol's bodies come from a preset fitted against JPL, so its belts have no business being
/// drawn from a seed -- an asteroid belt somewhere other than between Mars and Jupiter would
/// be the one wrong thing in the one system that is right. Masses are the published estimates;
/// what turns each into a cross-section is the same measured ratio the generator uses.
pub fn solar(pole: DVec3, tuning: &Tuning) -> Vec<Population> {
    let t = &tuning.belts;
    let mut dust = PerBand::splat(0.0f32);
    for b in em_spectra::Band::ALL {
        dust[b] = extinction::RATIO[b] as f32;
    }
    [
        // 3e21 kg between 2.1 and 3.3 astronomical units, inclined by up to twenty degrees.
        belt(pole, 3.0e21, (2.1 * AU, 3.3 * AU), (0.0, 0.3), 0.35, t.belt_area_per_kg, t.belt_element_m,
            PerBand::splat(1.0), 0),
        // The Kuiper belt proper, a fiftieth of an Earth mass between Neptune and the cliff.
        belt(pole, 0.02 * EARTH_MASS, (30.0 * AU, 50.0 * AU), (0.0, 0.25), 0.35, t.kuiper_area_per_kg,
            t.kuiper_element_m, PerBand::splat(1.0), 0),
        // A few Earth masses of comets, isotropic, out where the shell radius is.
        belt(DVec3::Z, 5.0 * EARTH_MASS, (t.oort_au.0 * AU, t.oort_au.1 * AU), t.oort_eccentricity,
            std::f64::consts::FRAC_PI_2, t.oort_area_per_kg, t.oort_element_m, dust, 0),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// One population, from the mass it holds and the surface that mass presents.
///
/// `area_per_kg` is what turns a mass into something a telescope sees; the element radius only
/// decides what one transit looks like. Splitting the two is what lets the asteroid belt and
/// the Oort cloud hold comparable mass and be nothing alike to look at.
#[allow(clippy::too_many_arguments)]
fn belt(
    pole: DVec3,
    mass_kg: f64,
    span_m: (f64, f64),
    eccentricity: (f64, f64),
    half_angle_rad: f64,
    area_per_kg: f64,
    element_m: f64,
    band_response: PerBand<f32>,
    h: u64,
) -> Option<Population> {
    if !(mass_kg > 0.0) || !(span_m.1 > span_m.0) {
        return None;
    }
    let cross_section = std::f64::consts::PI * element_m * element_m;
    let count = mass_kg * area_per_kg / cross_section;
    if !(count >= 1.0) {
        return None;
    }
    Some(Population {
        pole,
        semi_major: Distribution::uniform(span_m.0, span_m.1, 9),
        eccentricity: Distribution::uniform(eccentricity.0, eccentricity.1, 5),
        inclination: if half_angle_rad >= std::f64::consts::FRAC_PI_2 - 1.0e-6 {
            Inclination::isotropic()
        } else {
            Inclination::uniform_angle(0.0, half_angle_rad, 12)
        },
        // A natural population's mass is uncertain by a good deal more than a factor of two;
        // this is the least that can be said about it. A measured one is not jittered.
        count: if h == 0 { count } else { count * rng::uniform_in(h, 0.5, 2.0) },
        cross_section,
        band_response,
        radiating_ratio: Population::SPHERICAL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::architecture::{Class, architecture};
    use super::super::{Tuning, pole_for};
    use crate::sky::{AuthoredStars, CatalogueStar, StarId, StarProvider};
    use crate::star::Star;

    fn sun_like(key: u64) -> CatalogueStar {
        let mut s = AuthoredStars::sample().stars()[1].clone();
        s.id = StarId::synthesise("belt", key);
        s.star = Star::SOL;
        s.luminosity_solar = 1.0;
        s.mass_solar = 1.0;
        s.metallicity = 0.0;
        s
    }

    fn of(star: &CatalogueStar, tuning: &Tuning) -> (Architecture, Vec<Population>) {
        let arch = architecture(star, tuning);
        let pops = populations(&arch, pole_for(star.seed()), star.seed(), tuning);
        (arch, pops)
    }

    /// Every system leaves an outer disc behind, because the outer disc never finishes
    /// assembling. That one is not optional and nothing has to place it.
    #[test]
    fn every_system_keeps_what_its_outer_disc_never_assembled() {
        let t = Tuning::default();
        for k in 0..60u64 {
            let (arch, pops) = of(&sun_like(k), &t);
            let outermost = arch.planets().last().map(|r| r.semi_major_m).unwrap_or(0.0);
            // Reaching past the outermost planet, not centered past it: the outer disc starts
            // among the last planets and runs to the edge.
            let trans = pops.iter().find(|p| {
                p.semi_major.mean() < 1000.0 * AU
                    && p.extent().is_some_and(|e| e.outer_m > outermost)
            });
            assert!(trans.is_some(), "system {k} has no trans-planetary belt");
        }
    }

    /// **The Oort cloud is what the giants threw**, so a system with nothing massive on a wide
    /// orbit has nothing to throw with and ends up with almost no cloud.
    #[test]
    fn the_oort_cloud_needs_a_giant_to_have_made_it() {
        let t = Tuning::default();
        let mut with = Vec::new();
        let mut without = Vec::new();
        for k in 0..200u64 {
            let (arch, pops) = of(&sun_like(k), &t);
            let cloud: f64 = pops
                .iter()
                .filter(|p| p.semi_major.mean() > 1000.0 * AU)
                .map(|p| p.count * p.cross_section)
                .sum();
            if arch.planets().any(|r| r.class == Class::GasGiant) { &mut with } else { &mut without }.push(cloud);
        }
        assert!(with.len() > 40 && without.len() > 40, "{} against {}", with.len(), without.len());
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(mean(&with) > 20.0 * mean(&without), "{} against {}", mean(&with), mean(&without));
    }

    /// An Oort cloud is a record, not a sight: it is the reason the local shell reaches where
    /// it does, and no telescope will ever see one.
    #[test]
    fn an_oort_cloud_is_invisible() {
        let t = Tuning::default();
        for k in 0..40u64 {
            let (_, pops) = of(&sun_like(k), &t);
            for cloud in pops.iter().filter(|p| p.semi_major.mean() > 1000.0 * AU) {
                let deficit = cloud.mean_deficit(DVec3::X, &Star::SOL);
                assert!(deficit < 1.0e-9, "an Oort cloud at a deficit of {deficit}");
                assert!(cloud.inclination.max_inclination() > 1.5, "and it has to be isotropic");
            }
        }
    }

    /// Metals are the rock, and the belts are what the rock did not become.
    #[test]
    fn a_metal_poor_star_gets_thin_belts() {
        let t = Tuning::default();
        let area = |feh: f64| {
            (0..60u64)
                .map(|k| {
                    let mut s = sun_like(k);
                    s.metallicity = feh;
                    of(&s, &t).1.iter().map(|p| p.count * p.cross_section).sum::<f64>()
                })
                .sum::<f64>()
        };
        assert!(area(0.3) > 20.0 * area(-1.5), "{} against {}", area(0.3), area(-1.5));
    }

    /// Everything a population carries has to be finite and positive, or the photometry
    /// divides by it and the shell bakes a hole.
    #[test]
    fn a_population_is_always_well_formed() {
        let t = Tuning::default();
        for k in 0..120u64 {
            for p in of(&sun_like(k), &t).1 {
                assert!(p.count.is_finite() && p.count >= 1.0, "count {}", p.count);
                assert!(p.cross_section > 0.0);
                assert!(p.covering_fraction().is_finite());
                let extent = p.extent().expect("a population has a radius");
                assert!(extent.inner_m > 0.0 && extent.outer_m > extent.inner_m);
                assert!((p.pole.length() - 1.0).abs() < 1.0e-12);
            }
        }
    }
}
