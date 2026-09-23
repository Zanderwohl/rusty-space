//! What a log says, once it has been read: hypotheses with probabilities, and the evidence.
//!
//! Two questions, each with hypotheses summing to one: is something transiting, and is there a
//! swarm or only belts. Once the transit answer is settled, or the craft is out of room, the
//! samples are discarded, leaving the conclusion and a [`Digest`]. See
//! `lightcone/docs/24-standing-instruments.md`.

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::moments::Moments;
use super::prior::Prior;
use super::transit::{self, Candidate, Fold, Point};
use super::{Knowledge, Lineage, Subject, Witness, learned_s};

pub const SETTLED: f64 = 0.99;

/// Two fix a period; the third checks it.
pub const TRANSITS_TO_SETTLE: u32 = 3;

/// Samples a log gains before it is read again, at least.
pub const READ_EVERY: usize = 96;

/// And at least this fraction of what it held when last read. A read costs length times span,
/// so fixed intervals would cost as the square of a long watch; geometric ones cost a few times
/// the last read.
pub const READ_GROWTH: f64 = 0.5;

/// Folds either side of a settled planet's period, at its error, so later samples can move it.
const NEIGHBORS: i32 = 2;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conclusion {
    /// Who read the log.
    pub witness: Witness,
    /// Whose log it was.
    pub observer: Witness,
    /// Light-years. A transit is seen only from near its orbit's plane, so two observers in
    /// different places can both be right.
    pub from_ly: Option<glam::DVec3>,
    pub stated_s: f64,
    pub lineage: Lineage,
    /// Most probable first; sums to one.
    pub transits: Vec<Hypothesis>,
    /// Most probable first; sums to one. Empty when the log holds no visible band.
    pub populations: Vec<Hypothesis>,
    pub evidence: Evidence,
    pub covering: Covering,
    /// The reader discarded the log through this arrival time.
    pub discarded_s: Option<f64>,
}

impl Conclusion {
    pub fn leading(&self) -> Option<&Hypothesis> {
        self.transits.first()
    }

    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.stated_s)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hypothesis {
    pub probability: f64,
    pub kind: Kind,
}

/// Only what the generator makes: see `lightcone/docs/12-buildout.md`, 11d.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// No transiting planet; never more probable than [`Evidence::completeness`].
    Quiet,
    /// Provisional until the planet generator exists: see [`super::transit`].
    Planet { class: Class, transit: Candidate },
    /// A transiting planet this log could not have found yet.
    Unsearched,
    Swarm(Swarm),
    /// Only the belts every system has.
    Belts {
        /// Thermal-infrared glow beyond the star's own, as a fraction of it, and its error.
        excess: Option<(f64, f64)>,
    },
}

/// The inversion needs the star's radius and mass, which a craft has only as "a star as bright
/// as this one", and only with a distance.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Swarm {
    pub coverage: (f64, f64),
    pub flicker: Option<f64>,
    pub crossing_s: Option<f64>,
    /// Cross-section of one element, m^2.
    pub element_m2: Option<f64>,
    pub semi_major_au: Option<f64>,
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
    /// Over every pass, bands combined.
    pub samples: u64,
    /// One bit per [`Band::index`].
    pub bands: u16,
    pub ln_bayes: f64,
    pub prior: f64,
    /// `None` when the log was too short to search at all.
    pub periods_s: Option<(f64, f64)>,
    pub completeness: f64,
    /// Scatter beyond the error bars, as a fraction of the star's flux.
    pub jitter: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Covering {
    /// Arrival at the observer.
    pub observed_s: (f64, f64),
    /// `None` without a distance.
    pub light_age_s: Option<f64>,
}

impl Covering {
    pub fn emitted_s(&self) -> Option<(f64, f64)> {
        let age = self.light_age_s?;
        Some((self.observed_s.0 - age, self.observed_s.1 - age))
    }
}

/// What is kept of a consumed log. Never transmitted: a receiver can only trust a conclusion,
/// not rework it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Digest {
    pub observer: Witness,
    pub samples: u64,
    pub bands: u16,
    pub observed_s: (f64, f64),
    pub jitter: f64,
    pub moments: Moments,
    /// The most of any log so far, not combined: two logs that both missed long periods do not
    /// add up to one that did not.
    pub completeness: f64,
    /// Later passes update this search's odds rather than recompute them.
    pub planet: Option<Settled>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settled {
    pub ln_bayes: f64,
    pub delta_chi2: f64,
    pub prior: f64,
    pub periods_s: (f64, f64),
    pub folds: Vec<Fold>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Believed {
    pub planet: Option<Planet>,
    pub swarm: Option<(f64, Witness)>,
    pub readers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    pub probability: f64,
    pub class: Class,
    pub transit: Candidate,
    pub observer: Witness,
}

/// For the store to delete.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consumed {
    pub subject: Subject,
    pub witness: Witness,
    pub band: Band,
    pub through_s: f64,
}

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

/// From the median absolute deviation so that transits do not count as noise.
fn jitter_of(points: &[Point]) -> f64 {
    if points.len() < 8 {
        return 0.0;
    }
    let median = median_of(points.iter().map(|p| p.x).collect());
    let scale = 1.4826 * median_of(points.iter().map(|p| (p.x - median).abs()).collect());
    let var = median_of(points.iter().map(|p| 1.0 / p.w).collect());
    (scale * scale - var).max(0.0).sqrt()
}

/// Zero for none.
fn median_of(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}


impl Knowledge {
    pub fn retain_raw(&mut self, subject: impl Into<Subject>, keep: bool) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        if keep {
            self.analyzing.remove(&subject);
        }
        if file.retained != keep {
            file.retained = keep;
            self.changed.insert(subject);
        }
    }

    /// Read and consume every log of this craft's own, except retained ones, whatever they say.
    /// Queued at the usual read rate, since each read is a period search.
    pub fn analyze(&mut self) {
        let owner = self.owner;
        for (subject, file) in &self.files {
            if !file.retained && file.series.iter().any(|s| s.witness == owner && s.len() > 0) {
                self.analyzing.insert(*subject);
                self.unread.insert(*subject);
            }
        }
    }

    pub fn analyzing(&self) -> usize {
        self.analyzing.len()
    }

    pub fn retained_subjects(&self) -> Vec<Subject> {
        self.files.iter().filter(|(_, f)| f.retained).map(|(s, _)| *s).collect()
    }

    pub fn retained(&self, subject: impl Into<Subject>) -> bool {
        self.files.get(&subject.into()).is_some_and(|f| f.retained)
    }

    pub fn conclusion(&self, subject: impl Into<Subject>) -> Option<&Conclusion> {
        let owner = self.owner;
        self.files.get(&subject.into())?.conclusions.iter().find(|c| c.witness == owner && c.observer == owner)
    }

    /// This craft's own first, then the most recently stated.
    pub fn conclusions(&self, subject: impl Into<Subject>) -> Vec<&Conclusion> {
        let owner = self.owner;
        let mut all: Vec<&Conclusion> =
            self.files.get(&subject.into()).map(|f| f.conclusions.iter().collect()).unwrap_or_default();
        all.sort_by(|a, b| {
            (b.observer == owner).cmp(&(a.observer == owner)).then(b.stated_s.total_cmp(&a.stated_s))
        });
        all
    }

    /// A planet seen transiting from anywhere is there, so the strongest stands. A swarm looks
    /// the same from everywhere, so the longest log is believed.
    pub fn believed(&self, subject: impl Into<Subject>) -> Believed {
        let all = self.conclusions(subject);
        let planet = all
            .iter()
            .filter_map(|c| {
                let p: f64 = c.transits.iter().filter(|h| matches!(h.kind, Kind::Planet { .. })).map(|h| h.probability).sum();
                let best = c.transits.iter().find_map(|h| match h.kind {
                    Kind::Planet { class, transit } => Some((class, transit)),
                    _ => None,
                })?;
                Some(Planet { probability: p, class: best.0, transit: best.1, observer: c.observer })
            })
            .max_by(|a, b| a.probability.total_cmp(&b.probability));
        let swarm = all.iter().filter(|c| !c.populations.is_empty()).max_by_key(|c| c.evidence.samples).map(|c| {
            let p = c.populations.iter().filter(|h| matches!(h.kind, Kind::Swarm(_))).map(|h| h.probability).sum();
            (p, c.observer)
        });
        Believed { planet, swarm, readers: all.len() }
    }

    /// One per reader per observer, the later winning.
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
        self.refresh(subject);
    }

    pub fn take_consumed(&mut self) -> Vec<Consumed> {
        std::mem::take(&mut self.consumed)
    }

    /// Own logs grown enough to read again; nobody else's logs travel. A queue, so asking costs
    /// what is waiting rather than every file held.
    pub fn due(&self) -> Vec<(Subject, Witness)> {
        self.due_iter().collect()
    }

    /// The first of [`Knowledge::due`], without finding the rest.
    pub fn next_due(&self) -> Option<(Subject, Witness)> {
        self.due_iter().next()
    }

    fn due_iter(&self) -> impl Iterator<Item = (Subject, Witness)> + '_ {
        let owner = self.owner;
        let full = self.is_full();
        self.unread
            .iter()
            .filter(move |subject| {
                let Some(file) = self.files.get(subject) else { return false };
                let held = file.series.iter().filter(|s| s.witness == owner).map(|s| s.len()).max().unwrap_or(0);
                let read = file
                    .conclusions
                    .iter()
                    .find(|c| c.witness == owner && c.observer == owner)
                    .map_or(0, |c| c.evidence.samples as usize);
                let digested = file.digests.iter().find(|d| d.observer == owner).map_or(0, |d| d.samples as usize);
                let next = (read + READ_EVERY).max((read as f64 * (1.0 + READ_GROWTH)) as usize);
                // A full craft reads whatever it holds: reading is how it makes room.
                ((full || self.analyzing.contains(subject)) && held > 0) || held + digested >= next
            })
            .map(move |subject| (*subject, owner))
    }

    /// Unless the subject is retained, the log is consumed if the transit answer is settled, the
    /// craft is full, or it is being analyzed.
    pub fn read_log(&mut self, subject: Subject, observer: Witness, prior: &Prior, now_s: f64) -> Option<Conclusion> {
        self.unread.remove(&subject);
        let analyzing = self.analyzing.remove(&subject);
        let belief = self.belief(subject);
        let light_age_s = belief.and_then(|b| b.light_age_s());
        let host = belief.and_then(|b| prior.host_like(b.band, b.luminosity_w()?));
        let file = self.files.get(&subject)?;
        let series: Vec<&super::Series> = file.series.iter().filter(|s| s.witness == observer).collect();
        let digest = file.digests.iter().find(|d| d.observer == observer).cloned();
        let (mut points, bands) = combine(&series);
        if points.is_empty() && digest.is_none() {
            return None;
        }
        // After combining: the scatter is common to every band, and adding it per band would
        // have the combination average it away.
        let jitter = digest.as_ref().map_or_else(|| jitter_of(&points), |d| d.jitter);
        points.iter_mut().for_each(|p| p.w = 1.0 / (1.0 / p.w + jitter * jitter));
        let first = points.first().map_or(f64::INFINITY, |p| p.t);
        let last = points.last().map_or(f64::NEG_INFINITY, |p| p.t);
        let mut moments = digest.as_ref().map(|d| d.moments.clone()).unwrap_or_default();
        moments.add(&series);

        let settled = digest.as_ref().and_then(|d| d.planet.clone());
        let mut folds = Vec::new();
        let (ln_bayes, prior_p, periods, candidate, delta_chi2) = match &settled {
            Some(planet) => {
                folds = planet.folds.clone();
                folds.iter_mut().for_each(|f| f.add(&points));
                let best = folds.iter().filter_map(Fold::best).max_by(|a, b| a.delta_chi2.total_cmp(&b.delta_chi2));
                let best = best.map(|mut c| {
                    c.period_sigma_s = spacing(&folds).max(c.period_sigma_s);
                    c
                });
                let dchi = best.map_or(0.0, |c| c.delta_chi2);
                let ln_bayes = planet.ln_bayes + 0.5 * (dchi - planet.delta_chi2);
                (ln_bayes, planet.prior, Some(planet.periods_s), best, dchi)
            }
            None => match transit::search(&points, prior) {
                Some(search) => {
                    let dchi = search.best.map_or(0.0, |c| c.delta_chi2);
                    (search.ln_bayes, search.planet_prior, Some(search.periods_s), search.best, dchi)
                }
                None => (0.0, 0.0, None, None, 0.0),
            },
        };
        let planet = transit::posterior(prior_p, ln_bayes);
        let completeness = match periods {
            Some(periods) if !points.is_empty() => prior.completeness(periods, typical_sigma(&points), points.len()),
            _ => 0.0,
        }
        .max(digest.as_ref().map_or(0.0, |d| d.completeness));
        let mut transits = vec![
            Hypothesis { probability: (1.0 - planet) * completeness, kind: Kind::Quiet },
            Hypothesis { probability: (1.0 - planet) * (1.0 - completeness), kind: Kind::Unsearched },
        ];
        if let Some(c) = candidate {
            let rocky = prior.rocky_given(c.period_s, c.depth, c.depth_sigma).unwrap_or(0.5);
            for (class, share) in [(Class::Rocky, rocky), (Class::Giant, 1.0 - rocky)] {
                transits.push(Hypothesis { probability: planet * share, kind: Kind::Planet { class, transit: c } });
            }
        } else if planet > 0.0 {
            // Odds for a planet with no box: nothing to name, so it goes to Unsearched.
            if let Some(unsearched) = transits.iter_mut().find(|h| h.kind == Kind::Unsearched) {
                unsearched.probability += planet;
            }
        }
        transits.sort_by(|a, b| b.probability.total_cmp(&a.probability));
        let populations = populations(&moments, prior, host);

        let previous = digest.as_ref().map_or(0, |d| d.samples);
        let observed_s = match &digest {
            Some(d) => (d.observed_s.0.min(first), d.observed_s.1.max(last)),
            None => (first, last),
        };
        let from_ly = self.files.get(&subject).and_then(|f| {
            f.sightings
                .iter()
                .filter(|s| s.witness == observer)
                .max_by(|a, b| a.observed_s.total_cmp(&b.observed_s))
                .map(|s| s.bearing.observer_ly)
        });
        let mut conclusion = Conclusion {
            witness: self.owner,
            observer,
            from_ly,
            stated_s: now_s,
            lineage: Lineage::new(),
            evidence: Evidence {
                samples: previous + points.len() as u64,
                bands: bands | digest.as_ref().map_or(0, |d| d.bands),
                ln_bayes,
                prior: prior_p,
                periods_s: periods,
                completeness,
                jitter,
            },
            covering: Covering { observed_s, light_age_s },
            transits,
            populations,
            discarded_s: digest
                .as_ref()
                .and_then(|_| self.files.get(&subject)?.series.iter().find(|s| s.witness == observer).map(|s| s.consumed_s()))
                .filter(|t| t.is_finite()),
        };

        let leading = conclusion.leading().map(|h| (h.probability, h.kind));
        let planet_settled = match leading {
            Some((p, Kind::Planet { transit, .. })) => p >= SETTLED && transit.transits >= TRANSITS_TO_SETTLE,
            _ => false,
        };
        let quiet_settled = matches!(leading, Some((p, Kind::Quiet)) if p >= SETTLED);
        let retained = self.files.get(&subject).is_some_and(|f| f.retained);
        if (planet_settled || quiet_settled || analyzing || self.is_full()) && !retained && !points.is_empty() {
            let planet = match (&settled, planet_settled, leading) {
                (Some(held), _, _) => Some(Settled { folds, ln_bayes, delta_chi2, ..held.clone() }),
                (None, true, Some((_, Kind::Planet { transit, .. }))) => periods.map(|periods_s| Settled {
                    ln_bayes,
                    delta_chi2,
                    prior: prior_p,
                    periods_s,
                    folds: neighbors(&transit, &points),
                }),
                // Nothing settled: what the search had is lost.
                _ => None,
            };
            let digest = Digest {
                observer,
                samples: conclusion.evidence.samples,
                bands: conclusion.evidence.bands,
                observed_s,
                jitter,
                moments,
                completeness,
                planet,
            };
            self.consume(subject, observer, last, digest);
            conclusion.discarded_s = Some(last);
        }
        self.concluded(subject, conclusion.clone());
        Some(conclusion)
    }

    fn consume(&mut self, subject: Subject, observer: Witness, through_s: f64, digest: Digest) {
        let Some(file) = self.files.get_mut(&subject) else { return };
        for series in file.series.iter_mut().filter(|s| s.witness == observer) {
            series.consume_through(through_s);
            self.consumed.push(Consumed { subject, witness: observer, band: series.band, through_s });
        }
        self.unsaved
            .retain(|l| !(l.subject == subject && l.witness == observer && l.sample.observed_s <= through_s));
        match file.digests.iter_mut().find(|d| d.observer == observer) {
            Some(held) => *held = digest,
            None => file.digests.push(digest),
        }
        self.changed.insert(subject);
    }
}

fn populations(moments: &Moments, prior: &Prior, host: Option<crate::star::Star>) -> Vec<Hypothesis> {
    let Some(reading) = moments.read() else { return Vec::new() };
    let Some(swarm) = prior.swarm_given(reading.dim, reading.excess) else { return Vec::new() };
    let inverted = match (reading.flicker, reading.crossing_s, host) {
        (Some(rms), Some(crossing), Some(star)) => crate::emission::invert_moments(reading.dim.0, rms, crossing, &star),
        _ => None,
    };
    let mut out = vec![
        Hypothesis {
            probability: swarm,
            kind: Kind::Swarm(Swarm {
                coverage: reading.dim,
                flicker: reading.flicker,
                crossing_s: reading.crossing_s,
                element_m2: inverted.map(|i| i.element_area),
                semi_major_au: inverted.map(|i| i.semi_major / crate::sky::generate::AU),
            }),
        },
        Hypothesis { probability: 1.0 - swarm, kind: Kind::Belts { excess: reading.excess } },
    ];
    out.sort_by(|a, b| b.probability.total_cmp(&a.probability));
    out
}

fn typical_sigma(points: &[Point]) -> f64 {
    median_of(points.iter().map(|p| p.w.recip().sqrt()).collect())
}

fn neighbors(transit: &Candidate, points: &[Point]) -> Vec<Fold> {
    (-NEIGHBORS..=NEIGHBORS)
        .map(|k| {
            let mut fold = Fold::new(transit.period_s + k as f64 * transit.period_sigma_s, transit.epoch_s);
            fold.add(points);
            fold
        })
        .collect()
}

/// What the kept folds can still resolve.
fn spacing(folds: &[Fold]) -> f64 {
    folds.iter().zip(folds.iter().skip(1)).map(|(a, b)| (b.period_s - a.period_s).abs()).fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::DVec3;

    use super::*;
    use crate::instrument::Instrument;
    use crate::knowledge::observatory::{Sky, Station, photometry};
    use crate::rng;
    use crate::sky::generate::planets_of;
    use crate::sky::{CatalogStar, Component, Provenance, StarId};
    use crate::star::Star;

    const YEAR_S: f64 = crate::flight::JULIAN_YEAR_S;
    const CADENCE_S: f64 = 1800.0;

    fn star(key: u64, luminosity_solar: f64, position_ly: DVec3) -> CatalogStar {
        let l_w = luminosity_solar * em_spectra::stellar::SOLAR_LUMINOSITY;
        let teff = 5772.0 * luminosity_solar.powf(0.13);
        let mass = em_spectra::stellar::main_sequence_mass_solar(luminosity_solar);
        CatalogStar {
            id: StarId::synthesize("conclusion", key),
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

    /// Mostly red dwarfs, a few like the Sun, fewer brighter.
    fn neighborhood() -> Vec<CatalogStar> {
        (0..1500)
            .map(|k| {
                let u = rng::uniform(rng::hash(&[k, 0x6e]));
                star(k, 10f64.powf(-3.0 + 4.0 * u * u), DVec3::X * 100.0)
            })
            .collect()
    }

    /// A late M dwarf five light-years out with an inner planet under five days and deep
    /// enough to see, placed edge-on (its planets transit) or along its pole (they never do), and its
    /// planets' periods.
    ///
    /// Both conditions are searched for rather than assumed. The generator makes plenty of
    /// planets no sixty-day log would ever find, and a test of what a log concludes needs one
    /// it can conclude something about. A late M dwarf because that is what makes an
    /// Earth-sized planet a percent-deep transit, which is why the real search uses them too.
    fn red_dwarf(edge_on: bool) -> (CatalogStar, Vec<f64>) {
        (100_000..)
            .find_map(|key| {
                let mut s = star(key, 0.001, DVec3::ZERO);
                let pole = crate::sky::generate::pole_for(s.seed());
                s.position_ly = if edge_on { pole.any_orthonormal_vector() } else { pole } * 5.0;
                let planets = planets_of(&s);
                let ratio = planets.first()?.radius_m / s.star.radius_m;
                let periods: Vec<f64> = planets
                    .iter()
                    .map(|r| std::f64::consts::TAU * (r.semi_major_m.powi(3) / s.star.mu).sqrt())
                    .collect();
                (*periods.first()? < 5.0 * 86_400.0 && ratio * ratio > 1.2e-3).then_some((s, periods))
            })
            .unwrap()
    }

    fn stare(target: &CatalogStar, days: f64) -> (Knowledge, f64) {
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

    /// A generated planet is the leading hypothesis, its period within error, and its log is
    /// consumed.
    #[test]
    fn a_generated_planet_is_the_most_probable_reading_of_its_transits() {
        let (target, periods) = red_dwarf(true);
        let (mut knowledge, now) = stare(&target, 110.0);
        let mut replica = Knowledge::new(Witness(1));
        replica.absorb(&knowledge.report(crate::knowledge::Mark::default(), now));
        replica.copy_logs(&knowledge.logs_upto(f64::NEG_INFINITY, usize::MAX).0);
        assert!(replica.file(target.id).unwrap().series().iter().any(|s| !s.is_empty()));
        let prior = Prior::measure(&neighborhood());
        let subject = Subject::Star(target.id);
        assert_eq!(knowledge.due(), vec![(subject, Witness(1))]);

        let conclusion = knowledge.read_log(subject, Witness(1), &prior, now).expect("a log to read");
        let leading = conclusion.leading().unwrap();
        let Kind::Planet { transit, .. } = leading.kind else { panic!("{:?}", conclusion.transits) };
        // That there is a planet is settled. Which kind it is need not be: a 1.3-Earth-radius
        // body and a small ice giant make transits of nearly the same depth, and the prior
        // says so rather than pretending otherwise.
        let a_planet: f64 = conclusion
            .transits
            .iter()
            .filter(|h| matches!(h.kind, Kind::Planet { .. }))
            .map(|h| h.probability)
            .sum();
        assert!(a_planet > SETTLED, "{:?}", conclusion.transits);
        let off = periods.iter().map(|p| (transit.period_s - p).abs()).fold(f64::INFINITY, f64::min);
        assert!(off < 3.0 * transit.period_sigma_s, "{} against {periods:?}, sigma {}", transit.period_s, transit.period_sigma_s);

        assert!(knowledge.file(target.id).unwrap().series().iter().all(|s| s.is_empty()), "the samples are gone");
        assert!(!knowledge.take_consumed().is_empty(), "and the store is told to delete them");
        assert_eq!(knowledge.conclusion(target.id), Some(&conclusion));

        // What is gone stays gone, however it comes back.
        let old = crate::knowledge::Sample { observed_s: now - 86_400.0, deficit: 0.0, sigma: 1e-5 };
        knowledge.measured(target.id, Witness(1), em_spectra::Band::V, old);
        assert!(knowledge.file(target.id).unwrap().series().iter().all(|s| s.is_empty()));

        // A replica drops the log with its original; anyone else holds the conclusion as told.
        let report = knowledge.report(crate::knowledge::Mark::through(now - 1.0), now);
        replica.absorb(&report);
        assert!(replica.file(target.id).unwrap().series().iter().all(|s| s.is_empty()));
        assert_eq!(replica.conclusion(target.id), Some(&conclusion));
        let mut other = Knowledge::new(Witness(2));
        other.receive(&report, now + 10.0);
        let told = other.conclusions(target.id);
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].observer, told[0].lineage.len()), (Witness(1), 1), "on the observer's name, a hop away");
        assert!(other.conclusion(target.id).is_none(), "and not the receiver's own");
        let believed = other.believed(target.id);
        assert_eq!(believed.planet.map(|p| p.observer), Some(Witness(1)));

        // An observer off the orbit's plane saw nothing; both are held and the planet stands.
        let elsewhere = Conclusion {
            witness: Witness(5),
            observer: Witness(5),
            transits: vec![Hypothesis { probability: 1.0, kind: Kind::Quiet }],
            stated_s: now + 20.0,
            ..conclusion.clone()
        };
        other.concluded(target.id, elsewhere);
        assert_eq!(other.conclusions(target.id).len(), 2);
        let believed = other.believed(target.id);
        assert_eq!((believed.readers, believed.planet.map(|p| p.observer)), (2, Some(Witness(1))));
    }

    /// Seen along the pole, nothing transits, and Quiet is capped by what two months could see.
    #[test]
    fn a_star_seen_along_its_pole_is_quiet_only_as_far_as_the_log_could_see() {
        let (target, _) = red_dwarf(false);
        let (mut knowledge, now) = stare(&target, 110.0);
        let prior = Prior::measure(&neighborhood());
        let conclusion = knowledge.read_log(Subject::Star(target.id), Witness(1), &prior, now).unwrap();
        let chance = |f: fn(&Kind) -> bool| -> f64 {
            conclusion.transits.iter().filter(|h| f(&h.kind)).map(|h| h.probability).sum()
        };
        assert!(chance(|k| matches!(k, Kind::Planet { .. })) < 0.01, "{:?}", conclusion.transits);
        let quiet = chance(|k| matches!(k, Kind::Quiet));
        assert!(quiet <= conclusion.evidence.completeness + 1e-12);
        assert!(quiet < SETTLED, "two months cannot rule out a planet on a year's orbit: {quiet}");
        assert!(conclusion.discarded_s.is_none(), "so the log is kept");
        assert!((conclusion.transits.iter().map(|h| h.probability).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    /// A full craft consumes a log even with nothing settled, keeping moments and completeness.
    #[test]
    fn a_full_craft_consumes_what_it_could_not_settle() {
        let (target, _) = red_dwarf(false);
        let (mut knowledge, now) = stare(&target, 20.0);
        knowledge.fit_to(f64::INFINITY);
        let capacity = knowledge.occupied_bytes();
        knowledge.fit_to(capacity);
        assert!(knowledge.is_full());
        let prior = Prior::measure(&neighborhood());
        let read = knowledge.read_log(Subject::Star(target.id), Witness(1), &prior, now).unwrap();
        assert!(read.discarded_s.is_some());
        let file = knowledge.file(target.id).unwrap();
        assert!(file.series().iter().all(|s| s.is_empty()));
        let digest = &file.digests()[0];
        assert!(digest.planet.is_none() && digest.completeness == read.evidence.completeness);
        knowledge.fit_to(capacity);
        assert!(!knowledge.is_full(), "and there is room again");
    }

    /// Analyzing consumes every log as if the craft were full, except a retained one.
    #[test]
    fn analyzing_consumes_every_log_but_a_retained_one() {
        let (target, _) = red_dwarf(false);
        let (mut knowledge, now) = stare(&target, 20.0);
        let subject = Subject::Star(target.id);
        let prior = Prior::measure(&neighborhood());
        knowledge.read_log(subject, Witness(1), &prior, now).unwrap();
        assert!(knowledge.due().is_empty(), "read, and too little new since to read again");
        assert!(knowledge.bytes() > 0.0, "and kept, since nothing was settled");

        knowledge.retain_raw(subject, true);
        knowledge.analyze();
        assert_eq!(knowledge.analyzing(), 0, "a retained log is not analyzed");

        knowledge.retain_raw(subject, false);
        knowledge.analyze();
        assert_eq!((knowledge.analyzing(), knowledge.due()), (1, vec![(subject, Witness(1))]));
        let read = knowledge.read_log(subject, Witness(1), &prior, now).unwrap();
        assert!(read.discarded_s.is_some());
        assert_eq!((knowledge.analyzing(), knowledge.bytes()), (0, 0.0), "the room is free");
    }

    /// A generated swarm reads as a swarm with its coverage; a star without reads as belts.
    #[test]
    fn a_swarm_is_read_from_its_moments_and_belts_from_their_absence() {
        use crate::population::Population;
        let has_swarm = |s: &CatalogStar| {
            crate::sky::generate::system_for(s).populations.into_iter().find(|p| p.radiating_ratio == Population::PANEL)
        };
        let (with, swarm) = (200_000..)
            .find_map(|key| {
                let s = star(key, 0.01, DVec3::Z * 5.0);
                has_swarm(&s).map(|p| (s, p))
            })
            .unwrap();
        let without = (300_000..).map(|key| star(key, 0.01, DVec3::Z * 5.0)).find(|s| has_swarm(s).is_none()).unwrap();
        let prior = Prior::measure(&neighborhood());

        let (mut knowledge, now) = stare(&with, 20.0);
        let read = knowledge.read_log(Subject::Star(with.id), Witness(1), &prior, now).unwrap();
        let top = read.populations.first().expect("populations read");
        let Kind::Swarm(found) = top.kind else { panic!("{:?}", read.populations) };
        assert!(top.probability > SETTLED, "{:?}", read.populations);
        let truth = swarm.covering_fraction();
        assert!((found.coverage.0 / truth - 1.0).abs() < 0.2, "{:?} against {truth}", found.coverage);

        let (mut knowledge, now) = stare(&without, 20.0);
        let read = knowledge.read_log(Subject::Star(without.id), Witness(1), &prior, now).unwrap();
        let top = read.populations.first().expect("populations read");
        assert!(matches!(top.kind, Kind::Belts { .. }) && top.probability > SETTLED, "{:?}", read.populations);
    }
}
