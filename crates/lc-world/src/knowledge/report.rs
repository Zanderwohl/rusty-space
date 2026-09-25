//! What one craft tells another, and what a shard tells its own craft's client.
//!
//! A report never carries photometric logs, which are enormous next to what was learned from
//! them and cost energy to send. A craft's own client gets its logs separately, as [`Log`]s,
//! because it is a copy of the craft, not somebody it tells. See
//! `lightcone/docs/22-provenance.md` and `lightcone/docs/24-standing-instruments.md`.

use std::collections::{BTreeMap, BTreeSet};

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::{
    Claim, Colors, Conclusion, File, Hop, Knowledge, Lineage, Naming, Orbit, Sample, Sighting, Subject,
    Witness, learned_s,
};

/// How many systems one transmission carries: a few tens of kilobytes.
pub const ENTRIES_PER_REPORT: usize = 64;

/// Everything one report carries about one subject.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Part {
    pub subject: Subject,
    pub sightings: Vec<Sighting>,
    pub claims: Vec<Claim>,
    pub names: Vec<Naming>,
    pub orbits: Vec<Orbit>,
    /// Each on its observer's name.
    pub conclusions: Vec<Conclusion>,
    /// What each observer has folded of a body's colors. Fixed in size per witness however
    /// many visits went into it, which is what makes it cheap enough to send.
    pub colors: Vec<Colors>,
}

impl Part {
    fn is_empty(&self) -> bool {
        self.conclusions.is_empty()
            && self.sightings.is_empty()
            && self.claims.is_empty()
            && self.names.is_empty()
            && self.orbits.is_empty()
            && self.colors.is_empty()
    }

    fn learned_through(&self) -> f64 {
        let sightings = self.sightings.iter().map(Sighting::learned_s);
        let claims = self.claims.iter().map(|c| learned_s(&c.lineage, c.stated_s));
        let names = self.names.iter().map(|n| learned_s(&n.lineage, n.stated_s));
        let orbits = self.orbits.iter().map(|o| learned_s(&o.lineage, o.stated_s));
        let conclusions = self.conclusions.iter().map(Conclusion::learned_s);
        let colors = self.colors.iter().map(Colors::learned_s);
        sightings
            .chain(claims)
            .chain(names)
            .chain(orbits)
            .chain(conclusions)
            .chain(colors)
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// Everything one report carries about one system: the star and whatever belongs to it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    /// The star, or a craft, that the parts are grouped under.
    pub system: Subject,
    pub parts: Vec<Part>,
}

impl Entry {
    /// The most recent moment the sender learned any of this.
    pub fn learned_through(&self) -> f64 {
        self.parts.iter().map(Part::learned_through).fold(f64::NEG_INFINITY, f64::max)
    }
}

/// How far through its own backlog a craft has reported, keyed by recipient ship id, `0` for a
/// broadcast.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Reporting {
    told: BTreeMap<i64, Mark>,
}

impl Reporting {
    /// What a report to this recipient should resume from.
    pub fn since(&self, to: i64) -> Mark {
        self.told.get(&to).copied().unwrap_or_default()
    }

    /// A mark never moves back.
    pub fn sent(&mut self, to: i64, through: Mark) {
        let mark = self.told.entry(to).or_default();
        if through > *mark {
            *mark = through;
        }
    }

    /// Marks saved as bare times, each meaning everything through it.
    pub fn from_times(told: BTreeMap<i64, f64>) -> Self {
        Self { told: told.into_iter().map(|(to, at_s)| (to, Mark::through(at_s))).collect() }
    }
}

/// How far through a backlog something has got: everything learned before `at_s`, and at
/// exactly `at_s` everything about systems up to `after`.
///
/// A time alone is not enough: a sweep finds a hundred stars in one tick, and a page that could
/// only cut between times would have to carry all of them or none.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub at_s: f64,
    pub after: Option<Subject>,
}

impl Default for Mark {
    fn default() -> Self {
        Self { at_s: f64::NEG_INFINITY, after: None }
    }
}

impl Mark {
    /// Everything learned at or before `at_s`.
    pub fn through(at_s: f64) -> Self {
        Self { at_s, after: Some(Subject::Craft(i64::MAX)) }
    }

    fn key(&self) -> (At, Option<Subject>) {
        (At(self.at_s), self.after)
    }
}

impl PartialOrd for Mark {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.key().cmp(&other.key()))
    }
}

/// What one craft sends another.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Report {
    pub from: Witness,
    pub sent_s: f64,
    pub entries: Vec<Entry>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// What the sender should resume from next time. `None` for a report of nothing.
    pub fn learned_through(&self) -> Option<f64> {
        self.entries
            .iter()
            .map(Entry::learned_through)
            .fold(None, |best: Option<f64>, t| Some(best.map_or(t, |b| b.max(t))))
    }

    /// Systems it carries, which is what a transcript line counts.
    pub fn stars(&self) -> usize {
        self.entries.len()
    }
}

/// A craft's own samples of one subject in one band, for its own client only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Log {
    pub subject: Subject,
    pub band: Band,
    pub samples: Vec<Sample>,
}

/// A page of a craft's own logs for its own client, with the subjects it keeps raw, which no
/// report carries either.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Logs {
    pub logs: Vec<Log>,
    pub retained: Vec<Subject>,
}

/// A time that orders totally, for an index.
#[derive(Clone, Copy, Debug, PartialEq)]
struct At(f64);

impl Eq for At {}

impl PartialOrd for At {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for At {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Subjects by the latest time something happened to them, so "what is new since" does not
/// read every file.
#[derive(Clone, Debug, Default)]
pub(super) struct Recent {
    at: BTreeMap<Subject, f64>,
    order: BTreeSet<(At, Subject)>,
}

impl Recent {
    pub(super) fn touch(&mut self, subject: Subject, at_s: f64) {
        match self.at.get(&subject) {
            Some(&held) if held >= at_s => return,
            Some(&held) => {
                self.order.remove(&(At(held), subject));
            }
            None => {}
        }
        self.at.insert(subject, at_s);
        self.order.insert((At(at_s), subject));
    }

    /// Subjects touched after `since_s`, newest first.
    pub(super) fn after(&self, since_s: f64) -> impl Iterator<Item = Subject> + '_ {
        self.order.iter().rev().take_while(move |(at, _)| at.0 > since_s).map(|(_, s)| *s)
    }
}

/// Every reportable thing a craft holds, in the order a report drains it, so a page costs what
/// it carries, not what the craft knows.
///
/// Exactly the learned times the files hold now. It kept every time it had ever been handed, and
/// a survey replaces records every visit, so it grew for as long as anyone surveyed and a
/// report from the start walked all of it.
#[derive(Clone, Debug, Default)]
pub(super) struct Backlog {
    items: BTreeSet<(At, Subject, Subject)>,
    filed: BTreeMap<Subject, BTreeSet<At>>,
}

impl Backlog {
    pub(super) fn file(&mut self, subject: Subject, file: &File) {
        let now: BTreeSet<At> = file.learned_times().map(At).collect();
        let system = subject.system();
        let was = self.filed.entry(subject).or_default();
        for gone in was.difference(&now) {
            self.items.remove(&(*gone, system, subject));
        }
        for new in now.difference(was) {
            self.items.insert((*new, system, subject));
        }
        *was = now;
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Entries after `since` and not after `until_s`, oldest first.
    fn after(&self, since: Mark, until_s: f64) -> impl Iterator<Item = (f64, Subject, Subject)> + '_ {
        let least = Subject::Star(crate::sky::StarId::from_raw(0));
        let from = (At(since.at_s), since.after.unwrap_or(least), least);
        self.items
            .range(from..)
            .filter(move |(at, system, _)| (*at, Some(*system)) > since.key())
            .take_while(move |(at, _, _)| at.0 <= until_s)
            .map(|(at, system, subject)| (at.0, *system, *subject))
    }
}

impl File {
    /// Everything here whose learned time `fresh` accepts, or `None` if nothing.
    pub(super) fn part(&self, subject: Subject, fresh: impl Fn(f64) -> bool) -> Option<Part> {
        let part = Part {
            subject,
            sightings: self.sightings.iter().filter(|s| fresh(s.learned_s())).cloned().collect(),
            claims: self.claims.iter().filter(|c| fresh(learned_s(&c.lineage, c.stated_s))).cloned().collect(),
            names: self.names.iter().filter(|n| fresh(learned_s(&n.lineage, n.stated_s))).cloned().collect(),
            orbits: self.orbits.iter().filter(|o| fresh(learned_s(&o.lineage, o.stated_s))).cloned().collect(),
            conclusions: self.conclusions.iter().filter(|c| fresh(c.learned_s())).cloned().collect(),
            colors: self.colors.iter().filter(|c| fresh(c.learned_s())).cloned().collect(),
        };
        (!part.is_empty()).then_some(part)
    }

    /// When each reportable thing here was learned.
    fn learned_times(&self) -> impl Iterator<Item = f64> + '_ {
        let sightings = self.sightings.iter().map(Sighting::learned_s);
        let claims = self.claims.iter().map(|c| learned_s(&c.lineage, c.stated_s));
        let names = self.names.iter().map(|n| learned_s(&n.lineage, n.stated_s));
        let orbits = self.orbits.iter().map(|o| learned_s(&o.lineage, o.stated_s));
        let conclusions = self.conclusions.iter().map(Conclusion::learned_s);
        let colors = self.colors.iter().map(Colors::learned_s);
        sightings.chain(claims).chain(names).chain(orbits).chain(conclusions).chain(colors)
    }
}

impl Knowledge {
    /// Everything learned after `since`, by when it was learned rather than measured: a
    /// decade-old sighting relayed yesterday is news.
    pub fn report(&self, since: Mark, sent_s: f64) -> Report {
        self.report_upto(since, sent_s, usize::MAX).0
    }

    /// The same, oldest first, as much as `limit` systems will carry, and the mark to resume
    /// from (`None` when there is nothing to send). A planet rides with its star, and nothing
    /// learned after `sent_s` goes.
    pub fn report_upto(&self, since: Mark, sent_s: f64, limit: usize) -> (Report, Option<Mark>) {
        let mut systems: BTreeSet<Subject> = BTreeSet::new();
        let mut subjects: BTreeSet<Subject> = BTreeSet::new();
        let mut cut: Option<(At, Subject)> = None;
        for (t, system, subject) in self.backlog.after(since, sent_s) {
            if !systems.contains(&system) && systems.len() >= limit {
                break;
            }
            systems.insert(system);
            subjects.insert(subject);
            cut = Some((At(t), system));
        }
        let empty = Report { from: self.owner, sent_s, entries: Vec::new() };
        let Some(cut) = cut else { return (empty, None) };
        let within = |t: f64, system: Subject| {
            let key = (At(t), Some(system));
            key > since.key() && key <= (cut.0, Some(cut.1))
        };
        let mut parts: BTreeMap<Subject, Vec<Part>> = BTreeMap::new();
        for subject in subjects {
            let system = subject.system();
            if let Some(part) = self.files.get(&subject).and_then(|f| f.part(subject, |t| within(t, system))) {
                parts.entry(system).or_default().push(part);
            }
        }
        let mut entries: Vec<Entry> = parts.into_iter().map(|(system, parts)| Entry { system, parts }).collect();
        entries.sort_by(|a, b| a.learned_through().total_cmp(&b.learned_through()));
        let report = Report { entries, ..empty };
        (report, Some(Mark { at_s: cut.0 .0, after: Some(cut.1) }))
    }

    /// Fold in what somebody else sent, as of the moment its light landed. Every item gains a
    /// hop.
    pub fn receive(&mut self, report: &Report, received_s: f64) {
        let hop = Hop { from: report.from, to: self.owner, sent_s: report.sent_s, received_s };
        self.fold(report, Some(hop));
    }

    /// Take a copy of what this craft already knows from whoever holds the original. No hop:
    /// a replica is the same knowledge, not something it was told.
    pub fn absorb(&mut self, report: &Report) {
        self.fold(report, None);
    }

    fn fold(&mut self, report: &Report, hop: Option<Hop>) {
        let heard = |lineage: &Lineage| {
            let mut lineage = lineage.clone();
            lineage.extend(hop);
            lineage
        };
        for part in report.entries.iter().flat_map(|e| e.parts.iter()) {
            let subject = part.subject;
            for sighting in &part.sightings {
                let lineage = heard(&sighting.lineage);
                self.file_sighting(subject, Sighting { lineage, ..sighting.clone() });
            }
            for naming in &part.names {
                let lineage = heard(&naming.lineage);
                self.named(subject, Naming { lineage, ..naming.clone() });
            }
            for claim in &part.claims {
                let lineage = heard(&claim.lineage);
                self.told(subject, Claim { lineage, ..claim.clone() });
            }
            for orbit in &part.orbits {
                let lineage = heard(&orbit.lineage);
                self.orbits(subject, Orbit { lineage, ..orbit.clone() });
            }
            for digest in &part.colors {
                let lineage = heard(&digest.lineage);
                self.absorb_colors(subject, Colors { lineage, ..digest.clone() });
            }
            for conclusion in &part.conclusions {
                // A replica drops the samples its original consumed.
                if hop.is_none()
                    && let Some(through_s) = conclusion.discarded_s
                    && let Some(file) = self.files.edit().get_mut(&subject)
                {
                    file.series
                        .iter_mut()
                        .filter(|s| s.witness == conclusion.observer)
                        .for_each(|s| s.consume_through(through_s));
                    self.analyzing.remove(&subject);
                }
                let lineage = heard(&conclusion.lineage);
                self.concluded(subject, Conclusion { lineage, ..conclusion.clone() });
            }
            self.refresh(subject);
        }
    }

    /// This craft's own samples taken after `since_s`, oldest first, about `limit` of them, and
    /// the time to resume from. Every sample at the last time taken goes, so a tie is never split.
    pub fn logs_upto(&self, since_s: f64, limit: usize) -> (Vec<Log>, Option<f64>) {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        // Each series is in time order, so this merges their heads rather than sorting everything.
        let mut heads: BinaryHeap<Reverse<(At, Subject, Band, usize)>> = BinaryHeap::new();
        let mut runs: BTreeMap<(Subject, Band), &[Sample]> = BTreeMap::new();
        for subject in self.sampled.after(since_s) {
            let Some(file) = self.files.get(&subject) else { continue };
            for series in file.series.iter().filter(|s| s.witness == self.owner) {
                let samples = series.samples();
                let from = samples.partition_point(|s| s.observed_s <= since_s);
                if let Some(first) = samples.get(from) {
                    heads.push(Reverse((At(first.observed_s), subject, series.band, from)));
                    runs.insert((subject, series.band), samples);
                }
            }
        }
        let mut logs: BTreeMap<(Subject, Band), Vec<Sample>> = BTreeMap::new();
        let mut taken = 0;
        let mut through: Option<f64> = None;
        while let Some(Reverse((at, subject, band, i))) = heads.pop() {
            if taken >= limit && through != Some(at.0) {
                break;
            }
            let Some(run) = runs.get(&(subject, band)) else { continue };
            let Some(sample) = run.get(i) else { continue };
            logs.entry((subject, band)).or_default().push(*sample);
            taken += 1;
            through = Some(at.0);
            if let Some(next) = run.get(i + 1) {
                heads.push(Reverse((At(next.observed_s), subject, band, i + 1)));
            }
        }
        (logs.into_iter().map(|((subject, band), samples)| Log { subject, band, samples }).collect(), through)
    }

    /// Take a copy of this craft's own samples. Never charged for room or written down: the
    /// original does both.
    pub fn copy_logs(&mut self, logs: &[Log]) {
        let owner = self.owner;
        for log in logs {
            let file = self.files.edit().entry(log.subject).or_default();
            if !file.series.iter().any(|s| s.witness == owner && s.band == log.band) {
                file.series.push(super::Series::new(owner, log.band));
            }
            let Some(series) = file.series.iter_mut().find(|s| s.witness == owner && s.band == log.band) else {
                continue;
            };
            for sample in &log.samples {
                series.push(*sample);
            }
        }
    }
}
