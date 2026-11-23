//! What a craft's fitting answers, and the account as each settlement found it, so a moment already
//! settled past is read from the account in force then and not from today's.
//!
//! A settlement folds the account forward, and the closed form it leaves answers only from its own
//! instant: asked about an earlier one, it gives the settled terms, so every order rewrote what the
//! ship weighed — and so what its drive put out — in light still on its way. The same reason
//! [`Craft`] keeps the motives it has flown.

use super::{Craft, HISTORY_S};
use crate::fitting::Fitting;
use crate::motion::{self, Event, Rejected};

/// How many accounts a craft keeps. One is kept a settlement that found the account changed by
/// more than its own fold — an order, a grant, a starlight segment — so this holds a day's
/// segments across [`HISTORY_S`] with room for orders. Past it the oldest goes, and a moment
/// before the oldest kept is read from that one, the earliest it can answer with.
pub const HISTORY_ACCOUNTS: usize = 4096;

/// Oldest first, each in force from its own [`Fitting::since_s`] until the next.
#[derive(Clone, Debug, Default)]
pub(super) struct Accounts {
    kept: Vec<Fitting>,
    /// The live account as the last settlement left it. Found unchanged at the next, the account
    /// was only folded in between, and the one already kept still answers for that stretch.
    folded: Option<Fitting>,
}

impl Accounts {
    pub(super) fn clear(&mut self) {
        self.kept.clear();
        self.folded = None;
    }

    /// Before `live` is settled past its `since_s`.
    pub(super) fn settling(&mut self, live: &Fitting) {
        if self.folded.as_ref() != Some(live) {
            self.keep(live.clone());
        }
    }

    pub(super) fn settled(&mut self, live: &Fitting) {
        self.folded = Some(live.clone());
    }

    /// The last account a craft had, as it ended.
    pub(super) fn retire(&mut self, last: Fitting) {
        self.keep(last);
        self.folded = None;
    }

    fn keep(&mut self, account: Fitting) {
        let horizon = account.since_s() - HISTORY_S;
        // Two settlements at one instant: the later is the account from it.
        if self.kept.last().is_some_and(|last| last.since_s() == account.since_s()) {
            self.kept.pop();
        }
        self.kept.push(account);
        // The first is wanted only before the second began.
        let stale = self.kept.iter().skip(1).take_while(|a| a.since_s() < horizon).count();
        let excess = self.kept.len().saturating_sub(HISTORY_ACCOUNTS);
        self.kept.drain(..stale.max(excess));
    }

    /// The account in force at `t`, or the oldest kept before it.
    fn at(&self, t: f64) -> Option<&Fitting> {
        let after = self.kept.partition_point(|a| a.since_s() <= t);
        self.kept.get(after.saturating_sub(1))
    }
}

impl Craft {
    /// The account in force at `t`, coordinate seconds: the live one from its settlement on, and
    /// before it the one that settlement replaced. A wreck answers from the account it ended with.
    /// `None` for a craft that never had a fitting.
    pub fn fitting_at(&self, t: f64) -> Option<&Fitting> {
        match &self.fitting {
            Some(live) if t >= live.since_s() => Some(live),
            live => self.accounts.at(t).or(live.as_ref()),
        }
    }

    /// What it weighs at a coordinate second, kilograms: its modules and its stored energy as the
    /// account in force then had them, and [`Craft::mass_kg`] with no fitting.
    pub fn mass_kg_at(&self, t: f64) -> f64 {
        match self.fitting_at(t) {
            Some(fitting) => fitting.mass_kg_at(self.motion_at(t), t),
            None => self.mass_kg(),
        }
    }

    /// How fast it could turn at `t`, radians a second, by the hull it had then.
    pub(super) fn slew_rate_at(&self, t: f64) -> f64 {
        slew_rate(self.fitting_at(t), self.length_m)
    }

    /// Its hull's length at `t`, meters: its form's extent then, or its length with no form.
    pub(super) fn hull_m_at(&self, t: f64) -> f64 {
        self.fitting_at(t).map_or(self.length_m, |fitting| fitting.hull().extent_m)
    }

    /// How fast it can turn, radians a second: from its form's moments if it has one, and from
    /// its length if it is the ovoid. See [`crate::attitude`].
    pub fn slew_rate_rad_s(&self) -> f64 {
        slew_rate(self.fitting.as_ref(), self.length_m)
    }

    /// The drive a new order is clamped against: the kind's, turning at this hull's rate, and
    /// pulling what its aft engines can move this mass at if it has a form.
    pub fn rated_drive(&self, now_s: f64) -> crate::flight::Drive {
        let mut drive = self.turning(self.kind.drive());
        if let Some(fitting) = &self.fitting {
            drive.accel_g = fitting.rated_g_at(&self.motion, now_s);
        }
        drive
    }

    /// Energy stored and not committed, joules. Unlimited for a craft with no fitting.
    pub fn free_j_at(&self, now_s: f64) -> f64 {
        match &self.fitting {
            Some(fitting) => fitting.free_j_at(&self.motion, now_s),
            None => f64::INFINITY,
        }
    }

    /// Whether a refit is under way at `now_s`. Flying and refitting exclude each other.
    pub fn is_refitting(&self, now_s: f64) -> bool {
        self.fitting.as_ref().and_then(Fitting::refit).is_some_and(|r| !r.is_done(now_s))
    }

    /// What folding `event` would commit, joules, without folding it. Zero for an unfitted craft.
    ///
    /// The plan's whole remaining rapidity at this craft's mass now. [`Craft::apply`] commits the
    /// same number, because it is the same arithmetic on the same state.
    pub fn cost_of(&self, event: &Event) -> Result<f64, Rejected> {
        let Some(fitting) = &self.fitting else { return Ok(0.0) };
        let mut trial = self.motion.clone();
        motion::apply(&mut trial, self.system.as_deref(), event)?;
        let mut account = fitting.clone();
        account.settle(&self.motion, event.at_t);
        Ok(account.commit(&trial, event.at_t))
    }
}

/// From a form's moments if there is one, and from the ovoid's length if not. See
/// [`crate::attitude`].
pub(super) fn slew_rate(fitting: Option<&Fitting>, length_m: f64) -> f64 {
    match fitting {
        Some(fitting) => crate::attitude::rate_for_gyration(fitting.hull().gyration_m),
        None => crate::attitude::rate_rad_s(length_m),
    }
}
