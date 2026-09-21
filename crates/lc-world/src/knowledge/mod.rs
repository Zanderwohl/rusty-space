//! What one craft has actually seen, and who told it the rest.
//!
//! There is no current state of a distant system, only measurements: each was taken somewhere,
//! at some time, by somebody, and reached here by some route. A belief is a fold over the
//! measurements held, and two craft folding different sets are both right.
//!
//! Nothing here knows the truth. Everything in a [`Knowledge`] came in through
//! [`survey`](crate::knowledge::survey) or through a [`Report`] from another witness, and a
//! star nobody has seen is simply absent. See `lightcone/docs/22-provenance.md`.

use std::collections::BTreeMap;

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use crate::sky::StarId;

pub mod astrometry;
pub mod names;
pub mod observatory;
pub mod record;
pub mod subject;
pub mod survey;

pub use astrometry::{Bearing, Distance};
pub use names::designation;
pub use record::{
    Claim, Hop, Lineage, NameKind, Naming, Orbit, SAMPLES_KEPT, Sample, Series, Sighting, Witness,
    learnt_s,
};
pub use subject::{BodyId, Subject};

/// How many bearings per subject per witness are kept.
///
/// Distance comes from the spread of the observing positions, so the reservoir keeps the
/// widest spread rather than the most recent: dropping the closest pair costs the least
/// baseline. Sixteen well-spread bearings measure a parallax as well as a thousand.
pub const BEARINGS_KEPT: usize = 16;

fn sigma_of(claim: &Claim) -> f64 {
    match claim.distance {
        Distance::Measured { sigma_ly, .. } => sigma_ly,
        _ => f64::INFINITY,
    }
}

/// Which of two namings a craft goes by: a chosen name over an assigned one, its own over
/// somebody else's, and the more recent over the older.
fn better_name(held: Option<&Naming>, new: &Naming, owner: Witness) -> bool {
    let rank = |n: &Naming| (n.kind.chosen(), n.witness == owner, n.stated_s);
    match held {
        None => true,
        Some(held) => rank(new) > rank(held),
    }
}

/// What is believed about something seen in the sky, folded from everything held about it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Belief {
    pub subject: Subject,
    /// The naming this craft goes by, and who said it. `None` for something detected and not
    /// yet written down anywhere. A relative naming is only a suffix: what to *show* is
    /// [`Knowledge::name_of`], which reads it after the star's own name.
    pub name: Option<Naming>,
    /// The most recent bearing, from wherever that witness was.
    pub bearing: Bearing,
    pub distance: Distance,
    /// Whether the distance was worked out from bearings this craft holds — its own, or
    /// somebody's raw measurements relayed to it — rather than taken from a stated number.
    ///
    /// Raw bearings from a probe are still a measurement: they can be re-solved, combined with
    /// this craft's own, and checked. A [`Claim`] cannot be, which is the distinction. Whose
    /// bearings they were is [`Belief::witnesses`] and [`Belief::hops`].
    pub triangulated: bool,
    pub band: Band,
    pub flux: f64,
    /// Coordinate seconds the most recent light held arrived — at its witness, not here.
    pub observed_s: f64,
    /// Coordinate seconds this craft learnt of it.
    pub learnt_s: f64,
    pub sightings: usize,
    pub witnesses: usize,
    /// Hops on the shortest route any of it took here. Zero for something seen directly.
    pub hops: usize,
}

impl Belief {
    /// The star this is a belief about, if it is one.
    pub fn star(&self) -> Option<StarId> {
        self.subject.as_star()
    }

    /// Seconds the light had been travelling, once there is a distance to say so.
    pub fn light_age_s(&self) -> Option<f64> {
        self.distance
            .from(self.bearing.observer_ly)
            .map(|ly| ly * crate::flight::JULIAN_YEAR_S)
    }

    /// Coordinate seconds the light held was emitted, if the distance is known.
    pub fn emitted_s(&self) -> Option<f64> {
        self.light_age_s().map(|age| self.observed_s - age)
    }

    /// Band luminosity implied by the flux and the distance, watts.
    pub fn luminosity_w(&self) -> Option<f64> {
        let d = self.distance.from(self.bearing.observer_ly)? * crate::system::M_PER_LY;
        Some(4.0 * std::f64::consts::PI * d * d * self.flux)
    }
}

/// Everything held about one subject.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct File {
    sightings: Vec<Sighting>,
    series: Vec<Series>,
    claims: Vec<Claim>,
    names: Vec<Naming>,
    orbits: Vec<Orbit>,
}

impl File {
    pub fn sightings(&self) -> &[Sighting] {
        &self.sightings
    }

    pub fn series(&self) -> &[Series] {
        &self.series
    }

    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    pub fn names(&self) -> &[Naming] {
        &self.names
    }

    pub fn orbits(&self) -> &[Orbit] {
        &self.orbits
    }

    pub fn series_in(&self, band: Band) -> Option<&Series> {
        self.series.iter().find(|s| s.band == band)
    }

    /// The file as it is written down: everything but the samples, which are logged apart.
    pub fn header(&self) -> File {
        File { series: self.series.iter().map(Series::emptied).collect(), ..self.clone() }
    }

    fn naming(&self, owner: Witness) -> Option<&Naming> {
        let mut name: Option<&Naming> = None;
        for naming in &self.names {
            if better_name(name, naming, owner) {
                name = Some(naming);
            }
        }
        name
    }

    /// The orbit this craft goes by: its own statement if it has made one, otherwise the most
    /// recent anybody made.
    fn orbit(&self, owner: Witness) -> Option<&Orbit> {
        self.orbits.iter().max_by(|a, b| {
            (a.witness == owner, a.stated_s).partial_cmp(&(b.witness == owner, b.stated_s)).unwrap()
        })
    }

    fn believe(&self, subject: Subject, owner: Witness) -> Option<Belief> {
        let latest = self
            .sightings
            .iter()
            .max_by(|a, b| a.observed_s.total_cmp(&b.observed_s))?;
        let bearings: Vec<Bearing> = self.sightings.iter().map(|s| s.bearing).collect();
        let mut witnesses: Vec<Witness> = self.sightings.iter().map(|s| s.witness).collect();
        witnesses.sort_unstable();
        witnesses.dedup();
        // Measured here beats stated by somebody else, whatever error bars either carries:
        // one is a measurement this craft can check and the other is a thing it was told.
        let measured = astrometry::triangulate(&bearings);
        let taken = matches!(measured, Distance::Measured { .. });
        let claimed = self.claims.iter().min_by(|a, b| sigma_of(a).total_cmp(&sigma_of(b)));
        Some(Belief {
            subject,
            name: self.naming(owner).cloned(),
            bearing: latest.bearing,
            distance: match (taken, claimed) {
                (false, Some(claim)) => claim.distance,
                _ => measured,
            },
            triangulated: taken,
            band: latest.band,
            flux: latest.flux,
            observed_s: latest.observed_s,
            learnt_s: self.sightings.iter().map(Sighting::learnt_s).fold(f64::INFINITY, f64::min),
            sightings: self.sightings.len(),
            witnesses: witnesses.len(),
            hops: self.sightings.iter().map(|s| s.lineage.len()).min().unwrap_or(0),
        })
    }

    /// Keep the bearings that are farthest apart, which is what a parallax is made of.
    fn decimate(&mut self, witness: Witness) {
        while self.sightings.iter().filter(|s| s.witness == witness).count() > BEARINGS_KEPT {
            let held: Vec<usize> =
                (0..self.sightings.len()).filter(|i| self.sightings[*i].witness == witness).collect();
            // Never the newest: it is what the display reads, and a curve of one stale
            // bearing is worse than a slightly narrower baseline.
            let newest = *held
                .iter()
                .max_by(|a, b| self.sightings[**a].observed_s.total_cmp(&self.sightings[**b].observed_s))
                .unwrap();
            let mut drop = (f64::INFINITY, held[0]);
            for (n, i) in held.iter().enumerate() {
                for j in held.iter().skip(n + 1) {
                    let gap =
                        self.sightings[*i].bearing.observer_ly.distance(self.sightings[*j].bearing.observer_ly);
                    let older =
                        if self.sightings[*i].observed_s < self.sightings[*j].observed_s { *i } else { *j };
                    let loser = if older == newest { if *i == newest { *j } else { *i } } else { older };
                    if gap < drop.0 {
                        drop = (gap, loser);
                    }
                }
            }
            self.sightings.remove(drop.1);
        }
    }

    /// Everything here learnt after `since_s`, or `None` if nothing was.
    fn since(&self, subject: Subject, since_s: f64) -> Option<Part> {
        let fresh = |lineage: &Lineage, at: f64| learnt_s(lineage, at) > since_s;
        let part = Part {
            subject,
            sightings: self.sightings.iter().filter(|s| s.learnt_s() > since_s).cloned().collect(),
            series: self.series.iter().filter_map(|s| s.after(since_s)).collect(),
            claims: self.claims.iter().filter(|c| fresh(&c.lineage, c.stated_s)).cloned().collect(),
            names: self.names.iter().filter(|n| fresh(&n.lineage, n.stated_s)).cloned().collect(),
            orbits: self.orbits.iter().filter(|o| fresh(&o.lineage, o.stated_s)).cloned().collect(),
        };
        (!part.is_empty()).then_some(part)
    }
}

/// One photometric sample, as a log row: what it is of, whose it is, and when this craft learnt
/// it — which for a relayed series is when it arrived, not when it was measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Logged {
    pub subject: Subject,
    pub witness: Witness,
    pub band: Band,
    pub sample: Sample,
    pub learnt_s: f64,
}

/// One craft's view of the sky.
#[derive(Clone, Debug)]
pub struct Knowledge {
    pub owner: Witness,
    files: BTreeMap<Subject, File>,
    beliefs: BTreeMap<Subject, Belief>,
    /// Files changed since they were last written down, and samples taken since then. Not part
    /// of what a craft knows — two copies that differ only in what has been saved are the same
    /// knowledge — so equality ignores them.
    changed: std::collections::BTreeSet<Subject>,
    unsaved: Vec<Logged>,
}

impl PartialEq for Knowledge {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.files == other.files && self.beliefs == other.beliefs
    }
}

impl Knowledge {
    pub fn new(owner: Witness) -> Self {
        Self {
            owner,
            files: BTreeMap::new(),
            beliefs: BTreeMap::new(),
            changed: Default::default(),
            unsaved: Vec::new(),
        }
    }

    /// What has changed since the last call: every file touched, with its samples taken out,
    /// and every sample added, as log rows. Clears both.
    ///
    /// Files and samples are stored apart because they grow apart. A file is bounded and is
    /// rewritten whole when it changes; a watched star's samples are appended every integration
    /// and would make rewriting its file the most expensive thing a shard did.
    pub fn take_changes(&mut self) -> (Vec<(Subject, File)>, Vec<Logged>) {
        let files = std::mem::take(&mut self.changed)
            .into_iter()
            .filter_map(|s| Some((s, self.files.get(&s)?.header())))
            .collect();
        (files, std::mem::take(&mut self.unsaved))
    }

    /// Every file as it would be written, for a first save.
    pub fn files(&self) -> impl Iterator<Item = (Subject, File)> + '_ {
        self.files.iter().map(|(s, f)| (*s, f.header()))
    }

    /// Rebuild a craft's knowledge from what was written down: its files, then its samples in the
    /// order they were taken. Nothing restored counts as changed.
    pub fn restore(
        owner: Witness,
        files: impl IntoIterator<Item = (Subject, File)>,
        logs: impl IntoIterator<Item = Logged>,
    ) -> Self {
        let mut knowledge = Self::new(owner);
        knowledge.files = files.into_iter().collect();
        for log in logs {
            let file = knowledge.files.entry(log.subject).or_default();
            match file.series.iter_mut().find(|s| s.witness == log.witness && s.band == log.band) {
                Some(series) => {
                    series.push(log.sample);
                }
                None => {
                    let mut series = Series::new(log.witness, log.band);
                    series.push(log.sample);
                    file.series.push(series);
                }
            }
        }
        let subjects: Vec<Subject> = knowledge.files.keys().copied().collect();
        for subject in subjects {
            knowledge.refresh(subject);
        }
        knowledge.changed.clear();
        knowledge
    }

    /// Take a new identity, carrying this craft's own records over to it.
    ///
    /// A ship does not know what it is called until a shard tells it, and everything it
    /// measured before then is stamped with the placeholder. Left alone, two craft would both
    /// be witness zero and their records would collide the first time either reported to the
    /// other — one ship's bearings filed as the other's own, and a dedup that threw away real
    /// measurements because they shared a witness and an instant.
    pub fn rebrand(&mut self, owner: Witness) {
        let was = self.owner;
        if was == owner {
            return;
        }
        self.owner = owner;
        let swap = |w: &mut Witness| {
            if *w == was {
                *w = owner;
            }
        };
        for file in self.files.values_mut() {
            file.sightings.iter_mut().for_each(|s| swap(&mut s.witness));
            file.series.iter_mut().for_each(|s| swap(&mut s.witness));
            file.names.iter_mut().for_each(|n| swap(&mut n.witness));
            file.claims.iter_mut().for_each(|c| swap(&mut c.witness));
            file.orbits.iter_mut().for_each(|o| swap(&mut o.witness));
            // Hops name the ends of a handover, so a craft that has renamed itself must not
            // keep telling people its old name.
            for hop in file
                .sightings
                .iter_mut()
                .flat_map(|s| s.lineage.iter_mut())
                .chain(file.series.iter_mut().flat_map(|s| s.lineage.iter_mut()))
                .chain(file.names.iter_mut().flat_map(|n| n.lineage.iter_mut()))
                .chain(file.claims.iter_mut().flat_map(|c| c.lineage.iter_mut()))
                .chain(file.orbits.iter_mut().flat_map(|o| o.lineage.iter_mut()))
            {
                swap(&mut hop.from);
                swap(&mut hop.to);
            }
        }
        let subjects: Vec<Subject> = self.files.keys().copied().collect();
        for subject in subjects {
            self.refresh(subject);
        }
    }

    /// Subjects held: stars, and whatever else has been learnt about.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn knows(&self, subject: impl Into<Subject>) -> bool {
        self.files.contains_key(&subject.into())
    }

    pub fn file(&self, subject: impl Into<Subject>) -> Option<&File> {
        self.files.get(&subject.into())
    }

    pub fn belief(&self, subject: impl Into<Subject>) -> Option<&Belief> {
        self.beliefs.get(&subject.into())
    }

    pub fn beliefs(&self) -> impl Iterator<Item = &Belief> {
        self.beliefs.values()
    }

    /// Beliefs about stars, which is what a map of the sky is drawn from.
    pub fn stars(&self) -> impl Iterator<Item = (StarId, &Belief)> {
        self.beliefs.values().filter_map(|b| Some((b.star()?, b)))
    }

    /// Everything held about a star's system: its planets, belts and the rest.
    pub fn members(&self, star: StarId) -> impl Iterator<Item = (Subject, &File)> {
        self.files
            .iter()
            .filter(move |(s, _)| s.star() == Some(star) && s.as_star().is_none())
            .map(|(s, f)| (*s, f))
    }

    /// What this craft calls something, as a player should see it.
    ///
    /// A relative naming is read after the star's name — **this** craft's name for the star,
    /// whoever assigned the suffix — so a planet a probe called "Kettle b" is shown as
    /// "Hearthlight b" aboard a ship that calls the star Hearthlight, and follows the star when
    /// it is renamed.
    pub fn name_of(&self, subject: impl Into<Subject>) -> Option<String> {
        let subject = subject.into();
        let naming = self.files.get(&subject)?.naming(self.owner)?;
        match naming.kind {
            NameKind::Relative => {
                let star = self.name_of(subject.star()?).unwrap_or_else(|| "?".into());
                Some(format!("{star} {}", naming.name))
            }
            _ => Some(naming.name.clone()),
        }
    }

    /// File a sighting this craft made itself.
    pub fn sighted(&mut self, subject: impl Into<Subject>, sighting: Sighting) {
        self.file_sighting(subject.into(), sighting);
    }

    /// File a distance somebody states. Nothing is taken on trust that a measurement of its
    /// own would not override.
    pub fn told(&mut self, subject: impl Into<Subject>, claim: Claim) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file.claims.iter_mut().find(|c| c.witness == claim.witness) {
            Some(held) if held.stated_s >= claim.stated_s => {}
            Some(held) => *held = claim,
            None => file.claims.push(claim),
        }
        self.refresh(subject);
    }

    /// File where somebody says a body orbits. One statement per witness, the later winning.
    pub fn orbits(&mut self, subject: impl Into<Subject>, orbit: Orbit) {
        let subject = subject.into();
        self.changed.insert(subject);
        let file = self.files.entry(subject).or_default();
        match file.orbits.iter_mut().find(|o| o.witness == orbit.witness) {
            Some(held) if held.stated_s >= orbit.stated_s => {}
            Some(held) => *held = orbit,
            None => file.orbits.push(orbit),
        }
    }

    /// File what somebody calls something. One name per witness: renaming is stating a new
    /// one, and the later statement is what that witness calls it now.
    pub fn named(&mut self, subject: impl Into<Subject>, naming: Naming) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file.names.iter_mut().find(|n| n.witness == naming.witness) {
            Some(held) if held.stated_s > naming.stated_s => {}
            // A rule's assignment is frozen; only a chosen name replaces it.
            Some(held) if !held.kind.chosen() && !naming.kind.chosen() && held.witness == naming.witness => {}
            Some(held) => *held = naming,
            None => file.names.push(naming),
        }
        self.refresh(subject);
    }

    /// Give something this craft's own name for it.
    pub fn name_it(&mut self, subject: impl Into<Subject>, name: impl Into<String>, now_s: f64) {
        let naming = Naming {
            witness: self.owner,
            name: name.into(),
            kind: NameKind::Given,
            stated_s: now_s,
            lineage: Lineage::new(),
        };
        self.named(subject, naming);
    }

    /// Record a planet this craft has found, and letter it.
    ///
    /// The orbit is filed as this craft's own statement and the letter as a relative naming,
    /// placed against the letters this craft already goes by for the same star — see
    /// [`names::planet_letter`]. A planet already lettered keeps its letter: nothing a rule
    /// assigned is ever reassigned. Returns the letter.
    pub fn found_planet(
        &mut self,
        star: StarId,
        body: BodyId,
        semi_major_au: f64,
        luminosity_solar: f64,
        now_s: f64,
    ) -> String {
        let subject = Subject::Body { star, body };
        let owner = self.owner;
        self.orbits(
            subject,
            Orbit { witness: owner, semi_major_au, stated_s: now_s, lineage: Lineage::new() },
        );
        if let Some(held) = self.files[&subject]
            .names
            .iter()
            .find(|n| n.witness == owner && n.kind == NameKind::Relative)
        {
            return held.name.clone();
        }
        let placed: Vec<(String, f64)> = self
            .members(star)
            .filter(|(s, _)| *s != subject && matches!(s, Subject::Body { .. }))
            .filter_map(|(_, file)| {
                let naming = file.naming(owner).filter(|n| n.kind == NameKind::Relative)?;
                Some((naming.name.clone(), file.orbit(owner)?.semi_major_au))
            })
            .collect();
        let letter = names::planet_letter(&placed, semi_major_au, luminosity_solar, names::SPACING);
        self.named(
            subject,
            Naming {
                witness: owner,
                name: letter.clone(),
                kind: NameKind::Relative,
                stated_s: now_s,
                lineage: Lineage::new(),
            },
        );
        letter
    }

    /// File a photometric sample this craft measured itself.
    pub fn measured(&mut self, subject: impl Into<Subject>, witness: Witness, band: Band, sample: Sample) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        let added = match file.series.iter_mut().find(|s| s.witness == witness && s.band == band) {
            Some(series) => series.push(sample),
            None => {
                let mut series = Series::new(witness, band);
                series.push(sample);
                file.series.push(series);
                // A new series is a new header in the file, as well as a sample in the log.
                self.changed.insert(subject);
                true
            }
        };
        if added {
            let learnt_s = sample.observed_s;
            self.unsaved.push(Logged { subject, witness, band, sample, learnt_s });
        }
    }

    /// Photometry this craft took itself, for one subject and band.
    pub fn own_series(&self, subject: impl Into<Subject>, band: Band) -> Option<&Series> {
        let file = self.files.get(&subject.into())?;
        file.series.iter().find(|s| s.witness == self.owner && s.band == band)
    }

    /// Everything learnt after `since_s`, ready to transmit.
    ///
    /// By what this craft *learnt* rather than by when it was measured: a report is a statement
    /// about what the sender has, and a decade-old sighting relayed yesterday is news.
    pub fn report(&self, since_s: f64, sent_s: f64) -> Report {
        self.report_upto(since_s, sent_s, usize::MAX)
    }

    /// The same, as much of it as `limit` systems will carry.
    ///
    /// A surveyed sky does not fit in one transmission, so a report is a piece of a backlog:
    /// oldest first, and the sender resumes from [`Report::learnt_through`] next time. A system
    /// is never split — its planets ride with its star — and ties at the cut all go in the same
    /// report, because the sender has only one number to resume from and anything on the wrong
    /// side of it would never be sent at all.
    pub fn report_upto(&self, since_s: f64, sent_s: f64, limit: usize) -> Report {
        let mut systems: BTreeMap<Subject, Vec<Part>> = BTreeMap::new();
        for (subject, file) in &self.files {
            if let Some(part) = file.since(*subject, since_s) {
                systems.entry(subject.system()).or_default().push(part);
            }
        }
        let mut entries: Vec<Entry> =
            systems.into_iter().map(|(system, parts)| Entry { system, parts }).collect();
        entries.sort_by(|a, b| a.learnt_through().total_cmp(&b.learnt_through()));
        if entries.len() > limit {
            let cut = entries[limit.saturating_sub(1)].learnt_through();
            entries.retain(|e| e.learnt_through() <= cut);
        }
        Report { from: self.owner, sent_s, entries }
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
            for series in &part.series {
                let file = self.files.entry(subject).or_default();
                let (taken, lineage) =
                    match file.series.iter_mut().find(|s| s.witness == series.witness && s.band == series.band) {
                        Some(held) => (held.absorb(series), held.lineage.clone()),
                        None => {
                            let mut taken = series.clone();
                            taken.lineage.extend(hop);
                            let lineage = taken.lineage.clone();
                            file.series.push(taken);
                            (series.samples().to_vec(), lineage)
                        }
                    };
                for sample in taken {
                    let learnt_s = learnt_s(&lineage, sample.observed_s);
                    self.unsaved.push(Logged { subject, witness: series.witness, band: series.band, sample, learnt_s });
                }
            }
            self.refresh(subject);
        }
    }

    fn file_sighting(&mut self, subject: Subject, sighting: Sighting) {
        let witness = sighting.witness;
        let owner = self.owner;
        let file = self.files.entry(subject).or_default();
        if file.sightings.iter().any(|s| s.same_as(&sighting)) {
            return;
        }
        // Finding something is writing it down. A craft's own first detection gets a
        // designation from the direction it was found in, so the log has something to call it
        // until somebody names it properly.
        if witness == owner && file.names.is_empty() {
            file.names.push(Naming {
                witness: owner,
                name: designation(sighting.bearing.toward),
                kind: NameKind::Designation,
                stated_s: sighting.observed_s,
                lineage: Lineage::new(),
            });
        }
        file.sightings.push(sighting);
        file.decimate(witness);
        self.refresh(subject);
    }

    fn refresh(&mut self, subject: Subject) {
        self.changed.insert(subject);
        if let Some(belief) = self.files.get(&subject).and_then(|f| f.believe(subject, self.owner)) {
            self.beliefs.insert(subject, belief);
        }
    }
}

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
    pub series: Vec<Series>,
    /// Conclusions rather than measurements: see [`Claim`].
    pub claims: Vec<Claim>,
    /// What the sender and whoever told them call it: see [`Naming`].
    pub names: Vec<Naming>,
    pub orbits: Vec<Orbit>,
}

impl Part {
    fn is_empty(&self) -> bool {
        self.sightings.is_empty()
            && self.series.is_empty()
            && self.claims.is_empty()
            && self.names.is_empty()
            && self.orbits.is_empty()
    }

    fn learnt_through(&self) -> f64 {
        let sightings = self.sightings.iter().map(Sighting::learnt_s);
        let claims = self.claims.iter().map(|c| learnt_s(&c.lineage, c.stated_s));
        let names = self.names.iter().map(|n| learnt_s(&n.lineage, n.stated_s));
        let orbits = self.orbits.iter().map(|o| learnt_s(&o.lineage, o.stated_s));
        let series =
            self.series.iter().filter_map(|s| s.last().map(|x| learnt_s(&s.lineage, x.observed_s)));
        sightings.chain(claims).chain(names).chain(orbits).chain(series).fold(f64::NEG_INFINITY, f64::max)
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
    /// The most recent moment the sender learnt any of this.
    pub fn learnt_through(&self) -> f64 {
        self.parts.iter().map(Part::learnt_through).fold(f64::NEG_INFINITY, f64::max)
    }
}

/// How far through its own backlog a craft has reported, per recipient.
///
/// Reports drain a backlog, so the sender has to remember how far it has got with each
/// recipient. The key is the recipient's ship id, `0` for a broadcast — what was shouted to
/// nobody in particular is its own backlog.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Reporting {
    told: BTreeMap<i64, f64>,
}

impl Reporting {
    /// What a report to this recipient should resume from.
    pub fn since(&self, to: i64) -> f64 {
        self.told.get(&to).copied().unwrap_or(f64::NEG_INFINITY)
    }

    /// A report to this recipient has gone out, carrying everything through `through`.
    pub fn sent(&mut self, to: i64, through: f64) {
        let mark = self.told.entry(to).or_insert(f64::NEG_INFINITY);
        *mark = mark.max(through);
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
    pub fn learnt_through(&self) -> Option<f64> {
        self.entries
            .iter()
            .map(Entry::learnt_through)
            .fold(None, |best: Option<f64>, t| Some(best.map_or(t, |b| b.max(t))))
    }

    /// Systems it carries, which is what a transcript line counts.
    pub fn stars(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;

    const AU_LY: f64 = 1.581_250_7e-5;

    fn star_id(key: u64) -> StarId {
        StarId::synthesise("test", key)
    }

    fn sighting(witness: u64, at: DVec3, toward: DVec3, observed_s: f64) -> Sighting {
        Sighting {
            witness: Witness(witness),
            observed_s,
            bearing: Bearing {
                observer_ly: at,
                toward: toward.normalize(),
                sigma_rad: 1e-9,
            },
            band: Band::V,
            flux: 1e-12,
            flux_sigma: 1e-15,
            lineage: Lineage::new(),
        }
    }

    /// Bearings to `star` from points around a one AU circle.
    fn looks(witness: u64, star: DVec3, n: u64, from_s: f64) -> Vec<Sighting> {
        (0..n)
            .map(|i| {
                let phase = std::f64::consts::PI * i as f64 / n as f64;
                let at = DVec3::new(phase.cos(), phase.sin(), 0.0) * AU_LY;
                sighting(witness, at, star - at, from_s + i as f64 * 1e6)
            })
            .collect()
    }

    #[test]
    fn a_star_nobody_has_seen_is_absent() {
        let k = Knowledge::new(Witness(1));
        assert!(!k.knows(star_id(1)));
        assert!(k.belief(star_id(1)).is_none());
        assert!(k.is_empty());
    }

    #[test]
    fn one_look_gives_a_bearing_and_no_distance() {
        let mut k = Knowledge::new(Witness(1));
        let star = star_id(1);
        k.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 0.0));
        let belief = k.belief(star).unwrap();
        assert_eq!(belief.distance, Distance::Unknown);
        assert!(
            belief.luminosity_w().is_none(),
            "no distance, no luminosity"
        );
        assert!(
            belief.emitted_s().is_none(),
            "light age is a distance in disguise"
        );
        assert_eq!(belief.hops, 0);
    }

    #[test]
    fn looking_from_around_an_orbit_gives_a_distance() {
        let mut k = Knowledge::new(Witness(1));
        let star = star_id(1);
        let truth = DVec3::new(0.0, 0.0, 6.0);
        for s in looks(1, truth, 8, 0.0) {
            k.sighted(star, s);
        }
        let belief = k.belief(star).unwrap();
        let Distance::Measured { position_ly, .. } = belief.distance else {
            panic!("{:?}", belief.distance);
        };
        assert!(position_ly.distance(truth) < 0.05, "{position_ly}");
        assert!(belief.luminosity_w().is_some());
        let age = belief.light_age_s().unwrap();
        assert!((age / crate::flight::JULIAN_YEAR_S - 6.0).abs() < 0.05);
        assert!(belief.emitted_s().unwrap() < belief.observed_s);
    }

    /// The reservoir keeps the baseline, which is the thing distance is made of.
    #[test]
    fn a_long_watch_keeps_the_widest_bearings() {
        let mut k = Knowledge::new(Witness(1));
        let star = star_id(1);
        let truth = DVec3::new(0.0, 0.0, 6.0);
        for s in looks(1, truth, 200, 0.0) {
            k.sighted(star, s);
        }
        let file = k.file(star).unwrap();
        assert_eq!(file.sightings().len(), BEARINGS_KEPT);
        let Distance::Measured { position_ly, .. } = k.belief(star).unwrap().distance else {
            panic!("a decimated file still measures");
        };
        assert!(position_ly.distance(truth) < 0.05, "{position_ly}");
        let newest = file
            .sightings()
            .iter()
            .map(|s| s.observed_s)
            .fold(0.0, f64::max);
        assert_eq!(newest, 199.0 * 1e6, "the latest look is never dropped");
    }

    #[test]
    fn a_report_carries_what_was_learnt_and_gains_a_hop() {
        let star = star_id(7);
        let mut probe = Knowledge::new(Witness(2));
        for s in looks(2, DVec3::new(0.0, 0.0, 6.0), 4, 0.0) {
            probe.sighted(star, s);
        }
        probe.measured(
            star,
            Witness(2),
            Band::V,
            Sample {
                observed_s: 10.0,
                deficit: 1e-4,
                sigma: 1e-5,
            },
        );

        let report = probe.report(f64::NEG_INFINITY, 1.0e7);
        assert_eq!(report.stars(), 1);

        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&report, 1.2e8);
        let belief = ship.belief(star).unwrap();
        assert_eq!(belief.hops, 1, "second hand, and it says so");
        assert_eq!(
            belief.witnesses, 1,
            "the probe did the looking, not the ship"
        );
        assert_eq!(
            belief.learnt_s, 1.2e8,
            "learnt when the light landed, not when taken"
        );
        assert!(matches!(belief.distance, Distance::Measured { .. }));
        assert_eq!(
            ship.file(star).unwrap().series_in(Band::V).unwrap().len(),
            1
        );
    }

    #[test]
    fn the_same_measurement_by_two_routes_is_one_measurement() {
        let star = star_id(7);
        let mut probe = Knowledge::new(Witness(2));
        for s in looks(2, DVec3::new(0.0, 0.0, 6.0), 4, 0.0) {
            probe.sighted(star, s);
        }
        probe.measured(
            star,
            Witness(2),
            Band::V,
            Sample {
                observed_s: 10.0,
                deficit: 1e-4,
                sigma: 1e-5,
            },
        );
        let report = probe.report(f64::NEG_INFINITY, 1.0e7);

        let mut relay = Knowledge::new(Witness(3));
        relay.receive(&report, 2.0e7);
        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&report, 3.0e7);
        ship.receive(&relay.report(f64::NEG_INFINITY, 2.5e7), 4.0e7);

        let file = ship.file(star).unwrap();
        assert_eq!(file.sightings().len(), 4, "not eight");
        assert_eq!(file.series_in(Band::V).unwrap().len(), 1);
        assert_eq!(
            ship.belief(star).unwrap().hops,
            1,
            "the shortest route it came by"
        );
    }

    /// Two craft far apart measure a distance neither could alone.
    #[test]
    fn two_witnesses_triangulate_what_one_cannot() {
        let star = star_id(9);
        let truth = DVec3::new(0.0, 0.0, 400.0);
        let mut here = Knowledge::new(Witness(1));
        here.sighted(star, sighting(1, DVec3::ZERO, truth, 0.0));
        assert_eq!(here.belief(star).unwrap().distance, Distance::Unknown);

        let far = DVec3::X * 4.0;
        let mut there = Knowledge::new(Witness(2));
        there.sighted(star, sighting(2, far, truth - far, 1.0));
        here.receive(&there.report(f64::NEG_INFINITY, 2.0), 3.0);

        let belief = here.belief(star).unwrap();
        assert_eq!(belief.witnesses, 2);
        let Distance::Measured { position_ly, .. } = belief.distance else {
            panic!("a four light-year baseline is a parallax of a degree at 400 ly");
        };
        assert!(position_ly.distance(truth) < 1.0, "{position_ly}");
    }

    /// A chart is somebody's word, and one look of your own does not overturn it — but a
    /// parallax does.
    #[test]
    fn a_measurement_of_your_own_beats_a_distance_you_were_told() {
        let star = star_id(11);
        let truth = DVec3::new(0.0, 0.0, 6.0);
        let mut k = Knowledge::new(Witness(1));
        k.sighted(star, sighting(1, DVec3::ZERO, truth, 0.0));
        k.told(
            star,
            Claim {
                witness: Witness(99),
                distance: Distance::Measured {
                    position_ly: DVec3::new(0.0, 0.0, 9.0),
                    sigma_ly: 1.0,
                },
                stated_s: -1.0e6,
                lineage: vec![Hop {
                    from: Witness(99),
                    to: Witness(1),
                    sent_s: -1.0e6,
                    received_s: 0.0,
                }],
            },
        );
        let belief = k.belief(star).unwrap();
        assert!(!belief.triangulated, "held on somebody's word");
        assert_eq!(
            belief.distance.position_ly(),
            Some(DVec3::new(0.0, 0.0, 9.0))
        );

        for s in looks(1, truth, 8, 1.0) {
            k.sighted(star, s);
        }
        let belief = k.belief(star).unwrap();
        assert!(belief.triangulated);
        assert!(
            belief.distance.position_ly().unwrap().distance(truth) < 0.05,
            "the chart was wrong"
        );
    }

    #[test]
    fn a_claim_travels_on_and_says_who_said_it() {
        let star = star_id(12);
        let mut office = Knowledge::new(Witness(50));
        office.sighted(star, sighting(50, DVec3::ZERO, DVec3::Z, 0.0));
        office.told(
            star,
            Claim {
                witness: Witness(50),
                distance: Distance::Measured {
                    position_ly: DVec3::Z * 12.0,
                    sigma_ly: 0.1,
                },
                stated_s: 0.0,
                lineage: Lineage::new(),
            },
        );
        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&office.report(f64::NEG_INFINITY, 10.0), 20.0);
        let file = ship.file(star).unwrap();
        assert_eq!(file.claims().len(), 1);
        assert_eq!(
            file.claims()[0].lineage.len(),
            1,
            "one hop, from the office"
        );
        assert_eq!(
            file.claims()[0].witness,
            Witness(50),
            "and it is still the office's claim"
        );
        assert_eq!(
            ship.belief(star).unwrap().distance.position_ly(),
            Some(DVec3::Z * 12.0)
        );
    }

    /// Finding something is writing it down, and what gets written down is a designation —
    /// beaten by any name a crew actually chooses.
    #[test]
    fn a_found_star_gets_a_designation_and_a_named_one_keeps_its_name() {
        let star = star_id(20);
        let mut k = Knowledge::new(Witness(1));
        k.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 0.0));
        let found = k.belief(star).unwrap().name.clone().unwrap();
        assert_eq!(found.kind, NameKind::Designation);
        assert_eq!(found.witness, Witness(1));
        assert_eq!(found.name, designation(DVec3::X));

        k.name_it(star, "Kettle", 100.0);
        let named = k.belief(star).unwrap().name.clone().unwrap();
        assert_eq!(named.name, "Kettle");
        assert_eq!(named.kind, NameKind::Given);

        // A later sighting does not re-designate something that has a name.
        k.sighted(star, sighting(1, DVec3::Y * 0.1, DVec3::X, 200.0));
        assert_eq!(
            k.belief(star).unwrap().name.as_ref().unwrap().name,
            "Kettle"
        );
    }

    /// A name travels like anything else, and two crews may hold different names for the same
    /// light. Your own wins on your own screen; theirs is still there, with their name on it.
    #[test]
    fn a_name_is_something_somebody_said_and_carries_who_said_it() {
        let star = star_id(21);
        let mut theirs = Knowledge::new(Witness(2));
        theirs.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        theirs.name_it(star, "Hearthlight", 10.0);

        let mut ours = Knowledge::new(Witness(1));
        ours.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 5.0));
        ours.receive(&theirs.report(f64::NEG_INFINITY, 20.0), 30.0);

        // Theirs is a chosen name and ours is only a designation, so theirs is what we go by.
        let belief = ours.belief(star).unwrap();
        assert_eq!(belief.name.as_ref().unwrap().name, "Hearthlight");
        assert_eq!(belief.name.as_ref().unwrap().witness, Witness(2));
        assert_eq!(
            belief.name.as_ref().unwrap().lineage.len(),
            1,
            "heard, not coined"
        );

        // Until we name it ourselves, at which point ours is what we call it and theirs is
        // still on file.
        ours.name_it(star, "The Kettle", 40.0);
        assert_eq!(
            ours.belief(star).unwrap().name.as_ref().unwrap().name,
            "The Kettle"
        );
        assert_eq!(ours.file(star).unwrap().names().len(), 2);
    }

    #[test]
    fn a_designation_says_which_way_it_was_found() {
        let along_x = designation(DVec3::X);
        let up = designation(DVec3::Z);
        assert_ne!(along_x, up);
        assert!(up.contains("+90"), "{up}");
        assert_eq!(
            designation(DVec3::X * 3.0),
            along_x,
            "a direction, not a distance"
        );
    }

    /// A craft is issued its name by a shard, after it has already measured things. Both
    /// ships starting as witness zero is what would make two crews' records collide.
    #[test]
    fn taking_a_name_carries_this_craft_own_records_over() {
        let star = star_id(31);
        let mut k = Knowledge::new(Witness(0));
        for s in looks(0, DVec3::new(0.0, 0.0, 6.0), 4, 0.0) {
            k.sighted(star, s);
        }
        k.name_it(star, "Ours", 5.0);
        k.measured(
            star,
            Witness(0),
            Band::V,
            Sample {
                observed_s: 1.0,
                deficit: 0.0,
                sigma: 0.1,
            },
        );
        // Somebody else's record, which must not be touched.
        k.told(
            star,
            Claim {
                witness: Witness(9),
                distance: Distance::AtLeast(1.0),
                stated_s: 0.0,
                lineage: Lineage::new(),
            },
        );

        k.rebrand(Witness(42));
        assert_eq!(k.owner, Witness(42));
        let file = k.file(star).unwrap();
        assert!(file.sightings().iter().all(|s| s.witness == Witness(42)));
        assert_eq!(file.names()[0].witness, Witness(42));
        assert_eq!(
            file.claims()[0].witness,
            Witness(9),
            "somebody else's stays theirs"
        );
        assert!(
            k.own_series(star, Band::V).is_some(),
            "its own photometry follows it"
        );
        assert_eq!(k.belief(star).unwrap().witnesses, 1);
    }

    fn planet(star: StarId, key: &str) -> (BodyId, Subject) {
        let body = BodyId::of(star, key);
        (body, Subject::Body { star, body })
    }

    /// A planet is named after its star, and the star is whatever *this* craft calls it.
    #[test]
    fn a_planet_is_read_after_this_craft_name_for_its_star() {
        let star = star_id(40);
        let mut k = Knowledge::new(Witness(1));
        k.name_it(star, "Kettle", 0.0);
        let (body, subject) = planet(star, "one");
        assert_eq!(k.found_planet(star, body, 0.7, 1.0, 1.0), "b");
        assert_eq!(k.name_of(subject).as_deref(), Some("Kettle b"));

        k.name_it(star, "The Kettle", 2.0);
        assert_eq!(k.name_of(subject).as_deref(), Some("The Kettle b"), "it follows the star");

        k.name_it(subject, "Spout", 3.0);
        assert_eq!(k.name_of(subject).as_deref(), Some("Spout"), "until somebody names it");
    }

    /// A letter is frozen: a better orbit later does not move it.
    #[test]
    fn a_letter_once_assigned_is_never_reassigned() {
        let star = star_id(41);
        let mut k = Knowledge::new(Witness(1));
        let (body, subject) = planet(star, "one");
        assert_eq!(k.found_planet(star, body, 0.7, 1.0, 0.0), "b");
        assert_eq!(k.found_planet(star, body, 4.3, 1.0, 5.0), "b");
        assert_eq!(k.file(subject).unwrap().orbits()[0].semi_major_au, 4.3, "the orbit is updated");
    }

    #[test]
    fn letters_this_craft_already_uses_leave_room_for_the_next() {
        let star = star_id(42);
        let mut k = Knowledge::new(Witness(1));
        let (outer, _) = planet(star, "outer");
        let (inner, _) = planet(star, "inner");
        let (between, _) = planet(star, "between");
        assert_eq!(k.found_planet(star, outer, 1.28, 1.0, 0.0), "c");
        assert_eq!(k.found_planet(star, inner, 0.7, 1.0, 1.0), "b");
        assert_eq!(k.found_planet(star, between, 0.9, 1.0, 2.0), "bb", "no single letter left");
    }

    /// Everything about a system rides with its star: one entry, however many planets.
    #[test]
    fn a_report_carries_a_system_as_one_entry() {
        let star = star_id(43);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        probe.name_it(star, "Kettle", 1.0);
        for (key, a) in [("one", 0.7), ("two", 1.28), ("three", 2.35)] {
            let (body, _) = planet(star, key);
            probe.found_planet(star, body, a, 1.0, 2.0);
        }
        let report = probe.report(f64::NEG_INFINITY, 3.0);
        assert_eq!(report.stars(), 1, "one system, not four entries");
        assert_eq!(report.entries[0].parts.len(), 4, "the star and its three planets");

        // Capped by systems: a second star's system does not split the first.
        let other = star_id(44);
        probe.sighted(other, sighting(2, DVec3::ZERO, DVec3::Y, 10.0));
        let first = probe.report_upto(f64::NEG_INFINITY, 11.0, 1);
        assert_eq!(first.stars(), 1);
        assert_eq!(first.entries[0].parts.len(), 4);
    }

    /// The whole of 11a: a receiver sees another craft's planets named after its own name for
    /// the star, and renaming the star renames them.
    #[test]
    fn a_receiver_reads_somebody_else_planets_after_its_own_star_name() {
        let star = star_id(45);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        probe.name_it(star, "Kettle", 1.0);
        let (body, subject) = planet(star, "one");
        probe.found_planet(star, body, 1.28, 1.0, 2.0);
        assert_eq!(probe.name_of(subject).as_deref(), Some("Kettle c"));

        let mut ship = Knowledge::new(Witness(1));
        ship.sighted(star, sighting(1, DVec3::Y * 0.1, DVec3::X, 0.5));
        ship.name_it(star, "Hearthlight", 0.6);
        ship.receive(&probe.report(f64::NEG_INFINITY, 3.0), 10.0);

        assert_eq!(ship.name_of(star).as_deref(), Some("Hearthlight"), "ours, not theirs");
        assert_eq!(ship.name_of(subject).as_deref(), Some("Hearthlight c"));
        let orbit = &ship.file(subject).unwrap().orbits()[0];
        assert_eq!(orbit.witness, Witness(2), "the orbit is still the probe's statement");
        assert_eq!(orbit.lineage.len(), 1);

        ship.name_it(star, "Home", 11.0);
        assert_eq!(ship.name_of(subject).as_deref(), Some("Home c"));

        // And its own later find is lettered around the letter it was told.
        let (inner, inner_subject) = planet(star, "inner");
        assert_eq!(ship.found_planet(star, inner, 0.7, 1.0, 12.0), "b");
        assert_eq!(ship.name_of(inner_subject).as_deref(), Some("Home b"));
    }

    #[test]
    fn a_planet_nobody_named_the_star_of_still_has_a_name() {
        let star = star_id(46);
        let mut k = Knowledge::new(Witness(1));
        let (body, subject) = planet(star, "one");
        k.found_planet(star, body, 0.7, 1.0, 0.0);
        assert_eq!(k.name_of(subject).as_deref(), Some("? b"), "its star is not written down");
        k.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 1.0));
        let designation = designation(DVec3::X);
        assert_eq!(k.name_of(subject), Some(format!("{designation} b")));
    }

    #[test]
    fn stars_are_what_a_map_draws_and_members_are_what_belongs_to_them() {
        let star = star_id(47);
        let mut k = Knowledge::new(Witness(1));
        k.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 0.0));
        let (body, subject) = planet(star, "one");
        k.found_planet(star, body, 0.7, 1.0, 1.0);
        assert_eq!(k.len(), 2, "the star and its planet");
        assert_eq!(k.stars().count(), 1);
        assert_eq!(k.members(star).map(|(s, _)| s).collect::<Vec<_>>(), vec![subject]);
    }

    /// A replica is kept current by reports from the craft itself, and those add no hop: the
    /// copy is the same knowledge, not something it was told.
    #[test]
    fn a_replica_absorbs_without_a_hop_and_only_new_samples_travel() {
        let star = star_id(60);
        let mut held = Knowledge::new(Witness(3));
        held.sighted(star, sighting(3, DVec3::ZERO, DVec3::X, 1.0));
        for t in 1..=5 {
            held.measured(star, Witness(3), Band::V, Sample { observed_s: t as f64, deficit: 0.0, sigma: 0.1 });
        }
        let mut copy = Knowledge::new(Witness(3));
        let first = held.report(f64::NEG_INFINITY, 5.0);
        copy.absorb(&first);
        assert_eq!(copy.belief(star).unwrap().hops, 0, "its own, not relayed");
        assert_eq!(copy.own_series(star, Band::V).unwrap().len(), 5);

        held.measured(star, Witness(3), Band::V, Sample { observed_s: 6.0, deficit: 0.0, sigma: 0.1 });
        let delta = held.report(first.learnt_through().unwrap(), 6.0);
        let sent = &delta.entries[0].parts[0].series[0];
        assert_eq!(sent.len(), 1, "one new sample, not the whole curve again");
        copy.absorb(&delta);
        assert_eq!(copy.own_series(star, Band::V).unwrap().len(), 6);
        assert_eq!(copy, held, "and the copy is the original");
    }

    /// What is written down is files without their samples and samples as log rows, and the two
    /// together rebuild the same knowledge — the map after a restart is the map before it.
    #[test]
    fn what_is_written_down_rebuilds_the_same_knowledge() {
        let star = star_id(70);
        let mut probe = Knowledge::new(Witness(2));
        for t in 1..=3 {
            probe.measured(star, Witness(2), Band::V, Sample { observed_s: t as f64, deficit: 0.1, sigma: 0.01 });
        }
        let mut k = Knowledge::new(Witness(1));
        for s in looks(1, DVec3::new(0.0, 0.0, 6.0), 6, 0.0) {
            k.sighted(star, s);
        }
        k.name_it(star, "Kettle", 1.0);
        let (body, _) = planet(star, "one");
        k.found_planet(star, body, 0.7, 1.0, 2.0);
        for t in 10..=14 {
            k.measured(star, Witness(1), Band::K, Sample { observed_s: t as f64, deficit: 0.0, sigma: 0.01 });
        }
        k.receive(&probe.report(f64::NEG_INFINITY, 4.0), 9.0);

        let (files, logs) = k.take_changes();
        assert_eq!(files.len(), 2, "the star and its planet");
        assert!(files.iter().all(|(_, f)| f.series().iter().all(|s| s.is_empty())), "samples are logged apart");
        assert_eq!(logs.len(), 5 + 3, "our five, and the probe's three");
        let relayed = logs.iter().find(|l| l.witness == Witness(2)).unwrap();
        assert_eq!(relayed.learnt_s, 9.0, "a relayed sample is learnt when it arrived");

        let back = Knowledge::restore(Witness(1), files, logs);
        assert_eq!(back, k, "the same knowledge");
        assert_eq!(back.name_of(star).as_deref(), Some("Kettle"));

        let mut back = back;
        let (files, logs) = back.take_changes();
        assert!(files.is_empty() && logs.is_empty(), "nothing restored counts as changed");
        back.measured(star, Witness(1), Band::K, Sample { observed_s: 20.0, deficit: 0.0, sigma: 0.01 });
        let (files, logs) = back.take_changes();
        assert!(files.is_empty(), "a sample on an existing series changes only the log");
        assert_eq!(logs.len(), 1);
    }

    #[test]
    fn reporting_remembers_how_far_it_got_with_each_recipient() {
        let mut marks = Reporting::default();
        assert_eq!(marks.since(7), f64::NEG_INFINITY);
        marks.sent(7, 10.0);
        marks.sent(7, 5.0);
        assert_eq!(marks.since(7), 10.0, "a mark never moves back");
        assert_eq!(marks.since(0), f64::NEG_INFINITY, "a broadcast is its own backlog");
    }

    #[test]
    fn a_report_only_carries_what_is_new() {
        let star = star_id(3);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 100.0));
        assert!(probe.report(200.0, 300.0).is_empty(), "nothing since then");
        assert_eq!(probe.report(50.0, 300.0).stars(), 1);
    }

    /// A report is a slice of a backlog: oldest first, resumed from where the last one ended,
    /// and never cut through the middle of one instant.
    #[test]
    fn a_capped_report_drains_the_backlog_in_order() {
        let mut probe = Knowledge::new(Witness(2));
        for k in 0..10u64 {
            probe.sighted(star_id(k), sighting(2, DVec3::ZERO, DVec3::X, k as f64));
        }
        let first = probe.report_upto(f64::NEG_INFINITY, 100.0, 4);
        assert_eq!(first.stars(), 4);
        let through = first.learnt_through().unwrap();
        assert_eq!(through, 3.0, "the four oldest");

        let second = probe.report_upto(through, 200.0, 4);
        assert_eq!(second.stars(), 4);
        assert_eq!(second.learnt_through(), Some(7.0));
        assert_eq!(probe.report_upto(7.0, 300.0, 4).stars(), 2, "and the rest");

        // Everything learnt at the same instant rides in the same report; the sender has one
        // number to resume from and anything left on the far side of it would never be sent.
        let mut tied = Knowledge::new(Witness(2));
        for k in 0..10u64 {
            tied.sighted(star_id(k), sighting(2, DVec3::ZERO, DVec3::X, 5.0));
        }
        assert_eq!(tied.report_upto(f64::NEG_INFINITY, 100.0, 4).stars(), 10);
    }

    #[test]
    fn a_series_belongs_to_the_witness_that_took_it() {
        let star = star_id(4);
        let mut k = Knowledge::new(Witness(1));
        for (w, t) in [(1u64, 10.0), (2, 20.0)] {
            k.measured(
                star,
                Witness(w),
                Band::V,
                Sample {
                    observed_s: t,
                    deficit: 1e-3,
                    sigma: 1e-5,
                },
            );
        }
        assert_eq!(k.file(star).unwrap().series().len(), 2);
        assert_eq!(k.own_series(star, Band::V).unwrap().len(), 1);
        assert!(k.own_series(star, Band::K).is_none());
    }

    /// Switching bands, or to another star and back, does not lose what was measured.
    #[test]
    fn photometry_is_kept_per_star_and_band() {
        let mut k = Knowledge::new(Witness(1));
        let (a, b) = (star_id(1), star_id(2));
        for (star, band) in [(a, Band::V), (a, Band::K), (b, Band::V)] {
            k.measured(
                star,
                Witness(1),
                band,
                Sample {
                    observed_s: 1.0,
                    deficit: 0.1,
                    sigma: 0.01,
                },
            );
        }
        assert_eq!(k.own_series(a, Band::V).unwrap().len(), 1);
        assert_eq!(k.own_series(a, Band::K).unwrap().len(), 1);
        assert_eq!(k.own_series(b, Band::V).unwrap().len(), 1);
    }
}
