//! Populations: swarms, belts and clouds, held as distributions rather than rosters.
//!
//! A population is uniform in longitude of ascending node, argument of periapsis and mean
//! anomaly. Those three angles are not stored, and assuming them uniform is exactly what
//! makes the population statistically steady and turns "which element is in front of the star
//! right now" into a closed-form probability. See `lightcone/docs/04-stellar-photometry.md`.

use std::f64::consts::PI;

use em_spectra::PerBand;
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::distribution::{Distribution, Inclination};
use crate::star::Star;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Population {
    /// Unit normal of the population's reference plane.
    pub pole: DVec3,
    /// Semi-major axis, metres.
    pub semi_major: Distribution,
    pub eccentricity: Distribution,
    pub inclination: Inclination,
    /// Element count. An `f64`, and deliberately not backed by a list: construction adds to
    /// it and losses subtract, and nothing enumerates the members.
    pub count: f64,
    /// Geometric cross-section per element, m^2.
    pub cross_section: f64,
    /// Per-band opacity. Flat for anything solid; an extinction curve for dust, which is what
    /// makes grey-versus-reddening a diagnostic.
    pub band_response: PerBand<f32>,
}

impl Population {
    /// `E[1/r^2]`, time-averaged over the orbits.
    ///
    /// `<1/r^2> = 1/(a^2 sqrt(1-e^2))` exactly, from `dt = (r^2/h) dtheta`, so the radial half
    /// of the occultation integral needs no quadrature.
    pub fn inv_r2(&self) -> f64 {
        self.semi_major.expectation(|a| 1.0 / (a * a))
            * self.eccentricity.expectation(|e| 1.0 / (1.0 - e * e).sqrt())
    }

    /// Latitude of a viewing direction relative to the population's plane, radians.
    #[inline]
    pub fn latitude(&self, direction: DVec3) -> f64 {
        direction.normalize().dot(self.pole.normalize()).clamp(-1.0, 1.0).asin()
    }

    /// Elements per steradian as seen from the star, in `direction`.
    pub fn sky_density(&self, direction: DVec3) -> f64 {
        self.count * self.inclination.sky_density(self.latitude(direction))
    }

    /// Angular radius of the stellar disc seen from a representative element, radians. This
    /// is the scale over which the cone average smooths.
    fn disc_half_angle(&self, star: &Star) -> f64 {
        star.radius_m / self.semi_major.mean()
    }

    /// Expected elements projected on the stellar disc, point-sampled.
    ///
    /// `m(n) = Sigma(n) * pi * R*^2 * E[1/r^2]`. Exact away from the caustic; at the
    /// inclination limit the density spikes and the observable is the cone average below.
    pub fn mean_count(&self, direction: DVec3, star: &Star) -> f64 {
        self.sky_density(direction) * star.disc_area() * self.inv_r2()
    }

    /// Expected elements on the disc, averaged over the disc's own angular extent.
    ///
    /// This is the observable, and it is what a shell should bake. The disc has finite
    /// angular size, so an observer integrates the sky density over a cone rather than
    /// sampling it at a point, which is what regularises the caustic at the inclination
    /// limit.
    pub fn mean_count_cone(&self, direction: DVec3, star: &Star) -> f64 {
        let phi = self.latitude(direction);
        let s = self.disc_half_angle(star);
        let density = cone_average(|p| self.inclination.sky_density(p), phi, s);
        self.count * density * star.disc_area() * self.inv_r2()
    }

    /// Expected fractional deficit in the stellar flux, cone-averaged.
    ///
    /// `d = m * sigma / (pi R*^2)`, so the stellar radius cancels out of the amplitude and
    /// survives only in the cone-average smoothing.
    pub fn mean_deficit(&self, direction: DVec3, star: &Star) -> f64 {
        self.mean_count_cone(direction, star) * self.single_event_depth(star)
    }

    /// Deficit contributed by one element crossing the disc.
    #[inline]
    pub fn single_event_depth(&self, star: &Star) -> f64 {
        self.cross_section / star.disc_area()
    }

    /// Time for one element to cross the stellar disc, seconds.
    pub fn crossing_time(&self, star: &Star) -> f64 {
        2.0 * star.radius_m / star.orbital_speed(self.semi_major.mean())
    }

    /// Covering fraction: the population's total cross-section over the area of the sphere it
    /// occupies. For an isotropic population this is also the deficit, in every direction.
    pub fn covering_fraction(&self) -> f64 {
        self.count * self.cross_section * self.inv_r2() / (4.0 * PI)
    }
}

/// Average `f` over a disc of angular radius `s` centred at latitude `phi`.
///
/// With `delta = s sin(theta)` the chord weighting becomes `cos^2(theta)`, and
/// `(2/pi) * integral cos^2 = 1`, so a constant integrand passes through unchanged.
fn cone_average(f: impl Fn(f64) -> f64, phi: f64, s: f64) -> f64 {
    const NODES: usize = 64;
    if s <= 0.0 {
        return f(phi);
    }
    let mut total = 0.0;
    for k in 0..NODES {
        let theta = -PI / 2.0 + (k as f64 + 0.5) * PI / NODES as f64;
        let w = theta.cos() * theta.cos();
        total += w * f(phi + s * theta.sin());
    }
    total * 2.0 / NODES as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng;

    const AU: f64 = 1.496e11;

    fn swarm(inc: Inclination, count: f64, a: f64) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(a),
            eccentricity: Distribution::delta(0.0),
            inclination: inc,
            count,
            cross_section: 1e12,
            band_response: PerBand::splat(1.0),
        }
    }

    /// Direct Monte Carlo: sample orbits, place each element, and count the ones projected
    /// onto the stellar disc. This is the check the whole model rests on.
    fn mc_mean_count(pop: &Population, direction: DVec3, star: &Star, samples: u64) -> f64 {
        let n = direction.normalize();
        let s = star.radius_m / pop.semi_major.mean();
        let s2 = s * s;
        let mut hits = 0u64;
        for k in 0..samples {
            let i = pop.inclination.sample(rng::hash(&[k, 1]));
            let node = rng::uniform_in(rng::hash(&[k, 2]), 0.0, std::f64::consts::TAU);
            let anomaly = rng::uniform_in(rng::hash(&[k, 3]), 0.0, std::f64::consts::TAU);
            let (co, so) = (node.cos(), node.sin());
            let (cm, sm) = (anomaly.cos(), anomaly.sin());
            let ci = i.cos();
            let p = DVec3::new(co * cm - so * sm * ci, so * cm + co * sm * ci, sm * i.sin());
            let dot = p.dot(n);
            if dot > 0.0 && 1.0 - dot * dot < s2 {
                hits += 1;
            }
        }
        pop.count * hits as f64 / samples as f64
    }

    #[test]
    fn an_isotropic_swarms_deficit_is_its_covering_fraction() {
        let pop = swarm(Inclination::isotropic(), 1.5e6, AU);
        let star = Star::SOL;
        let expect = pop.count * pop.cross_section / (4.0 * PI * AU * AU);
        for dir in [DVec3::X, DVec3::Y, DVec3::Z, DVec3::new(1.0, 2.0, 3.0)] {
            let d = pop.mean_deficit(dir, &star);
            assert!((d / expect - 1.0).abs() < 1e-9, "{dir}: {d} vs {expect}");
        }
        assert!((pop.covering_fraction() / expect - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_reference_swarm_matches_the_design_figures() {
        // 1.5e6 elements of 1e6 km^2 at 1 AU around a Sun-like star.
        let pop = swarm(Inclination::isotropic(), 1.5e6, AU);
        let star = Star::SOL;
        let m = pop.mean_count_cone(DVec3::X, &star);
        assert!((m - 8.11).abs() < 0.05, "m on the disc is {m}, expected 8.11");
        let d = pop.mean_deficit(DVec3::X, &star);
        assert!((d - 5.33e-6).abs() < 0.05e-6, "deficit is {d}, expected 5.33e-6");
        let t = pop.crossing_time(&star);
        assert!((t / 3600.0 - 13.0).abs() < 0.2, "crossing time is {} h", t / 3600.0);
    }

    #[test]
    fn the_integral_agrees_with_direct_orbit_sampling() {
        let star = Star::SOL;
        // A disc-to-orbit ratio large enough to give the Monte Carlo usable statistics.
        let a = star.radius_m / 0.05;
        for (label, inc) in [
            ("isotropic", Inclination::isotropic()),
            ("band 0.30", Inclination::band(0.30, 0.001, 1)),
            ("band 0.10", Inclination::band(0.10, 0.001, 1)),
            ("spread", Inclination::uniform_angle(0.0, 0.5, 24)),
        ] {
            let pop = swarm(inc, 1.0, a);
            for phi in [0.0f64, 0.05, 0.15, 0.25] {
                let dir = DVec3::new(phi.cos(), 0.0, phi.sin());
                let want = mc_mean_count(&pop, dir, &star, 400_000);
                if want <= 0.0 {
                    continue;
                }
                let got = pop.mean_count_cone(dir, &star);
                let poisson = 1.0 / (want * 400_000.0).sqrt();
                let tol = (4.0 * poisson).max(0.05);
                assert!(
                    (got / want - 1.0).abs() < tol,
                    "{label} phi={phi}: integral {got:.4e} vs Monte Carlo {want:.4e} \
                     (ratio {:.3}, tolerance {tol:.3})",
                    got / want
                );
            }
        }
    }

    #[test]
    fn the_cone_average_regularises_the_caustic() {
        // At the inclination limit the point-sampled density diverges; the observable does
        // not, because the stellar disc has finite angular size.
        let star = Star::SOL;
        let pop = swarm(Inclination::band(0.30, 0.0005, 1), 1.0, star.radius_m / 0.05);
        let (c, si) = (0.30f64.cos(), 0.30f64.sin());
        let at_caustic = DVec3::new(c, 0.0, si);
        let point = pop.mean_count(at_caustic, &star);
        let cone = pop.mean_count_cone(at_caustic, &star);
        assert!(cone.is_finite() && cone > 0.0);
        assert!(cone < point || !point.is_finite(), "cone {cone} must tame point {point}");
        let mc = mc_mean_count(&pop, at_caustic, &star, 800_000);
        assert!((cone / mc - 1.0).abs() < 0.10, "cone {cone:.4e} vs MC {mc:.4e}");
    }

    #[test]
    fn a_band_is_denser_in_plane_and_empty_over_the_pole() {
        let star = Star::SOL;
        let pop = swarm(Inclination::uniform_angle(0.0, 0.2, 16), 1e6, AU);
        let in_plane = pop.mean_deficit(DVec3::X, &star);
        let mid = pop.mean_deficit(DVec3::new(1.0, 0.0, 0.1), &star);
        assert!(in_plane > mid, "density must fall away from the plane");
        assert_eq!(pop.mean_deficit(DVec3::Z, &star), 0.0, "nothing transits over the pole");
    }

    #[test]
    fn eccentricity_raises_the_deficit_through_inverse_r_squared() {
        let star = Star::SOL;
        let mut circular = swarm(Inclination::isotropic(), 1e6, AU);
        let mut eccentric = circular.clone();
        eccentric.eccentricity = Distribution::delta(0.6);
        circular.eccentricity = Distribution::delta(0.0);
        let ratio = eccentric.mean_deficit(DVec3::X, &star) / circular.mean_deficit(DVec3::X, &star);
        assert!((ratio - 1.25).abs() < 1e-6, "1/sqrt(1-0.36) = 1.25, got {ratio}");
    }
}
