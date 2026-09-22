//! What one craft tells another, and what a shard tells its own craft's client.
//!
//! A report carries what a craft has concluded — sightings, claims, names, orbits and
//! conclusions — and never its photometric logs, which are enormous next to what was learned
//! from them and cost energy to send in proportion. A craft's own client is sent its logs
//! separately, as [`Log`]s, because a client is a copy of the craft and not somebody it tells.
//! See `lightcone/docs/22-provenance.md` and `lightcone/docs/24-standing-instruments.md`.

use std::collections::{BTreeMap, BTreeSet};

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::{
    Claim, Conclusion, File, Hop, Knowledge, Lineage, Naming, Orbit, Sample, Sighting, Subject, Witness,
    learned_s,
};

/// How many systems one transmission carries.
///
/// A surveyed sky has thousands of entries, so a report is a slice of a backlog rather than a
/// snapshot. Sixty-four systems is a few tens of kilobytes.
pub const ENTRIES_PER_REPORT: usize = 64;

/// Everything one report carries about one subject.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Part {
    pub subject: Subject,
    pub sightings: Vec<Sighting>,
    /// Distances somebody states: see [`Claim`].
    pub claims: Vec<Claim>,
    /// What the sender and whoever told them call it: see [`Naming`].
    pub names: Vec<Naming>,
    pub orbits: Vec<Orbit>,
    /// What each observer's log was read to say, each on that observer's name.
    pub conclusions: Vec<Conclusion>,
}

impl Part {
    fn is_empty(&self) -> bool {
        self.conclusions.is_empty()
            && self.sightings.is_empty()
            && self.claims.is_empty()
            && self.names.is_empty()
            && self.orbits.is_empty()
    }

    fn learned_through(&self) -> f64 {
        let sightings = self.sightings.iter().map(Sighting::learned_s);
        let claims = self.claims.iter().map(|c| learned_s(&c.lineage, c.stated_s));
        let names = self.names.iter().map(|n| learned_s(&n.lineage, n.stated_s));
        let orbits = self.orbits.iter().map(|o| learned_s(&o.lineage, o.stated_s));
        let conclusions = self.conclusions.iter().map(Conclusion::learned_s);
        sightings.chain(claims).chain(names).chain(orbits).chain(conclusions).fold(f64::NEG_INFINITY, f64::max)
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

/// How far through its own backlog a craft has reported, per recipient.
///
/// Reports drain a backlog, so the sender has to remember how far it has got with each
/// recipient. The key is the recipient's ship id, `0` for a broadcast — what was shouted to
/// nobody in particular is its own backlog.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Reporting {
    told: BTreeMap<i64, Mark>,
}

impl Reporting {
    /// What a report to this recipient should resume from.
    pub fn since(&self, to: i64) -> Mark {
        self.told.get(&to).copied().unwrap_or_default()
    }

    /// A report to this recipient has gone out, carrying everything through `through`.
    pub fn sent(&mut self, to: i64, through: Mark) {
        let mark = self.told.entry(to).or_default();
        if through > *mark {
            *mark = through;
        }
    }

    /// Marks as a craft saved them before they carried a system, when a time meant everything
    /// through it.
    pub fn from_times(told: BTreeMap<i64, f64>) -> Self {
        Self { told: told.into_iter().map(|(to, at_s)| (to, Mark::through(at_s))).collect() }
    }
}

/// How far through a backlog something has got: everything learned before `at_s`, and at
/// exactly `at_s` everything about systems up to `after`.
///
/// A time alone is not enough. A sweep finds a hundred stars in one tick and charts issue a
/// volume at one instant, so a page that could only cut between times would have to carry all
/// of them or none.
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

/// What one craft sends another. A message like any other: emitted somewhere, arriving later.
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

/// A craft's own samples of one subject in one band, for its own client. Never sent to another
/// craft.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Log {
    pub subject: Subject,
    pub band: Band,
    pub samples: Vec<Sample>,
}

/// A page of a craft's own logs for its own client, with the subjects it keeps raw — a choice
/// of the craft's own, which no report carries either.
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

/// Subjects by the latest time something happened to them, so that "what is new since" reads
/// only what is new instead of every file a craft holds.
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

/// Every reportable thing a craft holds, by when it was learned and the system it belongs to —
/// the order a report drains its backlog in — so a page costs what it carries, not what the
/// craft knows.
///
/// Entries are added and never removed. One left behind by a record that was since replaced
/// names a time at which that subject now has nothing, and costs a lookup that finds nothing.
#[derive(Clone, Debug, Default)]
pub(super) struct Backlog {
    items: BTreeSet<(At, Subject, Subject)>,
}

impl Backlog {
    pub(super) fn file(&mut self, subject: Subject, file: &File) {
        for t in file.learned_times() {
            self.items.insert((At(t), subject.system(), subject));
        }
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
    /// Everything here whose learned time `keep` accepts, or `None` if nothing.
    pub(super) fn part(&self, subject: Subject, fresh: impl Fn(f64) -> bool) -> Option<Part> {
        let part = Part {
            subject,
            sightings: self.sightings.iter().filter(|s| fresh(s.learned_s())).cloned().collect(),
            claims: self.claims.iter().filter(|c| fresh(learned_s(&c.lineage, c.stated_s))).cloned().collect(),
            names: self.names.iter().filter(|n| fresh(learned_s(&n.lineage, n.stated_s))).cloned().collect(),
            orbits: self.orbits.iter().filter(|o| fresh(learned_s(&o.lineage, o.stated_s))).cloned().collect(),
            conclusions: self.conclusions.iter().filter(|c| fresh(c.learned_s())).cloned().collect(),
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
        sightings.chain(claims).chain(names).chain(orbits).chain(conclusions)
    }
}

impl Knowledge {
    /// Everything learned after `since`, ready to transmit.
    ///
    /// By what this craft *learned* rather than by when it was measured: a report is a statement
    /// about what the sender has, and a decade-old sighting relayed yesterday is news.
    pub fn report(&self, since: Mark, sent_s: f64) -> Report {
        self.report_upto(since, sent_s, usize::MAX).0
    }

    /// The same, as much of it as `limit` systems will carry, and the mark to resume from.
    ///
    /// A surveyed sky does not fit in one transmission, so a report is a piece of a backlog,
    /// oldest first. Everything a system learned at one instant goes together — a planet rides
    /// with its star — and nothing learned after `sent_s` goes at all. `None` for the mark when
    /// there is nothing to send.
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

    /// Fold in what somebody else sent, as of the moment its light landed.
    ///
    /// Every item gains a hop, so where it came from survives however far it is passed on, and
    /// something already held by a shorter route is not taken twice.
    pub fn receive(&mut self, report: &Report, received_s: f64) {
        let hop = Hop { from: report.from, to: self.owner, sent_s: report.sent_s, received_s };
        self.fold(report, Some(hop));
    }

    /// Take a copy of what this craft already knows, handed over by whoever holds the original.
    ///
    /// No hop: a client's replica of its own craft's knowledge is the same knowledge, not
    /// something it was told. See `lightcone/docs/24-standing-instruments.md`.
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
            for conclusion in &part.conclusions {
                // A replica follows its original in throwing a log away, so that it does not
                // hold samples the craft itself no longer has.
                if hop.is_none()
                    && let Some(through_s) = conclusion.discarded_s
                    && let Some(file) = self.files.get_mut(&subject)
                {
                    file.series
                        .iter_mut()
                        .filter(|s| s.witness == conclusion.observer)
                        .for_each(|s| s.consume_through(through_s));
                }
                let lineage = heard(&conclusion.lineage);
                self.concluded(subject, Conclusion { lineage, ..conclusion.clone() });
            }
            self.refresh(subject);
        }
    }

    /// This craft's own samples taken after `since_s`, oldest first, at most about `limit` of
    /// them, and the time the client should resume from. Every sample at the last time taken
    /// goes, however many bands and subjects share it, for the same reason a report's cut does.
    pub fn logs_upto(&self, since_s: f64, limit: usize) -> (Vec<Log>, Option<f64>) {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        // Each series is already in time order, so the oldest `limit` across them are a merge
        // of their heads, not a sort of everything since the mark.
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
            // Every sample at the last time taken goes, whatever the limit.
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

    /// Take a copy of this craft's own samples, handed over by whoever holds the original.
    /// Never charged for room or written down: the original does both.
    pub fn copy_logs(&mut self, logs: &[Log]) {
        let owner = self.owner;
        for log in logs {
            let file = self.files.entry(log.subject).or_default();
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
