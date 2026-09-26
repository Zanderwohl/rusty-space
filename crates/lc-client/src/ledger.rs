//! The refit ledger as values: the round's budget, its phases and steps, and whether Apply may be
//! pressed and why not. The refit window and the editor both draw from here, so the numbers are
//! one computation. See `lightcone/docs/29-ship-form.md` §The budget and §Refits.
//!
//! A running round is read from the plan the shard's recipe solves to, never from the draft, so
//! the client's clock and the shard's round agree step for step.

use lc_world::fitting::Balance;
use lc_world::form::capacity::Capacities;
use lc_world::form::Form;
use lc_world::refit::rounds::{Change, Phase, Plan, Step};

use crate::draft::Draft;

/// What a round does to storage, joules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    /// Stored when the round began, plus the recovered share of everything it takes apart.
    pub available_j: f64,
    /// Mass-energy of everything it builds.
    pub spent_j: f64,
    /// In storage as the dismantle phase ends, against the capacity the form has then.
    pub peak_j: f64,
    pub peak_capacity_j: f64,
    /// Burst into the field over the round. C4 turns this into the field's peak temperature, and
    /// Apply's second question when it would collapse the field.
    pub vented_j: f64,
}

impl Budget {
    pub fn of(plan: &Plan, balance: &Balance) -> Self {
        let round = plan.round();
        let steps = plan.steps();
        let dismantling = |s: &&Step| s.change.phase() == Phase::Dismantle;
        let returned_j: f64 = steps.iter().filter(dismantling).map(|s| s.gross_j).sum();
        let spent_j = steps.iter().filter(|s| s.change.phase() == Phase::Build).map(|s| s.gross_j).sum();
        let peak_j = round.stored_j + steps.iter().filter(dismantling).map(|s| s.stored_j - s.spilled_j).sum::<f64>();
        let dismantled_s = steps.iter().filter(dismantling).map(Step::ends_s).fold(0.0, f64::max);
        let peak_form = plan.at(round.start_s + dismantled_s).form;
        Self {
            available_j: round.stored_j + balance.recovery * returned_j,
            spent_j,
            peak_j,
            peak_capacity_j: Capacities::of(&peak_form, balance).storage_j,
            vented_j: plan.vented_j(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepState {
    Done,
    /// How far through, 0 to 1.
    Working(f64),
    Waiting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepRow {
    pub what: String,
    pub state: StepState,
    pub duration_s: f64,
    /// Into storage over the step, less what a shrinking store spills: negative for a build.
    pub stored_j: f64,
    pub vented_j: f64,
}

/// All three phases, in order, each with its steps, empty or not.
pub fn phases(plan: &Plan, now_s: f64) -> Vec<(Phase, Vec<StepRow>)> {
    let progress = plan.at(now_s);
    let state = |i: usize| match progress.current {
        _ if i < progress.finished => StepState::Done,
        Some((at, fraction)) if at == i => StepState::Working(fraction),
        // Past the round's end `at` finishes every step.
        _ => StepState::Waiting,
    };
    [Phase::Dismantle, Phase::Move, Phase::Build]
        .into_iter()
        .map(|phase| {
            let rows = plan
                .steps()
                .iter()
                .enumerate()
                .filter(|(_, s)| s.change.phase() == phase)
                .map(|(i, s)| StepRow {
                    what: step_name(s),
                    state: state(i),
                    duration_s: s.duration_s,
                    stored_j: s.stored_j - s.spilled_j,
                    vented_j: s.vented_j,
                })
                .collect();
            (phase, rows)
        })
        .collect()
}

pub fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Dismantle => "dismantle",
        Phase::Move => "move",
        Phase::Build => "build",
    }
}

pub fn step_name(step: &Step) -> String {
    let doing = match step.change {
        Change::Grow => "grow",
        Change::Shrink => "shrink",
        Change::Add => "build",
        Change::Remove => "take apart",
        Change::Move => "move",
    };
    format!("{doing} {} {}", crate::draft::kind_name(step.kind), step.part.0)
}

/// Where a running round stands, for a line of text or a bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Standing {
    /// Of the round's time, 0 to 1.
    pub fraction: f64,
    pub left_s: f64,
    /// The step under way, its index counted from one, and how far through it is.
    pub step: Option<(String, usize, f64)>,
    pub total: usize,
}

pub fn standing(plan: &Plan, now_s: f64) -> Standing {
    let duration_s = plan.duration_s();
    let since = now_s - plan.round().start_s;
    let progress = plan.at(now_s);
    Standing {
        fraction: if duration_s > 0.0 { (since / duration_s).clamp(0.0, 1.0) } else { 1.0 },
        left_s: (duration_s - since).max(0.0),
        step: progress.current.map(|(i, f)| (step_name(&plan.steps()[i]), i + 1, f)),
        total: plan.steps().len(),
    }
}

impl Standing {
    pub fn line(&self) -> String {
        let to_go = span(self.left_s);
        match &self.step {
            Some((what, n, f)) => format!("refitting: {what}, {:.0}%, step {n} of {}, {to_go} to go", f * 100.0, self.total),
            None => format!("refitting: {to_go} to go"),
        }
    }
}

/// A duration a refit is measured in: days, or years past a few hundred of them.
pub fn span(seconds: f64) -> String {
    let days = seconds / 86_400.0;
    if days < 400.0 { format!("{days:.1} days") } else { format!("{:.1} years", days / 365.25) }
}

/// Where the last Apply stands. A refusal belongs to the target it refused, so it goes quiet as
/// soon as the draft moves on.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Applying {
    #[default]
    Idle,
    Sent(Form),
    Refused { target: Form, why: String },
}

impl Applying {
    pub fn refusal(&self, draft: &Draft) -> Option<&str> {
        match self {
            Applying::Refused { target, why } if *target == draft.form => Some(why),
            _ => None,
        }
    }
}

/// Why Apply cannot be pressed, in the order a player would want to hear it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    Offline,
    Sent,
    Refitting,
    UnderWay,
    NoChange,
}

impl Blocked {
    pub fn reason(self) -> &'static str {
        match self {
            Blocked::Offline => "no shard to refit at",
            Blocked::Sent => "waiting for the shard",
            Blocked::Refitting => "a refit is running",
            Blocked::UnderWay => "under way: cut the drive before refitting",
            Blocked::NoChange => "no change to apply",
        }
    }
}

/// What Apply needs to know about the ship.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Situation {
    pub remote: bool,
    pub under_way: bool,
    pub refitting: bool,
}

impl Situation {
    pub fn of(session: &crate::session::Session) -> Self {
        let now = session.coordinate_time_s();
        Self {
            remote: session.remote,
            under_way: session.ship.motion.is_under_way(),
            refitting: session.ship.is_refitting(now),
        }
    }
}

pub fn gate(draft: &Draft, applying: &Applying, ship: Situation) -> Result<(), Blocked> {
    if !ship.remote {
        Err(Blocked::Offline)
    } else if matches!(applying, Applying::Sent(_)) {
        Err(Blocked::Sent)
    } else if ship.refitting {
        Err(Blocked::Refitting)
    } else if ship.under_way {
        Err(Blocked::UnderWay)
    } else if draft.form == draft.ship {
        Err(Blocked::NoChange)
    } else {
        Ok(())
    }
}

/// What the draft is edited against: the target of a round the shard is running, since that is
/// what the ship becomes, and otherwise the form the ship has settled into, partway after a cancel.
pub fn base(fitting: &lc_world::fitting::Fitting) -> &Form {
    fitting.refit().map_or(fitting.form(), Plan::target)
}

#[cfg(test)]
mod tests {
    use lc_world::form::{Kind, PartId};
    use lc_world::refit::rounds::Round;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    /// The data taken apart and a bay built, from full storage.
    fn plan() -> Plan {
        let from = Form::starting();
        let mut d = Draft::new(from.clone());
        let data = d.form.parts.iter().find(|p| p.kind == Kind::Data).unwrap().id;
        d.apply(&d.remove(data).unwrap(), &B).unwrap();
        let storage = d.form.parts.iter().find(|p| p.kind == Kind::Storage).unwrap().id;
        let bay = d.add(storage, Kind::Bay, crate::draft::PRIMITIVES[3], glam::DVec3::NEG_Y, &B).unwrap();
        d.apply(&bay, &B).unwrap();
        let stored_j = Capacities::of(&from, &B).storage_j;
        Round { from, target: d.form, stored_j, start_s: 1000.0 }.solve(&B).expect("the round solves")
    }

    #[test]
    fn the_budget_balances_what_is_taken_apart_against_what_is_built() {
        let p = plan();
        let budget = Budget::of(&p, &B);
        let gross = |phase| p.steps().iter().filter(|s| s.change.phase() == phase).map(|s| s.gross_j).sum::<f64>();
        assert!(gross(Phase::Dismantle) > 0.0 && gross(Phase::Build) > 0.0, "premise: both phases work");
        assert_eq!(budget.spent_j, gross(Phase::Build));
        assert!((budget.available_j - (p.round().stored_j + 0.95 * gross(Phase::Dismantle))).abs() < 1.0);
        // Storage began full, so nothing the dismantle returns fits and all of it vents.
        assert!((budget.peak_j - p.round().stored_j.min(budget.peak_capacity_j)).abs() < 1.0, "{budget:?}");
        assert!(budget.vented_j > 0.0, "a full store has no room for the return");
        assert_eq!(budget.vented_j, p.vented_j());
    }

    #[test]
    fn the_rows_follow_the_round_through_its_phases() {
        let p = plan();
        let start = p.round().start_s;
        let before = phases(&p, start);
        assert_eq!(before.iter().map(|(ph, _)| *ph).collect::<Vec<_>>(), [Phase::Dismantle, Phase::Move, Phase::Build]);
        assert!(before.iter().flat_map(|(_, r)| r).all(|r| r.state == StepState::Waiting));
        assert_eq!(before[0].1[0].what, format!("take apart data {}", before_data()));

        let first = &p.steps()[0];
        let midway = phases(&p, start + first.begins_s + 0.5 * first.duration_s);
        assert_eq!(midway[0].1[0].state, StepState::Working(0.5));

        let built = phases(&p, start + p.duration_s());
        assert!(built.iter().flat_map(|(_, r)| r).all(|r| r.state == StepState::Done), "{built:?}");
        let build = &built[2].1;
        assert!(build.iter().any(|r| r.what.starts_with("build bay") && r.stored_j < 0.0), "a build is paid from storage: {build:?}");
    }

    fn before_data() -> u16 {
        Form::starting().parts.iter().find(|p| p.kind == Kind::Data).unwrap().id.0
    }

    #[test]
    fn standing_counts_steps_from_one_and_time_to_the_end() {
        let p = plan();
        let first = &p.steps()[0];
        let now = p.round().start_s + first.begins_s + 0.25 * first.duration_s;
        let s = standing(&p, now);
        assert_eq!(s.step.as_ref().map(|(_, n, _)| *n), Some(1));
        assert!((s.left_s - (p.duration_s() - 0.25 * first.duration_s)).abs() < 1e-6);
        assert!(s.line().contains("step 1 of"), "{}", s.line());
        assert_eq!(standing(&p, p.round().start_s + 2.0 * p.duration_s()).fraction, 1.0);
    }

    fn edited() -> Draft {
        let mut d = Draft::new(Form::starting());
        d.apply(&d.twist(PartId(4), 0.3).unwrap(), &B).unwrap();
        d
    }

    const DOCKED: Situation = Situation { remote: true, under_way: false, refitting: false };

    #[test]
    fn apply_is_open_only_for_a_change_on_a_docked_idle_ship() {
        assert_eq!(gate(&edited(), &Applying::Idle, DOCKED), Ok(()));
        assert_eq!(gate(&Draft::new(Form::starting()), &Applying::Idle, DOCKED), Err(Blocked::NoChange));
        assert_eq!(gate(&edited(), &Applying::Idle, Situation { under_way: true, ..DOCKED }), Err(Blocked::UnderWay));
        assert_eq!(gate(&edited(), &Applying::Idle, Situation { refitting: true, ..DOCKED }), Err(Blocked::Refitting));
        assert_eq!(gate(&edited(), &Applying::Idle, Situation { remote: false, ..DOCKED }), Err(Blocked::Offline));
        let sent = Applying::Sent(edited().form);
        assert_eq!(gate(&edited(), &sent, DOCKED), Err(Blocked::Sent), "one answer at a time");
    }

    #[test]
    fn a_refusal_is_shown_only_against_the_target_it_refused() {
        let d = edited();
        let refused = Applying::Refused { target: d.form.clone(), why: "part 2 must point fore or aft".into() };
        assert_eq!(refused.refusal(&d), Some("part 2 must point fore or aft"));
        assert_eq!(gate(&d, &refused, DOCKED), Ok(()), "a refused target may be sent again");
        let mut moved = d.clone();
        moved.apply(&moved.twist(PartId(4), 0.6).unwrap(), &B).unwrap();
        assert_eq!(refused.refusal(&moved), None);
    }

    #[test]
    fn the_draft_is_edited_against_the_target_of_a_running_round() {
        let p = plan();
        let mut fitting = lc_world::fitting::Fitting::full(Form::starting(), B, 1000.0);
        assert_eq!(base(&fitting), &Form::starting());
        fitting.begin_refit(p.clone());
        assert_eq!(base(&fitting), p.target());
    }

    #[test]
    fn a_draft_is_based_on_its_ship_in_any_order() {
        let d = edited();
        let mut shuffled = Form::starting();
        shuffled.parts.reverse();
        assert!(d.is_based_on(&shuffled));
        assert!(!d.is_based_on(&d.form), "the draft's own edit is not the ship");
        shuffled.parts.pop();
        assert!(!d.is_based_on(&shuffled), "a part fewer");
    }
}
