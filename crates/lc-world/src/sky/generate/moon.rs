//! Satellites, of two origins that look nothing alike.
//!
//! A regular moon condensed out of a disc around its planet, so it sits close in, in the
//! planet's equatorial plane, on a circle, going the same way the planet turns. A captured one
//! was a passing body the planet caught, so it sits far out at any inclination, on an ellipse,
//! and more often than not going backwards. Telling the two apart from their orbits alone is
//! the point: a retinue's plane *is* its planet's obliquity, and a swarm of retrograde
//! stragglers is a planet that has been catching things for four billion years.

use super::disc::{EARTH_MASS, SOLAR_MASS_KG};
use super::planet::Planet;
use super::tuning::Tuning;
use crate::rng;

/// A satellite of a generated planet.
#[derive(Clone, Debug, PartialEq)]
pub struct Moon {
    pub name: String,
    /// Meters from the planet.
    pub semi_major_m: f64,
    pub eccentricity: f64,
    /// Tilt from the planet's equator, radians, `0..=PI`. Past a right angle is retrograde.
    pub inclination_rad: f64,
    /// Where it crosses the planet's equator, radians.
    pub node_rad: f64,
    pub mean_anomaly_deg: f64,
    pub radius_m: f64,
    pub mass_kg: f64,
    /// Condensed in a disc rather than caught. What the difference in the orbits means.
    pub regular: bool,
}

impl Moon {
    pub fn retrograde(&self) -> bool {
        self.inclination_rad > std::f64::consts::FRAC_PI_2
    }
}

/// Furthest a prograde satellite stays bound, as a share of the Hill radius. A retrograde one
/// holds on half again as far out, which is part of why so many distant moons go backwards.
const PROGRADE_STABLE: f64 = 0.4;
const RETROGRADE_STABLE: f64 = 0.6;

/// Radius of the sphere a planet holds against its star, meters.
pub fn hill_radius_m(semi_major_m: f64, mass_kg: f64, star_mass_solar: f64) -> f64 {
    semi_major_m * (mass_kg / (3.0 * star_mass_solar.max(1.0e-3) * SOLAR_MASS_KG)).cbrt()
}

/// Everything orbiting one planet, innermost first.
pub fn moons_of(planet: &Planet, star_mass_solar: f64, seed: u64, index: usize, tuning: &Tuning) -> Vec<Moon> {
    let h = |tag: u64| rng::hash(&[seed, 0x_6d30_306e, index as u64, tag]);
    let hill = hill_radius_m(planet.semi_major_m, planet.mass_kg, star_mass_solar);
    let inner = planet.radius_m * tuning.moons.inner_radii;

    let mut out = regular(planet, inner, hill, h(1), tuning);
    out.extend(irregular(planet, hill, star_mass_solar, h(2), tuning));
    out.sort_by(|a, b| a.semi_major_m.total_cmp(&b.semi_major_m));
    for (j, moon) in out.iter_mut().enumerate() {
        moon.name = format!("{} {}", planet.name, roman(j as u32 + 1));
    }
    out
}

/// The retinue a planet grew for itself.
///
/// A giant's satellite system comes to a ten-thousandth of the planet, measured, and the same
/// across Jupiter, Saturn and Uranus. A rocky planet has no disc to grow one in, so its only
/// route is a giant impact -- which is rare, and which is why Luna has no counterpart anywhere
/// else in the inner solar system.
fn regular(planet: &Planet, inner: f64, hill: f64, h: u64, tuning: &Tuning) -> Vec<Moon> {
    let t = &tuning.moons;
    // A giant's moons condensed in a disc that reached a twentieth of the way to the Hill
    // radius. Everything else got its moon some other way and put it wherever one stays: an
    // impact moon tidally recedes, and Luna is already a quarter of the way out.
    let disc = planet.class.is_giant() && !planet.migrated;
    let outer = hill * if disc { t.regular_outer_hill } else { PROGRADE_STABLE };
    if outer <= inner {
        return Vec::new();
    }

    let (count, share) = if planet.class.is_giant() {
        let (lo, hi) = t.regular_count;
        (lo + (rng::uniform(h) * (hi - lo + 1) as f64) as u32, t.regular_mass_ratio)
    } else if rng::uniform(rng::mix(h)) < t.impact_moon_chance {
        (1, rng::uniform_in(h, t.impact_mass_ratio.0.ln(), t.impact_mass_ratio.1.ln()).exp())
    } else {
        (0, 0.0)
    };
    let count = count.min(t.regular_count.1);
    if count == 0 {
        return Vec::new();
    }

    // Split the retinue's mass unevenly: a real one has one Ganymede and three lesser bodies,
    // not four of a size.
    let weights: Vec<f64> = (0..count).map(|j| rng::uniform(rng::hash(&[h, 0x5ba7e, j as u64]))).collect();
    let total: f64 = weights.iter().sum::<f64>().max(1.0e-9);

    (0..count)
        .map(|j| {
            let g = |tag: u64| rng::hash(&[h, j as u64, tag]);
            let mass = planet.mass_kg * share * weights[j as usize] / total;
            Moon {
                name: String::new(),
                // Log-spaced, so a retinue spreads out the way a real one does rather than
                // clumping at one radius.
                semi_major_m: rng::uniform_in(g(1), inner.ln(), outer.ln()).exp(),
                eccentricity: rng::uniform_in(g(4), 0.0, 0.02),
                inclination_rad: rng::gaussian(g(6)).abs() * 0.02,
                node_rad: rng::uniform_in(g(7), 0.0, std::f64::consts::TAU),
                mean_anomaly_deg: rng::uniform_in(g(5), 0.0, 360.0),
                radius_m: radius_of(mass, rng::uniform_in(g(3), tuning.moons.density.0, tuning.moons.density.1)),
                mass_kg: mass,
                regular: true,
            }
        })
        .collect()
}

/// The stragglers a planet caught.
///
/// How many is set by the size of the sphere it holds against its star: capture is a
/// cross-section, so the count goes as the square of the Hill radius. A Jupiter at five
/// astronomical units comes out near the ninety-odd the real one has.
fn irregular(planet: &Planet, hill: f64, star_mass_solar: f64, h: u64, tuning: &Tuning) -> Vec<Moon> {
    let t = &tuning.moons;
    if !planet.class.is_giant() {
        return Vec::new();
    }
    // Against Jupiter's own Hill radius, so the scaling is anchored on the system that has
    // been counted.
    let reference = hill_radius_m(5.2 * super::AU, 317.83 * EARTH_MASS, star_mass_solar.max(1.0e-3));
    let scale = (hill / reference).powi(2).min(1.5);
    let count = (t.irregular_most as f64 * scale * rng::uniform(h).powf(t.irregular_index)) as u32;
    let count = count.min(t.irregular_most);

    (0..count)
        .map(|j| {
            let g = |tag: u64| rng::hash(&[h, 0xcab7, j as u64, tag]);
            let retrograde = rng::uniform(g(8)) < t.retrograde_share;
            // Isotropic within its half of the sky: a captured body remembers nothing about
            // the plane its planet formed in.
            let cosine = rng::uniform_in(g(6), 0.0, 1.0);
            let tilt = cosine.acos();
            let radius = rng::uniform_in(g(3), t.irregular_radius_m.0.ln(), t.irregular_radius_m.1.ln()).exp();
            let density = rng::uniform_in(g(2), t.density.0, t.density.1);
            Moon {
                name: String::new(),
                semi_major_m: hill
                    * rng::uniform_in(
                        g(1),
                        t.irregular_hill.0,
                        t.irregular_hill.1.min(if retrograde { RETROGRADE_STABLE } else { PROGRADE_STABLE }),
                    ),
                eccentricity: rng::uniform_in(g(4), t.irregular_eccentricity.0, t.irregular_eccentricity.1),
                inclination_rad: if retrograde { std::f64::consts::PI - tilt } else { tilt },
                node_rad: rng::uniform_in(g(7), 0.0, std::f64::consts::TAU),
                mean_anomaly_deg: rng::uniform_in(g(5), 0.0, 360.0),
                radius_m: radius,
                mass_kg: density * 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3),
                regular: false,
            }
        })
        .collect()
}

fn radius_of(mass_kg: f64, density: f64) -> f64 {
    (3.0 * mass_kg / (4.0 * std::f64::consts::PI * density.max(1.0))).cbrt()
}

/// Satellite numbering, as the IAU does it: Io is Jupiter I.
pub fn roman(n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"),
        (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut left = n.max(1);
    let mut out = String::new();
    for (value, numeral) in TABLE {
        while left >= value {
            out.push_str(numeral);
            left -= value;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::architecture::architecture;
    use super::super::planet;
    use crate::sky::{AuthoredStars, StarId, StarProvider};

    fn systems(count: u64) -> Vec<Vec<Planet>> {
        let t = Tuning::default();
        (0..count)
            .map(|k| {
                let mut s = AuthoredStars::sample().stars()[1].clone();
                s.id = StarId::synthesise("moon", k);
                s.star = crate::star::Star::SOL;
                s.luminosity_solar = 1.0;
                s.mass_solar = 1.0;
                s.metallicity = 0.0;
                let arch = architecture(&s, &t);
                let mut planets = planet::planets("S", &arch, &s, &t);
                for (j, p) in planets.iter_mut().enumerate() {
                    p.moons = moons_of(p, s.mass_solar, s.seed(), j, &t);
                }
                planets
            })
            .collect()
    }

    /// A moon has to be outside the planet and inside the sphere the planet holds against its
    /// star, or it is not a moon.
    #[test]
    fn every_moon_sits_where_one_can_stay() {
        let mut counted = 0;
        for planets in systems(40) {
            for p in &planets {
                let hill = hill_radius_m(p.semi_major_m, p.mass_kg, 1.0);
                for m in &p.moons {
                    counted += 1;
                    assert!(m.semi_major_m > p.radius_m, "{} is inside its planet", m.name);
                    assert!(m.semi_major_m < hill, "{} is outside the Hill radius", m.name);
                    assert!(m.mass_kg < p.mass_kg * 0.05, "{} is not a moon", m.name);
                    assert!(m.radius_m > 0.0 && m.radius_m < p.radius_m);
                    // Periapsis has to clear the planet too, which eccentricity can undo.
                    assert!(m.semi_major_m * (1.0 - m.eccentricity) > p.radius_m, "{}", m.name);
                }
            }
        }
        assert!(counted > 500, "only {counted} moons across forty systems");
    }

    /// **The measured constant this rests on.** Jupiter's, Saturn's and Uranus's satellite
    /// systems all come to about a ten-thousandth of their planet, and that one number is what
    /// sizes a generated retinue without any other tuning.
    #[test]
    fn a_giants_retinue_weighs_a_ten_thousandth_of_it() {
        let want = Tuning::default().moons.regular_mass_ratio;
        let mut checked = 0;
        for planets in systems(30) {
            for p in planets.iter().filter(|p| p.class.is_giant()) {
                let retinue: f64 = p.moons.iter().filter(|m| m.regular).map(|m| m.mass_kg).sum();
                assert!((retinue / (p.mass_kg * want) - 1.0).abs() < 1.0e-9, "{}", p.name);
                checked += 1;
            }
        }
        assert!(checked > 40, "only {checked} giants");
    }

    /// The two origins have to be tellable apart from their orbits alone, because that is how
    /// a survey would do it: a retinue is close, flat, circular and prograde, and a catch is
    /// none of those.
    #[test]
    fn a_caught_moon_does_not_look_like_a_grown_one() {
        let (mut regular, mut caught) = (Vec::new(), Vec::new());
        for planets in systems(40) {
            for p in &planets {
                let hill = hill_radius_m(p.semi_major_m, p.mass_kg, 1.0);
                for m in &p.moons {
                    if m.regular {
                        regular.push((m.clone(), hill, p.class.is_giant() && !p.migrated));
                    } else {
                        caught.push((m.clone(), hill));
                    }
                }
            }
        }
        assert!(regular.len() > 50 && caught.len() > 500);

        // A migrated giant's retinue was dragged inside a Hill sphere a tenth the size, so
        // what is left of it sits right out at the stability limit rather than in a disc.
        for (m, hill, disc) in &regular {
            let limit = if *disc { 0.06 } else { PROGRADE_STABLE };
            assert!(m.semi_major_m < limit * hill, "{} is too far out to have formed there", m.name);
            assert!(m.eccentricity < 0.05 && m.inclination_rad < 0.15, "{} is not flat", m.name);
            assert!(!m.retrograde());
        }
        let backwards = caught.iter().filter(|(m, _)| m.retrograde()).count();
        let share = backwards as f64 / caught.len() as f64;
        assert!((0.55..0.75).contains(&share), "{share} of catches go backwards");
        for (m, hill) in &caught {
            assert!(m.semi_major_m > 0.09 * hill, "{} is too close in to be a catch", m.name);
            assert!(m.eccentricity > 0.05, "{} is too circular to be a catch", m.name);
        }
    }

    /// Capture is a cross-section, so the count goes as the square of the sphere the planet
    /// holds -- which puts a Jupiter analogue near the ninety-odd the real one has.
    #[test]
    fn a_jupiter_catches_more_than_a_neptune() {
        let counts = |planets: &[Vec<Planet>], heavy: bool| {
            let matching: Vec<usize> = planets
                .iter()
                .flatten()
                .filter(|p| p.class.is_giant() && (p.mass_earths() > 100.0) == heavy)
                .map(|p| p.moons.iter().filter(|m| !m.regular).count())
                .collect();
            matching.iter().sum::<usize>() as f64 / matching.len().max(1) as f64
        };
        let all = systems(60);
        assert!(counts(&all, true) > 2.0 * counts(&all, false), "a heavy giant must catch far more");
        let most = all.iter().flatten().map(|p| p.moons.iter().filter(|m| !m.regular).count()).max().unwrap();
        assert!((60..=100).contains(&most), "the busiest planet holds {most} catches");
    }

    /// A planet with no moon has no mass anybody can measure, so every giant has to have one.
    #[test]
    fn every_giant_has_something_to_be_weighed_by() {
        for planets in systems(40) {
            for p in planets.iter().filter(|p| p.class.is_giant()) {
                assert!(p.moons.iter().any(|m| m.regular), "{} has no retinue", p.name);
            }
        }
    }

    #[test]
    fn satellites_are_numbered_the_way_the_union_numbers_them() {
        assert_eq!(roman(1), "I");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(14), "XIV");
        assert_eq!(roman(49), "XLIX");
        assert_eq!(roman(95), "XCV");
        // Numbered outward, so a planet's moons read in order.
        for planets in systems(5) {
            for p in &planets {
                for (j, m) in p.moons.iter().enumerate() {
                    assert!(m.name.ends_with(&format!(" {}", roman(j as u32 + 1))), "{}", m.name);
                }
            }
        }
    }
}
