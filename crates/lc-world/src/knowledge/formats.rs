//! A file as it is written down.
//!
//! The shard stores a [`File`] as CBOR (`lc_server::archive`), which names its fields, and a
//! format number rides with every stored file; anything but the current one is refused. See
//! `lightcone/docs/24-standing-instruments.md`.
//!
//! **Bump [`FILE_FORMAT`] when a field of [`File`], or of anything inside it, is renamed or
//! removed or comes to mean something else.** A field *added* with `#[serde(default)]` does not
//! bump it: an older file reads without it, as the default. There are no back-readers: the game
//! has no players, so a stored file is worth less than the ceremony of keeping it, and the readers
//! that existed each embedded the records by name — growing `Orbit` would have meant freezing a
//! copy of the old shape beside every one of them. Refusing an old file loudly is the point;
//! reading it wrong quietly is what the number prevents.
//!
//! 13 is the move from postcard, which was positional and made every added field a bump.

#[cfg(doc)]
use super::File;

/// What writes a file's bytes today.
pub const FILE_FORMAT: i32 = 13;
