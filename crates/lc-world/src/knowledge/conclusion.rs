//! What a log says, once it has been read: hypotheses with probabilities, and the evidence.
//!
//! A log is read when it has grown enough to change the answer. Once the answer is settled the
//! samples are thrown away and what survives is the conclusion and a [`Digest`] — the log
//! folded at the periods that matter, so later samples refine the answer instead of starting
//! it over. The loss is deliberate; see `lightcone/docs/24-standing-instruments.md`.

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::transit::{self, Candidate, Fold, Point, Prior};
use super::{Knowledge, Lineage, Subject, Witness, learnt_s};

/// Probability at which the leading hypothesis is taken as settled and its log consumed.
pub const SETTLED: f64 = 0.99;

/// Transits a planet needs to have shown before its log is consumed: two fix a period, and the
/// third is the check that it was the right one.
pub const TRANSITS_TO_SETTLE: u32 = 3;

/// How strongly a log has to argue against a planet before "nothing there" is settled: the data
/// disfavouring one twentyfold, not merely a small prior. Without this a short log would
/// settle as quiet on the prior alone.
pub const QUIET_LN_BAYES: f64 = -3.0;

/// Samples a log gains before it is read again.
pub const READ_EVERY: usize = 96;

/// Folds kept for a settled planet: its period and two either side at the period's error, so
/// later samples can still move the period.
const NEIGHBOURS: i32 = 2;

/// What somebody concluded from somebody's photometry of one subject.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conclusion {
    /// Who read the log.
    pub witness: Witness,
    /// Whose log it was.
    pub observer: Witness,
    pub stated_s: f64,
    pub lineage: Lineage,
    /// Most probable first; the probabilities sum to one.
    pub hypotheses: Vec<Hypothesis>,
    pub evidence: Evidence,
    pub covering: Covering,
    /// The reader threw the log away through this arrival time, and cannot be asked for it.
    pub discarded_s: Option<f64>,
}

impl Conclusion {
    pub fn leading(&self) -> Option<&Hypothesis> {
        self.hypotheses.first()
    }

    pub fn learnt_s(&self) -> f64 {
        learnt_s(&self.lineage, self.stated_s)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hypothesis {
    pub probability: f64,
    pub kind: Kind,
}

/// What might be there. Only what the generator makes: see
/// `lightcone/docs/12-buildout.md`, 11d.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// No transiting planet in the periods searched.
    Quiet,
    Planet { class: Class, transit: Candidate },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Rocky,
    Giant,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Rocky => "rocky planet",
            Class::Giant => "giant planet",
        }
    }
}

/// What the probabilities were drawn from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Evidence {
    /// Samples read, over every pass, bands combined.
    pub samples: u64,
    /// Bands used, one bit per [`Band::index`].
    pub bands: u16,
    /// ln of how much more likely the log is with a transiting planet than without.
    pub ln_bayes: f64,
    /// How often the generator puts a transiting planet in the periods searched.
    pub prior: f64,
    pub periods_s: (f64, f64),
    /// Scatter beyond the error bars, as a fraction of the star's flux, found and allowed for.
    pub jitter: f64,
}

/// The stretch of time a conclusion describes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Covering {
    /// When the light read arrived, at the observer.
    pub observed_s: (f64, f64),
    /// How long it had travelled, if the reader had a distance. Without one a conclusion is
    /// about some stretch of the star's past it cannot place.
    pub light_age_s: Option<f64>,
}

impl Covering {
    pub fn emitted_s(&self) -> Option<(f64, f64)> {
        let age = self.light_age_s?;
        Some((self.observed_s.0 - age, self.observed_s.1 - age))
    }
}

/// What is kept of a log once it has been consumed. The reader's own, never transmitted: a
/// receiver can only trust a conclusion, not rework it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Digest {
    pub observer: Witness,
    pub samples: u64,
    pub bands: u16,
    pub observed_s: (f64, f64),
    /// The search that settled it: its odds, prior and range, which later passes update rather
    /// than recompute.
    pub ln_bayes: f64,
    pub delta_chi2: f64,
    pub prior: f64,
    pub periods_s: (f64, f64),
    pub jitter: f64,
    pub folds: Vec<Fold>,
}

/// Samples consumed from one series, for the store to delete.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consumed {
    pub subject: Subject,
    pub witness: Witness,
    pub band: Band,
    /// Everything observed at or before this is gone.
    pub through_s: f64,
}

/// Combine every band's samples taken at the same instant into one point, weighted by inverse
/// variance.
fn combine(series: &[&super::Series]) -> (Vec<Point>, u16) {
    let mut by_time: std::collections::BTreeMap<u64, (f64, f64, f64)> = Default::default();
    let mut bands = 0u16;
    for s in series {
        for sample in s.samples() {
            if !(sample.sigma > 0.0) {
                continue;
            }
            bands |= 1 << s.band.index();
            let w = 1.0 / (sample.sigma * sample.sigma);
            // Positive times only: bit patterns of non-negative floats sort as the floats do.
            let key = sample.observed_s.max(0.0).to_bits();
            let at = by_time.entry(key).or_insert((sample.observed_s, 0.0, 0.0));
            at.1 += w;
            at.2 += w * sample.deficit;
        }
    }
    let points = by_time.into_values().map(|(t, w, wx)| Point { t, x: wx / w, w }).collect();
    (points, bands)
}

/// Scatter the error bars do not explain, from the median absolute deviation so that transits
/// themselves — a few percent of the points, far off — do not count as noise.
fn jitter_of(points: &[Point]) -> f64 {
    if points.len() < 8 {
        return 0.0;
    }
    let mut x: Vec<f64> = points.iter().map(|p| p.x).collect();
    x.sort_by(f64::total_cmp);
    let median = x[x.len() / 2];
    let mut dev: Vec<f64> = x.iter().map(|v| (v - median).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let scale = 1.4826 * dev[dev.len() / 2];
    let mut var: Vec<f64> = points.iter().map(|p| 1.0 / p.w).collect();
    var.sort_by(f64::total_cmp);
    (scale * scale - var[var.len() / 2]).max(0.0).sqrt()
}

impl Knowledge {
    /// Mark a subject's logs to be kept whatever the pipeline concludes, or not.
    pub fn retain_raw(&mut self, subject: impl Into<Subject>, keep: bool) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        if file.retained != keep {
            file.retained = keep;
            self.changed.insert(subject);
        }
    }

    pub fn retained(&self, subject: impl Into<Subject>) -> bool {
        self.files.get(&subject.into()).is_some_and(|f| f.retained)
    }

    /// The conclusion this craft goes by about a subject: its own if it has drawn one, else the
    /// most recently stated.
    pub fn conclusion(&self, subject: impl Into<Subject>) -> Option<&Conclusion> {
        let owner = self.owner;
        self.files.get(&subject.into())?.conclusions.iter().max_by(|a, b| {
            (a.witness == owner, a.stated_s).partial_cmp(&(b.witness == owner, b.stated_s)).unwrap()
        })
    }

    /// File a conclusion. One per reader per observer, the later winning.
    pub fn concluded(&mut self, subject: impl Into<Subject>, conclusion: Conclusion) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file
            .conclusions
            .iter_mut()
            .find(|c| c.witness == conclusion.witness && c.observer == conclusion.observer)
        {
            Some(held) if held.stated_s >= conclusion.stated_s => return,
            Some(held) => *held = conclusion,
            None => file.conclusions.push(conclusion),
        }
        self.changed.insert(subject);
    }

    /// Samples consumed since the last call, for the store to delete. Clears them.
    pub fn take_consumed(&mut self) -> Vec<Consumed> {
        std::mem::take(&mut self.consumed)
    }

    /// Logs this craft holds that have grown enough since last read to be worth reading again.
    pub fn due(&self) -> Vec<(Subject, Witness)> {
        let mut out = Vec::new();
        for (subject, file) in &self.files {
            let mut observers: Vec<Witness> = file.series.iter().map(|s| s.witness).collect();
            observers.sort_unstable();
            observers.dedup();
            for observer in observers {
                let held = file.series.iter().filter(|s| s.witness == observer).map(|s| s.len()).max().unwrap_or(0);
                let read = file
                    .conclusions
                    .iter()
                    .find(|c| c.witness == self.owner && c.observer == observer)
                    .map_or(0, |c| c.evidence.samples as usize);
                let digested = file.digests.iter().find(|d| d.observer == observer).map_or(0, |d| d.samples as usize);
                if held + digested >= read + READ_EVERY {
                    out.push((*subject, observer));
                }
            }
        }
        out
    }

    /// Read one observer's log of one subject and state what it says.
    ///
    /// The log is consumed if the answer is settled and the subject is not retained. Returns
    /// the conclusion drawn, or `None` if there was nothing to read.
    pub fn read_log(&mut self, subject: Subject, observer: Witness, prior: &Prior, now_s: f64) -> Option<Conclusion> {
        let light_age_s = self.belief(subject).and_then(|b| b.light_age_s());
        let file = self.files.get(&subject)?;
        let series: Vec<&super::Series> = file.series.iter().filter(|s| s.witness == observer).collect();
        let digest = file.digests.iter().find(|d| d.observer == observer).cloned();
        let (mut points, bands) = combine(&series);
        // After combining: whatever the scatter is, it is common to every band, and adding it per
        // band would have the combination average it away.
        let jitter = digest.as_ref().map_or_else(|| jitter_of(&points), |d| d.jitter);
        points.iter_mut().for_each(|p| p.w = 1.0 / (1.0 / p.w + jitter * jitter));
        if points.is_empty() && digest.is_none() {
            return None;
        }
        let first = points.first().map_or(f64::INFINITY, |p| p.t);
        let last = points.last().map_or(f64::NEG_INFINITY, |p| p.t);

        let mut folds = Vec::new();
        let (ln_bayes, prior_p, periods, candidate, delta_chi2) = match &digest {
            // A settled planet: fold the new samples into what was kept and read it again there.
            Some(d) if !d.folds.is_empty() => {
                folds = d.folds.clone();
                folds.iter_mut().for_each(|f| f.add(&points));
                let best = folds.iter().filter_map(Fold::best).max_by(|a, b| a.delta_chi2.total_cmp(&b.delta_chi2));
                let best = best.map(|mut c| {
                    c.period_sigma_s = spacing(&folds).max(c.period_sigma_s);
                    c
                });
                let dchi = best.map_or(0.0, |c| c.delta_chi2);
                (d.ln_bayes + 0.5 * (dchi - d.delta_chi2), d.prior, d.periods_s, best, dchi)
            }
            _ => {
                let search = transit::search(&points, prior)?;
                let dchi = search.best.map_or(0.0, |c| c.delta_chi2);
                (search.ln_bayes, search.planet_prior, search.periods_s, search.best, dchi)
            }
        };
        let planet = transit::posterior(prior_p, ln_bayes);
        let mut hypotheses = vec![Hypothesis { probability: 1.0 - planet, kind: Kind::Quiet }];
        if let Some(c) = candidate {
            let rocky = prior.rocky_given(c.period_s, c.depth, c.depth_sigma).unwrap_or(0.5);
            for (class, share) in [(Class::Rocky, rocky), (Class::Giant, 1.0 - rocky)] {
                hypotheses.push(Hypothesis { probability: planet * share, kind: Kind::Planet { class, transit: c } });
            }
        }
        hypotheses.sort_by(|a, b| b.probability.total_cmp(&a.probability));

        let previous = digest.as_ref().map_or(0, |d| d.samples);
        let observed_s = match &digest {
            Some(d) => (d.observed_s.0.min(first), d.observed_s.1.max(last)),
            None => (first, last),
        };
        let mut conclusion = Conclusion {
            witness: self.owner,
            observer,
            stated_s: now_s,
            lineage: Lineage::new(),
            evidence: Evidence {
                samples: previous + points.len() as u64,
                bands: bands | digest.as_ref().map_or(0, |d| d.bands),
                ln_bayes,
                prior: prior_p,
                periods_s: periods,
                jitter,
            },
            covering: Covering { observed_s, light_age_s },
            hypotheses,
            discarded_s: digest.as_ref().and_then(|_| self.files[&subject].series.iter().find(|s| s.witness == observer).map(|s| s.consumed_s())).filter(|t| t.is_finite()),
        };

        let settled = match conclusion.leading().map(|h| (h.probability, h.kind)) {
            Some((p, Kind::Planet { transit, .. })) => p >= SETTLED && transit.transits >= TRANSITS_TO_SETTLE,
            Some((p, Kind::Quiet)) => p >= SETTLED && ln_bayes <= QUIET_LN_BAYES,
            None => false,
        };
        let retained = self.files[&subject].retained;
        if settled && !retained && !points.is_empty() {
            if folds.is_empty()
                && let Some(Kind::Planet { transit, .. }) = conclusion.leading().map(|h| h.kind)
            {
                folds = neighbours(&transit, &points);
            }
            let digest = Digest {
                observer,
                samples: conclusion.evidence.samples,
                bands: conclusion.evidence.bands,
                observed_s,
                ln_bayes,
                delta_chi2,
                prior: prior_p,
                periods_s: periods,
                jitter,
                folds,
            };
            self.consume(subject, observer, last, digest);
            conclusion.discarded_s = Some(last);
        }
        self.concluded(subject, conclusion.clone());
        Some(conclusion)
    }

    /// Throw away one observer's samples of a subject through `through_s`, keeping the digest.
    fn consume(&mut self, subject: Subject, observer: Witness, through_s: f64, digest: Digest) {
        let Some(file) = self.files.get_mut(&subject) else { return };
        for series in file.series.iter_mut().filter(|s| s.witness == observer) {
            series.consume_through(through_s);
            self.consumed.push(Consumed { subject, witness: observer, band: series.band, through_s });
        }
        // Samples not yet written down are simply not written.
        self.unsaved
            .retain(|l| !(l.subject == subject && l.witness == observer && l.sample.observed_s <= through_s));
        match file.digests.iter_mut().find(|d| d.observer == observer) {
            Some(held) => *held = digest,
            None => file.digests.push(digest),
        }
        self.changed.insert(subject);
    }
}

/// Folds at a planet's period and either side of it at its error.
fn neighbours(transit: &Candidate, points: &[Point]) -> Vec<Fold> {
    (-NEIGHBOURS..=NEIGHBOURS)
        .map(|k| {
            let mut fold = Fold::new(transit.period_s + k as f64 * transit.period_sigma_s, transit.epoch_s);
            fold.add(points);
            fold
        })
        .collect()
}

/// The period step between kept folds: what they can still resolve.
fn spacing(folds: &[Fold]) -> f64 {
    folds.windows(2).map(|w| (w[1].period_s - w[0].period_s).abs()).fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;

    use super::*;
    use crate::instrument::Instrument;
    use crate::knowledge::observatory::{Sky, Station, photometry};
    use crate::rng;
    use crate::sky::generate::ladder;
    use crate::sky::{CatalogueStar, Component, Provenance, StarId};
    use crate::star::Star;

    const YEAR_S: f64 = crate::flight::JULIAN_YEAR_S;
    const CADENCE_S: f64 = 1800.0;

    fn star(key: u64, luminosity_solar: f64, position_ly: DVec3) -> CatalogueStar {
        let l_w = luminosity_solar * em_spectra::stellar::SOLAR_LUMINOSITY;
        let teff = 5772.0 * luminosity_solar.powf(0.13);
        let mass = em_spectra::stellar::main_sequence_mass_solar(luminosity_solar);
        CatalogueStar {
            id: StarId::synthesise("conclusion", key),
            provenance: Provenance { source: "conclusion".into(), key, name: None },
            position_ly,
            velocity: DVec3::ZERO,
            star: Star {
                radius_m: em_spectra::stellar::radius_from_luminosity(l_w, teff),
                teff_k: teff,
                mu: em_spectra::stellar::mu_from_mass_solar(mass),
                limb_darkening: (0.4, 0.26),
            },
            luminosity_solar,
            mass_solar: mass,
            metallicity: 0.0,
            component: Component { index: 1, group: None },
        }
    }

    /// A neighbourhood like the real one: mostly red dwarfs, a few like the Sun, fewer brighter.
    fn neighbourhood() -> Vec<CatalogueStar> {
        (0..1500)
            .map(|k| {
                let u = rng::uniform(rng::hash(&[k, 0x6e]));
                star(k, 10f64.powf(-3.0 + 4.0 * u * u), DVec3::X * 100.0)
            })
            .collect()
    }

    /// A red dwarf five light-years out along `toward`, with the periods of the planets the
    /// generator gave it, of which the innermost goes round in under five days. They orbit in the
    /// ecliptic, so seen along X they transit and seen along Z they never do.
    fn red_dwarf(toward: DVec3) -> (CatalogueStar, Vec<f64>) {
        (100_000..)
            .find_map(|key| {
                let s = star(key, 0.01, toward * 5.0);
                let periods: Vec<f64> = ladder(s.seed(), s.luminosity_solar, s.metallicity)
                    .iter()
                    .map(|r| std::f64::consts::TAU * (r.semi_major_m.powi(3) / s.star.mu).sqrt())
                    .collect();
                (*periods.first()? < 5.0 * 86_400.0).then_some((s, periods))
            })
            .unwrap()
    }

    /// Stare at a star from the origin for `days`, one sample every [`CADENCE_S`].
    fn stare(target: &CatalogueStar, days: f64) -> (Knowledge, f64) {
        let mut sky = Sky::new(Arc::new(vec![target.clone()]));
        let mut knowledge = Knowledge::new(Witness(1));
        let at = Station { position_ly: DVec3::ZERO, instrument: Instrument::SHIP };
        let start = 10.0 * YEAR_S;
        let steps = (days * 86_400.0 / CADENCE_S) as usize;
        for n in 1..=steps {
            photometry(&mut sky, &mut knowledge, at, target.id, CADENCE_S, start + n as f64 * CADENCE_S);
        }
        (knowledge, start + steps as f64 * CADENCE_S)
    }

    /// The whole of 11d, in the library: a planet the generator placed comes out as the most
    /// probable hypothesis, with its period within error, and the log that found it is gone.
    #[test]
    fn a_generated_planet_is_the_most_probable_reading_of_its_transits() {
        let (target, periods) = red_dwarf(DVec3::X);
        let (mut knowledge, now) = stare(&target, 60.0);
        let mut replica = Knowledge::new(Witness(1));
        replica.absorb(&knowledge.report(f64::NEG_INFINITY, now));
        let prior = Prior::measure(&neighbourhood());
        let subject = Subject::Star(target.id);
        assert_eq!(knowledge.due(), vec![(subject, Witness(1))]);

        let conclusion = knowledge.read_log(subject, Witness(1), &prior, now).expect("a log to read");
        let leading = conclusion.leading().unwrap();
        let Kind::Planet { transit, .. } = leading.kind else { panic!("{:?}", conclusion.hypotheses) };
        assert!(leading.probability > SETTLED);
        let off = periods.iter().map(|p| (transit.period_s - p).abs()).fold(f64::INFINITY, f64::min);
        assert!(off < 3.0 * transit.period_sigma_s, "{} against {periods:?}, sigma {}", transit.period_s, transit.period_sigma_s);

        assert!(knowledge.file(target.id).unwrap().series().iter().all(|s| s.is_empty()), "the samples are gone");
        assert!(!knowledge.take_consumed().is_empty(), "and the store is told to delete them");
        assert_eq!(knowledge.conclusion(target.id), Some(&conclusion));

        // What is gone stays gone, however it comes back.
        let old = crate::knowledge::Sample { observed_s: now - 86_400.0, deficit: 0.0, sigma: 1e-5 };
        knowledge.measured(target.id, Witness(1), em_spectra::Band::V, old);
        assert!(knowledge.file(target.id).unwrap().series().iter().all(|s| s.is_empty()));

        // The conclusion travels; a replica drops the log with its original, and anyone else
        // holds the conclusion as something it was told.
        let report = knowledge.report(now - 1.0, now);
        replica.absorb(&report);
        assert!(replica.file(target.id).unwrap().series().iter().all(|s| s.is_empty()));
        assert_eq!(replica.conclusion(target.id), Some(&conclusion));
        let mut other = Knowledge::new(Witness(2));
        other.receive(&report, now + 10.0);
        assert_eq!(other.conclusion(target.id).unwrap().lineage.len(), 1);
    }

    /// Seen along the pole the same planets never transit, and the log says so.
    #[test]
    fn a_star_seen_along_its_pole_reads_as_quiet() {
        let (target, _) = red_dwarf(DVec3::Z);
        let (mut knowledge, now) = stare(&target, 60.0);
        let prior = Prior::measure(&neighbourhood());
        let conclusion = knowledge.read_log(Subject::Star(target.id), Witness(1), &prior, now).unwrap();
        assert!(matches!(conclusion.leading().unwrap().kind, Kind::Quiet), "{:?}", conclusion.hypotheses);
    }
}
