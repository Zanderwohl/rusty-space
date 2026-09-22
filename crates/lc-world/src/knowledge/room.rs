//! How much room what a craft knows takes, and what happens when there is none left.
//!
//! Only raw logs take room. Everything else is small and bounded per subject, and charging for
//! it would leave a craft that had surveyed the sky full for good. A sample is a fixed game unit,
//! near enough what postcard writes. A full craft stops keeping samples; files are never dropped.
//! See `lightcone/docs/24-standing-instruments.md`.

use super::{File, Knowledge, SAMPLE_BYTES};

impl File {
    /// Room this file takes aboard, bytes.
    pub fn bytes(&self) -> f64 {
        self.samples() as f64 * SAMPLE_BYTES
    }

    /// Counted, not summed as bytes: a float `sum` of nothing is -0.0, which displays as "-0.00".
    fn samples(&self) -> usize {
        self.series.iter().map(|s| s.len()).sum()
    }
}

impl Knowledge {
    /// Set the room and recount what is used. Call it whenever the capacity may have changed or a
    /// log may have been consumed; a shard does, every tick.
    pub fn fit_to(&mut self, capacity_bytes: f64) {
        self.capacity_bytes = capacity_bytes;
        self.occupied_bytes = self.bytes();
    }

    /// Counted now, for display; [`Knowledge::fit_to`] is what enforces it.
    pub fn bytes(&self) -> f64 {
        self.files.values().map(File::samples).sum::<usize>() as f64 * SAMPLE_BYTES
    }

    pub fn capacity_bytes(&self) -> f64 {
        self.capacity_bytes
    }

    pub fn occupied_bytes(&self) -> f64 {
        self.occupied_bytes
    }

    pub fn is_full(&self) -> bool {
        self.occupied_bytes + SAMPLE_BYTES > self.capacity_bytes
    }

    /// Samples measured or received and not kept, for want of room, since this craft started.
    pub fn unkept(&self) -> u64 {
        self.unkept
    }

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
    fn nothing_held_is_positive_zero() {
        let k = Knowledge::new(Witness(1));
        assert_eq!(k.bytes().to_bits(), 0.0f64.to_bits());
    }

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
