//! What one craft has actually seen, and who told it the rest.
//!
//! A belief is a fold over the measurements held, each taken somewhere, at some time, by
//! somebody. Nothing here knows the truth: everything in a [`Knowledge`] came in through
//! [`survey`] or a [`Report`], and a star nobody has seen is absent.
//! See `lightcone/docs/22-provenance.md`.

// Nothing in the game loop panics: startup may, and past it a wire message, a row or another
// craft's report is data. Every exception carries an `allow` with its reason.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use std::collections::BTreeMap;

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use crate::sky::StarId;

pub mod arc;
pub mod astrometry;
pub mod body;
pub mod conclusion;
pub mod formats;
pub mod moments;
pub mod names;
pub mod observatory;
pub mod primary;
pub mod prior;
pub mod sort;
pub mod record;
pub mod report;
pub mod room;
pub mod subject;
pub mod survey;
pub mod transit;
pub mod turns;

pub use astrometry::{Bearing, Distance};
pub use body::{BodyBelief, Placed, SystemPlane};
pub use conclusion::{Conclusion, Consumed, Digest};
pub use names::designation;
pub use report::{ENTRIES_PER_REPORT, Entry, Log, Logs, Mark, Part, Report, Reporting};
pub use record::{
    Claim, Colors, Hop, Lineage, Method, NameKind, Naming, Orbit, Orientation, Sample, Series,
    Sighting, Witness,
    learned_s,
};
pub use subject::{BodyId, Subject};

/// Bearings kept per subject per witness. Distance comes from the spread of observing
/// positions, so the widest spread is kept rather than the most recent; sixteen well-spread
/// bearings measure a parallax as well as a thousand.
pub const BEARINGS_KEPT: usize = 16;

/// How many relative namings deep a name is read before it stops: see
/// [`Knowledge::name_of`].
pub const NAME_DEPTH: usize = 10;

/// What one photometric sample takes aboard, bytes: a time, a value and an error.
pub const SAMPLE_BYTES: f64 = 24.0;

fn sigma_of(claim: &Claim) -> f64 {
    match claim.distance {
        Distance::Measured { sigma_ly, .. } => sigma_ly,
        _ => f64::INFINITY,
    }
}

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
    /// `None` until written down anywhere. A relative naming is only a suffix; what to show is
    /// [`Knowledge::name_of`].
    pub name: Option<Naming>,
    /// The most recent bearing, from wherever that witness was.
    pub bearing: Bearing,
    pub distance: Distance,
    /// Whether the distance was solved from bearings held here, its own or relayed raw, rather
    /// than taken from a [`Claim`], which cannot be re-solved or checked.
    pub triangulated: bool,
    /// For a distance triangulated here, the angle its widest baseline subtended at the star.
    pub baseline_rad: Option<f64>,
    /// For a distance taken from a claim, whose word it is on.
    pub claimed_by: Option<Witness>,
    /// This craft's own lower bound, when a claim was believed over it, so a chart nearer than
    /// the craft's bearings allow shows as a disagreement.
    pub floor_ly: Option<f64>,
    pub band: Band,
    pub flux: f64,
    /// Coordinate seconds the most recent light held arrived — at its witness, not here.
    pub observed_s: f64,
    /// Coordinate seconds this craft learned of it.
    pub learned_s: f64,
    pub sightings: usize,
    pub witnesses: usize,
    /// Hops on the shortest route any of it took here. Zero for something seen directly.
    pub hops: usize,
}

impl Belief {
    pub fn star(&self) -> Option<StarId> {
        self.subject.as_star()
    }

    /// Seconds the light had been traveling, once there is a distance to say so.
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
    conclusions: Vec<Conclusion>,
    /// Per-band photometry, folded per visit and fixed in size. One per witness.
    colors: Vec<Colors>,
    /// This craft's own: what is left of logs it has consumed. Never transmitted.
    digests: Vec<Digest>,
    /// Keep this subject's logs whatever the pipeline concludes. This craft's own choice.
    retained: bool,
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

    pub fn conclusions(&self) -> &[Conclusion] {
        &self.conclusions
    }

    pub fn colors(&self) -> &[Colors] {
        &self.colors
    }

    pub fn digests(&self) -> &[Digest] {
        &self.digests
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

    /// Its own statement if it has made one, otherwise the most recent anybody made.
    fn orbit(&self, owner: Witness) -> Option<&Orbit> {
        self.orbits
            .iter()
            .max_by(|a, b| (a.witness == owner).cmp(&(b.witness == owner)).then(a.stated_s.total_cmp(&b.stated_s)))
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
        // Measured here beats a claim whatever error bars either carries: only one can be checked.
        //
        // Except for a body, which is never a static point. `triangulate` fits one to whatever
        // bearings it is given, and a body's bearings are all taken from inside its own system
        // where it moves appreciably between them -- so it returns a place the body was never
        // at, with the error bar of a fit that converged. Measured: a ship on a 5 AU orbit
        // surveying Sol put Jupiter at 1.63 AU plus or minus 9e-7, sixteen million sigma from
        // where it was. A body's distance comes from its orbit; see `knowledge::body`.
        let measured = match subject {
            Subject::Body { .. } => Distance::Unknown,
            _ => astrometry::triangulate(&bearings),
        };
        let taken = matches!(measured, Distance::Measured { .. });
        let claimed = self.claims.iter().min_by(|a, b| sigma_of(a).total_cmp(&sigma_of(b)));
        let believed_claim = claimed.filter(|_| !taken);
        let distance = match believed_claim {
            Some(claim) => claim.distance,
            None => measured,
        };
        Some(Belief {
            subject,
            name: self.naming(owner).cloned(),
            bearing: latest.bearing,
            distance,
            triangulated: taken,
            baseline_rad: match measured {
                Distance::Measured { position_ly, .. } => Some(astrometry::baseline_rad(&bearings, position_ly)),
                _ => None,
            },
            claimed_by: believed_claim.map(|c| c.witness),
            floor_ly: match (believed_claim, measured) {
                (Some(_), Distance::AtLeast(ly)) => Some(ly),
                _ => None,
            },
            band: latest.band,
            flux: latest.flux,
            observed_s: latest.observed_s,
            learned_s: self.sightings.iter().map(Sighting::learned_s).fold(f64::INFINITY, f64::min),
            sightings: self.sightings.len(),
            witnesses: witnesses.len(),
            hops: self.sightings.iter().map(|s| s.lineage.len()).min().unwrap_or(0),
        })
    }

    /// Keep the bearings that are farthest apart, in place **and** in time.
    ///
    /// Both baselines move a bearing, and which one is doing the work depends on what the craft
    /// was doing. Parallax needs the observer to have gone somewhere; a body's own motion needs
    /// only for time to have passed. A craft parked in a system has no spatial spread at all,
    /// so scoring on position alone left every pair tied at zero. Each gap is therefore measured
    /// against the widest of its own kind among the bearings held, which needs no scale chosen
    /// between a light-year and a year.
    ///
    /// What goes is the look whose removal costs the least spread: the sum of the distances to
    /// its two nearest neighbors. Not the older of the closest *pair* -- on an even cadence
    /// every pair ties, so that ate the watch from its oldest end and left the last sixteen
    /// minutes of a year-long arc. By this measure an interior look sits between two neighbors
    /// and an end one has only a far side, so the ends are what survive.
    fn decimate(&mut self, witness: Witness) {
        while self.sightings.iter().filter(|s| s.witness == witness).count() > BEARINGS_KEPT {
            let held: Vec<(usize, &Sighting)> =
                self.sightings.iter().enumerate().filter(|(_, s)| s.witness == witness).collect();
            // Never the newest: it is what the display reads.
            let Some(&(newest, _)) = held.iter().max_by(|a, b| a.1.observed_s.total_cmp(&b.1.observed_s)) else {
                return;
            };
            let apart = |a: &Sighting, b: &Sighting| {
                (a.bearing.observer_ly.distance(b.bearing.observer_ly), (a.observed_s - b.observed_s).abs())
            };
            let (mut widest_ly, mut widest_s) = (0.0f64, 0.0f64);
            for (n, &(_, a)) in held.iter().enumerate() {
                for &(_, b) in held.iter().skip(n + 1) {
                    let (ly, s) = apart(a, b);
                    widest_ly = widest_ly.max(ly);
                    widest_s = widest_s.max(s);
                }
            }
            // A baseline nobody has is a baseline nothing is lost by ignoring.
            let share = |v: f64, widest: f64| if widest > 0.0 { v / widest } else { 0.0 };

            let mut drop: Option<(f64, usize)> = None;
            for &(i, a) in held.iter().filter(|&&(i, _)| i != newest) {
                let mut near: Vec<f64> = held
                    .iter()
                    .filter(|&&(j, _)| j != i)
                    .map(|&(_, b)| {
                        let (ly, s) = apart(a, b);
                        share(ly, widest_ly).hypot(share(s, widest_s))
                    })
                    .collect();
                near.sort_by(f64::total_cmp);
                let cost: f64 = near.iter().take(2).sum();
                if drop.is_none_or(|(least, _)| cost < least) {
                    drop = Some((cost, i));
                }
            }
            let Some((_, loser)) = drop else { return };
            self.sightings.remove(loser);
        }
    }

}

/// One photometric sample as a log row. For a relayed series `learned_s` is when it arrived,
/// not when it was measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Logged {
    pub subject: Subject,
    pub witness: Witness,
    pub band: Band,
    pub sample: Sample,
    pub learned_s: f64,
}

/// One craft's view of the sky.
#[derive(Clone, Debug)]
pub struct Knowledge {
    pub owner: Witness,
    files: BTreeMap<Subject, File>,
    beliefs: BTreeMap<Subject, Belief>,
    /// Files and samples not yet written down. Not part of what a craft knows, so equality
    /// ignores them, as it does every field below `beliefs`.
    changed: std::collections::BTreeSet<Subject>,
    unsaved: Vec<Logged>,
    consumed: Vec<Consumed>,
    /// What changed when, so what is new since a mark is found without walking every file.
    backlog: report::Backlog,
    sampled: report::Recent,
    /// Subjects with samples added since their log was last read.
    unread: std::collections::BTreeSet<Subject>,
    /// Subjects whose logs are to be consumed whatever they say: see [`Knowledge::analyze`].
    analyzing: std::collections::BTreeSet<Subject>,
    /// When a fit was last *attempted* on each body, which is not when one last succeeded.
    ///
    /// Scheduling, not knowledge: a body whose arc cannot yet shape an orbit states nothing,
    /// so ranking the queue by what has been stated leaves that body at the front of it
    /// forever and every other body in the system is never fitted at all. Not compared, not
    /// saved and not reported, because an attempt is not something a craft knows.
    tried: BTreeMap<Subject, primary::Attempt>,
    /// See [`room`].
    capacity_bytes: f64,
    occupied_bytes: f64,
    unkept: u64,
}

impl PartialEq for Knowledge {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner && self.files == other.files && self.beliefs == other.beliefs
    }
}

impl Knowledge {
    pub fn new(owner: Witness) -> Self {
        Self {
            tried: BTreeMap::new(),
            owner,
            files: BTreeMap::new(),
            beliefs: BTreeMap::new(),
            changed: Default::default(),
            unsaved: Vec::new(),
            consumed: Vec::new(),
            backlog: Default::default(),
            sampled: Default::default(),
            unread: Default::default(),
            analyzing: Default::default(),
            capacity_bytes: f64::INFINITY,
            occupied_bytes: 0.0,
            unkept: 0,
        }
    }

    /// Every file touched since the last call, without its samples, and every sample added, as
    /// log rows. Clears both.
    ///
    /// Stored apart because a file is bounded and rewritten whole, while a watched star's samples
    /// are appended every integration and would make that rewrite the costliest thing a shard did.
    pub fn take_changes(&mut self) -> (Vec<(Subject, File)>, Vec<Logged>) {
        let files = std::mem::take(&mut self.changed)
            .into_iter()
            .filter_map(|s| Some((s, self.files.get(&s)?.header())))
            .collect();
        (files, std::mem::take(&mut self.unsaved))
    }

    /// Put back what [`Knowledge::take_changes`] and [`Knowledge::take_consumed`] handed over,
    /// because it was never written down. Anything changed since goes after it, in order.
    pub fn untake(&mut self, files: impl IntoIterator<Item = Subject>, logs: Vec<Logged>, consumed: Vec<Consumed>) {
        self.changed.extend(files);
        let later = std::mem::replace(&mut self.unsaved, logs);
        self.unsaved.extend(later);
        let later = std::mem::replace(&mut self.consumed, consumed);
        self.consumed.extend(later);
    }

    /// Every file as it would be written, for a first save.
    pub fn files(&self) -> impl Iterator<Item = (Subject, File)> + '_ {
        self.files.iter().map(|(s, f)| (*s, f.header()))
    }

    /// Rebuild from what was written down. Logs must be in the order taken; nothing restored
    /// counts as changed.
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
            let last = knowledge.files.get(&subject).and_then(|f| {
                f.series.iter().filter(|s| s.witness == owner).filter_map(|s| s.last()).map(|s| s.observed_s).reduce(f64::max)
            });
            if let Some(last) = last {
                knowledge.sampled.touch(subject, last);
                knowledge.unread.insert(subject);
            }
        }
        knowledge.changed.clear();
        knowledge
    }

    /// Subjects held: stars, and whatever else has been learned about.
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

    /// What this craft calls something, as a player should see it. A relative naming is read
    /// after this craft's name for the star, whoever assigned the suffix.
    pub fn name_of(&self, subject: impl Into<Subject>) -> Option<String> {
        self.name_at_depth(subject.into(), 0)
    }

    /// A report can carry a relative naming on a star, which would read after itself forever;
    /// past [`NAME_DEPTH`] the subject goes by its designation instead.
    fn name_at_depth(&self, subject: Subject, depth: usize) -> Option<String> {
        let file = self.files.get(&subject)?;
        let naming = file.naming(self.owner)?;
        match naming.kind {
            NameKind::Relative if depth >= NAME_DEPTH => {
                let toward = file.sightings.first().map_or(glam::DVec3::Z, |s| s.bearing.toward);
                Some(designation(toward))
            }
            NameKind::Relative => {
                let star = match subject.star() {
                    Some(star) => self.name_at_depth(Subject::Star(star), depth + 1),
                    None => None,
                };
                Some(format!("{} {}", star.unwrap_or_else(|| "?".into()), naming.name))
            }
            _ => Some(naming.name.clone()),
        }
    }

    /// File a sighting this craft made itself.
    pub fn sighted(&mut self, subject: impl Into<Subject>, sighting: Sighting) {
        self.file_sighting(subject.into(), sighting);
    }

    /// File a distance somebody states.
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

    /// File a digest somebody else folded. One per witness, the one that watched longer
    /// winning: a digest is not a statement to be corrected but a running total, and the
    /// longer run is the one with more in it.
    pub(super) fn absorb_colors(&mut self, subject: Subject, digest: Colors) {
        let file = self.files.entry(subject).or_default();
        match file.colors.iter_mut().find(|c| c.witness == digest.witness) {
            Some(held) if held.spanned_s.1 >= digest.spanned_s.1 => {}
            Some(held) => *held = digest,
            None => file.colors.push(digest),
        }
        self.refresh(subject);
    }

    /// Fold one visit's per-band fluxes into a body's digest, which is fixed in size however
    /// many visits it has taken. The row itself is never kept.
    pub fn measured_colors(
        &mut self,
        subject: impl Into<Subject>,
        witness: Witness,
        at_s: f64,
        flux: &em_spectra::PerBand<Option<(f64, f64)>>,
    ) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file.colors.iter_mut().find(|c| c.witness == witness) {
            Some(held) => held.fold(at_s, flux),
            None => {
                let mut fresh = Colors::new(witness);
                fresh.fold(at_s, flux);
                file.colors.push(fresh);
            }
        }
        self.refresh(subject);
    }

    /// File where somebody says a body orbits. One statement per witness *per method*, the
    /// later winning.
    ///
    /// Per method, as [`Knowledge::named`] is per kind and for the same reason: a craft that
    /// has both watched a body transit and fitted its arc holds two different statements about
    /// it, made two different ways, and neither is a correction of the other. Keyed by witness
    /// alone they overwrote each other on every pass -- a fit replaced by a transit's shell,
    /// which made the body look unfitted, which refitted it, forever -- and a transit's
    /// edge-on constraint could never tighten a fitted pole because the two were never held at
    /// once.
    pub fn orbits(&mut self, subject: impl Into<Subject>, orbit: Orbit) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file.orbits.iter_mut().find(|o| o.witness == orbit.witness && o.method == orbit.method) {
            Some(held) if held.stated_s >= orbit.stated_s => {}
            Some(held) => *held = orbit,
            None => file.orbits.push(orbit),
        }
        self.refresh(subject);
    }

    /// File what somebody calls something.
    ///
    /// Each witness holds one chosen name and one a rule assigned. The assigned one is frozen, so
    /// renaming a planet does not free its letter; a later chosen name replaces the earlier.
    pub fn named(&mut self, subject: impl Into<Subject>, naming: Naming) {
        let subject = subject.into();
        let file = self.files.entry(subject).or_default();
        match file.names.iter_mut().find(|n| n.witness == naming.witness && n.kind.chosen() == naming.kind.chosen()) {
            Some(held) if held.stated_s > naming.stated_s => {}
            // A rule's assignment is frozen.
            Some(held) if !held.kind.chosen() => {}
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

    /// Record a planet this craft has found, and letter it against the letters it already uses
    /// for the star (see [`names::planet_letter`]). A planet already lettered keeps its letter.
    /// Returns the letter.
    pub fn found_planet(
        &mut self,
        star: StarId,
        body: BodyId,
        orbit: Orbit,
        luminosity_solar: f64,
        now_s: f64,
    ) -> String {
        let subject = Subject::Body { star, body };
        let owner = self.owner;
        let semi_major_au = orbit.semi_major_au.0;
        // Stamped here rather than trusted from the caller: this records what *this* craft
        // found, and one witness per statement is what `orbits` files by.
        self.orbits(subject, Orbit { witness: owner, ..orbit });
        if let Some(held) = self
            .files
            .get(&subject)
            .into_iter()
            .flat_map(|f| f.names.iter())
            .find(|n| n.witness == owner && n.kind == NameKind::Relative)
        {
            return held.name.clone();
        }
        let placed: Vec<(String, f64)> = self
            .members(star)
            .filter(|(s, _)| *s != subject && matches!(s, Subject::Body { .. }))
            .filter_map(|(_, file)| {
                // The letter, not whatever name wins: a renamed planet still holds its place.
                let letter = file.names.iter().find(|n| n.witness == owner && n.kind == NameKind::Relative)?;
                Some((letter.name.clone(), file.orbit(owner)?.semi_major_au.0))
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
        if self.is_full() {
            self.unkept += 1;
            return;
        }
        let file = self.files.entry(subject).or_default();
        let added = match file.series.iter_mut().find(|s| s.witness == witness && s.band == band) {
            Some(series) => series.push(sample),
            None => {
                let mut series = Series::new(witness, band);
                let added = series.push(sample);
                file.series.push(series);
                // A new series is a new header in the file, as well as a sample in the log.
                self.changed.insert(subject);
                added
            }
        };
        // Room is charged for what was kept, and a sample out of order is not.
        if added {
            self.charge_sample();
            self.sampled.touch(subject, sample.observed_s);
            self.unread.insert(subject);
            let learned_s = sample.observed_s;
            self.unsaved.push(Logged { subject, witness, band, sample, learned_s });
        }
    }

    /// Photometry this craft took itself, for one subject and band.
    pub fn own_series(&self, subject: impl Into<Subject>, band: Band) -> Option<&Series> {
        let file = self.files.get(&subject.into())?;
        file.series.iter().find(|s| s.witness == self.owner && s.band == band)
    }

    fn file_sighting(&mut self, subject: Subject, sighting: Sighting) {
        let witness = sighting.witness;
        let owner = self.owner;
        let file = self.files.entry(subject).or_default();
        if file.sightings.iter().any(|s| s.same_as(&sighting)) {
            return;
        }
        // A craft's own first detection gets a designation, so the log has something to call it.
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
        if let Some(file) = self.files.get(&subject) {
            self.backlog.file(subject, file);
        }
        if let Some(belief) = self.files.get(&subject).and_then(|f| f.believe(subject, self.owner)) {
            self.beliefs.insert(subject, belief);
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;

    const AU_LY: f64 = 1.581_250_7e-5;

    fn star_id(key: u64) -> StarId {
        StarId::synthesize("test", key)
    }

    /// An orbit stated at `au`, with the period a Sun-like host gives it. Nothing here is
    /// testing the elements, only what the letters do with the distance.
    fn at_au(au: f64, stated_s: f64) -> Orbit {
        let a_m = au * crate::navigation::AU;
        let period = em_foundations::kepler::period::third_law(a_m, crate::star::Star::SOL.mu);
        Orbit {
            about: None,
            witness: Witness(0),
            period_s: (period, period * 1.0e-3),
            semi_major_au: (au, au * 1.0e-2),
            eccentricity: None,
            orientation: Orientation::Unknown,
            epoch_s: None,
            method: Method::Transit,
            stated_s,
            lineage: Lineage::new(),
        }
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
            size: None,
            range_m: None,
            spin_s: None,
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

    /// Past the reservoir the closest looks go, not the oldest.
    #[test]
    fn the_reservoir_drops_the_closest_look_not_the_oldest() {
        let mut k = Knowledge::new(Witness(1));
        let star = star_id(2);
        let truth = DVec3::new(0.0, 0.0, 6.0);
        // The first look is from far away; every later one from nearly the same place.
        let far = DVec3::X * AU_LY * 10.0;
        k.sighted(star, sighting(1, far, truth - far, 0.0));
        for i in 1..=(BEARINGS_KEPT as u64 + 4) {
            let at = DVec3::Y * AU_LY * (1.0 + i as f64 * 1e-3);
            k.sighted(star, sighting(1, at, truth - at, i as f64 * 1e6));
        }
        let kept = k.file(star).unwrap().sightings();
        assert_eq!(kept.len(), BEARINGS_KEPT);
        assert!(kept.iter().any(|s| s.observed_s == 0.0), "the oldest look, and the widest, is kept");
    }

    /// A craft parked in a system moves nowhere, so every pair of its looks is tied at zero
    /// parallax and the arc it spent months collecting was thrown away from the middle out.
    /// Time is a baseline too.
    #[test]
    fn a_parked_craft_keeps_the_span_of_its_watch() {
        let mut k = Knowledge::new(Witness(1));
        let star = star_id(3);
        let truth = DVec3::new(0.0, 0.0, 6.0);
        let at = DVec3::X * AU_LY;
        let last = (BEARINGS_KEPT as u64 + 40) as f64 * 1.0e6;
        for i in 0..=(BEARINGS_KEPT as u64 + 40) {
            k.sighted(star, sighting(1, at, truth - at, i as f64 * 1.0e6));
        }
        let kept = k.file(star).unwrap().sightings();
        assert_eq!(kept.len(), BEARINGS_KEPT);

        let times: Vec<f64> = kept.iter().map(|s| s.observed_s).collect();
        let (oldest, newest) = (
            times.iter().cloned().fold(f64::INFINITY, f64::min),
            times.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
        assert_eq!(newest, last, "the newest look is what the display reads");
        assert_eq!(oldest, 0.0, "the watch's own span was decimated away");
        // And spread across it, not clustered at one end.
        let middle = times.iter().filter(|t| (last * 0.25..last * 0.75).contains(t)).count();
        assert!(middle >= 4, "only {middle} of {BEARINGS_KEPT} in the middle half: {times:?}");
    }

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
    fn a_report_carries_what_was_learned_and_gains_a_hop() {
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

        let report = probe.report(Mark::default(), 1.0e7);
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
            belief.learned_s, 1.2e8,
            "learned when the light landed, not when taken"
        );
        assert!(matches!(belief.distance, Distance::Measured { .. }));
        assert!(ship.file(star).unwrap().series_in(Band::V).is_none(), "and none of its log");
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
        let report = probe.report(Mark::default(), 1.0e7);

        let mut relay = Knowledge::new(Witness(3));
        relay.receive(&report, 2.0e7);
        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&report, 3.0e7);
        ship.receive(&relay.report(Mark::default(), 2.5e7), 4.0e7);

        let file = ship.file(star).unwrap();
        assert_eq!(file.sightings().len(), 4, "not eight");
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
        here.receive(&there.report(Mark::default(), 2.0), 3.0);

        let belief = here.belief(star).unwrap();
        assert_eq!(belief.witnesses, 2);
        let Distance::Measured { position_ly, .. } = belief.distance else {
            panic!("a four light-year baseline is a parallax of a degree at 400 ly");
        };
        assert!(position_ly.distance(truth) < 1.0, "{position_ly}");
    }

    /// One look of your own does not overturn a claim, but a parallax does.
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
        ship.receive(&office.report(Mark::default(), 10.0), 20.0);
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

    /// A first sighting writes down a designation, beaten by any chosen name.
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

    /// Your own name wins on your own screen; theirs stays on file with its witness.
    #[test]
    fn a_name_is_something_somebody_said_and_carries_who_said_it() {
        let star = star_id(21);
        let mut theirs = Knowledge::new(Witness(2));
        theirs.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        theirs.name_it(star, "Hearthlight", 10.0);

        let mut ours = Knowledge::new(Witness(1));
        ours.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 5.0));
        ours.receive(&theirs.report(Mark::default(), 20.0), 30.0);

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
        // Each crew's designation and each crew's chosen name.
        assert_eq!(ours.file(star).unwrap().names().len(), 4);
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

    fn planet(star: StarId, key: &str) -> (BodyId, Subject) {
        let body = BodyId::of(star, key);
        (body, Subject::Body { star, body })
    }

    /// A planet is named after whatever this craft calls its star.
    #[test]
    fn a_planet_is_read_after_this_craft_name_for_its_star() {
        let star = star_id(40);
        let mut k = Knowledge::new(Witness(1));
        k.name_it(star, "Kettle", 0.0);
        let (body, subject) = planet(star, "one");
        assert_eq!(k.found_planet(star, body, at_au(0.06, 1.0), 1.0, 1.0), "b");
        assert_eq!(k.name_of(subject).as_deref(), Some("Kettle b"));

        k.name_it(star, "The Kettle", 2.0);
        assert_eq!(k.name_of(subject).as_deref(), Some("The Kettle b"), "it follows the star");

        k.name_it(subject, "Spout", 3.0);
        assert_eq!(k.name_of(subject).as_deref(), Some("Spout"), "until somebody names it");
    }

    #[test]
    fn a_letter_once_assigned_is_never_reassigned() {
        let star = star_id(41);
        let mut k = Knowledge::new(Witness(1));
        let (body, subject) = planet(star, "one");
        assert_eq!(k.found_planet(star, body, at_au(0.06, 0.0), 1.0, 0.0), "b");
        assert_eq!(k.found_planet(star, body, at_au(4.3, 5.0), 1.0, 5.0), "b");
        assert_eq!(k.file(subject).unwrap().orbits()[0].semi_major_au.0, 4.3, "the orbit is updated");
    }

    #[test]
    fn letters_this_craft_already_uses_leave_room_for_the_next() {
        let star = star_id(42);
        let mut k = Knowledge::new(Witness(1));
        let (outer, _) = planet(star, "outer");
        let (inner, _) = planet(star, "inner");
        let (between, _) = planet(star, "between");
        assert_eq!(k.found_planet(star, outer, at_au(0.09, 0.0), 1.0, 0.0), "c");
        assert_eq!(k.found_planet(star, inner, at_au(0.06, 1.0), 1.0, 1.0), "b");
        assert_eq!(k.found_planet(star, between, at_au(0.07, 2.0), 1.0, 2.0), "bb", "no single letter left");
    }

    #[test]
    fn a_report_carries_a_system_as_one_entry() {
        let star = star_id(43);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        probe.name_it(star, "Kettle", 1.0);
        for (key, a) in [("one", 0.06), ("two", 0.09), ("three", 0.135)] {
            let (body, _) = planet(star, key);
            probe.found_planet(star, body, at_au(a, 2.0), 1.0, 2.0);
        }
        let report = probe.report(Mark::default(), 3.0);
        assert_eq!(report.stars(), 1, "one system, not four entries");
        assert_eq!(report.entries[0].parts.len(), 4, "the star and its three planets");

        // Capped by systems: a second star's system does not split the first.
        let other = star_id(44);
        probe.sighted(other, sighting(2, DVec3::ZERO, DVec3::Y, 10.0));
        let (first, _) = probe.report_upto(Mark::default(), 11.0, 1);
        assert_eq!(first.stars(), 1);
        assert_eq!(first.entries[0].parts.len(), 4);
    }

    /// **A digest that never leaves the craft is not a digest.** `Colors` was folded, stored
    /// and read locally and was in no report, so a probe's months of photometry died with it
    /// and a ship it talked to learned nothing about what color anything was.
    #[test]
    fn a_report_carries_what_a_probe_measured_of_a_body() {
        use em_spectra::{Band, PerBand};

        let star = star_id(63);
        let (body, subject) = planet(star, "one");
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));

        let mut row = PerBand::splat(None);
        row[Band::V] = Some((1.0e-12, 1.0e-15));
        row[Band::R] = Some((2.0e-12, 2.0e-15));
        for visit in 0..6 {
            probe.measured_colors(subject, Witness(2), visit as f64, &row);
        }
        let mine = probe.file(subject).unwrap().colors()[0].color(Band::R, Band::V).unwrap();

        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&probe.report(Mark::default(), 100.0), 200.0);

        let held = ship.file(subject).expect("the body came across").colors();
        assert_eq!(held.len(), 1, "one digest, on the witness that folded it");
        assert_eq!(held[0].witness, Witness(2), "and it stays the probe's measurement");
        assert_eq!(held[0].lineage.len(), 1, "carried one hop");
        let theirs = held[0].color(Band::R, Band::V).unwrap();
        assert!((theirs.0 / mine.0 - 1.0).abs() < 1.0e-12, "{theirs:?} against {mine:?}");

        // The belief a panel reads is the one that arrived.
        let belief = ship.body_belief(star, body, 300.0).expect("a belief about it");
        assert!(belief.colors.is_some(), "and the digest is what a type hypothesis reads");

        // Absorbed again with nothing new in it, the longer run is kept rather than doubled.
        ship.receive(&probe.report(Mark::default(), 400.0), 500.0);
        assert_eq!(ship.file(subject).unwrap().colors().len(), 1);
    }

    /// A receiver reads another craft's planets after its own name for the star.
    #[test]
    fn a_receiver_reads_somebody_else_planets_after_its_own_star_name() {
        let star = star_id(45);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 0.0));
        probe.name_it(star, "Kettle", 1.0);
        let (body, subject) = planet(star, "one");
        probe.found_planet(star, body, at_au(0.09, 2.0), 1.0, 2.0);
        assert_eq!(probe.name_of(subject).as_deref(), Some("Kettle c"));

        let mut ship = Knowledge::new(Witness(1));
        ship.sighted(star, sighting(1, DVec3::Y * 0.1, DVec3::X, 0.5));
        ship.name_it(star, "Hearthlight", 0.6);
        ship.receive(&probe.report(Mark::default(), 3.0), 10.0);

        assert_eq!(ship.name_of(star).as_deref(), Some("Hearthlight"), "ours, not theirs");
        assert_eq!(ship.name_of(subject).as_deref(), Some("Hearthlight c"));
        let orbit = &ship.file(subject).unwrap().orbits()[0];
        assert_eq!(orbit.witness, Witness(2), "the orbit is still the probe's statement");
        assert_eq!(orbit.lineage.len(), 1);

        ship.name_it(star, "Home", 11.0);
        assert_eq!(ship.name_of(subject).as_deref(), Some("Home c"));

        // And its own later find is lettered around the letter it was told.
        let (inner, inner_subject) = planet(star, "inner");
        assert_eq!(ship.found_planet(star, inner, at_au(0.06, 12.0), 1.0, 12.0), "b");
        assert_eq!(ship.name_of(inner_subject).as_deref(), Some("Home b"));
    }

    #[test]
    fn a_planet_nobody_named_the_star_of_still_has_a_name() {
        let star = star_id(46);
        let mut k = Knowledge::new(Witness(1));
        let (body, subject) = planet(star, "one");
        k.found_planet(star, body, at_au(0.06, 0.0), 1.0, 0.0);
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
        k.found_planet(star, body, at_au(0.06, 1.0), 1.0, 1.0);
        assert_eq!(k.len(), 2, "the star and its planet");
        assert_eq!(k.stars().count(), 1);
        assert_eq!(k.members(star).map(|(s, _)| s).collect::<Vec<_>>(), vec![subject]);
    }

    /// A replica's reports add no hop, and its samples travel separately, only the new ones.
    #[test]
    fn a_replica_absorbs_without_a_hop_and_only_new_samples_travel() {
        let star = star_id(60);
        let mut held = Knowledge::new(Witness(3));
        held.sighted(star, sighting(3, DVec3::ZERO, DVec3::X, 1.0));
        for t in 1..=5 {
            held.measured(star, Witness(3), Band::V, Sample { observed_s: t as f64, deficit: 0.0, sigma: 0.1 });
        }
        let mut copy = Knowledge::new(Witness(3));
        let first = held.report(Mark::default(), 5.0);
        copy.absorb(&first);
        assert_eq!(copy.belief(star).unwrap().hops, 0, "its own, not relayed");
        assert!(copy.own_series(star, Band::V).is_none(), "a report carries no samples");
        let (logs, through) = held.logs_upto(f64::NEG_INFINITY, usize::MAX);
        copy.copy_logs(&logs);
        assert_eq!(copy.own_series(star, Band::V).unwrap().len(), 5);

        held.measured(star, Witness(3), Band::V, Sample { observed_s: 6.0, deficit: 0.0, sigma: 0.1 });
        let (delta, _) = held.logs_upto(through.unwrap(), usize::MAX);
        assert_eq!(delta.iter().map(|l| l.samples.len()).sum::<usize>(), 1, "one new sample, not the whole curve again");
        copy.copy_logs(&delta);
        assert_eq!(copy.own_series(star, Band::V).unwrap().len(), 6);
        assert_eq!(copy.file(star), held.file(star), "and the copy is the original");
    }

    /// Files without samples plus samples as log rows rebuild the same knowledge.
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
        k.found_planet(star, body, at_au(0.06, 2.0), 1.0, 2.0);
        for t in 10..=14 {
            k.measured(star, Witness(1), Band::K, Sample { observed_s: t as f64, deficit: 0.0, sigma: 0.01 });
        }
        k.receive(&probe.report(Mark::default(), 4.0), 9.0);

        let (files, logs) = k.take_changes();
        assert_eq!(files.len(), 2, "the star and its planet");
        assert!(files.iter().all(|(_, f)| f.series().iter().all(|s| s.is_empty())), "samples are logged apart");
        assert_eq!(logs.len(), 5, "our five; the probe's log stayed with the probe");

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
        assert_eq!(marks.since(7), Mark::default());
        marks.sent(7, Mark::through(10.0));
        marks.sent(7, Mark::through(5.0));
        assert_eq!(marks.since(7), Mark::through(10.0), "a mark never moves back");
        assert_eq!(marks.since(0), Mark::default(), "a broadcast is its own backlog");
    }

    #[test]
    fn a_report_only_carries_what_is_new() {
        let star = star_id(3);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 100.0));
        assert!(probe.report(Mark::through(200.0), 300.0).is_empty(), "nothing since then");
        assert_eq!(probe.report(Mark::through(50.0), 300.0).stars(), 1);
        assert!(probe.report(Mark::default(), 99.0).is_empty(), "nor anything learned after it was sent");
    }

    /// Oldest first, resumed where the last report ended, and pages through ties at one instant.
    #[test]
    fn a_capped_report_drains_the_backlog_in_order() {
        let mut probe = Knowledge::new(Witness(2));
        for k in 0..10u64 {
            probe.sighted(star_id(k), sighting(2, DVec3::ZERO, DVec3::X, k as f64));
        }
        let (first, through) = probe.report_upto(Mark::default(), 100.0, 4);
        assert_eq!(first.stars(), 4);
        assert_eq!(first.learned_through(), Some(3.0), "the four oldest");

        let (second, through) = probe.report_upto(through.unwrap(), 200.0, 4);
        assert_eq!(second.stars(), 4);
        assert_eq!(second.learned_through(), Some(7.0));
        assert_eq!(probe.report_upto(through.unwrap(), 300.0, 4).0.stars(), 2, "and the rest");

        // The mark carries the system it stopped at as well as the time.
        let mut tied = Knowledge::new(Witness(2));
        for k in 0..10u64 {
            tied.sighted(star_id(k), sighting(2, DVec3::ZERO, DVec3::X, 5.0));
        }
        let mut mark = Mark::default();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let (page, next) = tied.report_upto(mark, 100.0, 4);
            assert!(page.stars() <= 4);
            seen.extend(page.entries.iter().map(|e| e.system));
            mark = next.unwrap();
        }
        assert_eq!(seen.len(), 10, "all ten, over three pages");
        assert!(tied.report_upto(mark, 100.0, 4).1.is_none(), "and then nothing");
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

    /// A star named relative to itself reads as a name rather than overflowing the stack.
    #[test]
    fn a_hostile_relative_name_reads_as_something() {
        let star = star_id(99);
        let hostile = Report {
            from: Witness(9),
            sent_s: 1.0,
            entries: vec![Entry {
                system: Subject::Star(star),
                parts: vec![Part {
                    subject: Subject::Star(star),
                    sightings: vec![sighting(9, DVec3::ZERO, DVec3::X, 0.0)],
                    claims: Vec::new(),
                    names: vec![Naming { witness: Witness(9), name: "b".into(), kind: NameKind::Relative, stated_s: 1.0, lineage: Vec::new() }],
                    orbits: Vec::new(),
                    conclusions: Vec::new(),
                    colors: Vec::new(),
                }],
            }],
        };
        let mut ship = Knowledge::new(Witness(1));
        ship.receive(&hostile, 3.0);
        let name = ship.name_of(star).expect("a name");
        assert_eq!(name.matches(" b").count(), NAME_DEPTH, "{name}");
    }

    /// A refined orbit does not bring the letter back over a chosen name, and the letter stays taken.
    #[test]
    fn a_chosen_name_always_wins_and_keeps_its_letter() {
        let star = star_id(46);
        let mut k = Knowledge::new(Witness(1));
        k.sighted(star, sighting(1, DVec3::ZERO, DVec3::X, 0.0));
        let (inner, _) = planet(star, "inner");
        assert_eq!(k.found_planet(star, inner, at_au(0.06, 1.0), 1.0, 1.0), "b");
        k.name_it(Subject::Body { star, body: inner }, "Spout", 2.0);
        assert_eq!(k.found_planet(star, inner, at_au(0.061, 3.0), 1.0, 3.0), "b", "the letter is frozen");
        assert_eq!(k.name_of(Subject::Body { star, body: inner }).as_deref(), Some("Spout"), "and the name still wins");
        let (next, _) = planet(star, "next");
        assert_eq!(k.found_planet(star, next, at_au(0.062, 4.0), 1.0, 4.0), "bb", "b is still taken");
    }
}
