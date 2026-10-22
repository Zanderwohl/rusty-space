//! What a craft is doing with its instruments, and what it knows about.
//!
//! Mirrors of `lc_world::knowledge`'s duty and subject, as the wire carries them, with the
//! conversions on the other side of the boundary in `lc-world` — the same arrangement as
//! [`crate::Course`]. See `lightcone/docs/24-standing-instruments.md`.

use serde::{Deserialize, Serialize};

/// The longest name a craft may give anything, in bytes.
pub const NAME_LIMIT: usize = 64;

/// The longest note, in bytes.
pub const NOTE_LIMIT: usize = 1024;

/// What a thread entry says. See `lightcone/docs/23-factions.md` §Notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Writing {
    Note(String),
    /// What its author calls the subject from now on.
    Name(String),
    /// Which craft its author holds a collapse was, or, with `None`, that it holds it was nobody
    /// known. Appended last.
    Was(Option<crate::ShipId>),
}

impl Writing {
    /// Whether it says something, within its limit.
    pub fn is_valid(&self) -> bool {
        match self {
            Writing::Note(text) => !text.trim().is_empty() && text.len() <= NOTE_LIMIT,
            Writing::Name(name) => !name.trim().is_empty() && name.len() <= NAME_LIMIT,
            Writing::Was(_) => true,
        }
    }
}

/// Where an entry may travel: only ever in reports sealed to admit it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Never leaves the ship that wrote it.
    #[default]
    Own,
    /// In any report.
    Open,
    /// Only in reports sealed under this faction key.
    Faction(crate::FactionId),
}

/// The most stars one watch rotation may hold.
pub const WATCH_LIMIT: usize = 256;

/// Shortest and longest dwell a sweep field or a watch turn may have, seconds, and the longest
/// integration a stare may take before it records a sample.
pub const DWELL_MIN_S: f64 = 1.0;
pub const DWELL_MAX_S: f64 = 86_400.0;
pub const INTEGRATION_MAX_S: f64 = 30.0 * 86_400.0;

impl Duty {
    /// Whether every number in it is one a telescope could be set to: finite, in range, and a
    /// direction with a length. A duty that is not is refused, not clamped — `f64::clamp` passes
    /// a NaN through, and a sweep of NaN radius finds every star in a tick.
    pub fn is_valid(&self) -> bool {
        let dwell = |d: f64| (DWELL_MIN_S..=DWELL_MAX_S).contains(&d);
        match self {
            Duty::Idle | Duty::Stare { .. } => true,
            Duty::Sweep { center, radius_rad, dwell_s, started_s } => {
                let length = center.iter().map(|c| c * c).sum::<f64>().sqrt();
                center.iter().all(|c| c.is_finite())
                    && length > 0.0
                    && length.is_finite()
                    && radius_rad.is_finite()
                    && *radius_rad > 0.0
                    && *radius_rad <= std::f64::consts::PI
                    && dwell(*dwell_s)
                    && started_s.is_finite()
            }
            Duty::Watch { stars, dwell_s, started_s } => {
                !stars.is_empty() && stars.len() <= WATCH_LIMIT && dwell(*dwell_s) && started_s.is_finite()
            }
            Duty::Survey { started_s, .. } => started_s.is_finite(),
        }
    }
}

/// What a telescope is committed to.
///
/// A sweep's and a watch's start time is ignored in an order — an instrument cannot have begun
/// before it was told to — and is the shard's actual start in anything the shard says back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Duty {
    Idle,
    Stare { at: Gaze },
    Sweep { center: [f64; 3], radius_rad: f64, dwell_s: f64, started_s: f64 },
    Watch { stars: Vec<u64>, dwell_s: f64, started_s: f64 },
    /// Every body of one star's system in turn, brightest first. The bodies are not named
    /// because the craft does not know them yet: finding them is what the duty is for.
    Survey { star: u64, started_s: f64 },
}

impl Duty {
    pub fn stare(star: u64) -> Self {
        Self::Stare { at: Gaze::Star(star) }
    }
}

/// What a stare points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gaze {
    Star(u64),
    /// A point in the shard's frame, light-microseconds: where something was.
    Place([i64; 3]),
    /// Followed by its light, never by where it is now.
    Craft(i64),
}

/// Something a craft knows about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    Star(u64),
    Body { star: u64, body: u64 },
    Population { star: u64, index: u32 },
    Craft(i64),
    /// A point a stare was aimed at, light-microseconds. Appended last.
    Place([i64; 3]),
    /// A collapse seen by its light, by its moment and place, whoever it was. Appended last.
    Collapse(u64),
}
