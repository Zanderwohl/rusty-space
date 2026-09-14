//! The system the ship is inside: its bodies, propagated, as things to draw.
//!
//! Nothing here models anything. `em-sim` already holds and propagates systems, `em-sim`'s
//! presets already carry the solar system with its moons, and `lc-world::sky::generate` already
//! emits a generated system in the form `em-sim` consumes. This is the join.

use em_sim::id::BodyIndex;
use em_sim::system::System;
use em_sim::universe::UniverseFileContents;
use em_foundations::time::Instant;
use glam::DVec3;
use lc_world::sky::{CatalogueStar, StarId, generate};

/// Metres in a light-year.
pub const M_PER_LY: f64 = 9.460_730_472_580_8e15;

/// Metres in one render unit inside a system. An astronomical unit, so a belt's radius is a
/// number of order ten rather than of order 1e11.
pub const UNIT_M: f64 = 1.495_978_707e11;

/// Geometric albedo used for every body.
///
/// A placeholder, and the one number here that wants a real table: the solar system spans
/// 0.04 for a comet nucleus to 1.4 for Enceladus, and what the preset carries is a
/// visualisation colour rather than a measured albedo. Roughly the solar system's median.
pub const DEFAULT_ALBEDO: f64 = 0.3;

/// The catalogue name of the system whose data is real rather than generated.
pub const SOL: &str = "Sol";

/// One body, as the renderer wants it.
#[derive(Clone, Debug)]
pub struct Drawable {
    pub name: String,
    /// Where it is, light-years from the world origin, simulation axes.
    pub position_ly: DVec3,
    pub radius_m: f64,
    /// The radius a blackbody at the star's temperature would need to deliver this body's
    /// reflected flux. See [`effective_radius`].
    pub effective_radius_m: f64,
    /// What it re-radiates at, kelvin.
    pub equilibrium_k: f64,
}

pub struct LocalSystem {
    pub star: StarId,
    pub star_name: String,
    /// Swarms, belts and clouds. Generated even for the real solar system: `em-sim`'s preset
    /// carries bodies and no distributions, and a system with a Kuiper belt and no Kuiper belt
    /// in it would be the stranger of the two errors.
    pub populations: Vec<lc_world::population::Population>,
    /// Where the system's barycentre sits, light-years from the world origin.
    pub origin_ly: DVec3,
    sim: System,
    primary: BodyIndex,
    /// The primary's radius and temperature, which set every reflection in the system.
    star_radius_m: f64,
    star_teff_k: f64,
    star_luminosity_w: f64,
}

impl LocalSystem {
    /// Load the system around a star, real where there is real data and generated otherwise.
    pub fn for_star(star: &CatalogueStar) -> Option<Self> {
        let populations = generate::system_for(star).populations;
        let contents = Self::contents_for(star);
        let sim = System::from_contents(&contents).ok()?;
        // The most massive body is the primary. Not the first: a multiple is a barycentre with
        // children, and the barycentre is massless.
        let primary = sim
            .indices()
            .max_by(|a, b| sim.info(*a).mass.total_cmp(&sim.info(*b).mass))?;
        Some(Self {
            star: star.id,
            star_name: star.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get())),
            populations,
            origin_ly: star.position_ly,
            sim,
            primary,
            star_radius_m: star.star.radius_m,
            star_teff_k: star.star.teff_k,
            star_luminosity_w: star.star.luminosity(),
        })
    }

    fn contents_for(star: &CatalogueStar) -> UniverseFileContents {
        match star.name.as_deref() {
            // The one system with measured data rather than generated: two hundred and thirty
            // bodies fitted against JPL, moons and comets included.
            Some(SOL) => em_sim::presets::solar_system(),
            _ => generate::system_for(star).to_universe(),
        }
    }

    /// Propagate to a coordinate time, in seconds since the world origin.
    pub fn advance_to(&mut self, seconds: f64) {
        em_sim::propagate::evaluate_at(&mut self.sim, Instant::from_seconds_since_j2000(seconds));
    }

    pub fn len(&self) -> usize {
        self.sim.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sim.len() == 0
    }

    /// Every body except the primary, as seen from `observer_ly`.
    pub fn drawables(&self, observer_ly: DVec3) -> Vec<Drawable> {
        let star_at = self.sim.position(self.primary);
        let observer_m = (observer_ly - self.origin_ly) * M_PER_LY;
        self.sim
            .indices()
            .filter(|i| *i != self.primary)
            .filter_map(|i| {
                let at = self.sim.position(i);
                if !at.is_finite() {
                    return None;
                }
                let radius_m = self.sim.appearance(i).radius();
                let from_star = at - star_at;
                let distance_m = from_star.length();
                if radius_m <= 0.0 || distance_m <= 0.0 {
                    return None;
                }
                let phase = phase_factor(star_at - at, observer_m - at);
                Some(Drawable {
                    name: self.sim.info(i).name.clone().unwrap_or_else(|| self.sim.name(i).into()),
                    position_ly: self.origin_ly + at / M_PER_LY,
                    radius_m,
                    effective_radius_m: effective_radius(
                        self.star_radius_m,
                        radius_m,
                        DEFAULT_ALBEDO * phase,
                        distance_m,
                    ),
                    equilibrium_k: equilibrium_temperature(self.star_luminosity_w, distance_m),
                })
            })
            .collect()
    }

    pub fn star_teff_k(&self) -> f64 {
        self.star_teff_k
    }
}

/// The radius a blackbody at the star's temperature would need to deliver a body's reflected
/// flux, so that a lit body can be drawn by the same shader as the star lighting it.
///
/// `R_eff = R_star * R_body * sqrt(p) / d`, from equating `pi R_eff^2 B / D^2` with the
/// standard `L p R^2 / (4 pi d^2 D^2)`. Exact for a grey reflector, and checked against a
/// measured magnitude: Jupiter comes out at -2.72 against an observed -2.70.
///
/// Reflected light has the star's spectrum, which is what makes this work at all. A body with a
/// strongly coloured albedo — Mars — comes out the star's colour rather than its own, and that
/// is the approximation being made.
pub fn effective_radius(star_radius_m: f64, radius_m: f64, albedo: f64, distance_m: f64) -> f64 {
    if distance_m <= 0.0 || albedo <= 0.0 {
        return 0.0;
    }
    star_radius_m * radius_m * albedo.sqrt() / distance_m
}

/// Lambert phase function: how much of a lit sphere is turned toward the observer.
///
/// One at opposition, zero at conjunction. Not optional — Venus at a tenth of an astronomical
/// unit is a razor crescent, and ignoring phase makes it three magnitudes too bright.
pub fn phase_factor(to_star: DVec3, to_observer: DVec3) -> f64 {
    let (a, b) = (to_star.normalize_or_zero(), to_observer.normalize_or_zero());
    if a == DVec3::ZERO || b == DVec3::ZERO {
        return 1.0;
    }
    let alpha = a.dot(b).clamp(-1.0, 1.0).acos();
    ((alpha.sin() + (std::f64::consts::PI - alpha) * alpha.cos()) / std::f64::consts::PI).max(0.0)
}

/// What a body at `distance_m` from a star of `luminosity_w` settles at, kelvin.
///
/// The sphere case of the balance in `lc_world::population`: absorbing on a cross-section and
/// radiating from the whole surface.
pub fn equilibrium_temperature(luminosity_w: f64, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    let denominator =
        16.0 * std::f64::consts::PI * distance_m * distance_m * em_spectra::blackbody::SIGMA;
    (luminosity_w / denominator).powf(0.25)
}

#[cfg(test)]
mod tests {
    use lc_world::sky::{AuthoredStars, StarProvider};

    use super::*;

    const AU: f64 = 1.495_978_707e11;

    fn catalogue() -> Option<lc_world::sky::hyg::HygProvider> {
        lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv").ok()
    }

    #[test]
    fn the_solar_system_is_the_real_one_and_the_rest_are_generated() {
        let Some(provider) = catalogue() else { return };
        let sun = provider.stars().iter().find(|s| s.name.as_deref() == Some(SOL));
        let Some(sun) = sun else { panic!("the catalogue should carry Sol") };

        let real = LocalSystem::for_star(sun).expect("Sol loads");
        assert!(real.len() > 100, "the preset carries moons too, got {}", real.len());

        let other = provider.stars().iter().find(|s| s.name.as_deref() != Some(SOL)).unwrap();
        let made = LocalSystem::for_star(other).expect("a generated system loads");
        assert!(made.len() >= 1 && made.len() < real.len());
    }

    #[test]
    fn a_generated_system_loads_from_the_authored_sky() {
        let sky = AuthoredStars::sample();
        let star = &sky.stars()[0];
        let mut sys = LocalSystem::for_star(star).expect("a system");
        sys.advance_to(0.0);
        for d in sys.drawables(star.position_ly + DVec3::X * 1e-4) {
            assert!(d.position_ly.is_finite() && d.radius_m > 0.0, "{d:?}");
            assert!(d.equilibrium_k > 0.0);
        }
    }

    /// The formula the whole approach rests on, against a measured magnitude. Jupiter at
    /// opposition is magnitude -2.70; drawn as a blackbody of this radius at the Sun's
    /// temperature it comes out -2.72.
    #[test]
    fn the_effective_radius_reproduces_jupiters_magnitude() {
        let r_sun = 6.957e8;
        let r_eff = effective_radius(r_sun, 6.9911e7, 0.538, 5.204 * AU);
        // Flux against the Sun's, both seen from Earth at opposition.
        let d_obs = 4.204 * AU;
        let ratio = (r_eff / r_sun).powi(2) * (AU / d_obs).powi(2);
        let magnitude = -26.74 - 2.5 * ratio.log10();
        assert!((magnitude + 2.70).abs() < 0.1, "Jupiter came out at {magnitude}");
    }

    #[test]
    fn a_body_with_nowhere_to_be_or_nothing_to_reflect_is_not_drawn() {
        assert_eq!(effective_radius(6.957e8, 1e6, 0.3, 0.0), 0.0);
        assert_eq!(effective_radius(6.957e8, 1e6, 0.0, AU), 0.0);
        assert_eq!(equilibrium_temperature(3.8e26, 0.0), 0.0);
    }

    /// Ignoring this makes Venus three magnitudes too bright at inferior conjunction, which is
    /// how it was found.
    #[test]
    fn phase_is_one_at_opposition_and_zero_at_conjunction() {
        let full = phase_factor(DVec3::X, DVec3::X);
        let dark = phase_factor(DVec3::X, -DVec3::X);
        let half = phase_factor(DVec3::X, DVec3::Y);
        assert!((full - 1.0).abs() < 1e-12, "{full}");
        assert!(dark.abs() < 1e-12, "{dark}");
        assert!((half - 1.0 / std::f64::consts::PI).abs() < 1e-12, "{half}");
        assert!(half < full && dark < half);
    }

    #[test]
    fn equilibrium_temperature_puts_the_earth_where_it_belongs() {
        let t = equilibrium_temperature(3.828e26, AU);
        assert!((t - 278.3).abs() < 1.0, "{t} K at one astronomical unit");
        // And four times out is half as warm.
        let far = equilibrium_temperature(3.828e26, 4.0 * AU);
        assert!((t / far - 2.0).abs() < 1e-9);
    }

    #[test]
    fn propagating_moves_the_bodies() {
        let Some(provider) = catalogue() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.name.as_deref() == Some(SOL)) else {
            return;
        };
        let mut sys = LocalSystem::for_star(sun).unwrap();
        let observer = sun.position_ly + DVec3::X * (AU / M_PER_LY);

        sys.advance_to(0.0);
        let before = sys.drawables(observer);
        sys.advance_to(200.0 * 86_400.0);
        let after = sys.drawables(observer);

        assert_eq!(before.len(), after.len());
        assert!(!before.is_empty(), "the solar system should have something in it");
        let moved = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.position_ly.distance(b.position_ly) > 1e-9)
            .count();
        assert!(moved > before.len() / 2, "only {moved} of {} moved", before.len());
        assert!(after.iter().all(|d| d.position_ly.is_finite()));
    }

    /// Reflected brightness falls off far faster than distance alone: the flux reaching the
    /// body goes as one over the square, and what it returns goes as the square of that.
    #[test]
    fn a_body_further_out_reflects_far_less() {
        let near = effective_radius(6.957e8, 6.99e7, 0.5, AU);
        let far = effective_radius(6.957e8, 6.99e7, 0.5, 10.0 * AU);
        assert!((near / far - 10.0).abs() < 1e-9, "the effective radius goes as 1/d");
    }

    /// The end-to-end check: from where the Earth is, the brightest things in the solar system
    /// should be the planets a person can see, in roughly the order they see them.
    #[test]
    fn the_naked_eye_planets_are_the_brightest_things_in_the_sky() {
        let Some(provider) = catalogue() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.name.as_deref() == Some(SOL)) else {
            return;
        };
        let mut sys = LocalSystem::for_star(sun).unwrap();
        sys.advance_to(0.0);

        // Roughly where the Earth is at J2000, which is where the catalogue puts the observer.
        let earth = sys
            .drawables(sun.position_ly)
            .into_iter()
            .find(|d| d.name == "Earth")
            .expect("the preset carries the Earth");
        let observer = earth.position_ly;

        let mut lit = sys.drawables(observer);
        // Brightness goes as the effective radius squared over the distance squared.
        lit.sort_by(|a, b| {
            let flux = |d: &Drawable| {
                let r = d.effective_radius_m / observer.distance(d.position_ly).max(1e-30);
                -(r * r)
            };
            flux(a).total_cmp(&flux(b))
        });
        let top: Vec<&str> = lit.iter().take(8).map(|d| d.name.as_str()).collect();
        for want in ["Venus", "Jupiter", "Mars", "Saturn"] {
            assert!(top.contains(&want), "{want} should be among the brightest, got {top:?}");
        }
        // And the Moon, which is the brightest of all from here.
        assert!(top.contains(&"Luna") || top.contains(&"Moon"), "{top:?}");
    }
}
