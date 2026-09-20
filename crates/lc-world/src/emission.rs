//! The star's directional output: `L(n, t)`, combining every path.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use em_spectra::{Band, PerBand, blackbody};

use crate::flicker::Flicker;
use crate::occluder::Occluder;
use crate::population::Population;
use crate::shell::Shell;
use crate::star::Star;

/// Where a discrete occluder is at a given time.
///
/// Phase 4 replaces implementations of this with `em-sim` propagation over real elements;
/// the trait exists so photometry does not have to wait for it.
pub trait OccluderMotion {
    /// Star-centered position in meters at coordinate time `t`, seconds.
    fn position_at(&self, t: f64) -> DVec3;
}

/// A circular orbit, enough to exercise the analytic path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularOrbit {
    pub radius_m: f64,
    /// Orbit normal.
    pub pole: DVec3,
    pub phase0: f64,
    pub mu: f64,
}

impl OccluderMotion for CircularOrbit {
    fn position_at(&self, t: f64) -> DVec3 {
        let n = self.pole.normalize();
        let u = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let e1 = n.cross(u).normalize();
        let e2 = n.cross(e1);
        let omega = (self.mu / self.radius_m.powi(3)).sqrt();
        let a = self.phase0 + omega * t;
        (e1 * a.cos() + e2 * a.sin()) * self.radius_m
    }
}

pub struct Body {
    pub occluder: Occluder,
    pub motion: Box<dyn OccluderMotion + Send + Sync>,
}

/// Everything that shapes what leaves a system in a given direction.
pub struct EmissionModel {
    pub star: Star,
    /// Coherent occluders, evaluated exactly.
    pub bodies: Vec<Body>,
    /// Statistically steady occluders.
    pub populations: Vec<Population>,
    /// A bake of `populations`, used in place of evaluating them when present.
    pub shell: Option<Shell>,
    pub seed: u64,
}

impl EmissionModel {
    pub fn new(star: Star, seed: u64) -> Self {
        Self { star, bodies: Vec::new(), populations: Vec::new(), shell: None, seed }
    }

    /// Bake the populations at `level`, so later samples interpolate rather than integrate.
    pub fn bake(&mut self, level: u8) {
        self.shell = Some(Shell::bake(&self.star, &self.populations, level));
    }

    /// Optical depth from the populations, per band, including flicker.
    fn population_tau(&self, direction: DVec3, t: f64) -> PerBand<f64> {
        let mut tau = PerBand::splat(0.0f64);
        for (k, p) in self.populations.iter().enumerate() {
            let f = Flicker::of(p, direction, &self.star, self.seed ^ (k as u64 + 1));
            let gray = f.deficit_at(t);
            if gray <= 0.0 {
                continue;
            }
            for b in Band::ALL {
                tau[b] += gray * p.band_response[b] as f64;
            }
        }
        tau
    }

    /// Fraction of the star's flux removed, per band, at coordinate time `t` in seconds.
    pub fn deficit(&self, direction: DVec3, t: f64) -> PerBand<f32> {
        let tau = self.population_tau(direction, t);

        let mut discrete = PerBand::splat(1.0f64);
        for body in &self.bodies {
            let f = body.occluder.deficit(body.motion.position_at(t), direction, &self.star);
            if f > 0.0 {
                for b in Band::ALL {
                    discrete[b] *= 1.0 - f;
                }
            }
        }

        // Populations combine as exp(-tau) rather than a linear sum. The two agree to first
        // order, and only the exponential stays correct as a swarm approaches full coverage,
        // which is the end state the game is about.
        PerBand::new(std::array::from_fn(|i| {
            let b = Band::ALL[i];
            (1.0 - discrete[b] * (-tau[b]).exp()) as f32
        }))
    }

    /// Thermal re-emission from the populations, as a fraction of the star's own band flux.
    ///
    /// Steady, and deliberately not a function of direction or time. What the populations
    /// absorb is set by how much of the sky around the star they cover, which does not change
    /// as they orbit, and they radiate very nearly isotropically. So a swarm's transits flicker
    /// in the visible while its ten-micron excess sits perfectly still — which is itself the
    /// diagnostic, and the reason this is worth computing separately from the deficit.
    pub fn reradiated(&self) -> PerBand<f32> {
        let mut total = PerBand::splat(0.0f64);
        for p in &self.populations {
            let r = p.reradiated_radiance(&self.star);
            for b in Band::ALL {
                total[b] += r[b];
            }
        }
        PerBand::new(std::array::from_fn(|i| {
            let b = Band::ALL[i];
            (total[b] / blackbody::band_radiance(b, self.star.teff_k).max(f64::MIN_POSITIVE)) as f32
        }))
    }

    /// Radiance leaving the system in `direction`, per band, W m^-2 sr^-1.
    ///
    /// Occultation removes and re-emission adds, so in the thermal infrared this can exceed
    /// what the bare star puts out.
    pub fn radiance(&self, direction: DVec3, t: f64) -> PerBand<f32> {
        let deficit = self.deficit(direction, t);
        let extra = self.reradiated();
        PerBand::new(std::array::from_fn(|i| {
            let b = Band::ALL[i];
            let bare = blackbody::band_radiance(b, self.star.teff_k);
            (bare * (1.0 - deficit[b] as f64 + extra[b] as f64)) as f32
        }))
    }
}

/// A population's parameters as recovered from a light curve's first two moments.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Inverted {
    pub element_area: f64,
    pub element_count: f64,
    pub semi_major: f64,
    pub mean_count: f64,
}

/// Recover element size and count separately from the mean deficit, the rms flicker and the
/// spectral knee.
///
/// The mean and the variance of a Poisson process scale differently, which is what lets the
/// two separate: `k = f^2 / d` is the deficit one element contributes, and everything else
/// follows. This is the forward model's inverse, and the reason a light curve says how many
/// objects of what size orbit a star thirty light-years away.
pub fn invert_moments(
    mean_deficit: f64,
    rms_flicker: f64,
    crossing_time: f64,
    star: &Star,
) -> Option<Inverted> {
    if mean_deficit <= 0.0 || rms_flicker <= 0.0 || crossing_time <= 0.0 {
        return None;
    }
    let k = rms_flicker * rms_flicker / mean_deficit;
    let speed = 2.0 * star.radius_m / crossing_time;
    let semi_major = star.mu / (speed * speed);
    let mean_count = mean_deficit / k;
    Some(Inverted {
        element_area: k * star.disc_area(),
        element_count: 4.0 * mean_count * semi_major * semi_major / (star.radius_m * star.radius_m),
        semi_major,
        mean_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{Distribution, Inclination};

    const AU: f64 = 1.496e11;

    fn reference_swarm() -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(AU),
            eccentricity: Distribution::delta(0.0),
            inclination: Inclination::isotropic(),
            count: 1.5e6,
            cross_section: 1e12,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }

    /// The phase's headline result: a light curve's first two moments return the population
    /// that produced them, element size and count separately.
    #[test]
    fn the_moments_invert_to_the_population_that_made_them() {
        let star = Star::SOL;
        let pop = reference_swarm();
        let f = Flicker::of(&pop, DVec3::X, &star, 1);
        let inverted =
            invert_moments(f.mean_deficit, f.mean_deficit * f.relative_rms(), f.crossing_time, &star)
                .unwrap();

        assert!((inverted.element_area / 1e12 - 1.0).abs() < 1e-3, "area {}", inverted.element_area);
        assert!((inverted.element_count / 1.5e6 - 1.0).abs() < 1e-3, "count {}", inverted.element_count);
        assert!((inverted.semi_major / AU - 1.0).abs() < 1e-3, "a {}", inverted.semi_major);
        assert!((inverted.mean_count - 8.11).abs() < 0.05);
    }

    #[test]
    fn inversion_separates_many_small_elements_from_few_large_ones() {
        let star = Star::SOL;
        let mut few_large = reference_swarm();
        few_large.count = 1.5e3;
        few_large.cross_section = 1e15;
        // Same covering fraction, so the mean deficit alone cannot tell them apart.
        let a = Flicker::of(&reference_swarm(), DVec3::X, &star, 1);
        let b = Flicker::of(&few_large, DVec3::X, &star, 1);
        assert!((a.mean_deficit / b.mean_deficit - 1.0).abs() < 1e-9, "means must coincide");

        let inv = |f: &Flicker| {
            invert_moments(f.mean_deficit, f.mean_deficit * f.relative_rms(), f.crossing_time, &star)
                .unwrap()
        };
        assert!((inv(&a).element_count / 1.5e6 - 1.0).abs() < 1e-3);
        assert!((inv(&b).element_count / 1.5e3 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn inversion_refuses_a_degenerate_curve() {
        assert!(invert_moments(0.0, 1e-6, 1000.0, &Star::SOL).is_none());
        assert!(invert_moments(1e-6, 0.0, 1000.0, &Star::SOL).is_none());
    }

    #[test]
    fn a_bare_star_loses_nothing() {
        let m = EmissionModel::new(Star::SOL, 1);
        let d = m.deficit(DVec3::X, 0.0);
        for b in Band::ALL {
            assert_eq!(d[b], 0.0);
        }
        let r = m.radiance(DVec3::X, 0.0);
        assert!((r[Band::V] as f64 / blackbody::band_radiance(Band::V, 5772.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_transiting_planet_dims_the_star_periodically() {
        let star = Star::SOL;
        let mut m = EmissionModel::new(star, 1);
        let period = std::f64::consts::TAU * (AU.powi(3) / star.mu).sqrt();
        m.bodies.push(Body {
            occluder: Occluder::new(6.371e6),
            // Edge-on as seen from +X, so it transits once per orbit.
            motion: Box::new(CircularOrbit { radius_m: AU, pole: DVec3::Z, phase0: 0.0, mu: star.mu }),
        });

        let n = 4000;
        let deficits: Vec<f64> =
            (0..n).map(|k| m.deficit(DVec3::X, k as f64 * period / n as f64)[Band::V] as f64).collect();
        let deepest = deficits.iter().cloned().fold(0.0, f64::max);
        assert!((deepest - 1.0186e-4).abs() < 5e-6, "transit depth {deepest}");
        let in_transit = deficits.iter().filter(|d| **d > 0.0).count();
        assert!(in_transit > 0 && (in_transit as f64 / n as f64) < 0.02, "duty cycle too high");
    }

    #[test]
    fn a_population_and_a_planet_combine_without_exceeding_total_extinction() {
        let star = Star::SOL;
        let mut m = EmissionModel::new(star, 3);
        let mut opaque = reference_swarm();
        opaque.cross_section = 1e18; // a nearly complete swarm
        m.populations.push(opaque);
        m.bodies.push(Body {
            occluder: Occluder::new(7.1e7),
            motion: Box::new(CircularOrbit { radius_m: AU, pole: DVec3::Z, phase0: 0.0, mu: star.mu }),
        });
        for k in 0..200 {
            let d = m.deficit(DVec3::X, k as f64 * 1e5)[Band::V];
            assert!((0.0..=1.0).contains(&d), "deficit {d} left [0, 1]");
        }
        assert!(m.deficit(DVec3::X, 0.0)[Band::V] > 0.9, "a near-complete swarm should be opaque");
    }

    #[test]
    fn a_baked_shell_stands_in_for_its_populations() {
        let mut m = EmissionModel::new(Star::SOL, 9);
        m.populations.push(reference_swarm());
        let direct = m.populations[0].mean_deficit(DVec3::new(1.0, 0.3, 0.2), &Star::SOL);
        m.bake(4);
        let baked = m.shell.as_ref().unwrap().sample(DVec3::new(1.0, 0.3, 0.2));
        assert!((baked.deficit[Band::V] as f64 / direct - 1.0).abs() < 1e-3);
        assert!((baked.crossing_time / m.populations[0].crossing_time(&Star::SOL) - 1.0).abs() < 1e-3);
    }
}
