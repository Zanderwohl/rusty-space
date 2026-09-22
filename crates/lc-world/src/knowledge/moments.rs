//! The steady part of a light curve, and its flicker: what belts and swarms show.
//!
//! A swarm dims a star in every visible band alike, flickers about that mean as its elements
//! cross the disc, and glows in the thermal infrared from the light it caught. A belt is too
//! thin to dim anything a craft can measure and shows only as that glow. All three are
//! moments — sums, sums of squares, and products at a lag — so, unlike a transit search, they
//! lose nothing when the samples behind them are thrown away. See
//! `lightcone/docs/05-observation.md`.

use std::collections::BTreeMap;

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::Series;

/// The bands a swarm dims alike. K and past it the elements' own glow starts to count.
const VISIBLE: [Band; 4] = [Band::B, Band::V, Band::R, Band::I];

/// Lags the flicker is correlated at, seconds: a quarter hour to five days, each a factor of
/// root two past the last. A crossing takes about two hours at a red dwarf's light radius and
/// two days at a hot star's.
const LAG_MIN_S: f64 = 900.0;
const LAGS: usize = 19;

/// Width of the bins points are averaged into before they are paired: half the shortest lag.
const BIN_S: f64 = LAG_MIN_S / 2.0;

/// Significance flicker needs before it is reported: its covariance at the shortest lag this
/// many errors above zero.
const FLICKER_SIGNIFICANCE: f64 = 3.0;

/// Sums over one band's samples, weighted by inverse variance.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
struct Sums {
    n: f64,
    w: f64,
    wx: f64,
    wxx: f64,
}

/// Sums over pairs of visible points at one lag.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
struct Pairs {
    n: f64,
    xy: f64,
    x: f64,
    y: f64,
    xx: f64,
}

/// A log's moments. Sufficient for what is read from them, and additive: a log read in
/// pieces gives the same answer as one read whole, less the pairs that straddle a piece's end.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Moments {
    bands: [Sums; 7],
    lags: Vec<Pairs>,
    /// Arrival times of the first and last sample, for how many independent stretches of
    /// flicker the log spans.
    span_s: Option<(f64, f64)>,
}

/// What the moments say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// Mean fraction of the visible light blocked, and its error.
    pub dim: (f64, f64),
    /// Thermal-infrared glow beyond the star's own, as a fraction of it, if the instrument sees
    /// that band.
    pub excess: Option<(f64, f64)>,
    /// Scatter of the dimming about its mean, if it stands out of the noise.
    pub flicker: Option<f64>,
    /// How long an element takes to cross the disc, from where the flicker stops being
    /// correlated with itself.
    pub crossing_s: Option<f64>,
}

impl Moments {
    /// Add every sample of these series, and the visible points they combine to.
    #[allow(clippy::indexing_slicing)] // band indices come from `Band::index`, below seven, and lag bins from `lag_bin`, below `LAGS`
    pub fn add(&mut self, series: &[&Series]) {
        let mut visible: BTreeMap<u64, (f64, f64, f64)> = Default::default();
        for s in series {
            let sums = &mut self.bands[s.band.index()];
            for sample in s.samples() {
                if !(sample.sigma > 0.0) {
                    continue;
                }
                let w = 1.0 / (sample.sigma * sample.sigma);
                sums.n += 1.0;
                sums.w += w;
                sums.wx += w * sample.deficit;
                sums.wxx += w * sample.deficit * sample.deficit;
                if VISIBLE.contains(&s.band) {
                    let at = visible.entry(sample.observed_s.max(0.0).to_bits()).or_insert((sample.observed_s, 0.0, 0.0));
                    at.1 += w;
                    at.2 += w * sample.deficit;
                }
            }
        }
        let points: Vec<(f64, f64)> = visible.into_values().map(|(t, w, wx)| (t, wx / w)).collect();
        // Paired as bins, not as points: a dense log pairs every sample with every other inside
        // five days, which is quadratic, and nothing shorter than the shortest lag is asked of it.
        let mut binned: BTreeMap<i64, (f64, f64, f64)> = BTreeMap::new();
        for &(t, x) in &points {
            let bin = binned.entry((t / BIN_S).floor() as i64).or_insert((0.0, 0.0, 0.0));
            bin.0 += t;
            bin.1 += x;
            bin.2 += 1.0;
        }
        let bins: Vec<(f64, f64)> = binned.into_values().map(|(t, x, n)| (t / n, x / n)).collect();
        if let (Some(first), Some(last)) = (points.first(), points.last()) {
            self.span_s = Some(match self.span_s {
                Some((a, b)) => (a.min(first.0), b.max(last.0)),
                None => (first.0, last.0),
            });
        }
        if self.lags.len() != LAGS {
            self.lags = vec![Pairs::default(); LAGS];
        }
        let reach = lag_edge(LAGS);
        for (i, &(t, x)) in bins.iter().enumerate() {
            for &(u, y) in bins.iter().skip(i + 1) {
                let lag = u - t;
                if lag >= reach {
                    break;
                }
                let Some(k) = lag_bin(lag) else { continue };
                let p = &mut self.lags[k];
                p.n += 1.0;
                p.xy += x * y;
                p.x += x;
                p.y += y;
                p.xx += x * x;
            }
        }
    }

    pub fn merge(&mut self, other: &Moments) {
        for (a, b) in self.bands.iter_mut().zip(&other.bands) {
            a.n += b.n;
            a.w += b.w;
            a.wx += b.wx;
            a.wxx += b.wxx;
        }
        if self.lags.len() != LAGS {
            self.lags = vec![Pairs::default(); LAGS];
        }
        for (a, b) in self.lags.iter_mut().zip(&other.lags) {
            a.n += b.n;
            a.xy += b.xy;
            a.x += b.x;
            a.y += b.y;
            a.xx += b.xx;
        }
        self.span_s = match (self.span_s, other.span_s) {
            (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            (a, b) => a.or(b),
        };
    }

    /// Mean and error of one band. The error is the larger of the photon noise and the scatter
    /// the samples actually show, which is what flicker is.
    #[allow(clippy::indexing_slicing)] // band indices come from `Band::index`, below seven, and lag bins from `lag_bin`, below `LAGS`
    fn mean(&self, band: Band) -> Option<(f64, f64)> {
        let s = self.bands[band.index()];
        if s.n < 2.0 || s.w <= 0.0 {
            return None;
        }
        let mean = s.wx / s.w;
        let scatter = (s.wxx / s.w - mean * mean).max(0.0) / (s.n - 1.0);
        Some((mean, (1.0 / s.w).max(scatter).sqrt()))
    }

    /// Covariance of the visible points with themselves at lag bin `k`, and its error.
    fn covariance(&self, k: usize) -> Option<(f64, f64)> {
        let p = self.lags.get(k)?;
        if p.n < 8.0 {
            return None;
        }
        let cov = p.xy / p.n - (p.x / p.n) * (p.y / p.n);
        let var = (p.xx / p.n - (p.x / p.n).powi(2)).max(0.0);
        Some((cov, var / p.n.sqrt()))
    }

    pub fn read(&self) -> Option<Reading> {
        let visible: Vec<(f64, f64)> = VISIBLE.iter().filter_map(|b| self.mean(*b)).collect();
        if visible.is_empty() {
            return None;
        }
        let w: f64 = visible.iter().map(|(_, s)| 1.0 / (s * s)).sum();
        let dim_mean = visible.iter().map(|(m, s)| m / (s * s)).sum::<f64>() / w;

        let shortest = (0..LAGS).find_map(|k| self.covariance(k).map(|c| (k, c)));
        let flicker = shortest
            .filter(|(_, (cov, err))| *cov > FLICKER_SIGNIFICANCE * err)
            .map(|(k, (cov, _))| (k, cov));
        // A crossing is a box in time, so its flicker's correlation falls as a triangle and
        // is half gone at half a crossing.
        let crossing_s = flicker.and_then(|(k0, cov0)| {
            (k0 + 1..LAGS).find_map(|k| {
                let (cov, _) = self.covariance(k)?;
                (cov < 0.5 * cov0).then(|| 2.0 * lag_center(k))
            })
        });
        // Flicker makes neighboring samples one measurement, not many: the mean is only as
        // good as the number of crossings the log spans.
        let span = self.span_s.map_or(0.0, |(a, b)| b - a);
        let independent = crossing_s.map_or(f64::INFINITY, |t| (span / t).max(1.0));
        let dim_sigma = (1.0 / w + flicker.map_or(0.0, |(_, c)| c) / independent).sqrt();

        let excess = self.mean(Band::ThermalIr).map(|(m, s)| (dim_mean - m, (s * s + dim_sigma * dim_sigma).sqrt()));
        Some(Reading {
            dim: (dim_mean, dim_sigma),
            excess,
            flicker: flicker.map(|(_, c)| c.sqrt()),
            crossing_s,
        })
    }
}

fn lag_edge(k: usize) -> f64 {
    LAG_MIN_S * std::f64::consts::SQRT_2.powi(k as i32)
}

fn lag_center(k: usize) -> f64 {
    (lag_edge(k) * lag_edge(k + 1)).sqrt()
}

/// Which lag bin a lag falls in; `None` below the shortest.
fn lag_bin(lag: f64) -> Option<usize> {
    if lag < LAG_MIN_S {
        return None;
    }
    let k = ((lag / LAG_MIN_S).ln() / std::f64::consts::SQRT_2.ln()) as usize;
    (k < LAGS).then_some(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Sample, Witness};
    use crate::rng;

    /// Every band dimmed by `dim` with flicker of `rms` that stays correlated for `crossing_s`,
    /// the thermal band less `excess`, sampled every half hour for sixty days.
    fn log(dim: f64, rms: f64, crossing_s: f64, excess: f64) -> Vec<Series> {
        let steps = 60 * 48;
        let flicker: Vec<f64> = {
            // Box-filtered white noise: correlated over one crossing, as a swarm is.
            let raw: Vec<f64> = (0..steps + 200).map(|k| rng::gaussian(rng::hash(&[5, k as u64]))).collect();
            let width = ((crossing_s / 1800.0).round() as usize).max(1);
            (0..steps)
                .map(|k| raw[k..k + width].iter().sum::<f64>() / (width as f64).sqrt() * rms)
                .collect()
        };
        Band::ALL
            .iter()
            .take(6)
            .map(|&band| {
                let mut s = Series::new(Witness(1), band);
                for k in 0..steps {
                    let noise = 1e-6 * rng::gaussian(rng::hash(&[9, band.index() as u64, k as u64]));
                    let glow = if band == Band::ThermalIr { excess } else { 0.0 };
                    let deficit = dim + flicker[k] - glow + noise;
                    s.push(Sample { observed_s: k as f64 * 1800.0, deficit, sigma: 1e-6 });
                }
                s
            })
            .collect()
    }

    #[test]
    fn a_swarm_reads_as_dimming_flicker_a_crossing_and_a_glow() {
        let series = log(0.01, 0.003, 6.0 * 3600.0, 0.002);
        let mut m = Moments::default();
        m.add(&series.iter().collect::<Vec<_>>());
        let r = m.read().unwrap();
        assert!((r.dim.0 - 0.01).abs() < 3.0 * r.dim.1 + 1e-4, "{r:?}");
        let flicker = r.flicker.expect("flicker");
        assert!((flicker / 0.003 - 1.0).abs() < 0.3, "{r:?}");
        let crossing = r.crossing_s.expect("a crossing");
        assert!((crossing / (6.0 * 3600.0) - 1.0).abs() < 0.6, "{crossing}");
        let (excess, sigma) = r.excess.unwrap();
        assert!((excess - 0.002).abs() < 3.0 * sigma + 1e-5, "{excess} ± {sigma}");
    }

    #[test]
    fn a_quiet_star_has_no_flicker_and_moments_read_in_pieces_read_the_same() {
        let series = log(0.0, 0.0, 1800.0, 0.0);
        let mut whole = Moments::default();
        whole.add(&series.iter().collect::<Vec<_>>());
        assert!(whole.read().unwrap().flicker.is_none());

        let mut pieces = Moments::default();
        for chunk in [0..1000usize, 1000..2880] {
            let parts: Vec<Series> = series
                .iter()
                .map(|s| {
                    let mut p = Series::new(Witness(1), s.band);
                    s.samples()[chunk.clone()].iter().for_each(|x| {
                        p.push(*x);
                    });
                    p
                })
                .collect();
            let mut m = Moments::default();
            m.add(&parts.iter().collect::<Vec<_>>());
            pieces.merge(&m);
        }
        let (a, b) = (whole.read().unwrap(), pieces.read().unwrap());
        assert!((a.dim.0 - b.dim.0).abs() < 1e-12);
    }
}
