//! The forms a craft has had, so an observer is shown the one its light left with. See
//! `lightcone/docs/29-ship-form.md` §Protocol and persistence.

use std::sync::Arc;

use crate::craft::HISTORY_S;
use crate::fitting::Fitting;
use crate::form::{Form, Part, PartId};
use crate::refit::rounds::{Change, Plan};

/// How many forms a craft remembers having had. A round is a step per part change, so this holds
/// a few rounds of a large form; past it, the oldest is forgotten, and an observer asking about
/// then is told nothing rather than a later form.
pub const HISTORY_FORMS: usize = 1024;

/// A form a craft had, the length it measured and the refit it was in, from the coordinate second
/// it took effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub from_s: f64,
    pub form: Arc<Form>,
    pub length_m: f64,
    pub refit: Option<Refitting>,
}

/// A round kept only to say what step was under way when; an observer is told that, never this.
#[derive(Clone, Debug, PartialEq)]
pub enum Refitting {
    Running(Arc<Plan>),
    /// Stopped at `at_s`. The step under way then runs back when `reverses`: a dismantling storage
    /// could not pay back was finished instead.
    Canceled { plan: Arc<Plan>, at_s: f64, reverses: bool },
}

/// The refit step a craft had under way at some moment, as its light shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Underway {
    pub step: usize,
    pub part: PartId,
    pub change: Change,
    /// The part as the step leaves it.
    pub after: Option<Part>,
    pub fraction: f64,
    pub duration_s: f64,
    pub reversing: bool,
}

impl Seen {
    fn looks_like(&self, other: &Seen) -> bool {
        self.form == other.form && self.length_m == other.length_m && self.refit == other.refit
    }

    /// At `t`, coordinate seconds, no earlier than [`Seen::from_s`].
    pub fn underway(&self, t: f64) -> Option<Underway> {
        let (plan, (i, fraction), reversing) = match self.refit.as_ref()? {
            Refitting::Running(plan) => (plan, plan.at(t).current?, false),
            Refitting::Canceled { plan, at_s, reverses } => (plan, reversal(plan, *at_s, *reverses, t)?, true),
        };
        let step = plan.steps()[i];
        Some(Underway {
            step: i,
            part: step.part,
            change: step.change,
            after: step.after,
            fraction,
            duration_s: step.duration_s,
            reversing,
        })
    }
}

/// The step a cancel at `at_s` is running back at `now_s`, and how far through it still is. `None`
/// once it is back, and at once for a cancel that does not reverse.
pub fn reversal(plan: &Plan, at_s: f64, reverses: bool, now_s: f64) -> Option<(usize, f64)> {
    let (i, fraction) = plan.at(at_s).current.filter(|_| reverses)?;
    let back = fraction - (now_s - at_s).max(0.0) / plan.steps()[i].duration_s;
    (back > 0.0).then_some((i, back))
}

/// The round `fitting` is running, and the coordinate second it ends.
pub(crate) fn running(fitting: &Fitting) -> Option<(Refitting, f64)> {
    let plan = fitting.refit()?;
    Some((Refitting::Running(Arc::new(plan.clone())), plan.round().start_s + plan.duration_s()))
}

impl From<Change> for lc_proto::Change {
    fn from(c: Change) -> Self {
        match c {
            Change::Grow => Self::Grow,
            Change::Shrink => Self::Shrink,
            Change::Add => Self::Add,
            Change::Remove => Self::Remove,
            Change::Move => Self::Move,
        }
    }
}

impl From<lc_proto::Change> for Change {
    fn from(c: lc_proto::Change) -> Self {
        match c {
            lc_proto::Change::Grow => Self::Grow,
            lc_proto::Change::Shrink => Self::Shrink,
            lc_proto::Change::Add => Self::Add,
            lc_proto::Change::Remove => Self::Remove,
            lc_proto::Change::Move => Self::Move,
        }
    }
}

impl From<Underway> for lc_proto::Building {
    fn from(u: Underway) -> Self {
        Self {
            step: u.step as u32,
            part: u.part.into(),
            change: u.change.into(),
            after: u.after.map(Into::into),
            fraction: u.fraction,
            duration_s: u.duration_s,
            reversing: u.reversing,
        }
    }
}

impl From<&lc_proto::Building> for Underway {
    fn from(b: &lc_proto::Building) -> Self {
        Self {
            step: b.step as usize,
            part: b.part.into(),
            change: b.change.into(),
            after: b.after.map(Into::into),
            fraction: b.fraction,
            duration_s: b.duration_s,
            reversing: b.reversing,
        }
    }
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
            // One plan a round, however many steps it has.
            if let (Some(Refitting::Running(a)), Some(Refitting::Running(b))) = (&last.refit, &seen.refit)
                && a == b
            {
                seen.refit = last.refit.clone();
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
