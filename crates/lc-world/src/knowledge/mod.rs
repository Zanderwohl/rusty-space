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
pub mod survey;

pub use astrometry::{Bearing, Distance};

/// How many bearings per star per witness are kept.
///
/// Distance comes from the spread of the observing positions, so the reservoir keeps the
/// widest spread rather than the most recent: dropping the closest pair costs the least
/// baseline. Sixteen well-spread bearings measure a parallax as well as a thousand.
pub const BEARINGS_KEPT: usize = 16;

/// Photometric samples kept per star, per witness, per band.
pub const SAMPLES_KEPT: usize = 4000;

/// Whoever took a measurement: a ship, a probe, a telescope.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Witness(pub u64);

/// One handover of a measurement from whoever held it to whoever holds it now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    pub from: Witness,
    pub to: Witness,
    /// Coordinate seconds the report was transmitted.
    pub sent_s: f64,
    /// Coordinate seconds its light landed. Never earlier than `sent_s` by more than the
    /// distance between the two, because that is what carried it.
    pub received_s: f64,
}

/// The route a measurement took, oldest hop first. Empty for one this craft made itself.
pub type Lineage = Vec<Hop>;

/// When the holder learnt something measured at `observed_s`.
pub fn learnt_s(lineage: &Lineage, observed_s: f64) -> f64 {
    lineage.last().map(|h| h.received_s).unwrap_or(observed_s)
}

/// One detection of a star: which way, how bright, from where, by whom.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sighting {
    pub witness: Witness,
    /// Coordinate seconds the light arrived. The light *left* `distance` years earlier, which
    /// is not known until the distance is.
    pub observed_s: f64,
    pub bearing: Bearing,
    pub band: Band,
    /// Flux in `band`, W/m^2, as measured. Luminosity only follows once distance does.
    pub flux: f64,
    pub flux_sigma: f64,
    pub lineage: Lineage,
}

impl Sighting {
    pub fn learnt_s(&self) -> f64 {
        learnt_s(&self.lineage, self.observed_s)
    }

    fn same_as(&self, other: &Self) -> bool {
        self.witness == other.witness && self.observed_s.to_bits() == other.observed_s.to_bits()
    }
}

/// One photometric measurement in a series.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub observed_s: f64,
    /// Signed fractional change against the bare star: positive is a deficit.
    pub deficit: f64,
    pub sigma: f64,
}

/// A run of photometry on one star, in one band, by one witness.
///
/// Kept per witness because merging two observers' curves would splice series taken at
/// different distances — and therefore of different epochs of the same star.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Series {
    pub witness: Witness,
    pub band: Band,
    pub lineage: Lineage,
    samples: Vec<Sample>,
}

impl Series {
    pub fn new(witness: Witness, band: Band) -> Self {
        Self {
            witness,
            band,
            lineage: Lineage::new(),
            samples: Vec::new(),
        }
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn last(&self) -> Option<&Sample> {
        self.samples.last()
    }

    pub fn push(&mut self, sample: Sample) {
        if self
            .samples
            .last()
            .is_some_and(|s| sample.observed_s < s.observed_s)
        {
            return;
        }
        if self.samples.len() >= SAMPLES_KEPT {
            self.samples.remove(0);
        }
        self.samples.push(sample);
    }

    /// Take whatever `other` has that this does not.
    ///
    /// Arrival order is the witness's own, so "later than the last held" is the whole test: a
    /// series only ever grows at its end, and the same run arriving twice by two routes adds
    /// nothing the second time.
    pub fn absorb(&mut self, other: &Series) {
        let from = self
            .samples
            .last()
            .map(|s| s.observed_s)
            .unwrap_or(f64::NEG_INFINITY);
        for sample in other.samples.iter().filter(|s| s.observed_s > from) {
            self.push(*sample);
        }
    }

    /// Emission times and deficits, for a plot.
    ///
    /// The x axis is when the light *left*, which needs a distance; without one the samples
    /// are returned against arrival time and the caller says so.
    pub fn against_emission(&self, light_age_s: Option<f64>) -> Vec<(f64, f64)> {
        let shift = light_age_s.unwrap_or(0.0);
        self.samples
            .iter()
            .map(|s| (s.observed_s - shift, s.deficit))
            .collect()
    }
}

/// What somebody calls a star.
///
/// **Nothing has a name of its own.** A name is a thing an observer gave a star and may have
/// passed on, so it travels like every other record: with a witness, a time and a lineage, and
/// two crews may hold different names for the same light without either being wrong.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Naming {
    pub witness: Witness,
    pub name: String,
    pub kind: NameKind,
    pub stated_s: f64,
    pub lineage: Lineage,
}

/// Whether a name was chosen or merely assigned.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// Somebody decided to call it this.
    Given,
    /// What an instrument wrote down when it found it, so that the log has something to say.
    /// Always beaten by a name somebody chose.
    Designation,
}

/// A designation from the direction something was found in, ecliptic degrees.
///
/// Fixed at discovery rather than recomputed, because the bearing changes as the observer
/// moves and a catalogue number that drifted would be no use for talking about.
pub fn designation(toward: glam::DVec3) -> String {
    let toward = toward.normalize_or(glam::DVec3::X);
    let longitude = toward
        .y
        .atan2(toward.x)
        .rem_euclid(std::f64::consts::TAU)
        .to_degrees();
    let latitude = toward.z.clamp(-1.0, 1.0).asin().to_degrees();
    format!("{longitude:05.1}{latitude:+05.1}")
}

/// A distance somebody states, as opposed to bearings this craft can triangulate itself.
///
/// This is how a conclusion travels when the measurements behind it do not — a charting
/// office's parallax programme, a faction's shared catalogue, a probe with more data than
/// bandwidth. It is believed because of who said it, which is the honest way to hold it, and
/// a craft's own triangulation overrides it the moment it has one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Claim {
    pub witness: Witness,
    pub distance: Distance,
    /// Coordinate seconds the claimant stated it.
    pub stated_s: f64,
    pub lineage: Lineage,
}

fn sigma_of(claim: &Claim) -> f64 {
    match claim.distance {
        Distance::Measured { sigma_ly, .. } => sigma_ly,
        _ => f64::INFINITY,
    }
}

/// Which of two namings a craft goes by: a chosen name over a designation, its own over
/// somebody else's, and the more recent over the older.
fn better_name<'a>(held: Option<&'a Naming>, new: &'a Naming, owner: Witness) -> bool {
    let rank = |n: &Naming| {
        (
            matches!(n.kind, NameKind::Given),
            n.witness == owner,
            n.stated_s,
        )
    };
    match held {
        None => true,
        Some(held) => rank(new) > rank(held),
    }
}

/// What is believed about one star, folded from everything held about it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Belief {
    pub star: StarId,
    /// What this craft calls it, and who said so. `None` for something detected and not yet
    /// written down anywhere — which the interface shows as an unnamed source rather than
    /// inventing something.
    pub name: Option<Naming>,
    /// The most recent bearing, from wherever that witness was.
    pub bearing: Bearing,
    pub distance: Distance,
    /// Whether that distance is this craft's own triangulation rather than somebody's word.
    pub measured_here: bool,
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

/// Everything held about one star.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct StarFile {
    sightings: Vec<Sighting>,
    series: Vec<Series>,
    claims: Vec<Claim>,
    names: Vec<Naming>,
}

impl StarFile {
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

    pub fn series_in(&self, band: Band) -> Option<&Series> {
        self.series.iter().find(|s| s.band == band)
    }

    fn believe(&self, star: StarId, owner: Witness) -> Option<Belief> {
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
        let claimed = self
            .claims
            .iter()
            .min_by(|a, b| sigma_of(a).total_cmp(&sigma_of(b)));
        let mut name: Option<&Naming> = None;
        for naming in &self.names {
            if better_name(name, naming, owner) {
                name = Some(naming);
            }
        }
        Some(Belief {
            star,
            name: name.cloned(),
            bearing: latest.bearing,
            distance: match (taken, claimed) {
                (false, Some(claim)) => claim.distance,
                _ => measured,
            },
            measured_here: taken,
            band: latest.band,
            flux: latest.flux,
            observed_s: latest.observed_s,
            learnt_s: self
                .sightings
                .iter()
                .map(Sighting::learnt_s)
                .fold(f64::INFINITY, f64::min),
            sightings: self.sightings.len(),
            witnesses: witnesses.len(),
            hops: self
                .sightings
                .iter()
                .map(|s| s.lineage.len())
                .min()
                .unwrap_or(0),
        })
    }

    /// Keep the bearings that are farthest apart, which is what a parallax is made of.
    fn decimate(&mut self, witness: Witness) {
        while self
            .sightings
            .iter()
            .filter(|s| s.witness == witness)
            .count()
            > BEARINGS_KEPT
        {
            let held: Vec<usize> = (0..self.sightings.len())
                .filter(|i| self.sightings[*i].witness == witness)
                .collect();
            // Never the newest: it is what the display reads, and a curve of one stale
            // bearing is worse than a slightly narrower baseline.
            let newest = *held
                .iter()
                .max_by(|a, b| {
                    self.sightings[**a]
                        .observed_s
                        .total_cmp(&self.sightings[**b].observed_s)
                })
                .unwrap();
            let mut drop = (f64::INFINITY, held[0]);
            for (n, i) in held.iter().enumerate() {
                for j in held.iter().skip(n + 1) {
                    let gap = self.sightings[*i]
                        .bearing
                        .observer_ly
                        .distance(self.sightings[*j].bearing.observer_ly);
                    let older = if self.sightings[*i].observed_s < self.sightings[*j].observed_s {
                        *i
                    } else {
                        *j
                    };
                    let loser = if older == newest {
                        if *i == newest { *j } else { *i }
                    } else {
                        older
                    };
                    if gap < drop.0 {
                        drop = (gap, loser);
                    }
                }
            }
            self.sightings.remove(drop.1);
        }
    }
}

/// One craft's view of the sky.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Knowledge {
    pub owner: Witness,
    files: BTreeMap<StarId, StarFile>,
    beliefs: BTreeMap<StarId, Belief>,
}

impl Knowledge {
    pub fn new(owner: Witness) -> Self {
        Self {
            owner,
            files: BTreeMap::new(),
            beliefs: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn knows(&self, star: StarId) -> bool {
        self.files.contains_key(&star)
    }

    pub fn file(&self, star: StarId) -> Option<&StarFile> {
        self.files.get(&star)
    }

    pub fn belief(&self, star: StarId) -> Option<&Belief> {
        self.beliefs.get(&star)
    }

    pub fn beliefs(&self) -> impl Iterator<Item = &Belief> {
        self.beliefs.values()
    }

    /// File a sighting this craft made itself.
    pub fn sighted(&mut self, star: StarId, sighting: Sighting) {
        self.file_sighting(star, sighting);
    }

    /// File a distance somebody states. Nothing is taken on trust that a measurement of its
    /// own would not override.
    pub fn told(&mut self, star: StarId, claim: Claim) {
        let file = self.files.entry(star).or_default();
        match file.claims.iter_mut().find(|c| c.witness == claim.witness) {
            Some(held) if held.stated_s >= claim.stated_s => {}
            Some(held) => *held = claim,
            None => file.claims.push(claim),
        }
        self.refresh(star);
    }

    /// File what somebody calls a star. One name per witness: renaming is stating a new one,
    /// and the later statement is what that witness calls it now.
    pub fn named(&mut self, star: StarId, naming: Naming) {
        let file = self.files.entry(star).or_default();
        match file.names.iter_mut().find(|n| n.witness == naming.witness) {
            Some(held) if held.stated_s > naming.stated_s => {}
            Some(held) => *held = naming,
            None => file.names.push(naming),
        }
        self.refresh(star);
    }

    /// Give a star this craft's own name for it.
    pub fn name_it(&mut self, star: StarId, name: impl Into<String>, now_s: f64) {
        let naming = Naming {
            witness: self.owner,
            name: name.into(),
            kind: NameKind::Given,
            stated_s: now_s,
            lineage: Lineage::new(),
        };
        self.named(star, naming);
    }

    /// File a photometric sample this craft measured itself.
    pub fn measured(&mut self, star: StarId, witness: Witness, band: Band, sample: Sample) {
        let file = self.files.entry(star).or_default();
        match file
            .series
            .iter_mut()
            .find(|s| s.witness == witness && s.band == band)
        {
            Some(series) => series.push(sample),
            None => {
                let mut series = Series::new(witness, band);
                series.push(sample);
                file.series.push(series);
            }
        }
    }

    /// Photometry this craft took itself, for one star and band.
    pub fn own_series(&self, star: StarId, band: Band) -> Option<&Series> {
        let file = self.files.get(&star)?;
        file.series
            .iter()
            .find(|s| s.witness == self.owner && s.band == band)
    }

    /// Everything learnt after `since_s`, ready to transmit.
    ///
    /// By what this craft *learnt* rather than by when it was measured: a report is a statement
    /// about what the sender has, and a decade-old sighting relayed yesterday is news.
    pub fn report(&self, since_s: f64, sent_s: f64) -> Report {
        let mut entries = Vec::new();
        for (star, file) in &self.files {
            let sightings: Vec<Sighting> = file
                .sightings
                .iter()
                .filter(|s| s.learnt_s() > since_s)
                .cloned()
                .collect();
            let series: Vec<Series> = file
                .series
                .iter()
                .filter(|s| {
                    s.last()
                        .is_some_and(|x| learnt_s(&s.lineage, x.observed_s) > since_s)
                })
                .cloned()
                .collect();
            let names: Vec<Naming> = file
                .names
                .iter()
                .filter(|n| learnt_s(&n.lineage, n.stated_s) > since_s)
                .cloned()
                .collect();
            let claims: Vec<Claim> = file
                .claims
                .iter()
                .filter(|c| learnt_s(&c.lineage, c.stated_s) > since_s)
                .cloned()
                .collect();
            if !sightings.is_empty()
                || !series.is_empty()
                || !claims.is_empty()
                || !names.is_empty()
            {
                entries.push(Entry {
                    star: *star,
                    sightings,
                    series,
                    claims,
                    names,
                });
            }
        }
        Report {
            from: self.owner,
            sent_s,
            entries,
        }
    }

    /// Fold in what somebody else sent, as of the moment its light landed.
    ///
    /// Every item gains a hop, so where it came from survives however far it is passed on, and
    /// something already held by a shorter route is not taken twice.
    pub fn receive(&mut self, report: &Report, received_s: f64) {
        let hop = Hop {
            from: report.from,
            to: self.owner,
            sent_s: report.sent_s,
            received_s,
        };
        for entry in &report.entries {
            for sighting in &entry.sightings {
                let mut sighting = sighting.clone();
                sighting.lineage.push(hop);
                self.file_sighting(entry.star, sighting);
            }
            for naming in &entry.names {
                let mut naming = naming.clone();
                naming.lineage.push(hop);
                self.named(entry.star, naming);
            }
            for claim in &entry.claims {
                let mut claim = claim.clone();
                claim.lineage.push(hop);
                self.told(entry.star, claim);
            }
            for series in &entry.series {
                let file = self.files.entry(entry.star).or_default();
                match file
                    .series
                    .iter_mut()
                    .find(|s| s.witness == series.witness && s.band == series.band)
                {
                    Some(held) => held.absorb(series),
                    None => {
                        let mut taken = series.clone();
                        taken.lineage.push(hop);
                        file.series.push(taken);
                    }
                }
            }
            self.refresh(entry.star);
        }
    }

    fn file_sighting(&mut self, star: StarId, sighting: Sighting) {
        let witness = sighting.witness;
        let owner = self.owner;
        let file = self.files.entry(star).or_default();
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
        self.refresh(star);
    }

    fn refresh(&mut self, star: StarId) {
        if let Some(belief) = self
            .files
            .get(&star)
            .and_then(|f| f.believe(star, self.owner))
        {
            self.beliefs.insert(star, belief);
        }
    }
}

/// Everything one report carries about one star.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub star: StarId,
    pub sightings: Vec<Sighting>,
    pub series: Vec<Series>,
    /// Conclusions rather than measurements: see [`Claim`].
    pub claims: Vec<Claim>,
    /// What the sender and whoever told them call it: see [`Naming`].
    pub names: Vec<Naming>,
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
        assert!(!belief.measured_here, "held on somebody's word");
        assert_eq!(
            belief.distance.position_ly(),
            Some(DVec3::new(0.0, 0.0, 9.0))
        );

        for s in looks(1, truth, 8, 1.0) {
            k.sighted(star, s);
        }
        let belief = k.belief(star).unwrap();
        assert!(belief.measured_here);
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

    #[test]
    fn a_report_only_carries_what_is_new() {
        let star = star_id(3);
        let mut probe = Knowledge::new(Witness(2));
        probe.sighted(star, sighting(2, DVec3::ZERO, DVec3::X, 100.0));
        assert!(probe.report(200.0, 300.0).is_empty(), "nothing since then");
        assert_eq!(probe.report(50.0, 300.0).stars(), 1);
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
