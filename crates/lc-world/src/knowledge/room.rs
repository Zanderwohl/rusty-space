//! How much room what a craft knows takes, and what happens when there is none left.
//!
//! Bytes here are a game unit: a fixed size per record, near enough what postcard writes, so
//! the count is cheap and does not move when a record's encoding does. Files are never dropped
//! for room — losing what you know because you learned something else would be a worse
//! mechanic than refusing to learn more — so what stops when a craft is full is its logs. See
//! `lightcone/docs/24-standing-instruments.md`.

use super::transit::BINS;
use super::{File, Knowledge, SAMPLE_BYTES};

const FILE_BYTES: f64 = 32.0;
const SIGHTING_BYTES: f64 = 96.0;
const NAMING_BYTES: f64 = 48.0;
const CLAIM_BYTES: f64 = 64.0;
const ORBIT_BYTES: f64 = 40.0;
const SERIES_BYTES: f64 = 32.0;
const HOP_BYTES: f64 = 32.0;
const CONCLUSION_BYTES: f64 = 320.0;
/// A digest's header and its population moments.
const DIGEST_BYTES: f64 = 1024.0;
const FOLD_BYTES: f64 = BINS as f64 * 16.0 + 48.0;

impl File {
    /// Room this file takes aboard.
    pub fn bytes(&self) -> f64 {
        let hops = |n: usize| n as f64 * HOP_BYTES;
        let sightings: f64 = self.sightings.iter().map(|s| SIGHTING_BYTES + hops(s.lineage.len())).sum();
        let names: f64 = self.names.iter().map(|n| NAMING_BYTES + n.name.len() as f64 + hops(n.lineage.len())).sum();
        let claims: f64 = self.claims.iter().map(|c| CLAIM_BYTES + hops(c.lineage.len())).sum();
        let orbits: f64 = self.orbits.iter().map(|o| ORBIT_BYTES + hops(o.lineage.len())).sum();
        let series: f64 = self
            .series
            .iter()
            .map(|s| SERIES_BYTES + s.len() as f64 * SAMPLE_BYTES)
            .sum();
        let conclusions: f64 = self.conclusions.iter().map(|c| CONCLUSION_BYTES + hops(c.lineage.len())).sum();
        let digests: f64 = self
            .digests
            .iter()
            .map(|d| DIGEST_BYTES + d.planet.as_ref().map_or(0, |p| p.folds.len()) as f64 * FOLD_BYTES)
            .sum();
        FILE_BYTES + sightings + names + claims + orbits + series + conclusions + digests
    }
}

impl Knowledge {
    /// Set how much room this craft has, and count what it is using. The count is kept up to
    /// date as samples arrive and recounted here, so call it whenever the capacity may have
    /// changed or a log may have been consumed — a shard does, every tick it runs the craft.
    pub fn fit_to(&mut self, capacity_bytes: f64) {
        self.capacity_bytes = capacity_bytes;
        self.occupied_bytes = self.files.values().map(File::bytes).sum();
    }

    /// Room everything held takes, counted now. What a display shows; [`Knowledge::fit_to`] is
    /// what enforces it.
    pub fn bytes(&self) -> f64 {
        self.files.values().map(File::bytes).sum()
    }

    pub fn capacity_bytes(&self) -> f64 {
        self.capacity_bytes
    }

    pub fn occupied_bytes(&self) -> f64 {
        self.occupied_bytes
    }

    /// Whether there is no room left for another sample.
    pub fn is_full(&self) -> bool {
        self.occupied_bytes + SAMPLE_BYTES > self.capacity_bytes
    }

    /// Samples measured or received and not kept, for want of room, since this craft started.
    pub fn unkept(&self) -> u64 {
        self.unkept
    }

    /// Count one kept sample against the room.
    pub(super) fn charge_sample(&mut self) {
        self.occupied_bytes += SAMPLE_BYTES;
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::Band;

    use super::*;
    use crate::knowledge::{Sample, Subject, Witness};
    use crate::sky::StarId;

    #[test]
    fn a_full_craft_stops_keeping_samples_and_keeps_everything_else() {
        let star = Subject::Star(StarId::synthesise("room", 1));
        let mut k = Knowledge::new(Witness(1));
        k.name_it(star, "Kettle", 0.0);
        k.fit_to(f64::INFINITY);
        // The running count charges samples only; a new series' header is noticed at the next
        // recount, which here finds the craft over.
        k.fit_to(k.occupied_bytes() + 11.0 * SAMPLE_BYTES);
        for n in 0..20 {
            k.measured(star, Witness(1), Band::V, Sample { observed_s: n as f64, deficit: 0.0, sigma: 1e-5 });
        }
        k.fit_to(k.capacity_bytes());
        assert_eq!(k.own_series(star, Band::V).unwrap().len(), 11);
        assert_eq!(k.unkept(), 9);
        assert!(k.is_full());

        k.name_it(star, "Hearth", 1.0);
        assert_eq!(k.name_of(star).as_deref(), Some("Hearth"), "a full craft still learns names");
    }
}
