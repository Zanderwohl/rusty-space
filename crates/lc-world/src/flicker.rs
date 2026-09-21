//! The residual scatter a population leaves on a light curve.
//!
//! The mean is not the whole signal. Two regimes of the same Poisson process, and the choice
//! between them is the diagnostic `rms/mean = 1/sqrt(m)`: once that approaches unity the
//! signal is isolated events, not noise about a mean, and reporting the mean would lose it.

use std::f64::consts::{PI, TAU};

use glam::DVec3;

use crate::population::Population;
use crate::rng;
use crate::star::Star;

/// Below this expected count on the disc, events stop overlapping and the mean becomes a
/// fiction. The transition band either side of it is approximated by whichever branch
/// applies; see `lightcone/docs/09-open-questions.md`.
pub const GAUSSIAN_THRESHOLD: f64 = 1.0;

/// Sinusoids in the Gaussian branch. Enough to read as noise, few enough to be cheap.
const TERMS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// Elements overlap continuously; fluctuation about a mean.
    Gaussian,
    /// Isolated occultations, mostly nothing.
    Sparse,
}

/// A population's flicker as seen from one direction, evaluable at any time.
///
/// Evaluable at *arbitrary* time, not stepped, for the same reason a worldline must be:
/// retarded-time observation asks what happened at times that were never the current tick.
#[derive(Clone, Copy, Debug)]
pub struct Flicker {
    pub mean_deficit: f64,
    pub mean_count: f64,
    pub single_event_depth: f64,
    pub crossing_time: f64,
    pub seed: u64,
}

impl Flicker {
    pub fn of(pop: &Population, direction: DVec3, star: &Star, seed: u64) -> Self {
        Self {
            mean_deficit: pop.mean_deficit(direction, star),
            mean_count: pop.mean_count_cone(direction, star),
            single_event_depth: pop.single_event_depth(star),
            crossing_time: pop.crossing_time(star),
            seed,
        }
    }

    pub fn regime(&self) -> Regime {
        if self.mean_count >= GAUSSIAN_THRESHOLD {
            Regime::Gaussian
        } else {
            Regime::Sparse
        }
    }

    /// `1/sqrt(m)`. Above about 1/3 the Gaussian description is no longer describing
    /// fluctuation about anything.
    pub fn relative_rms(&self) -> f64 {
        if self.mean_count > 0.0 {
            self.mean_count.sqrt().recip()
        } else {
            f64::INFINITY
        }
    }

    /// Occultation events per second, in the sparse regime.
    pub fn event_rate(&self) -> f64 {
        if self.crossing_time > 0.0 {
            self.mean_count / self.crossing_time
        } else {
            0.0
        }
    }

    /// Frequency at which the power spectrum turns over. Fitting it gives the orbital
    /// velocity, and therefore the semi-major axis.
    pub fn knee_hz(&self) -> f64 {
        if self.crossing_time > 0.0 {
            1.0 / (TAU * self.crossing_time)
        } else {
            0.0
        }
    }

    /// Fractional deficit at time `t`, seconds, in whichever regime applies.
    pub fn deficit_at(&self, t: f64) -> f64 {
        if self.mean_count <= 0.0 || self.crossing_time <= 0.0 {
            return 0.0;
        }
        match self.regime() {
            Regime::Gaussian => {
                (self.mean_deficit * (1.0 + self.relative_rms() * self.noise(t))).max(0.0)
            }
            Regime::Sparse => self.event_train(t),
        }
    }

    /// Unit-variance stationary noise with a knee at the disc crossing time.
    ///
    /// A sum of sinusoids rather than interpolated samples: the variance is then exactly one
    /// at every `t` rather than dipping between sample points, and any time can be evaluated
    /// without having generated the times before it.
    fn noise(&self, t: f64) -> f64 {
        let f_max = 2.0 / self.crossing_time;
        let mut amplitudes = [0.0f64; TERMS];
        let mut freqs = [0.0f64; TERMS];
        let mut phases = [0.0f64; TERMS];
        let mut power = 0.0;
        for k in 0..TERMS {
            let h = rng::hash(&[self.seed, k as u64, 0x5f1c]);
            let f = rng::uniform_in(h, 0.0, f_max);
            // Power of a box of width t_cross goes as sinc^2, so amplitude as |sinc|.
            let x = PI * f * self.crossing_time;
            let a = if x.abs() < 1e-9 {
                1.0
            } else {
                (x.sin() / x).abs()
            };
            freqs[k] = f;
            amplitudes[k] = a;
            phases[k] = rng::uniform_in(rng::mix(h), 0.0, TAU);
            power += a * a * 0.5;
        }
        if power <= 0.0 {
            return 0.0;
        }
        let scale = power.sqrt().recip();
        (0..TERMS)
            .map(|k| amplitudes[k] * (TAU * freqs[k] * t + phases[k]).cos())
            .sum::<f64>()
            * scale
    }

    /// A Poisson train of isolated occultations.
    ///
    /// Each event is a raised cosine of half-width `crossing_time`, whose integral is
    /// `depth * crossing_time`, so the long-run mean returns `mean_deficit`.
    fn event_train(&self, t: f64) -> f64 {
        let rate = self.event_rate();
        if rate <= 0.0 {
            return 0.0;
        }
        let bucket = 1.0 / rate;
        let first = ((t - self.crossing_time) / bucket).floor() as i64;
        let last = ((t + self.crossing_time) / bucket).floor() as i64;
        // In the sparse regime crossing_time << bucket, so this is one or two iterations.
        debug_assert!(last - first < 1024);
        let mut total = 0.0;
        for b in first..=last.min(first + 1023) {
            let h = rng::hash(&[self.seed, b as u64, 0xe0e1]);
            for j in 0..rng::poisson(h, 1.0) {
                let at = (b as f64 + rng::uniform(rng::hash(&[h, j as u64]))) * bucket;
                let d = t - at;
                if d.abs() < self.crossing_time {
                    let w = (PI * d / (2.0 * self.crossing_time)).cos();
                    total += self.single_event_depth * w * w;
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{Distribution, Inclination};
    use em_spectra::PerBand;

    const AU: f64 = 1.496e11;

    fn pop(count: f64, cross_section: f64, a: f64) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(a),
            eccentricity: Distribution::delta(0.0),
            inclination: Inclination::isotropic(),
            count,
            cross_section,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }

    fn reference_swarm() -> Flicker {
        Flicker::of(&pop(1.5e6, 1e12, AU), DVec3::X, &Star::SOL, 42)
    }

    fn sparse_fragments() -> Flicker {
        // 1000 fragments of 1000 km radius at 2 AU: m = 1.4e-3, so the mean is a fiction.
        let sigma = PI * 1e6 * 1e6;
        Flicker::of(&pop(1e3, sigma, 2.0 * AU), DVec3::X, &Star::SOL, 7)
    }

    #[test]
    fn the_reference_swarm_is_gaussian_with_a_35_percent_rms() {
        let f = reference_swarm();
        assert_eq!(f.regime(), Regime::Gaussian);
        assert!((f.mean_count - 8.11).abs() < 0.05);
        assert!(
            (f.relative_rms() - 0.351).abs() < 0.005,
            "rms/mean is {}",
            f.relative_rms()
        );
        assert!((f.crossing_time / 3600.0 - 13.0).abs() < 0.2);
    }

    #[test]
    fn gaussian_flicker_has_the_requested_mean_and_variance() {
        let f = reference_swarm();
        let n = 200_000;
        let dt = f.crossing_time / 3.0;
        let vals: Vec<f64> = (0..n).map(|k| f.deficit_at(k as f64 * dt)).collect();
        let mean = vals.iter().sum::<f64>() / n as f64;
        let rms = (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
        assert!(
            (mean / f.mean_deficit - 1.0).abs() < 0.02,
            "mean {mean} vs {}",
            f.mean_deficit
        );
        let want = f.mean_deficit * f.relative_rms();
        assert!((rms / want - 1.0).abs() < 0.15, "rms {rms} vs {want}");
    }

    #[test]
    fn the_sparse_branch_reports_events_not_the_mean() {
        let f = sparse_fragments();
        assert_eq!(f.regime(), Regime::Sparse);
        assert!(
            (f.mean_count / 1.35e-3 - 1.0).abs() < 0.05,
            "m is {}",
            f.mean_count
        );
        // The mean is undetectable; a single event is three orders of magnitude deeper.
        assert!(f.mean_deficit < 5e-9, "mean deficit {}", f.mean_deficit);
        assert!(
            f.single_event_depth > 1e-6,
            "event depth {}",
            f.single_event_depth
        );
        assert!(f.single_event_depth / f.mean_deficit > 500.0);
    }

    #[test]
    fn the_sparse_train_is_mostly_nothing_and_sometimes_deep() {
        let f = sparse_fragments();
        // Sparse means the run has to be long to hold any events at all: at this rate a span
        // of 5e5 crossing times contains about 675, so the sample mean carries about 4% of
        // Poisson error and the tolerance below is roughly four sigma.
        let n = 1_000_000;
        let dt = f.crossing_time / 2.0;
        let vals: Vec<f64> = (0..n).map(|k| f.deficit_at(k as f64 * dt)).collect();

        let quiet = vals
            .iter()
            .filter(|v| **v < f.single_event_depth * 0.01)
            .count();
        assert!(
            quiet as f64 / n as f64 > 0.9,
            "should be quiet most of the time"
        );
        let deepest = vals.iter().cloned().fold(0.0, f64::max);
        assert!(
            deepest > f.single_event_depth * 0.5,
            "events must actually reach their depth"
        );

        let expected_events = f.event_rate() * n as f64 * dt;
        assert!(
            expected_events > 400.0,
            "test is undersampled: {expected_events} events"
        );
        let mean = vals.iter().sum::<f64>() / n as f64;
        assert!(
            (mean / f.mean_deficit - 1.0).abs() < 0.15,
            "long-run mean {mean} should return the integral's {} over {expected_events} events",
            f.mean_deficit
        );
    }

    #[test]
    fn flicker_is_deterministic_and_seed_dependent() {
        let a = reference_swarm();
        let mut b = a;
        b.seed = 43;
        for t in [0.0, 1e4, 1e7] {
            assert_eq!(a.deficit_at(t), a.deficit_at(t));
            assert_ne!(a.deficit_at(t), b.deficit_at(t));
        }
    }

    #[test]
    fn a_population_that_is_not_there_flickers_not_at_all() {
        let f = Flicker::of(&pop(0.0, 1e12, AU), DVec3::X, &Star::SOL, 1);
        assert_eq!(f.deficit_at(123.0), 0.0);
    }

    #[test]
    fn the_knee_gives_back_the_orbital_radius() {
        // Fitting the knee is how an observer recovers a, so the relation must invert.
        let star = Star::SOL;
        let f = reference_swarm();
        let v = 2.0 * star.radius_m / f.crossing_time;
        let a = star.mu / (v * v);
        assert!((a / AU - 1.0).abs() < 1e-6, "recovered a = {} AU", a / AU);
        assert!((f.knee_hz() - 1.0 / (TAU * f.crossing_time)).abs() < 1e-18);
    }
}
