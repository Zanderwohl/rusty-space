//! The records a craft holds: who measured what, when, and how it got here.
//!
//! Each record carries a witness and a lineage, and nothing here folds them into a belief —
//! that is [`super::Knowledge`]'s job. See `lightcone/docs/22-provenance.md`.

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::astrometry::{Bearing, Distance};

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

    pub(crate) fn same_as(&self, other: &Self) -> bool {
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
    /// Everything observed at or before this was read and thrown away, and is not taken back
    /// however it arrives.
    consumed_s: f64,
}

impl Series {
    pub fn new(witness: Witness, band: Band) -> Self {
        Self {
            witness,
            band,
            lineage: Lineage::new(),
            samples: Vec::new(),
            consumed_s: f64::NEG_INFINITY,
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

    /// Add a sample at the end. False if it was older than the last one held, and so not added.
    pub fn push(&mut self, sample: Sample) -> bool {
        if sample.observed_s <= self.consumed_s
            || self
                .samples
                .last()
                .is_some_and(|s| sample.observed_s < s.observed_s)
        {
            return false;
        }
        if self.samples.len() >= SAMPLES_KEPT {
            self.samples.remove(0);
        }
        self.samples.push(sample);
        true
    }

    /// Take whatever `other` has that this does not.
    ///
    /// Arrival order is the witness's own, so "later than the last held" is the whole test: a
    /// series only ever grows at its end, and the same run arriving twice by two routes adds
    /// nothing the second time.
    ///
    /// Returns the samples it took, which is what has to be written down.
    pub fn absorb(&mut self, other: &Series) -> Vec<Sample> {
        let from = self
            .samples
            .last()
            .map(|s| s.observed_s)
            .unwrap_or(f64::NEG_INFINITY)
            .max(self.consumed_s);
        let taken: Vec<Sample> = other.samples.iter().filter(|s| s.observed_s > from).copied().collect();
        for sample in &taken {
            self.push(*sample);
        }
        taken
    }

    /// Drop every sample observed at or before `through_s`, for good.
    pub fn consume_through(&mut self, through_s: f64) {
        self.samples.retain(|s| s.observed_s > through_s);
        self.consumed_s = self.consumed_s.max(through_s);
    }

    pub fn consumed_s(&self) -> f64 {
        self.consumed_s
    }

    /// The series with its samples taken out: what a stored file holds, the samples being
    /// written to a log of their own. See `lightcone/docs/24-standing-instruments.md`.
    pub fn emptied(&self) -> Series {
        Series { samples: Vec::new(), ..self.clone() }
    }

    /// The part of this series learnt after `since_s`, or `None` if none of it was.
    ///
    /// A series this craft took itself is learnt sample by sample, so only the new samples go;
    /// one it was handed was learnt all at once, when it arrived, and goes whole or not at all.
    /// Sending a watched star's whole curve every time one sample was added would be most of
    /// what a report carried.
    pub fn after(&self, since_s: f64) -> Option<Series> {
        let samples: Vec<Sample> = if self.lineage.is_empty() {
            self.samples.iter().filter(|s| s.observed_s > since_s).copied().collect()
        } else if learnt_s(&self.lineage, 0.0) > since_s {
            self.samples.clone()
        } else {
            Vec::new()
        };
        (!samples.is_empty()).then(|| Series { samples, ..self.clone() })
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

/// What somebody calls something.
///
/// **Nothing has a name of its own.** A name is a thing an observer gave a star, a planet or a
/// craft and may have passed on, so it travels like every other record: with a witness, a time
/// and a lineage, and two crews may hold different names for the same light without either
/// being wrong.
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
    /// Assigned, and read after whatever the holder calls the subject's star: `b` on a planet
    /// is shown as "Kettle b" by a craft that calls the star the Kettle, and as "Hearthlight b"
    /// by one that calls it Hearthlight. Ranked as a designation.
    Relative,
}

impl NameKind {
    /// Whether somebody chose it, rather than a rule assigning it.
    pub fn chosen(self) -> bool {
        matches!(self, Self::Given)
    }
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

/// Where somebody says a body orbits: a semi-major axis about its star.
///
/// A statement, like a [`Claim`], because a craft outside a system does not watch its planets
/// go round — it infers an orbit from a period, or is told one. Letters are assigned against
/// these, so they travel with the system they describe.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Orbit {
    pub witness: Witness,
    pub semi_major_au: f64,
    pub stated_s: f64,
    pub lineage: Lineage,
}
