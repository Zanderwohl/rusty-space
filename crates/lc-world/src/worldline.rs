//! A ship's worldline with its history, for the light-delay solve.

use std::sync::Arc;

use glam::DVec3;
use lc_spacetime::Worldline;
use smallvec::SmallVec;

use crate::motion::{LIGHT_US_PER_LY, ShipState, state_at};
use crate::system::LocalSystem;

/// A stretch of a worldline that is over: what a ship was doing, and when it stopped.
///
/// See [`Flight`] for why a craft keeps these at all.
#[derive(Clone)]
pub struct Past {
    /// Coordinate seconds at which this stopped being in force.
    pub until_s: f64,
    pub motion: ShipState,
    /// The system it was flown in. A station is defined against bodies, and read against
    /// whatever system the craft is in *now* it froze where it left the moment the craft went
    /// anywhere else — which an observer still watching the old light could see at once.
    pub system: Option<Arc<LocalSystem>>,
    /// Whether it ended in a jump rather than joining the next stretch. See
    /// [`Worldline::breaks`].
    pub broken: bool,
}

impl std::fmt::Debug for Past {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Past")
            .field("until_s", &self.until_s)
            .field("motion", &self.motion)
            .field("star", &self.system.as_ref().map(|s| s.star))
            .field("broken", &self.broken)
            .finish()
    }
}

/// A ship's worldline, for the light-delay solve.
///
/// Borrowed rather than owned: the motive and the system are the truth, and a copy of them in
/// another shape is a copy that can be stale. Holding the two together is what makes the
/// worldline total — a station and a conic mean nothing without the bodies they are defined
/// against.
///
/// **A worldline has a past, and that is not decoration.** A motive is a closed form total in
/// `t`, so evaluating the *current* one at an earlier time answers about a ship that did not
/// exist yet: `Motive::Drifting` extrapolates backwards, so a burn would retroactively rewrite
/// where the ship was an hour ago and how fast. Every retarded solve then reads the new motion
/// at the old time — which is a client watching a maneuvere the instant it happens, at any
/// range, and the end of the game this is all built to be.
///
/// So a craft keeps the motives it has flown, stamped with when each stopped, and this picks
/// the one that was in force. The same shape `crate::observation::Target` uses for emission
/// models, for the same reason: an event does not alter the past, it appends.
///
/// The memory is bounded, so the past runs out. [`Flight::defined_over`] says where, and a
/// solve that falls off the end returns nothing at all rather than a guess — not seeing
/// something is always safe, and inventing where it was is not.
///
/// The frame is `lc-spacetime`'s: light-microseconds from the world origin, and coordinate
/// microseconds. Note the precision this costs at galactic distances — a position is a `f64`
/// count of light-microseconds, so a ship in a system a hundred light-years out is at `3e15`
/// and resolves to about a hundred meters. Fine for a light-delay solve, useless for an orbit,
/// and the reason doc 08 shards the frame rather than keeping one origin for everything.
pub struct Flight<'a> {
    state: &'a ShipState,
    system: Option<&'a LocalSystem>,
    /// Oldest first, and each one's `until_s` later than the last.
    past: &'a [Past],
    /// The earliest coordinate second this can answer for. `-inf` when nothing has been
    /// forgotten, which is the case for a craft that has never changed what it was doing.
    known_from_s: f64,
}

impl<'a> Flight<'a> {
    /// A worldline with no past: whatever it is doing now, it has always been doing.
    ///
    /// True only of a craft that has never changed its motive. Anything the world has run is
    /// built by [`crate::craft::Craft::worldline`], which carries the real history.
    pub fn new(state: &'a ShipState, system: Option<&'a LocalSystem>) -> Self {
        Self { state, system, past: &[], known_from_s: f64::NEG_INFINITY }
    }

    pub fn with_past(
        state: &'a ShipState,
        system: Option<&'a LocalSystem>,
        past: &'a [Past],
        known_from_s: f64,
    ) -> Self {
        Self { state, system, past, known_from_s }
    }

    /// What the ship was doing at a coordinate microsecond.
    ///
    /// The first stretch that had not ended yet, or the current motive when none of them
    /// apply. `past` is ordered, so the first match is the right one.
    ///
    /// Compared in microseconds, the unit [`Worldline::breaks`] states them in: a break read back
    /// through seconds can round to just before itself, and the piece after a jump then starts
    /// on the position it jumped from.
    fn doing_at(&self, t_us: f64) -> (&ShipState, Option<&LocalSystem>) {
        self.past
            .iter()
            .find(|entry| t_us < entry.until_s * 1.0e6)
            .map(|entry| (&entry.motion, entry.system.as_deref()))
            .unwrap_or((self.state, self.system))
    }

    /// Position and beta at a coordinate microsecond, in light-years.
    ///
    /// Falls back to the ship's last known position when the motive cannot be evaluated — a
    /// body that has gone, or a chain that is integrated. The same choice [`advance`] makes:
    /// keep what is known rather than invent a position from nothing.
    fn read(&self, t_us: f64) -> (DVec3, DVec3) {
        let s = t_us * 1.0e-6;
        let (state, system) = self.doing_at(t_us);
        state_at(state, system, s).unwrap_or((state.position_ly, state.beta))
    }
}

impl Worldline for Flight<'_> {
    fn position_at(&self, t: f64) -> DVec3 {
        self.read(t).0 * LIGHT_US_PER_LY
    }

    fn velocity_at(&self, t: f64) -> DVec3 {
        self.read(t).1
    }

    /// Forward forever, and back as far as the craft still remembers.
    ///
    /// Every arm of a motive answers everywhere, so the forward end never runs out. The back
    /// end is where the history was pruned: before it this would have to extrapolate a motive
    /// the ship was not yet flying, and the solver's contract is that a root outside this range
    /// is no root at all. Which is the answer that is safe — an observer far enough away that
    /// the light it wants left before the shard remembers simply sees nothing.
    fn defined_over(&self) -> (f64, f64) {
        (self.known_from_s * 1.0e6, f64::INFINITY)
    }

    fn breaks(&self) -> SmallVec<[f64; 2]> {
        self.past.iter().filter(|entry| entry.broken).map(|entry| entry.until_s * 1.0e6).collect()
    }
}
