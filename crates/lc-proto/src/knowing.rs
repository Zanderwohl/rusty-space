//! What a craft is doing with its instruments, and what it knows about.
//!
//! Mirrors of `lc_world::knowledge`'s duty and subject, as the wire carries them, with the
//! conversions on the other side of the boundary in `lc-world` — the same arrangement as
//! [`crate::Course`]. See `lightcone/docs/24-standing-instruments.md`.

use serde::{Deserialize, Serialize};

/// The longest name a craft may give anything, in bytes.
pub const NAME_LIMIT: usize = 64;

/// What a telescope is committed to.
///
/// A sweep's and a watch's start time is ignored in an order — an instrument cannot have begun
/// before it was told to — and is the shard's actual start in anything the shard says back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Duty {
    Idle,
    Stare { star: u64 },
    Sweep { center: [f64; 3], radius_rad: f64, dwell_s: f64, started_s: f64 },
    Watch { stars: Vec<u64>, dwell_s: f64, started_s: f64 },
}

/// Something a craft knows about. Mirrors `lc_world::knowledge::Subject`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    Star(u64),
    Body { star: u64, body: u64 },
    Population { star: u64, index: u32 },
    Craft(i64),
}
