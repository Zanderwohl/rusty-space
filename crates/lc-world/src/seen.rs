//! The forms a craft has had, so an observer is shown the one its light left with. See
//! `lightcone/docs/29-ship-form.md` §Protocol and persistence.

use std::sync::Arc;

use crate::craft::HISTORY_S;
use crate::fitting::Fitting;
use crate::form::Form;
use crate::refit::rounds::Round;

/// How many forms a craft remembers having had. A round is a step per part change, so this holds
/// a few rounds of a large form; past it, the oldest is forgotten, and an observer asking about
/// then is told nothing rather than a later form.
pub const HISTORY_FORMS: usize = 1024;

/// A form a craft had, the length it measured and the round it was running, from the coordinate
/// second it took effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub from_s: f64,
    pub form: Arc<Form>,
    pub length_m: f64,
    /// As its recipe, which is what an observer draws the building from.
    pub round: Option<Arc<Round>>,
}

impl Seen {
    fn looks_like(&self, other: &Seen) -> bool {
        self.form == other.form && self.length_m == other.length_m && self.round == other.round
    }
}

/// The round `fitting` is running, and the coordinate second it ends.
pub(crate) fn running(fitting: &Fitting) -> Option<(Arc<Round>, f64)> {
    fitting.refit().map(|plan| (Arc::new(plan.round().clone()), plan.round().start_s + plan.duration_s()))
}

/// Oldest first.
#[derive(Clone, Debug, Default)]
pub struct History(Vec<Seen>);

impl History {
    /// The one in force at `t`, coordinate seconds; `None` before the oldest remembered.
    pub fn at(&self, t: f64) -> Option<&Seen> {
        let after = self.0.partition_point(|seen| seen.from_s <= t);
        after.checked_sub(1).map(|i| &self.0[i])
    }

    pub fn first(&self) -> Option<&Seen> {
        self.0.first()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Nothing is noted when nothing an observer sees has changed.
    pub fn push(&mut self, mut seen: Seen) {
        if let Some(last) = self.0.last() {
            if last.looks_like(&seen) {
                return;
            }
            // One allocation a round, however many steps it has.
            if let (Some(a), Some(b)) = (&last.round, &seen.round)
                && a == b
            {
                seen.round = Some(a.clone());
            }
        }
        let horizon = seen.from_s - HISTORY_S;
        self.0.push(seen);
        // The first is wanted only before the second began.
        let stale = self.0.iter().skip(1).take_while(|s| s.from_s < horizon).count();
        let excess = self.0.len().saturating_sub(HISTORY_FORMS);
        self.0.drain(..stale.max(excess));
    }
}
