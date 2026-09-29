//! The forms a craft has had and the field it has worn, so an observer is shown the ones its light
//! left with. See `lightcone/docs/29-ship-form.md` §Protocol and persistence and
//! `lightcone/docs/30-the-field.md` §What an observer sees.

use std::sync::Arc;

use crate::craft::HISTORY_S;
use crate::field::{Field, Mode};
use crate::fitting::{Fitting, Switch};
use crate::form::{Form, Part, PartId};
use crate::refit::rounds::{Change, Plan};

/// How many forms a craft remembers having had. A round is a step per part change, so this holds
/// a few rounds of a large form; past it, the oldest is forgotten, and an observer asking about
/// then is told nothing rather than a later form.
pub const HISTORY_FORMS: usize = 1024;

/// How many samples of its field a craft keeps. A field at rest is one sample for as long as it
/// rests; one changing is kept at about a sample every tenth of a time constant.
pub const HISTORY_GLOWS: usize = 4096;

/// Of its heat, how far a sample may sit off the line through its neighbors and still be dropped.
const GLOW_TOLERANCE: f64 = 1.0e-4;

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

/// The field as a settlement left it, from `at_s`, coordinate seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glowed {
    pub at_s: f64,
    pub heat_j: f64,
    /// Kept so a wreck's light, with no fitting left to ask, still has a temperature.
    pub field: Field,
    pub shade: Mode,
    /// Not yet taken: the shade is its `to` from its `done_s`.
    pub switch: Option<Switch>,
}

impl Glowed {
    fn shade_at(&self, t: f64) -> Mode {
        match self.switch {
            Some(switch) if switch.done_s <= t => switch.to,
            _ => self.shade,
        }
    }

    fn switching_at(&self, t: f64) -> Option<Switch> {
        self.switch.filter(|s| s.done_s > t)
    }

    fn worn_alike(&self, other: &Glowed) -> bool {
        self.field == other.field && self.shade == other.shade && self.switch == other.switch
    }
}

/// Oldest first. Between samples heat is read off the line joining them, so at most two share an
/// instant: before a burst and after it.
#[derive(Clone, Debug, Default)]
pub struct Glows {
    kept: Vec<Glowed>,
    /// The slopes out of the second-last sample that still pass every sample dropped since it, J/s.
    window: Option<(f64, f64)>,
    /// Whether the front was ever dropped. Until it is, the oldest is the field for all time before.
    forgot: bool,
}

impl Glows {
    pub fn clear(&mut self) {
        self.kept.clear();
        self.window = None;
        self.forgot = false;
    }

    /// Drops the last sample where the line from the one before it to `next` passes within
    /// [`GLOW_TOLERANCE`] of it and of every sample dropped before it.
    pub fn push(&mut self, next: Glowed) {
        if self.kept.last() == Some(&next) {
            return;
        }
        while self.kept.last().is_some_and(|last| last.at_s > next.at_s) {
            self.kept.pop();
            self.window = None;
        }
        let mut window = None;
        if let [.., a, b] = self.kept.as_slice() {
            let dropped = if b.at_s == next.at_s {
                a.at_s == b.at_s
            } else if a.at_s < b.at_s && a.worn_alike(b) && b.worn_alike(&next) {
                let off = GLOW_TOLERANCE * b.heat_j.abs().max(f64::MIN_POSITIVE);
                let dt = b.at_s - a.at_s;
                let (lo, hi) = self.window.unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
                let (lo, hi) = (lo.max((b.heat_j - off - a.heat_j) / dt), hi.min((b.heat_j + off - a.heat_j) / dt));
                let slope = (next.heat_j - a.heat_j) / (next.at_s - a.at_s);
                window = Some((lo, hi));
                lo <= slope && slope <= hi
            } else {
                false
            };
            if dropped {
                self.kept.pop();
            } else {
                window = None;
            }
        }
        self.window = window;
        let horizon = next.at_s - HISTORY_S;
        self.kept.push(next);
        let stale = self.kept.iter().skip(1).take_while(|g| g.at_s < horizon).count();
        let excess = self.kept.len().saturating_sub(HISTORY_GLOWS);
        let dropped = stale.max(excess);
        self.forgot |= dropped > 0;
        self.kept.drain(..dropped);
    }

    /// Heat, field, shade and any switch under way at `t`. Before the oldest sample, the oldest's, unless older ones were
    /// forgotten: then nothing, as a form too old to remember is.
    pub fn at(&self, t: f64) -> Option<(f64, Field, Mode, Option<Switch>)> {
        let after = self.kept.partition_point(|g| g.at_s <= t);
        let Some(a) = after.checked_sub(1).map(|i| &self.kept[i]) else {
            return self.kept.first().filter(|_| !self.forgot).map(|g| (g.heat_j, g.field, g.shade, g.switching_at(t)));
        };
        let heat_j = match self.kept.get(after) {
            Some(b) => a.heat_j + (b.heat_j - a.heat_j) * (t - a.at_s) / (b.at_s - a.at_s),
            None => a.heat_j,
        };
        Some((heat_j, a.field, a.shade_at(t), a.switching_at(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(at_s: f64, heat_j: f64) -> Glowed {
        Glowed { at_s, heat_j, field: Field::of(1.0, &crate::fitting::Balance::DEFAULT), shade: Mode::Clear, switch: None }
    }

    #[test]
    fn a_field_at_rest_is_one_line_however_often_it_settles() {
        let mut glows = Glows::default();
        for k in 0..1000 {
            glows.push(at(f64::from(k), 5.0));
        }
        assert_eq!(glows.kept.len(), 2);
        assert_eq!(glows.at(500.5).map(|g| g.0), Some(5.0));
    }

    /// A burst is a step, read on either side of its instant, and never smoothed into a ramp.
    #[test]
    fn a_burst_is_kept_as_a_step() {
        let mut glows = Glows::default();
        for (t, q) in [(0.0, 1.0), (10.0, 1.0), (10.0, 1.0), (10.0, 9.0), (20.0, 9.0), (30.0, 9.0)] {
            glows.push(at(t, q));
        }
        assert_eq!(glows.at(9.999).map(|g| g.0), Some(1.0));
        assert_eq!(glows.at(10.0).map(|g| g.0), Some(9.0));
        assert_eq!(glows.at(25.0).map(|g| g.0), Some(9.0));
    }

    /// Settled every step of a curve, the kept samples still read it back to the tolerance.
    #[test]
    fn a_curve_is_kept_to_its_tolerance_in_far_fewer_samples() {
        let tau = 1.0e4;
        let q = |t: f64| 10.0 - 9.0 * (-t / tau).exp();
        let mut glows = Glows::default();
        for k in 0..=20_000 {
            glows.push(at(f64::from(k), q(f64::from(k))));
        }
        assert!(glows.kept.len() < 2000, "{} kept", glows.kept.len());
        for k in 0..2000 {
            let t = f64::from(k) * 10.0 + 3.3;
            let (read, ..) = glows.at(t).unwrap();
            assert!((read / q(t) - 1.0).abs() < 2.0 * GLOW_TOLERANCE, "at {t}: {read} for {}", q(t));
        }
    }

    /// Once the front has been forgotten, a moment before the oldest kept is not answered with a
    /// later field.
    #[test]
    fn a_forgotten_past_is_not_answered() {
        let mut glows = Glows::default();
        glows.push(at(0.0, 1.0));
        assert!(glows.at(-5.0).is_some(), "before anything was forgotten, the oldest stands for all time");
        for k in 1..=HISTORY_GLOWS + 10 {
            let shade = if k % 2 == 0 { Mode::Clear } else { Mode::Black };
            glows.push(Glowed { shade, ..at(k as f64, 1.0) });
        }
        assert_eq!(glows.at(1.0), None);
        assert!(glows.at((HISTORY_GLOWS + 5) as f64).is_some());
    }

    #[test]
    fn a_switch_turns_the_shade_at_its_done_time_between_samples() {
        let mut glows = Glows::default();
        let switch = Some(Switch { to: Mode::Black, done_s: 15.0 });
        glows.push(Glowed { switch, ..at(10.0, 1.0) });
        glows.push(Glowed { switch, ..at(20.0, 1.0) });
        assert_eq!(glows.at(14.0).map(|g| g.2), Some(Mode::Clear));
        assert_eq!(glows.at(16.0).map(|g| g.2), Some(Mode::Black));
    }
}
