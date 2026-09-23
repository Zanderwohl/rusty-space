//! A file as it is written down.
//!
//! Postcard is positional and cannot notice an older shape, so a format number rides with every
//! stored file and anything but the current one is refused. See
//! `lightcone/docs/24-standing-instruments.md`.
//!
//! **Bump [`FILE_FORMAT`] when [`File`], or anything inside it, changes shape.** There are no
//! back-readers: the game has no players, so a stored file is worth less than the ceremony of
//! keeping it, and the readers that existed each embedded the records by name — growing `Orbit`
//! would have meant freezing a copy of the old shape beside every one of them. Refusing an old
//! file loudly is the point; reading it wrong quietly is what the number prevents.

use super::File;

/// What writes a file's bytes today.
pub const FILE_FORMAT: i32 = 8;

/// The oldest format still read. Anything older is refused.
pub const OLDEST_FILE_FORMAT: i32 = FILE_FORMAT;

/// A file's bytes, in whichever format wrote them.
pub fn decode(format: i32, bytes: &[u8]) -> Result<File, String> {
    match format {
        FILE_FORMAT => lc_proto::decode(bytes).map_err(|why| why.to_string()),
        other => Err(format!("knowledge format {other} is not {OLDEST_FILE_FORMAT} to {FILE_FORMAT}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{NameKind, Naming, Witness};

    fn naming() -> Naming {
        Naming { witness: Witness(4), name: "Kettle".into(), kind: NameKind::Given, stated_s: 7.0, lineage: Vec::new() }
    }

    /// A file written today reads back, and any other format is refused rather than guessed at.
    #[test]
    fn the_current_format_round_trips_and_every_other_is_refused() {
        let mut file = File::default();
        file.names.push(naming());
        assert_eq!(decode(FILE_FORMAT, &lc_proto::encode(&file)).unwrap(), file);
        assert!(decode(FILE_FORMAT - 1, &lc_proto::encode(&file)).is_err(), "an older file is read");
        assert!(decode(FILE_FORMAT + 1, &lc_proto::encode(&file)).is_err());
    }
}
