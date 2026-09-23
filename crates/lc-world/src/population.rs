//! Populations: swarms, belts and clouds, held as distributions rather than rosters.
//!
//! A population is uniform in longitude of ascending node, argument of periapsis and mean
//! anomaly. Those three angles are not stored, and assuming them uniform is exactly what
//! makes the population statistically steady and turns "which element is in front of the star
//! right now" into a closed-form probability. See `lightcone/docs/04-stellar-photometry.md`.

use std::f64::consts::PI;

use em_spectra::{Band, PerBand};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::distribution::{Distribution, Inclination};
use crate::star::Star;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Population {
    /// Unit normal of the population's reference plane.
    pub pole: DVec3,
    /// Semi-major axis, meters.
    pub semi_major: Distribution,
    pub eccentricity: Distribution,
    pub inclination: Inclination,
    /// Element count. An `f64`, and deliberately not backed by a list: construction adds to
    /// it and losses subtract, and nothing enumerates the members.
    pub count: f64,
    /// Geometric cross-section per element, m^2.
    pub cross_section: f64,
    /// Per-band opacity. Flat for anything solid; an extinction curve for dust, which is what
    /// makes gray-versus-reddening a diagnostic.
    pub band_response: PerBand<f32>,
    /// Radiating area over intercepting cross-section, which is what sets the temperature the
    /// elements settle at.
    ///
    /// [`SPHERICAL`] for rubble and grains, [`PANEL`] for anything engineered. The two differ
    /// by 2^(1/4) in temperature, 278 K against 331 K at one astronomical unit from a
    /// sun-like star, and both peak inside the 10 micron band.
    ///
    /// [`SPHERICAL`]: Population::SPHERICAL
    /// [`PANEL`]: Population::PANEL
    pub radiating_ratio: f64,
}

/// How far a population reaches: a torus, as the three distributions describe one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    /// Nearest any element comes to the star, meters.
    pub inner_m: f64,
    /// Furthest any element goes, meters.
    pub outer_m: f64,
    /// Angle the inclinations tip it through: the half-thickness of the tube, as an angle from
    /// the plane. A right angle for an isotropic cloud, which is what makes one a shell.
    pub half_angle_rad: f64,
}

impl Extent {
    /// Radius of the tube's center line.
    pub fn core_m(&self) -> f64 {
        (self.inner_m + self.outer_m) * 0.5
    }

    /// Half-width of the tube in the plane, meters.
    pub fn half_width_m(&self) -> f64 {
        (self.outer_m - self.inner_m) * 0.5
    }

    /// Half-height of the tube out of the plane, meters.
    pub fn half_height_m(&self) -> f64 {
        self.core_m() * self.half_angle_rad.sin()
    }
}

impl Population {
    /// A body absorbing on its cross-section and radiating from its whole surface.
    pub const SPHERICAL: f64 = 4.0;

    /// A flat collector absorbing on one face and radiating from both.
    pub const PANEL: f64 = 2.0;

    /// The torus the population occupies.
    ///
    /// The eccentricity matters as much as the semi-major axis does, and is the part it is easy
    /// to leave out: an element with semi-major axis `a` and eccentricity `e` is somewhere
    /// between `a(1-e)` and `a(1+e)` over its year, so the tube is wider than the spread of `a`.
    /// For the generated asteroid belt it is half again as wide.
    ///
    /// `None` for a population with no radius, which is one with nothing in it.
    pub fn extent(&self) -> Option<Extent> {
        let axes = self.semi_major.nodes();
        let a_lo = axes.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min);
        let a_hi = axes.iter().map(|(v, _)| *v).fold(0.0, f64::max);
        // Short of one, or a near-parabolic cloud has an inner radius of zero and a tube that
        // swallows its own center.
        let e_hi = self.eccentricity.nodes().iter().map(|(v, _)| *v).fold(0.0, f64::max).clamp(0.0, 0.95);
        let inner_m = a_lo * (1.0 - e_hi);
        let outer_m = a_hi * (1.0 + e_hi);
        (inner_m > 0.0 && outer_m > inner_m).then_some(Extent {
            inner_m,
            outer_m,
            half_angle_rad: self.inclination.max_inclination(),
        })
    }

    /// Fraction of the star's output the population intercepts.
    ///
    /// Exponential rather than the bare covering fraction, for the same reason the deficit is:
    /// past a covering fraction of a few tenths the elements start shadowing each other, and
    /// only the exponential stays right as a swarm approaches completion.
    pub fn absorbed_fraction(&self) -> f64 {
        1.0 - (-self.covering_fraction()).exp()
    }

    /// Flux-weighted orbital radius, meters.
    ///
    /// From `E[1/r^2]`, not from the mean semi-major axis: what sets an element's temperature
    /// is the flux it receives, and that is what `inv_r2` already averages correctly.
    pub fn thermal_radius(&self) -> f64 {
        let inv = self.inv_r2();
        if inv > 0.0 { inv.sqrt().recip() } else { 0.0 }
    }

    /// Temperature the elements settle at, kelvin.
    ///
    /// Absorbed power equals radiated power. Write the illuminated area as
    /// `f * 4 pi a^2` and the radiating area as `ratio` times that, and the absorbed fraction
    /// cancels out of both sides: an element at radius `a` reaches the same temperature alone
    /// as it does in a complete shell. Only mutual heating would change that, and a swarm thick
    /// enough for it is one where the inner elements are shadowed anyway.
    pub fn equilibrium_temperature(&self, star: &Star) -> f64 {
        self.equilibrium_temperature_under(star.luminosity())
    }

    /// The same, for a caller that has the luminosity but not the star.
    ///
    /// The renderer is one: it carries a system's radius and temperature and not its `mu`, and
    /// a second copy of this formula there is a second chance to have it wrong.
    pub fn equilibrium_temperature_under(&self, luminosity_w: f64) -> f64 {
        let a = self.thermal_radius();
        if a <= 0.0 || self.radiating_ratio <= 0.0 {
            return 0.0;
        }
        let denominator = self.radiating_ratio * 4.0 * PI * a * a * em_spectra::blackbody::SIGMA;
        (luminosity_w / denominator).powf(0.25)
    }

    /// Total radiating area, m^2. Only the illuminated part radiates.
    pub fn radiating_area(&self) -> f64 {
        let a = self.thermal_radius();
        self.radiating_ratio * self.absorbed_fraction() * 4.0 * PI * a * a
    }

    /// Thermal re-emission as an equivalent radiance over the *star's* disc, so that a caller
    /// already multiplying by `pi R^2 / d^2` gets the right flux and needs no second geometry.
    ///
    /// Not where the light physically comes from: the swarm is hundreds of stellar radii
    /// across. Correct for anything unresolved, which at interstellar range is everything, and
    /// the reason it works is that bolometrically the result is exactly
    /// [`Population::absorbed_fraction`] times the star's own flux. Energy in, energy out.
    pub fn reradiated_radiance(&self, star: &Star) -> PerBand<f64> {
        let t = self.equilibrium_temperature(star);
        let scale = self.radiating_area() / (4.0 * PI * star.radius_m * star.radius_m);
        if t <= 0.0 || scale <= 0.0 {
            return PerBand::splat(0.0);
        }
        PerBand::new(std::array::from_fn(|i| {
            em_spectra::blackbody::band_radiance(Band::ALL[i], t) * scale
        }))
    }

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
    /// sampling it at a point, which is what regularizes the caustic at the inclination
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

/// Average `f` over a disc of angular radius `s` centered at latitude `phi`.
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
            radiating_ratio: Population::SPHERICAL,
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
    fn the_cone_average_regularizes_the_caustic() {
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

    /// One astronomical unit from a sun-like star, which is the case the ten-micron band was
    /// chosen for. A sphere settles at Earth's equilibrium temperature; a panel radiating from
    /// both faces runs 2^(1/4) hotter.
    #[test]
    fn elements_at_one_astronomical_unit_settle_where_the_thermal_band_is_looking() {
        let mut p = swarm(Inclination::isotropic(), 1e6, AU);
        p.eccentricity = Distribution::uniform(0.0, 0.0, 1);

        p.radiating_ratio = Population::SPHERICAL;
        let sphere = p.equilibrium_temperature(&Star::SOL);
        p.radiating_ratio = Population::PANEL;
        let panel = p.equilibrium_temperature(&Star::SOL);
        assert!((sphere - 278.3).abs() < 0.5, "{sphere} K");
        assert!((panel - 331.0).abs() < 0.5, "{panel} K");
        assert!((panel / sphere - 2f64.powf(0.25)).abs() < 1e-9);

        // Both peak inside the band that exists to catch them.
        let (lo, hi) = Band::ThermalIr.limits_m();
        for t in [sphere, panel] {
            let peak = em_spectra::blackbody::WIEN_B / t;
            assert!(peak > lo && peak < hi, "{t} K peaks at {:.2} um, band is {:.1} to {:.1}",
                peak * 1e6, lo * 1e6, hi * 1e6);
        }
    }

    /// An element does not care how many neighbors it has. The absorbed fraction appears on
    /// both sides of the energy balance and cancels.
    #[test]
    fn coverage_does_not_change_the_temperature() {
        let sparse = swarm(Inclination::isotropic(), 1e3, AU);
        let dense = swarm(Inclination::isotropic(), 1e10, AU);
        assert!(dense.covering_fraction() > 100.0 * sparse.covering_fraction());
        let (a, b) = (
            sparse.equilibrium_temperature(&Star::SOL),
            dense.equilibrium_temperature(&Star::SOL),
        );
        assert!((a - b).abs() < 1e-9, "{a} against {b}");
    }

    #[test]
    fn a_wider_orbit_is_colder_as_the_inverse_square_root_of_the_radius() {
        let near = swarm(Inclination::isotropic(), 1e6, AU);
        let far = swarm(Inclination::isotropic(), 1e6, 4.0 * AU);
        let (a, b) = (
            near.equilibrium_temperature(&Star::SOL),
            far.equilibrium_temperature(&Star::SOL),
        );
        assert!((a / b - 2.0).abs() < 1e-6, "four times out should be half as warm: {a} / {b}");
    }

    /// The claim the whole re-emission model rests on: what is absorbed is what is radiated.
    ///
    /// Bolometrically the re-emission is exactly `absorbed_fraction` times the star's own flux,
    /// for any coverage, radius or geometry. Nothing else has to be checked for conservation —
    /// this is the statement.
    #[test]
    fn everything_absorbed_is_radiated_again() {
        for ratio in [Population::SPHERICAL, Population::PANEL] {
            for count in [1e3, 1e6, 1e8, 1e10] {
                for radius in [0.2 * AU, AU, 30.0 * AU] {
                    let mut p = swarm(Inclination::isotropic(), count, radius);
                    p.radiating_ratio = ratio;
                    let out = p.radiating_area() * em_spectra::blackbody::SIGMA
                        * p.equilibrium_temperature(&Star::SOL).powi(4);
                    let want = p.absorbed_fraction() * Star::SOL.luminosity();
                    assert!((out / want - 1.0).abs() < 1e-9, "{out} radiated against {want} absorbed");
                }
            }
        }
    }

    /// A swarm is a visible-light shadow and a thermal-infrared source, and the two are
    /// enormously far apart. This is what the thermal preset exists to show.
    #[test]
    fn a_swarm_is_a_shadow_in_the_visible_and_a_source_at_ten_microns() {
        let mut p = swarm(Inclination::isotropic(), 0.0, AU);
        p.radiating_ratio = Population::PANEL;
        // Half the sphere covered.
        p.count = 0.5 * 4.0 * PI * AU * AU / p.cross_section;
        assert!((p.absorbed_fraction() - (1.0 - (-0.5f64).exp())).abs() < 1e-12);

        let r = p.reradiated_radiance(&Star::SOL);
        let against_star = |b: Band| r[b] / em_spectra::blackbody::band_radiance(b, Star::SOL.teff_k);

        assert!(against_star(Band::V) < 1e-20, "a 331 K body emits no visible light at all");
        assert!(against_star(Band::K) < 1e-2, "and next to nothing at two microns");
        assert!(against_star(Band::ThermalIr) > 50.0,
            "but it should swamp the star at ten microns, got {}", against_star(Band::ThermalIr));
    }

    #[test]
    fn a_population_with_nothing_in_it_radiates_nothing() {
        let mut p = swarm(Inclination::isotropic(), 0.0, AU);
        assert_eq!(p.absorbed_fraction(), 0.0);
        assert_eq!(p.radiating_area(), 0.0);
        p.radiating_ratio = 0.0;
        assert_eq!(p.equilibrium_temperature(&Star::SOL), 0.0);
        assert!(Band::ALL.iter().all(|b| p.reradiated_radiance(&Star::SOL)[*b] == 0.0));
    }
}
