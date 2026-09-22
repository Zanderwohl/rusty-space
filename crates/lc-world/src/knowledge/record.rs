//! The records a craft holds: who measured what, when, and how it got here.
//!
//! Nothing here folds records into a belief; that is [`super::Knowledge`]. See
//! `lightcone/docs/22-provenance.md`.

use em_spectra::Band;
use serde::{Deserialize, Serialize};

use super::astrometry::{Bearing, Distance};

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
    /// Coordinate seconds its light landed: at least `sent_s` plus the light time between them.
    pub received_s: f64,
}

/// The route a measurement took, oldest hop first. Empty for one this craft made itself.
pub type Lineage = Vec<Hop>;

/// When the holder learned something measured at `observed_s`.
pub fn learned_s(lineage: &Lineage, observed_s: f64) -> f64 {
    lineage.last().map(|h| h.received_s).unwrap_or(observed_s)
}

/// One detection of a star.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sighting {
    pub witness: Witness,
    /// Coordinate seconds the light arrived, not when it left.
    pub observed_s: f64,
    pub bearing: Bearing,
    pub band: Band,
    /// Flux in `band`, W/m^2.
    pub flux: f64,
    pub flux_sigma: f64,
    pub lineage: Lineage,
}

impl Sighting {
    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.observed_s)
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
/// Kept per witness because two observers at different distances see different epochs of the
/// same star. Never sent to another craft: what a log says travels as a conclusion. Nothing
/// bounds it but the room aboard.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Series {
    pub witness: Witness,
    pub band: Band,
    samples: Vec<Sample>,
    /// Everything observed at or before this was consumed and is refused however it arrives.
    consumed_s: f64,
}

impl Series {
    pub fn new(witness: Witness, band: Band) -> Self {
        Self {
            witness,
            band,
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

    /// False if it was not kept: older than the last one held, or already consumed.
    pub fn push(&mut self, sample: Sample) -> bool {
        if sample.observed_s <= self.consumed_s
            || self
                .samples
                .last()
                .is_some_and(|s| sample.observed_s < s.observed_s)
        {
            return false;
        }
        self.samples.push(sample);
        true
    }

    /// Drop every sample observed at or before `through_s`, for good.
    pub fn consume_through(&mut self, through_s: f64) {
        self.samples.retain(|s| s.observed_s > through_s);
        self.consumed_s = self.consumed_s.max(through_s);
    }

    pub fn consumed_s(&self) -> f64 {
        self.consumed_s
    }

    /// What a stored file holds; the samples go to a log of their own. See
    /// `lightcone/docs/24-standing-instruments.md`.
    pub fn emptied(&self) -> Series {
        Series { samples: Vec::new(), ..self.clone() }
    }

    /// Emission times and deficits, for a plot. Without a light age the samples are against
    /// arrival time, and the caller must say so.
    pub fn against_emission(&self, light_age_s: Option<f64>) -> Vec<(f64, f64)> {
        let shift = light_age_s.unwrap_or(0.0);
        self.samples
            .iter()
            .map(|s| (s.observed_s - shift, s.deficit))
            .collect()
    }
}

/// What somebody calls something. A name travels like every other record, and two crews may
/// hold different names for the same light.
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
    /// What an instrument wrote down when it found it. Always beaten by a name somebody chose.
    Designation,
    /// Assigned, and read after whatever the holder calls the subject's star. Ranked as a
    /// designation.
    Relative,
}

impl NameKind {
    pub fn chosen(self) -> bool {
        matches!(self, Self::Given)
    }
}

/// A distance somebody states, for when the measurements behind it do not travel. A craft's own
/// triangulation overrides it.
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
/// A statement, like a [`Claim`], because a craft outside a system infers an orbit from a period
/// or is told one. Letters are assigned against these.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Orbit {
    pub witness: Witness,
    pub semi_major_au: f64,
    pub stated_s: f64,
    pub lineage: Lineage,
}
