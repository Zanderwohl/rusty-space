//! How much room what a craft knows takes, and what happens when there is none left.
//!
//! Only raw logs take room. Bearings, names, claims, conclusions and the digests of consumed logs
//! are small next to a log and bounded per subject, and charging for them left a craft that had
//! surveyed the sky full for good, with reading its logs making it worse. A sample is a fixed
//! game unit, near enough what postcard writes. Files are never dropped for room; what stops
//! when a craft is full is its logs. See `lightcone/docs/24-standing-instruments.md`.

use super::{File, Knowledge, SAMPLE_BYTES};

impl File {
    /// Room this file takes aboard: its raw samples, whoever took them.
    pub fn bytes(&self) -> f64 {
        self.series.iter().map(|s| s.len() as f64 * SAMPLE_BYTES).sum()
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
