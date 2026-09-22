//! What a craft is doing with its instruments, and what it knows about.
//!
//! Mirrors of `lc_world::knowledge`'s duty and subject, as the wire carries them, with the
//! conversions on the other side of the boundary in `lc-world` — the same arrangement as
//! [`crate::Course`]. See `lightcone/docs/24-standing-instruments.md`.

use serde::{Deserialize, Serialize};

/// The longest name a craft may give anything, in bytes.
pub const NAME_LIMIT: usize = 64;

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
    Stare { star: u64 },
    Sweep { center: [f64; 3], radius_rad: f64, dwell_s: f64, started_s: f64 },
    Watch { stars: Vec<u64>, dwell_s: f64, started_s: f64 },
}

/// Something a craft knows about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    Star(u64),
    Body { star: u64, body: u64 },
    Population { star: u64, index: u32 },
    Craft(i64),
}
