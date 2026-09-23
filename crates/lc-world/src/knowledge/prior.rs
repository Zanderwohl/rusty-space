//! What the generator makes, measured, so that a log is read against the galaxy it came from.
//!
//! The prior is the generator's own distribution: every star's ladder of planets, and every
//! system's belts and swarm seen from directions spread over the sky. See
//! `lightcone/docs/24-standing-instruments.md`.

use em_spectra::{Band, blackbody};
use glam::DVec3;

use crate::population::Population;
use crate::rng;
use crate::sky::CatalogStar;
use crate::sky::generate::{self, planets_of};
use crate::star::Star;
use crate::system::M_PER_LY;

/// Catalog stars the planet prior is measured over, at most.
const PRIOR_STARS: usize = 4000;

/// Belts are flat, so they are seen from several directions; a swarm is a shell.
const POPULATION_STARS: usize = 1500;
const DIRECTIONS: u64 = 4;

/// Summed over the log. The usual threshold, and close to where the search's look-elsewhere
/// cost puts it.
pub const DETECTION_SNR: f64 = 7.1;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drawn {
    ln_period: f64,
    ln_depth: f64,
    rocky: bool,
    /// `R* / a`. A system's planets share a plane, so the ones that transit are all those with
    /// `R* / a` above the observer's `|sin latitude|`.
    reach: f64,
}

/// Orientation is integrated exactly rather than sampled: with a star's planets sorted by
/// `reach`, each slice of `|sin latitude|` between two of them is a set of transiting planets.
#[derive(Clone, Debug, Default)]
pub struct Prior {
    systems: Vec<Vec<Drawn>>,
    populations: Vec<Seen>,
    /// With their luminosity in every band, watts.
    hosts: Vec<(Star, [f64; 7])>,
}

/// A system's belts and swarm, seen from one direction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Seen {
    dim: f64,
    /// As a fraction of the star's own light in the thermal band.
    excess: f64,
    swarm: bool,
}

/// Spread in log depth between what a planet would show crossing the middle of its star and
/// what it shows crossing wherever it actually does. A factor of about 1.7, one sigma.
const DEPTH_WIDTH: f64 = 0.55;

/// Fractional error on a host's mass when the neighborhood holds nothing to compare it with.
///
/// The main-sequence mass-luminosity relation scatters by about this much at a fixed
/// luminosity, from rotation, age and metallicity, so this is what one star alone is worth.
const LONE_HOST_SPREAD: f64 = 0.3;

impl Prior {
    /// Pass the stars a craft cannot tell this one apart from; knowing nothing, the catalog.
    pub fn measure<'a>(stars: impl IntoIterator<Item = &'a CatalogStar>) -> Self {
        let stars: Vec<&CatalogStar> = stars.into_iter().collect();
        let stride = stars.len().div_ceil(PRIOR_STARS).max(1);
        let systems = stars
            .iter()
            .step_by(stride)
            .map(|star| {
                let radius = star.star.radius_m;
                planets_of(star)
                    .iter()
                    .map(|p| {
                        let period = std::f64::consts::TAU * (p.semi_major_m.powi(3) / star.star.mu).sqrt();
                        let ratio = p.radius_m / radius;
                        Drawn {
                            ln_period: period.ln(),
                            ln_depth: (ratio * ratio).min(1.0).ln(),
                            rocky: p.class.is_rocky(),
                            reach: (radius / p.semi_major_m).min(1.0),
                        }
                    })
                    .collect()
            })
            .collect();
        let stride = stars.len().div_ceil(POPULATION_STARS).max(1);
        let mut populations = Vec::new();
        let mut hosts = Vec::new();
        for star in stars.iter().step_by(stride) {
            let system = generate::system_for(star);
            let ir = blackbody::band_radiance(Band::ThermalIr, star.star.teff_k).max(f64::MIN_POSITIVE);
            let excess: f64 =
                system.populations.iter().map(|p| p.reradiated_radiance(&star.star)[Band::ThermalIr]).sum::<f64>() / ir;
            let swarm = system.populations.iter().any(|p| p.radiating_ratio == Population::PANEL);
            for j in 0..DIRECTIONS {
                let toward = direction(rng::hash(&[star.seed(), 0x5ee_d1e5, j]));
                let dim = system
                    .populations
                    .iter()
                    .map(|p| p.mean_deficit(toward, &star.star) * p.band_response[Band::V] as f64)
                    .sum();
                populations.push(Seen { dim, excess, swarm });
            }
            let luminosity = Band::ALL.map(|band| {
                4.0 * std::f64::consts::PI * M_PER_LY * M_PER_LY * super::survey::flux_from(&star.star, band, M_PER_LY)
            });
            hosts.push((star.star, luminosity));
        }
        Prior { systems, populations, hosts }
    }

    /// Chance a log would have found a transiting planet if the star has one: the share of
    /// transiting planets at every period that fall in the periods searched and stand
    /// [`DETECTION_SNR`] out of the noise.
    #[allow(clippy::indexing_slicing)] // grid indices are clamped into the grid before they index it
    pub fn completeness(&self, periods_s: (f64, f64), sigma: f64, points: usize) -> f64 {
        let (lo, hi) = (periods_s.0.ln(), periods_s.1.ln());
        let (mut found, mut all) = (0.0, 0.0);
        for planets in &self.systems {
            let mut sorted: Vec<Drawn> = planets.clone();
            sorted.sort_by(|a, b| b.reach.total_cmp(&a.reach));
            for j in 0..sorted.len() {
                let next = sorted.get(j + 1).map_or(0.0, |d| d.reach);
                let each = (sorted[j].reach - next) / (j + 1) as f64;
                for d in &sorted[..=j] {
                    all += each;
                    // A central transit lasts P R / (pi a): the fraction of the log inside
                    // one is `reach / pi`.
                    let inside = points as f64 * d.reach / std::f64::consts::PI;
                    let snr = d.ln_depth.exp() * inside.sqrt() / sigma;
                    if (lo..hi).contains(&d.ln_period) && snr >= DETECTION_SNR {
                        found += each;
                    }
                }
            }
        }
        if all > 0.0 { found / all } else { 0.0 }
    }

    /// Chance a star has a swarm rather than only the belts every system has.
    pub fn swarm_given(&self, dim: (f64, f64), excess: Option<(f64, f64)>) -> Option<f64> {
        // Each generated system gets a spread of its own beside the measurement's.
        let ln_like = |seen: &Seen| {
            let var = dim.1 * dim.1 + (0.2 * seen.dim).powi(2);
            let mut ll = -0.5 * (dim.0 - seen.dim).powi(2) / var - 0.5 * var.ln();
            if let Some((e, sigma)) = excess {
                let var = sigma * sigma + (0.3 * seen.excess).powi(2);
                ll += -0.5 * (e - seen.excess).powi(2) / var - 0.5 * var.ln();
            }
            ll
        };
        let lse = |swarm: bool| {
            let lls: Vec<f64> = self.populations.iter().filter(|s| s.swarm == swarm).map(ln_like).collect();
            let max = lls.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if max == f64::NEG_INFINITY {
                return max;
            }
            max + lls.iter().map(|l| (l - max).exp()).sum::<f64>().ln()
        };
        let (yes, no) = (lse(true), lse(false));
        match (yes.is_finite(), no.is_finite()) {
            (true, true) => Some(1.0 / (1.0 + (no - yes).exp())),
            (true, false) => Some(1.0),
            (false, true) => Some(0.0),
            _ => None,
        }
    }

    /// The measured star most like one of this luminosity in this band, for when the star
    /// itself is only a brightness and a distance.
    #[allow(clippy::indexing_slicing)] // band indices come from `Band::index`, below seven
    pub fn host_like(&self, band: Band, luminosity_w: f64) -> Option<Star> {
        if !(luminosity_w > 0.0) {
            return None;
        }
        self.hosts
            .iter()
            .min_by(|a, b| {
                let miss = |l: f64| (l / luminosity_w).ln().abs();
                miss(a.1[band.index()]).total_cmp(&miss(b.1[band.index()]))
            })
            .map(|(star, _)| *star)
    }

    /// How wide a band of luminosity counts as "a star like this one" when the spread of their
    /// masses is being measured. A factor either way, not a fraction.
    ///
    /// Wide, because the mass–luminosity relation is steep: a factor of two in luminosity is
    /// only about a fifth in mass, so a narrow window would report a confidence the relation
    /// does not have and a wide one costs little.
    const LIKE_ENOUGH: f64 = 2.0;

    /// A host's gravitational parameter and how well it is known, as a fraction.
    ///
    /// The fraction is **measured from the prior's own sample** rather than stated: the spread of
    /// `mu` across the stars whose luminosity in this band is within [`Prior::LIKE_ENOUGH`] of
    /// the one asked about. That is exactly the thing a craft does not know when all it has is a
    /// brightness and a distance, and it is what carries into the distance of every planet found
    /// by transit. See `lightcone/docs/25-system-knowledge.md#from-outside-transits`.
    ///
    /// `None` when the luminosity is not positive, as for [`Prior::host_like`]. A sample of one
    /// reports no spread, which is honest about the sample and not about the relation — callers
    /// with one host are reading a prior built from one star.
    #[allow(clippy::indexing_slicing)] // band indices come from `Band::index`, below seven
    pub fn host_mass(&self, band: Band, luminosity_w: f64) -> Option<(f64, f64)> {
        let host = self.host_like(band, luminosity_w)?;
        let like: Vec<f64> = self
            .hosts
            .iter()
            .filter(|(_, l)| {
                let ratio = l[band.index()] / luminosity_w;
                ratio > 1.0 / Self::LIKE_ENOUGH && ratio < Self::LIKE_ENOUGH
            })
            .map(|(star, _)| star.mu)
            .collect();
        if like.len() < 2 {
            // Not zero. One comparison star says nothing about the spread, and a mass with no
            // error on it hands a transit's distance the period's precision -- which
            // `25-system-knowledge.md` rule 4 says it never has, because the error *is* mostly
            // the mass's. The main-sequence relation's own scatter is what is left to report.
            return Some((host.mu, LONE_HOST_SPREAD));
        }
        let mean = like.iter().sum::<f64>() / like.len() as f64;
        let variance =
            like.iter().map(|mu| (mu - mean) * (mu - mean)).sum::<f64>() / (like.len() - 1) as f64;
        Some((host.mu, variance.sqrt() / mean.max(f64::MIN_POSITIVE)))
    }

    /// Chance a star has at least one transiting planet in the periods.
    pub fn planet_prior(&self, periods_s: (f64, f64)) -> f64 {
        if self.systems.is_empty() {
            return 0.0;
        }
        let (lo, hi) = (periods_s.0.ln(), periods_s.1.ln());
        let total: f64 = self
            .systems
            .iter()
            .map(|planets| {
                planets
                    .iter()
                    .filter(|d| (lo..hi).contains(&d.ln_period))
                    .map(|d| d.reach)
                    .fold(0.0, f64::max)
            })
            .sum();
        total / self.systems.len() as f64
    }

    /// Density of (ln period, ln depth) of the planet a detection would be, over the periods.
    #[allow(clippy::indexing_slicing)] // grid indices are clamped into the grid before they index it
    pub(super) fn density(&self, periods_s: (f64, f64)) -> Density {
        let (lo, hi) = (periods_s.0.ln(), periods_s.1.ln());
        let mut weighted = Vec::new();
        for planets in &self.systems {
            let mut sorted: Vec<Drawn> = planets.clone();
            sorted.sort_by(|a, b| b.reach.total_cmp(&a.reach));
            // Slice j: the first j+1 planets transit; a detection is any in range, equally likely.
            for j in 0..sorted.len() {
                let next = sorted.get(j + 1).map_or(0.0, |d| d.reach);
                let slice = sorted[j].reach - next;
                let within: Vec<&Drawn> =
                    sorted[..=j].iter().filter(|d| (lo..hi).contains(&d.ln_period)).collect();
                for d in &within {
                    weighted.push((d.ln_period, d.ln_depth, slice / within.len() as f64));
                }
            }
        }
        Density::of(&weighted, lo, hi)
    }

    /// Chance a transit of this period and depth is of a rocky planet rather than a giant;
    /// `None` when the generator makes nothing like it.
    ///
    /// Compared in log depth, at a width that is not the measurement's. A generated planet's
    /// depth is its central one and a real transit crosses at whatever impact parameter it
    /// happens to have, so a measured depth is anywhere from that down to nothing -- a
    /// half-milli-magnitude measurement of a grazing transit is a precise number for a planet
    /// half the size. [`DEPTH_WIDTH`] is that spread, and it is far wider than the error bar.
    /// Rocky and giant are two orders of magnitude apart, so it costs nothing to tell them
    /// apart and everything to be strict about it.
    pub fn rocky_given(&self, period_s: f64, depth: f64, depth_sigma: f64) -> Option<f64> {
        if !(depth > 0.0) {
            return None;
        }
        let (lp, ld) = (period_s.ln(), depth.ln());
        let width = DEPTH_WIDTH.max(depth_sigma / depth);
        let (mut rocky, mut all) = (0.0, 0.0);
        for d in self.systems.iter().flatten() {
            let near = (d.ln_period - lp) / 0.15;
            if near.abs() > 4.0 {
                continue;
            }
            let miss = (d.ln_depth - ld) / width;
            let w = d.reach * (-0.5 * (near * near + miss * miss)).exp();
            all += w;
            if d.rocky {
                rocky += w;
            }
        }
        (all > 0.0).then(|| rocky / all)
    }
}

/// A smoothed histogram over (ln period, ln depth), normalized as a density.
pub(super) struct Density {
    lp0: f64,
    ld0: f64,
    np: usize,
    values: Vec<f64>,
}

const DENSITY_LP: f64 = 0.05;
const DENSITY_LD: f64 = 0.1;
const DENSITY_LD_MIN: f64 = -18.4; // ln 1e-8
const DENSITY_ND: usize = 185;

impl Density {
    #[allow(clippy::indexing_slicing)] // grid indices are clamped into the grid before they index it
    fn of(weighted: &[(f64, f64, f64)], lo: f64, hi: f64) -> Self {
        let np = (((hi - lo) / DENSITY_LP).ceil() as usize).max(1);
        let mut values = vec![0.0; np * DENSITY_ND];
        for &(lp, ld, w) in weighted {
            let ip = (((lp - lo) / DENSITY_LP) as usize).min(np - 1);
            let id = ((ld - DENSITY_LD_MIN) / DENSITY_LD).clamp(0.0, (DENSITY_ND - 1) as f64) as usize;
            values[ip * DENSITY_ND + id] += w;
        }
        let values = smooth(&values, np, DENSITY_ND, 3.0, 3.0);
        let total: f64 = values.iter().sum::<f64>() * DENSITY_LP * DENSITY_LD;
        let values = if total > 0.0 { values.iter().map(|v| v / total).collect() } else { values };
        Density { lp0: lo, ld0: DENSITY_LD_MIN, np, values }
    }

    #[allow(clippy::indexing_slicing)] // grid indices are clamped into the grid before they index it
    pub(super) fn ln_at(&self, ln_period: f64, ln_depth: f64) -> f64 {
        let ip = (((ln_period - self.lp0) / DENSITY_LP).max(0.0) as usize).min(self.np - 1);
        let id = ((ln_depth - self.ld0) / DENSITY_LD).clamp(0.0, (DENSITY_ND - 1) as f64) as usize;
        let v = self.values[ip * DENSITY_ND + id];
        if v > 0.0 { v.ln() } else { f64::NEG_INFINITY }
    }
}

/// Separable Gaussian blur, in bins.
#[allow(clippy::indexing_slicing)] // grid indices are clamped into the grid before they index it
fn smooth(values: &[f64], rows: usize, cols: usize, sigma_r: f64, sigma_c: f64) -> Vec<f64> {
    let kernel = |sigma: f64| -> Vec<f64> {
        let reach = (3.0 * sigma).ceil() as i64;
        (-reach..=reach).map(|k| (-0.5 * (k as f64 / sigma).powi(2)).exp()).collect()
    };
    let (kr, kc) = (kernel(sigma_r), kernel(sigma_c));
    let (hr, hc) = ((kr.len() / 2) as i64, (kc.len() / 2) as i64);
    let mut across = vec![0.0; values.len()];
    for r in 0..rows {
        for c in 0..cols {
            let v = values[r * cols + c];
            if v == 0.0 {
                continue;
            }
            for (k, w) in kc.iter().enumerate() {
                let cc = c as i64 + k as i64 - hc;
                if (0..cols as i64).contains(&cc) {
                    across[r * cols + cc as usize] += v * w;
                }
            }
        }
    }
    let mut out = vec![0.0; values.len()];
    for r in 0..rows {
        for c in 0..cols {
            let v = across[r * cols + c];
            if v == 0.0 {
                continue;
            }
            for (k, w) in kr.iter().enumerate() {
                let rr = r as i64 + k as i64 - hr;
                if (0..rows as i64).contains(&rr) {
                    out[rr as usize * cols + c] += v * w;
                }
            }
        }
    }
    out
}


fn direction(h: u64) -> DVec3 {
    let z = rng::uniform(h) * 2.0 - 1.0;
    let phi = rng::uniform(rng::mix(h)) * std::f64::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    DVec3::new(r * phi.cos(), r * phi.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{AuthoredStars, StarProvider};

    const DAY_S: f64 = 86_400.0;

    fn prior() -> Prior {
        Prior::measure(AuthoredStars::sample().stars())
    }

    #[test]
    fn the_prior_has_planets_and_they_are_where_the_generator_puts_them() {
        let prior = prior();
        assert!(!prior.systems.is_empty());
        let near = prior.planet_prior((0.2 * DAY_S, 30.0 * DAY_S));
        let all = prior.planet_prior((0.2 * DAY_S, 1.0e6 * DAY_S));
        assert!(near <= all && all > 0.0 && all < 1.0, "{near} of {all}");
    }

    /// Nothing seen proves little until the log could have seen something.
    #[test]
    fn completeness_grows_with_the_periods_searched_and_the_precision() {
        let prior = prior();
        let short = prior.completeness((0.2 * DAY_S, 5.0 * DAY_S), 1e-5, 2000);
        let long = prior.completeness((0.2 * DAY_S, 5_000.0 * DAY_S), 1e-5, 2000);
        let noisy = prior.completeness((0.2 * DAY_S, 5_000.0 * DAY_S), 1e-1, 2000);
        assert!(short < long && noisy < long, "{short} {long} {noisy}");
        assert!(long <= 1.0);
    }
}

