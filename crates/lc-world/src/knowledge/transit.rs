//! Finding a transiting planet in a star's log, and how likely it is that there is one.
//!
//! Box least squares over period, phase and duration, as
//! `lightcone/docs/05-observation.md` describes. The probability is a marginal likelihood: the
//! box's likelihood ratio averaged over every period, phase, duration and depth a planet could
//! have, weighted by how often the generator actually makes one there. So the only planets
//! this can find are ones the generator can place, and a 70% detection is right seven times in
//! ten. See `lightcone/docs/24-standing-instruments.md`.
//!
//! Times are coordinate seconds of arrival at the observer. That is the right clock for a period
//! while the observer is still; a moving one sees periods Doppler-shifted, which at the speeds
//! in play is below what a log resolves.

use serde::{Deserialize, Serialize};

use super::prior::Prior;
use crate::sky::generate::AU;

const DAY_S: f64 = 86_400.0;

/// Phase bins a fold has. A bin is well under the shortest transit worth searching for at any
/// period the search reaches in a log of a few months.
pub const BINS: usize = 512;

/// Transit durations searched, hours: a close-in planet of a red dwarf to a wide one of a hot
/// star.
const DURATIONS_H: [f64; 8] = [0.6, 1.0, 1.6, 2.6, 4.2, 6.7, 10.7, 17.0];


/// The shortest period searched. Shorter than anything the generator places around the
/// dimmest star it has.
pub const PERIOD_MIN_S: f64 = 0.2 * DAY_S;

/// Transits a log has to be able to hold before a period counts as searched.
pub const TRANSITS_NEEDED: f64 = 2.0;

/// One brightness measurement, bands already combined: arrival time, deficit, and weight
/// `1 / sigma^2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub t: f64,
    pub x: f64,
    pub w: f64,
}

/// A log folded at one period: sufficient for a box search at that period, whatever is added
/// to it later. Weighted sums per phase bin, the deficit itself rather than its departure from a
/// mean, because the mean moves as samples arrive.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Fold {
    pub period_s: f64,
    pub epoch_s: f64,
    weight: Vec<f64>,
    sum: Vec<f64>,
    /// Arrival times of the first and last sample folded in, for counting transits.
    pub span_s: (f64, f64),
}

impl Fold {
    pub fn new(period_s: f64, epoch_s: f64) -> Self {
        Self {
            period_s,
            epoch_s,
            weight: vec![0.0; BINS],
            sum: vec![0.0; BINS],
            span_s: (f64::INFINITY, f64::NEG_INFINITY),
        }
    }

    pub fn add(&mut self, points: &[Point]) {
        for p in points {
            let b = bin_of(p.t, self.epoch_s, self.period_s);
            self.weight[b] += p.w;
            self.sum[b] += p.w * p.x;
            self.span_s = (self.span_s.0.min(p.t), self.span_s.1.max(p.t));
        }
    }

    /// The deepest box this fold holds, at its own period.
    pub fn best(&self) -> Option<Candidate> {
        let total_w: f64 = self.weight.iter().sum();
        let total_x: f64 = self.sum.iter().sum();
        if total_w <= 0.0 {
            return None;
        }
        let mean = total_x / total_w;
        let centered: Vec<f64> = self.sum.iter().zip(&self.weight).map(|(x, w)| x - w * mean).collect();
        let (cw, cy) = prefix(&self.weight, &centered);
        let mut best: Option<Box> = None;
        for k in durations_in_bins(self.period_s) {
            for s in 0..BINS {
                if let Some(b) = box_at(&cw, &cy, total_w, s, k)
                    && b.z > best.map_or(0.0, |x| x.z)
                {
                    best = Some(b);
                }
            }
        }
        let b = best?;
        let (first, last) = self.span_s;
        Some(b.candidate(self.period_s, self.epoch_s, first, last, None))
    }
}

/// The best box at one period, as found.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub period_s: f64,
    pub period_sigma_s: f64,
    /// Arrival time of the middle of the first transit in the log.
    pub epoch_s: f64,
    pub duration_s: f64,
    pub depth: f64,
    pub depth_sigma: f64,
    /// How much better a box fits than a flat line.
    pub delta_chi2: f64,
    /// Transits the log spans at this period. An upper bound on how many were seen: a gap in
    /// the log can fall across one.
    pub transits: u32,
}

/// Lone events a search will set aside before giving up on finding a period.
const SET_ASIDE_MAX: u32 = 4;

/// The result of a search: the odds the log gives for a transiting planet, and the best one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Search {
    /// ln of the marginal likelihood of a transiting planet over that of none.
    pub ln_bayes: f64,
    /// How often the generator puts a transiting planet in the periods searched, before any data.
    pub planet_prior: f64,
    pub periods_s: (f64, f64),
    pub best: Option<Candidate>,
    /// Dips that no period in range repeats — a planet too wide to have transited twice in the
    /// log, most likely — taken out before the search that found `best`.
    pub set_aside: u32,
}

impl Search {
    /// Probability of a transiting planet in the periods searched, given the log.
    pub fn planet_probability(&self) -> f64 {
        posterior(self.planet_prior, self.ln_bayes)
    }
}

/// Prior odds updated by a log Bayes factor, without overflowing on a clear detection.
pub fn posterior(prior: f64, ln_bayes: f64) -> f64 {
    if prior <= 0.0 {
        return 0.0;
    }
    if prior >= 1.0 {
        return 1.0;
    }
    let ln_odds = (prior / (1.0 - prior)).ln() + ln_bayes;
    1.0 / (1.0 + (-ln_odds).exp())
}

/// Search a log for a transiting planet. `None` when the log is too short to hold two transits
/// of anything the search reaches.
///
/// One deep transit of a planet too wide to repeat in the log fits under a box at almost any
/// period, and would win. So a period is only believed if every transit it predicts that the log
/// covers is there; when the best is not, the dip carrying it is set aside and the log searched
/// again without it.
pub fn search(points: &[Point], prior: &Prior) -> Option<Search> {
    let mut points = points.to_vec();
    points.retain(|p| p.w > 0.0 && p.w.is_finite() && p.x.is_finite());
    points.sort_by(|a, b| a.t.total_cmp(&b.t));
    let span = points.last()?.t - points.first()?.t;
    let lone = lone_events(&points, (PERIOD_MIN_S, span / TRANSITS_NEEDED));
    for event in &lone {
        points.retain(|p| (p.t - event.t).abs() > event.duration);
    }
    let mut set_aside = lone.len() as u32;
    loop {
        let mut found = search_once(&points, prior)?;
        found.set_aside = set_aside;
        let Some(best) = found.best else { return Some(found) };
        match repeats(&points, &best) {
            Ok(()) => return Some(found),
            Err(_) if set_aside >= SET_ASIDE_MAX => return Some(Search { best: None, ..found }),
            Err(lone) => {
                points.retain(|p| (p.t - lone).abs() > best.duration_s);
                set_aside += 1;
            }
        }
    }
}

/// A single dip, found by sliding a box along the log in time.
#[derive(Clone, Copy, Debug)]
struct Event {
    t: f64,
    duration: f64,
    depth: f64,
    z: f64,
}

/// A dip this far out of the noise is found alone, without folding, and has to repeat to count.
const Z_EVENT: f64 = 10.0;

/// Dips too strong for their period to be left to the fold: ones no period in range repeats.
///
/// Cheap next to a search, which is why it goes first rather than searching again after each.
fn lone_events(points: &[Point], periods: (f64, f64)) -> Vec<Event> {
    let events = events(points, Z_EVENT / 3.0);
    events
        .iter()
        .filter(|e| e.z >= Z_EVENT)
        .filter(|e| {
            !events.iter().any(|other| {
                let alike = other.t != e.t
                    && (e.depth / 3.0..e.depth * 3.0).contains(&other.depth)
                    && (e.duration / 2.0..=e.duration * 2.0).contains(&other.duration);
                if !alike {
                    return false;
                }
                let gap = (other.t - e.t).abs();
                (1..).map(|k| gap / k as f64).take_while(|p| *p >= periods.0).any(|p| {
                    p <= periods.1 && recurs(points, e, p)
                })
            })
        })
        .copied()
        .collect()
}

/// Whether a dip is there again at every multiple of `period` the log covers, and the log covers
/// at least one.
fn recurs(points: &[Point], e: &Event, period: f64) -> bool {
    let (first, last) = (points[0].t, points[points.len() - 1].t);
    let from = ((first - e.t) / period).ceil() as i64;
    let to = ((last - e.t) / period).floor() as i64;
    let mut covered = 0;
    for n in from..=to {
        if n == 0 {
            continue;
        }
        let at = e.t + n as f64 * period;
        let lo = points.partition_point(|p| p.t < at - e.duration / 4.0);
        let hi = points.partition_point(|p| p.t <= at + e.duration / 4.0);
        if lo == hi {
            continue;
        }
        let w: f64 = points[lo..hi].iter().map(|p| p.w).sum();
        let depth = points[lo..hi].iter().map(|p| p.w * p.x).sum::<f64>() / w - baseline(points);
        if depth < e.depth / 3.0 - 3.0 / w.sqrt() {
            return false;
        }
        covered += 1;
    }
    covered > 0
}

fn baseline(points: &[Point]) -> f64 {
    let w: f64 = points.iter().map(|p| p.w).sum();
    points.iter().map(|p| p.w * p.x).sum::<f64>() / w
}

/// Every dip at least `z_min` deep, strongest first, none overlapping another.
fn events(points: &[Point], z_min: f64) -> Vec<Event> {
    let mean = baseline(points);
    let mut found: Vec<Event> = Vec::new();
    for hours in DURATIONS_H {
        let duration = hours * 3600.0;
        let (mut hi, mut w, mut wx) = (0, 0.0, 0.0);
        for lo in 0..points.len() {
            while hi < points.len() && points[hi].t < points[lo].t + duration {
                w += points[hi].w;
                wx += points[hi].w * (points[hi].x - mean);
                hi += 1;
            }
            if w > 0.0 {
                let depth = wx / w;
                let z = depth * w.sqrt();
                if z >= z_min {
                    found.push(Event { t: points[lo].t + duration / 2.0, duration, depth, z });
                }
            }
            w -= points[lo].w;
            wx -= points[lo].w * (points[lo].x - mean);
        }
    }
    found.sort_by(|a, b| b.z.total_cmp(&a.z));
    let mut kept: Vec<Event> = Vec::new();
    for e in found {
        if kept.iter().all(|k| (k.t - e.t).abs() > (k.duration + e.duration) / 2.0) {
            kept.push(e);
        }
    }
    kept
}

/// Whether every transit a candidate predicts, where the log covers it, is there. `Err` with
/// the middle of the deepest one when not.
///
/// "There" is lenient — a third of the mean depth, less three errors — because a partly covered
/// transit is shallower than a box, and limb darkening makes even a covered one uneven.
fn repeats(points: &[Point], c: &Candidate) -> Result<(), f64> {
    let total_w: f64 = points.iter().map(|p| p.w).sum();
    let mean = points.iter().map(|p| p.w * p.x).sum::<f64>() / total_w;
    let mut epochs: std::collections::BTreeMap<i64, (f64, f64)> = Default::default();
    for p in points {
        let n = ((p.t - c.epoch_s) / c.period_s).round();
        let off = p.t - (c.epoch_s + n * c.period_s);
        if off.abs() <= c.duration_s / 4.0 {
            let e = epochs.entry(n as i64).or_insert((0.0, 0.0));
            e.0 += p.w;
            e.1 += p.w * (p.x - mean);
        }
    }
    let depths: Vec<(i64, f64, f64)> =
        epochs.iter().map(|(n, (w, wx))| (*n, wx / w, 1.0 / w.sqrt())).collect();
    let (deepest, _, _) = *depths.iter().max_by(|a, b| a.1.total_cmp(&b.1)).ok_or(c.epoch_s)?;
    let lone = c.epoch_s + deepest as f64 * c.period_s;
    if depths.len() < 2 {
        return Err(lone);
    }
    let all_w: f64 = depths.iter().map(|(_, _, s)| 1.0 / (s * s)).sum();
    let mean_depth = depths.iter().map(|(_, d, s)| d / (s * s)).sum::<f64>() / all_w;
    if depths.iter().all(|(_, d, s)| *d > mean_depth / 3.0 - 3.0 * s) { Ok(()) } else { Err(lone) }
}

fn search_once(points: &[Point], prior: &Prior) -> Option<Search> {
    let (first, last) = (points.first()?.t, points.last()?.t);
    let span = last - first;
    let periods = (PERIOD_MIN_S, span / TRANSITS_NEEDED);
    if periods.1 <= periods.0 * 1.5 {
        return None;
    }
    let total_w: f64 = points.iter().map(|p| p.w).sum();
    let mean = points.iter().map(|p| p.w * p.x).sum::<f64>() / total_w;
    let centered: Vec<Point> = points.iter().map(|p| Point { x: p.x - mean, ..*p }).collect();
    let dt: Vec<f64> = centered.iter().map(|p| p.t - first).collect();
    let w: Vec<f64> = centered.iter().map(|p| p.w).collect();
    let wy: Vec<f64> = centered.iter().map(|p| p.w * p.x).collect();

    let density = prior.density(periods);
    let mut ln_bayes = Lse::default();
    let mut best: Option<(f64, f64, Box)> = None;
    let mut weight = vec![0.0; BINS];
    let mut sum = vec![0.0; BINS];
    let mut cw = vec![0.0; 2 * BINS + 1];
    let mut cy = vec![0.0; 2 * BINS + 1];
    let mut ln_f = -periods.1.ln();
    while ln_f <= -periods.0.ln() {
        let period = (-ln_f).exp();
        let (shortest, longest) = durations_for(period);
        // A step in frequency that moves the fold by half a typical transit over the log. A
        // grazing one, shorter, is found a little weaker.
        let step = typical_duration(period) / (2.0 * span);
        let bins = ((2.0 * period / shortest).ceil() as usize).clamp(32, BINS);
        let freq = 1.0 / period;
        weight[..bins].fill(0.0);
        sum[..bins].fill(0.0);
        for i in 0..dt.len() {
            let phase = dt[i] * freq;
            let b = (((phase - phase.floor()) * bins as f64) as usize).min(bins - 1);
            weight[b] += w[i];
            sum[b] += wy[i];
        }
        for i in 0..2 * bins {
            cw[i + 1] = cw[i] + weight[i % bins];
            cy[i + 1] = cy[i] + sum[i % bins];
        }
        let width = period / bins as f64;
        let mut durations: Vec<usize> = DURATIONS_H
            .iter()
            .map(|h| h * 3600.0)
            .filter(|d| (shortest..=longest).contains(d))
            .map(|d| ((d / width).round() as usize).max(1))
            .filter(|&k| k <= bins / 8)
            .collect();
        durations.dedup();
        let mut here = Lse::default();
        for &k in &durations {
            let mut phases = Lse::default();
            let mut modest = 0.0;
            let mut deepest: Option<Box> = None;
            let mut deepest_z = f64::NEG_INFINITY;
            for s in 0..bins {
                let w_in = cw[s + k] - cw[s];
                let w_out = total_w - w_in;
                if w_in <= 0.0 || w_out <= 0.0 {
                    continue;
                }
                let var = total_w / (w_in * w_out);
                let z = (cy[s + k] - cy[s]) * var.sqrt();
                // Almost every cell is noise with |z| of a few, summed directly from the table;
                // only the rare large one needs a running log-sum.
                if z < Z_TABLE_MIN {
                    continue;
                } else if z < Z_TABLE_MAX {
                    modest += ratio_table(z);
                } else {
                    phases.add(0.5 * z * z + ln_phi(z));
                }
                if z > deepest_z {
                    deepest_z = z;
                    deepest = Some(Box { start: s, bins: k, depth: (cy[s + k] - cy[s]) * var, sigma: var.sqrt(), z });
                }
            }
            if modest > 0.0 {
                phases.add(modest.ln());
            }
            let Some(d) = deepest else { continue };
            // The depth integral, at the phase that dominates it: the likelihood's width in
            // depth, times the prior density there.
            let depth = d.depth.max(d.sigma);
            let occam = (std::f64::consts::TAU.sqrt() * d.sigma).ln()
                + density.ln_at(period.ln(), depth.ln())
                - depth.ln();
            here.add(phases.value() - (bins as f64).ln() + occam);
            if best.is_none_or(|(_, _, b)| d.z > b.z) {
                best = Some((period, step, d));
            }
        }
        if !durations.is_empty() {
            ln_bayes.add(here.value() - (durations.len() as f64).ln() + step.ln());
        }
        ln_f += step;
    }
    let best = best.map(|(period, step, b)| refine(&centered, total_w, first, last, period, step, b));
    Some(Search {
        ln_bayes: ln_bayes.value(),
        planet_prior: prior.planet_prior(periods),
        periods_s: periods,
        best,
        set_aside: 0,
    })
}

/// Rescan around a peak finer than the grid, for the period and how well it is pinned.
fn refine(points: &[Point], total_w: f64, first: f64, last: f64, period: f64, step: f64, coarse: Box) -> Candidate {
    const FINE: i64 = 20;
    let mut scan = Vec::with_capacity(2 * FINE as usize + 1);
    for k in -FINE..=FINE {
        let p = period * (k as f64 * step / FINE as f64 * 2.0).exp();
        let mut fold = Fold::new(p, first);
        fold.add(points);
        let centered: Vec<f64> = fold.sum.clone();
        let (cw, cy) = prefix(&fold.weight, &centered);
        let mut top = None::<Box>;
        for kk in durations_in_bins(p) {
            for s in 0..BINS {
                if let Some(b) = box_at(&cw, &cy, total_w, s, kk)
                    && b.z > top.map_or(0.0, |t| t.z)
                {
                    top = Some(b);
                }
            }
        }
        scan.push((p, top));
    }
    let (p_best, b_best) = scan
        .iter()
        .filter_map(|(p, b)| Some((*p, (*b)?)))
        .max_by(|a, b| a.1.z.total_cmp(&b.1.z))
        .unwrap_or((period, coarse));
    let chi_best = b_best.z * b_best.z;
    let within: Vec<f64> = scan
        .iter()
        .filter(|(_, b)| b.is_some_and(|b| b.z * b.z >= chi_best - 1.0))
        .map(|(p, _)| *p)
        .collect();
    let fine_step = p_best * step * 2.0 / FINE as f64;
    let spread = within.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - within.iter().cloned().fold(f64::INFINITY, f64::min);
    let statistical = (spread / 2.0).max(fine_step / 12f64.sqrt());
    let found = b_best.candidate(p_best, first, first, last, None);
    // Two things the width of the peak does not know. A box is the wrong shape for a
    // limb-darkened transit, which at high signal to noise shows as scatter inside the box far
    // beyond the error bars; and the fold resolves the epoch only to a bin.
    let misfit = misfit(points, &found).max(1.0);
    let binned = p_best / BINS as f64 / 12f64.sqrt() / f64::from(found.transits.saturating_sub(1).max(1));
    let sigma = (statistical * statistical * misfit + binned * binned).sqrt();
    Candidate { period_sigma_s: sigma, ..found }
}

/// Reduced chi-squared of the points inside a candidate's transits about their own mean.
fn misfit(points: &[Point], c: &Candidate) -> f64 {
    let inside: Vec<&Point> = points
        .iter()
        .filter(|p| {
            let n = ((p.t - c.epoch_s) / c.period_s).round();
            (p.t - c.epoch_s - n * c.period_s).abs() < c.duration_s / 2.0
        })
        .collect();
    if inside.len() < 3 {
        return 1.0;
    }
    let w: f64 = inside.iter().map(|p| p.w).sum();
    let mean = inside.iter().map(|p| p.w * p.x).sum::<f64>() / w;
    inside.iter().map(|p| p.w * (p.x - mean).powi(2)).sum::<f64>() / (inside.len() - 1) as f64
}

/// A box of `k` bins starting at bin `s`, against the rest of the fold.
#[derive(Clone, Copy, Debug)]
struct Box {
    start: usize,
    bins: usize,
    depth: f64,
    sigma: f64,
    /// Depth over its error: positive for a dip.
    z: f64,
}

impl Box {
    fn candidate(&self, period: f64, epoch: f64, first: f64, last: f64, sigma: Option<f64>) -> Candidate {
        let width = period / BINS as f64;
        let duration = self.bins as f64 * width;
        let mut mid = epoch + (self.start as f64 + self.bins as f64 / 2.0) * width;
        while mid - period >= first - duration / 2.0 {
            mid -= period;
        }
        while mid < first - duration / 2.0 {
            mid += period;
        }
        let transits = if last >= mid - duration / 2.0 { ((last - mid + duration / 2.0) / period) as u32 + 1 } else { 0 };
        Candidate {
            period_s: period,
            period_sigma_s: sigma.unwrap_or(period / BINS as f64),
            epoch_s: mid,
            duration_s: duration,
            depth: self.depth,
            depth_sigma: self.sigma,
            delta_chi2: self.z.max(0.0).powi(2),
            transits,
        }
    }
}

fn box_at(cw: &[f64], cy: &[f64], total_w: f64, s: usize, k: usize) -> Option<Box> {
    let w_in = cw[s + k] - cw[s];
    let y_in = cy[s + k] - cy[s];
    let w_out = total_w - w_in;
    if w_in <= 0.0 || w_out <= 0.0 {
        return None;
    }
    let var = total_w / (w_in * w_out);
    let depth = y_in * var;
    let sigma = var.sqrt();
    Some(Box { start: s, bins: k, depth, sigma, z: depth / sigma })
}

/// Prefix sums over the fold laid out twice, so a box can wrap past phase one.
fn prefix(weight: &[f64], sum: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut cw = Vec::with_capacity(2 * BINS + 1);
    let mut cy = Vec::with_capacity(2 * BINS + 1);
    cw.push(0.0);
    cy.push(0.0);
    for i in 0..2 * BINS {
        cw.push(cw[i] + weight[i % BINS]);
        cy.push(cy[i] + sum[i % BINS]);
    }
    (cw, cy)
}

/// The transit durations worth trying at a period, seconds. A central transit lasts
/// `P / pi * R / a`, which goes as `P^(1/3)`: about an hour at one day for a red dwarf, two for
/// the Sun, three for a hot star. The range reaches below that for grazing transits.
fn durations_for(period: f64) -> (f64, f64) {
    let scale = typical_duration(period);
    (0.3 * scale, 4.5 * scale)
}

fn typical_duration(period: f64) -> f64 {
    3600.0 * (period / DAY_S).cbrt()
}

fn durations_in_bins(period: f64) -> Vec<usize> {
    let width = period / BINS as f64;
    let mut out: Vec<usize> = DURATIONS_H
        .iter()
        .map(|h| ((h * 3600.0 / width).round() as usize).max(1))
        .filter(|&k| k <= BINS / 8)
        .collect();
    out.dedup();
    out
}

fn bin_of(t: f64, epoch: f64, period: f64) -> usize {
    let phase = ((t - epoch) / period).rem_euclid(1.0);
    ((phase * BINS as f64) as usize).min(BINS - 1)
}

/// A running log-sum-exp.
#[derive(Clone, Copy, Debug)]
struct Lse {
    max: f64,
    sum: f64,
}

impl Default for Lse {
    fn default() -> Self {
        Self { max: f64::NEG_INFINITY, sum: 0.0 }
    }
}

impl Lse {
    fn add(&mut self, v: f64) {
        if v == f64::NEG_INFINITY {
            return;
        }
        if v > self.max {
            self.sum = self.sum * (self.max - v).exp() + 1.0;
            self.max = v;
        } else {
            self.sum += (v - self.max).exp();
        }
    }

    fn value(&self) -> f64 {
        if self.sum > 0.0 { self.max + self.sum.ln() } else { f64::NEG_INFINITY }
    }
}

const Z_TABLE_MIN: f64 = -6.0;
const Z_TABLE_MAX: f64 = 6.0;
const Z_TABLE_STEPS: usize = 12_000;

/// `exp(z^2 / 2) Phi(z)`, the likelihood ratio of one box integrated over positive depths up to
/// the depth prior's factor, interpolated from a table over `[Z_TABLE_MIN, Z_TABLE_MAX)`.
fn ratio_table(z: f64) -> f64 {
    static TABLE: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..=Z_TABLE_STEPS)
            .map(|i| {
                let z = Z_TABLE_MIN + (Z_TABLE_MAX - Z_TABLE_MIN) * i as f64 / Z_TABLE_STEPS as f64;
                (0.5 * z * z + ln_phi(z)).exp()
            })
            .collect()
    });
    let x = (z - Z_TABLE_MIN) / (Z_TABLE_MAX - Z_TABLE_MIN) * Z_TABLE_STEPS as f64;
    let i = (x as usize).min(Z_TABLE_STEPS - 1);
    let f = x - i as f64;
    table[i] * (1.0 - f) + table[i + 1] * f
}

/// ln of the standard normal CDF: what is left of a Gaussian in depth once depths below zero,
/// which no planet has, are cut away.
fn ln_phi(z: f64) -> f64 {
    if z > 5.0 {
        return 0.0;
    }
    let tail = 0.5 * erfc(-z / std::f64::consts::SQRT_2);
    if tail > 0.0 { tail.ln() } else { -0.5 * z * z - (-z * std::f64::consts::TAU.sqrt()).ln() }
}

/// Complementary error function, to 1.2e-7 relative everywhere (Numerical Recipes' `erfcc`).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

/// Semi-major axis of a period around a star of gravitational parameter `mu`, in AU.
pub fn semi_major_au(period_s: f64, mu: f64) -> f64 {
    (mu * (period_s / std::f64::consts::TAU).powi(2)).cbrt() / AU
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng;

    fn gaussian(seed: u64, n: u64) -> f64 {
        rng::gaussian(rng::hash(&[seed, n]))
    }

    /// A box-shaped transit in white noise.
    fn log(period_s: f64, depth: f64, duration_s: f64, sigma: f64, cadence_s: f64, span_s: f64) -> Vec<Point> {
        let n = (span_s / cadence_s) as u64;
        (0..n)
            .map(|k| {
                let t = k as f64 * cadence_s + 1_000.0;
                let phase = ((t - 7_000.0) / period_s).rem_euclid(1.0) * period_s;
                let dip = if phase < duration_s { depth } else { 0.0 };
                Point { t, x: dip + sigma * gaussian(11, k), w: 1.0 / (sigma * sigma) }
            })
            .collect()
    }

    #[test]
    fn erfc_matches_known_values() {
        assert!((erfc(0.0) - 1.0).abs() < 1e-7);
        assert!((erfc(1.0) - 0.157_299_207).abs() < 1e-7);
        assert!((erfc(-1.0) - 1.842_700_793).abs() < 1e-7);
        assert!((ln_phi(0.0) - 0.5f64.ln()).abs() < 1e-7);
    }

    #[test]
    fn a_fold_keeps_enough_to_find_the_box_again() {
        let period = 3.1 * DAY_S;
        let points = log(period, 0.01, 2.0 * 3600.0, 0.001, 1800.0, 40.0 * DAY_S);
        let mut fold = Fold::new(period, 0.0);
        fold.add(&points[..points.len() / 2]);
        fold.add(&points[points.len() / 2..]);
        let best = fold.best().unwrap();
        assert!((best.depth - 0.01).abs() < 0.001, "{best:?}");
        assert!(best.delta_chi2 > 1000.0);
    }
}
