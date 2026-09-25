//! A refit round: from one form to another in three strict phases, one step per part change.
//!
//! As with the loadout planner above it, the [`Round`] is the recipe, both ends run
//! [`Round::solve`], and progress is a closed form in time. The ledger is the store's own: what
//! the round takes and returns, with drain and income left to the caller. A form partway through
//! a round need not validate, since a moved part may wait for a parent the build phase has not
//! made yet. See `lightcone/docs/29-ship-form.md` §Refits.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::fitting::{Balance, C2};
use crate::form::capacity::{Capacities, Transfer, part_kg};
use crate::form::{Form, FormError, Kind, Part, PartId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Dismantle,
    Move,
    Build,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Grow,
    Shrink,
    Add,
    /// Also the first half of a reshape or a change of kind, whose second is an `Add` of the same
    /// part.
    Remove,
    /// Carries everything that hung from the part when the round began.
    Move,
}

impl Change {
    pub fn phase(self) -> Phase {
        match self {
            Change::Shrink | Change::Remove => Phase::Dismantle,
            Change::Move => Phase::Move,
            Change::Grow | Change::Add => Phase::Build,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub part: PartId,
    pub change: Change,
    pub kind: Kind,
    /// Seconds after the round began.
    pub begins_s: f64,
    pub duration_s: f64,
    /// Mass-energy built or taken apart, joules. Zero for a move.
    pub gross_j: f64,
    /// Into storage over the step, joules: minus a build's cost, or as much of a return as there
    /// is room for.
    pub stored_j: f64,
    /// Taken out of storage at the step's end, joules: what a shrinking store held past its new
    /// capacity. Part of `vented_j`.
    pub spilled_j: f64,
    /// Burst into the field at the step's end, joules.
    pub vented_j: f64,
    /// The part once the step is done, `None` once it is gone.
    pub after: Option<Part>,
}

impl Step {
    pub fn ends_s(&self) -> f64 {
        self.begins_s + self.duration_s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub from: Form,
    pub target: Form,
    /// Stored energy when it began, joules.
    pub stored_j: f64,
    /// Coordinate seconds it began.
    pub start_s: f64,
}

impl Round {
    pub fn solve(&self, balance: &Balance) -> Result<Plan, Refusal> {
        Plan::new(self.clone(), balance)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    Form(FormError),
    /// Less than `min_drone_m3` of drone in the target.
    TooFewDrones { drone_m3: f64 },
    /// The target's Mind is not the ship's Mind.
    Mind(PartId),
    /// A step would begin with no drone to do it, as when every drone is being reshaped.
    NoDrones { part: PartId },
    /// The build phase costs more than the dismantle phase leaves in storage.
    Energy { short_j: f64 },
}

/// A burst of heat from a finished step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vent {
    pub step: usize,
    /// Coordinate seconds.
    pub at_s: f64,
    pub joules: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// Counting only finished steps.
    pub form: Form,
    /// The round's ledger: what it began with, plus what it has returned, less what it has taken.
    pub stored_j: f64,
    /// Mass in the step under way that is in neither the form nor the store: a part half built,
    /// or (negative) the half of one already taken apart, less the return awaiting its vent.
    pub in_hand_kg: f64,
    /// The index of the step under way, and how far through it is.
    pub current: Option<(usize, f64)>,
    pub finished: usize,
}

/// What stopping a round leaves.
#[derive(Clone, Debug, PartialEq)]
pub struct Canceled {
    pub form: Form,
    pub stored_j: f64,
    /// A burst, at once: what the reversed step returned and storage had no room for.
    pub vented_j: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    round: Round,
    balance: Balance,
    steps: Vec<Step>,
}

type Parts = BTreeMap<PartId, Part>;

fn capacities(parts: &Parts, balance: &Balance) -> Capacities {
    Capacities::of(&Form { parts: parts.values().copied().collect() }, balance)
}

fn work_factor(kind: Kind, balance: &Balance) -> f64 {
    if kind == Kind::Data { balance.data_work_factor } else { 1.0 }
}

fn apply(parts: &mut Parts, id: PartId, after: Option<Part>) {
    match after {
        Some(part) => parts.insert(id, part),
        None => parts.remove(&id),
    };
}

/// A step before it is timed.
struct Pending {
    part: PartId,
    change: Change,
    kind: Kind,
    transfer: Transfer,
    after: Option<Part>,
}

impl Pending {
    fn remove(part: &Part, balance: &Balance) -> Self {
        let transfer = Transfer::of(Some(part), None, balance);
        Self { part: part.id, change: Change::Remove, kind: part.kind, transfer, after: None }
    }

    fn add(part: &Part, balance: &Balance) -> Self {
        let transfer = Transfer::of(None, Some(part), balance);
        Self { part: part.id, change: Change::Add, kind: part.kind, transfer, after: Some(*part) }
    }
}

impl Plan {
    fn new(round: Round, balance: &Balance) -> Result<Self, Refusal> {
        round.target.validate().map_err(Refusal::Form)?;
        let drone_m3: f64 = round.target.parts.iter().filter(|p| p.kind == Kind::Drone).map(|p| p.volume_m3).sum();
        if !(drone_m3 >= balance.min_drone_m3) {
            return Err(Refusal::TooFewDrones { drone_m3 });
        }
        let from: Parts = round.from.parts.iter().map(|p| (p.id, *p)).collect();
        let target: Parts = round.target.parts.iter().map(|p| (p.id, *p)).collect();
        let mind = *round.target.parts.iter().find(|p| p.kind == Kind::Mind).expect("validated");
        if from.get(&mind.id).map(|p| p.kind) != Some(Kind::Mind) {
            return Err(Refusal::Mind(mind.id));
        }

        let mut dismantles = Vec::new();
        let mut moves = Vec::new();
        let mut builds = Vec::new();
        for id in from.keys().chain(target.keys()).copied().collect::<BTreeSet<_>>() {
            match (from.get(&id), target.get(&id)) {
                (Some(f), None) => dismantles.push(Pending::remove(f, balance)),
                (None, Some(t)) => builds.push(Pending::add(t, balance)),
                (Some(f), Some(t)) => {
                    // The Mind's stored shape and size are ignored, and it cannot move.
                    if f.kind == Kind::Mind {
                        continue;
                    }
                    if f.kind != t.kind || f.primitive != t.primitive {
                        dismantles.push(Pending::remove(f, balance));
                        builds.push(Pending::add(t, balance));
                    } else if f.volume_m3 != t.volume_m3 {
                        let resized = Part { volume_m3: t.volume_m3, ..*f };
                        let transfer = Transfer::of(Some(f), Some(t), balance);
                        let (change, after, list) = match transfer {
                            Transfer::Build { .. } => (Change::Grow, *t, &mut builds),
                            Transfer::Dismantle { .. } => (Change::Shrink, resized, &mut dismantles),
                        };
                        list.push(Pending { part: id, change, kind: f.kind, transfer, after: Some(after) });
                    }
                    if f.placement != t.placement {
                        moves.push(id);
                    }
                }
                (None, None) => unreachable!(),
            }
        }
        // Stable, so each group stays in id order.
        dismantles.sort_by_key(|p| p.kind == Kind::Drone);
        builds.sort_by_key(|p| p.kind != Kind::Drone);

        let mut parts = from.clone();
        parts.insert(mind.id, mind);
        let mut steps = Vec::new();
        let mut elapsed = 0.0;
        let mut stored = round.stored_j;

        // Room is judged against the capacity the whole phase leaves, so a return is not taken in
        // by a store that a later step of the same phase takes apart.
        let left_j = {
            let mut after = parts.clone();
            for p in &dismantles {
                apply(&mut after, p.part, p.after);
            }
            capacities(&after, balance).storage_j
        };
        for p in dismantles {
            let power = capacities(&parts, balance).building_w;
            if !(power > 0.0) {
                return Err(Refusal::NoDrones { part: p.part });
            }
            let Transfer::Dismantle { gross_j, returned_j } = p.transfer else { unreachable!() };
            let duration_s = gross_j * work_factor(p.kind, balance) / power;
            apply(&mut parts, p.part, p.after);
            let kept = returned_j.min((left_j - stored).max(0.0));
            stored += kept;
            let spilled = (stored - capacities(&parts, balance).storage_j).max(0.0);
            stored -= spilled;
            let vented_j = (returned_j - kept) + spilled;
            steps.push(Step {
                part: p.part,
                change: p.change,
                kind: p.kind,
                begins_s: elapsed,
                duration_s,
                gross_j,
                stored_j: kept,
                spilled_j: spilled,
                vented_j,
                after: p.after,
            });
            elapsed += duration_s;
        }

        let children = {
            let mut children: HashMap<PartId, Vec<PartId>> = HashMap::new();
            for part in from.values() {
                if let Some(placement) = part.placement {
                    children.entry(placement.parent).or_default().push(part.id);
                }
            }
            children
        };
        let depth = |mut id: PartId| {
            let mut depth = 0;
            while let Some(parent) = from.get(&id).and_then(|p| p.placement).map(|p| p.parent) {
                id = parent;
                depth += 1;
                if depth > from.len() {
                    break;
                }
            }
            depth
        };
        moves.sort_by_key(|&id| (depth(id), id));
        for id in moves {
            let power = capacities(&parts, balance).building_w;
            if !(power > 0.0) {
                return Err(Refusal::NoDrones { part: id });
            }
            // Through a part already taken apart too: what hung from it is still carried.
            let mut carried_j = 0.0;
            let mut stack = vec![id];
            while let Some(at) = stack.pop() {
                if let Some(part) = parts.get(&at) {
                    carried_j += part_kg(part, balance) * C2 * work_factor(part.kind, balance);
                }
                stack.extend(children.get(&at).into_iter().flatten());
            }
            let duration_s = balance.move_work_factor * carried_j / power;
            let after = parts.get(&id).map(|p| Part { placement: target[&id].placement, ..*p });
            apply(&mut parts, id, after);
            steps.push(Step {
                part: id,
                change: Change::Move,
                kind: target[&id].kind,
                begins_s: elapsed,
                duration_s,
                gross_j: 0.0,
                stored_j: 0.0,
                spilled_j: 0.0,
                vented_j: 0.0,
                after,
            });
            elapsed += duration_s;
        }

        let cost_j = |p: &Pending| match p.transfer {
            Transfer::Build { cost_j } => cost_j,
            Transfer::Dismantle { .. } => unreachable!(),
        };
        let total_j: f64 = builds.iter().map(cost_j).sum();
        if total_j > stored {
            return Err(Refusal::Energy { short_j: total_j - stored });
        }
        for p in builds {
            let power = capacities(&parts, balance).building_w;
            if !(power > 0.0) {
                return Err(Refusal::NoDrones { part: p.part });
            }
            let gross_j = cost_j(&p);
            let duration_s = gross_j * work_factor(p.kind, balance) / power;
            apply(&mut parts, p.part, p.after);
            steps.push(Step {
                part: p.part,
                change: p.change,
                kind: p.kind,
                begins_s: elapsed,
                duration_s,
                gross_j,
                stored_j: -gross_j,
                spilled_j: 0.0,
                vented_j: 0.0,
                after: p.after,
            });
            elapsed += duration_s;
        }
        Ok(Self { round, balance: *balance, steps })
    }

    pub fn round(&self) -> &Round {
        &self.round
    }

    pub fn target(&self) -> &Form {
        &self.round.target
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn duration_s(&self) -> f64 {
        self.steps.last().map_or(0.0, Step::ends_s)
    }

    pub fn is_done(&self, now_s: f64) -> bool {
        now_s >= self.round.start_s + self.duration_s()
    }

    /// Energy taken from storage over the whole round, joules. Negative when it returns more.
    pub fn net_j(&self) -> f64 {
        self.steps.iter().map(|s| s.spilled_j - s.stored_j).sum()
    }

    /// Everything the round bursts into the field, joules, not counting the 5% a dismantling
    /// radiates as it goes.
    pub fn vented_j(&self) -> f64 {
        self.steps.iter().map(|s| s.vented_j).sum()
    }

    pub fn vents(&self) -> impl Iterator<Item = Vent> + '_ {
        let start_s = self.round.start_s;
        self.steps.iter().enumerate().filter(|(_, s)| s.vented_j > 0.0).map(move |(step, s)| Vent {
            step,
            at_s: start_s + s.ends_s(),
            joules: s.vented_j,
        })
    }

    fn start(&self) -> Parts {
        let mut parts: Parts = self.round.from.parts.iter().map(|p| (p.id, *p)).collect();
        let mind = self.round.target.parts.iter().find(|p| p.kind == Kind::Mind).expect("solved");
        parts.insert(mind.id, *mind);
        parts
    }

    pub fn at(&self, now_s: f64) -> Progress {
        let since = now_s - self.round.start_s;
        let mut parts = self.start();
        let mut progress =
            Progress { form: Form::default(), stored_j: self.round.stored_j, in_hand_kg: 0.0, current: None, finished: 0 };
        for (i, step) in self.steps.iter().enumerate() {
            if since >= step.ends_s() {
                apply(&mut parts, step.part, step.after);
                progress.stored_j += step.stored_j - step.spilled_j;
                progress.finished += 1;
                continue;
            }
            if since > step.begins_s {
                let fraction = (since - step.begins_s) / step.duration_s;
                progress.stored_j += step.stored_j * fraction;
                progress.current = Some((i, fraction));
                progress.in_hand_kg = fraction
                    * match step.change.phase() {
                        Phase::Build => step.gross_j,
                        Phase::Move => 0.0,
                        // Taken apart, less what reached the store and the loss radiated as it went.
                        Phase::Dismantle => -(1.0 - self.balance.recovery) * step.gross_j - step.stored_j,
                    }
                    / C2;
            }
            break;
        }
        progress.form = Form { parts: parts.into_values().collect() };
        progress
    }

    /// Stop at `now_s` with `stored_j` actually in storage. Finished steps stay, and the step under
    /// way is undone. A build returns what it had taken at the recovery rate, as far as there is
    /// room; a dismantling puts back into the part what it had returned to storage.
    pub fn cancel(&self, now_s: f64, stored_j: f64) -> Canceled {
        let progress = self.at(now_s);
        let form = progress.form;
        let Some((i, fraction)) = progress.current else {
            return Canceled { form, stored_j, vented_j: 0.0 };
        };
        let step = &self.steps[i];
        match step.change.phase() {
            Phase::Build => {
                let returned_j = self.balance.recovery * step.gross_j * fraction;
                let room_j = (Capacities::of(&form, &self.balance).storage_j - stored_j).max(0.0);
                let kept = returned_j.min(room_j);
                Canceled { form, stored_j: stored_j + kept, vented_j: returned_j - kept }
            }
            Phase::Dismantle => {
                Canceled { form, stored_j: (stored_j - step.stored_j * fraction).max(0.0), vented_j: 0.0 }
            }
            Phase::Move => Canceled { form, stored_j, vented_j: 0.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::*;
    use crate::form::{Mount, Placement, Primitive};

    const B: Balance = Balance::DEFAULT;
    const TANK: Primitive = Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) };
    const ROD: Primitive = Primitive::Capsule { length: 4.0 };
    const NOZZLE: Primitive = Primitive::Frustum { length: 1.5, taper: 0.6 };

    fn part(id: u16, kind: Kind, primitive: Primitive, slots: f64, parent: u16) -> Part {
        let placement = Placement {
            parent: PartId(parent),
            mount: Mount::Attached { anchor: DVec3::X, standoff: 0.0 },
            twist: 0.0,
            tilt: DVec2::ZERO,
            blend: 0.0,
            mirror: false,
        };
        Part { id: PartId(id), kind, primitive, volume_m3: slots * B.slot_volume_m3, placement: Some(placement) }
    }

    /// Two slots of storage, which hold 10 ME, a slot of drone, and an engine with living space
    /// hanging from it.
    fn ship() -> Form {
        Form {
            parts: vec![
                Part::mind(PartId(0), B.min_part_m3),
                part(1, Kind::Storage, TANK, 2.0, 0),
                part(2, Kind::Drone, ROD, 1.0, 1),
                part(3, Kind::Engine, NOZZLE, 1.0, 1),
                part(4, Kind::Living, ROD, 0.5, 3),
            ],
        }
    }

    fn with(form: &Form, id: u16, edit: impl FnOnce(&mut Part)) -> Form {
        let mut form = form.clone();
        edit(form.parts.iter_mut().find(|p| p.id == PartId(id)).unwrap());
        form
    }

    fn without(form: &Form, ids: &[u16]) -> Form {
        Form { parts: form.parts.iter().filter(|p| !ids.contains(&p.id.0)).copied().collect() }
    }

    fn adding(form: &Form, parts: &[Part]) -> Form {
        Form { parts: form.parts.iter().chain(parts).copied().collect() }
    }

    fn get(form: &Form, id: u16) -> Part {
        *form.parts.iter().find(|p| p.id == PartId(id)).unwrap()
    }

    fn sorted(form: &Form) -> Vec<Part> {
        let mut parts = form.parts.clone();
        parts.sort_by_key(|p| p.id);
        parts
    }

    fn energy(part: &Part) -> f64 {
        part_kg(part, &B) * C2
    }

    fn capacity(form: &Form) -> f64 {
        Capacities::of(form, &B).storage_j
    }

    fn power(form: &Form) -> f64 {
        Capacities::of(form, &B).building_w
    }

    fn solve_with(balance: &Balance, from: &Form, target: &Form, stored_j: f64) -> Result<Plan, Refusal> {
        Round { from: from.clone(), target: target.clone(), stored_j, start_s: 100.0 }.solve(balance)
    }

    fn solve(from: &Form, target: &Form, stored_j: f64) -> Result<Plan, Refusal> {
        solve_with(&B, from, target, stored_j)
    }

    fn close(a: f64, b: f64) -> bool {
        ((a - b) / b).abs() < 1.0e-9
    }

    fn changes(plan: &Plan) -> Vec<(u16, Change)> {
        plan.steps().iter().map(|s| (s.part.0, s.change)).collect()
    }

    #[test]
    fn nothing_to_do_is_done_at_once() {
        let plan = solve(&ship(), &ship(), 0.0).unwrap();
        assert!(plan.steps().is_empty());
        assert!(plan.is_done(100.0));
        assert_eq!(plan.net_j(), 0.0);
    }

    #[test]
    fn a_round_that_cannot_pay_for_its_builds_is_refused() {
        let engine = part(5, Kind::Engine, NOZZLE, 1.0, 1);
        let cost = energy(&engine);
        let target = adding(&ship(), &[engine]);
        let Err(Refusal::Energy { short_j }) = solve(&ship(), &target, 0.5 * cost) else { panic!("refused") };
        assert!(close(short_j, 0.5 * cost));
        assert!(solve(&ship(), &target, cost).is_ok());

        // What the dismantle phase returns pays for the build phase.
        let living = energy(&get(&ship(), 4));
        let stored = cost - 0.95 * living;
        assert!(solve(&ship(), &target, 0.99 * stored).is_err());
        let plan = solve(&ship(), &without(&target, &[4]), stored).unwrap();
        assert_eq!(changes(&plan), [(4, Change::Remove), (5, Change::Add)]);
        assert!(close(plan.net_j(), stored));
    }

    #[test]
    fn an_overflow_is_vented_at_the_end_of_the_step_that_frees_it() {
        let from = ship();
        let full = capacity(&from);
        let plan = solve(&from, &without(&from, &[3, 4]), full).unwrap();
        assert_eq!(changes(&plan), [(3, Change::Remove), (4, Change::Remove)]);
        let vents: Vec<_> = plan.vents().collect();
        assert_eq!(vents.len(), 2);
        for (vent, id) in vents.iter().zip([3, 4]) {
            let step = plan.steps()[vent.step];
            assert_eq!(step.part.0, id);
            assert_eq!(vent.at_s, 100.0 + step.ends_s());
            assert!(close(vent.joules, 0.95 * energy(&get(&from, id))));
        }
        assert_eq!(plan.net_j(), 0.0);
        // Nothing is vented before the step ends, and the ledger never rises past full.
        assert!(close(plan.at(100.0 + 0.5 * plan.steps()[0].duration_s).stored_j, full));

        let half = solve(&from, &without(&from, &[3, 4]), 0.5 * full).unwrap();
        assert_eq!(half.vents().count(), 0);
    }

    #[test]
    fn room_is_what_the_phase_leaves_after_its_own_storage_losses() {
        let from = adding(&ship(), &[part(9, Kind::Storage, TANK, 2.0, 1)]);
        // The living space goes first, then the second store shrinks by half.
        let target = with(&without(&from, &[4]), 9, |p| p.volume_m3 = B.slot_volume_m3);
        let left = capacity(&target);
        let living = 0.95 * energy(&get(&from, 4));
        let plan = solve(&from, &target, left - 0.1 * living).unwrap();
        assert_eq!(changes(&plan), [(4, Change::Remove), (9, Change::Shrink)]);
        let [first, second] = [plan.steps()[0], plan.steps()[1]];
        // There is room now, but not once the store is smaller: the living space's return is what
        // vents, when it is taken apart.
        assert!(close(first.stored_j, 0.1 * living));
        assert!(close(first.vented_j, 0.9 * living));
        assert_eq!(second.spilled_j, 0.0);
        assert!(close(second.vented_j, second.gross_j * 0.95));
        assert!(close(plan.at(1.0e30).stored_j, left));

        // A full store that shrinks spills what it held past its new capacity.
        let full = capacity(&from);
        let plan = solve(&from, &target, full).unwrap();
        let [first, second] = [plan.steps()[0], plan.steps()[1]];
        assert!(close(first.vented_j, living));
        assert!(close(second.spilled_j, full - left));
        assert!(close(second.vented_j, full - left + 0.95 * second.gross_j));
        assert!(close(plan.net_j(), full - left));
    }

    #[test]
    fn a_reshape_loses_five_percent_of_all_of_it() {
        let from = ship();
        let target = with(&from, 3, |p| p.primitive = Primitive::Cylinder { length: 3.0 });
        let plan = solve(&from, &target, 0.5 * capacity(&from)).unwrap();
        assert_eq!(changes(&plan), [(3, Change::Remove), (3, Change::Add)]);
        // Without structure the two shapes weigh the same.
        assert!(close(plan.net_j(), 0.05 * energy(&get(&from, 3))));

        // With it, each is priced whole.
        let b = Balance { hull_areal_density: 500.0, ..B };
        let whole = |p: &Part| part_kg(p, &b) * C2;
        let plan = solve_with(&b, &from, &target, 0.5 * capacity(&from)).unwrap();
        let want = whole(&get(&target, 3)) - 0.95 * whole(&get(&from, 3));
        assert!(close(plan.net_j(), want), "{} vs {want}", plan.net_j());

        let rekind = with(&from, 3, |p| p.kind = Kind::Storage);
        let plan = solve(&from, &rekind, 0.5 * capacity(&from)).unwrap();
        assert_eq!(changes(&plan), [(3, Change::Remove), (3, Change::Add)]);
        assert_eq!(plan.steps()[0].kind, Kind::Engine);
        assert_eq!(plan.steps()[1].kind, Kind::Storage);
    }

    #[test]
    fn a_move_costs_nothing_and_its_time_is_what_it_carries() {
        let from = ship();
        let p = power(&from);
        let turned = |form: &Form, id| with(form, id, |p| p.placement.as_mut().unwrap().twist = 0.5);

        let engine = solve(&from, &turned(&from, 3), 0.0).unwrap();
        assert_eq!(changes(&engine), [(3, Change::Move)]);
        assert_eq!(engine.net_j(), 0.0);
        let carried = energy(&get(&from, 3)) + energy(&get(&from, 4));
        assert!(close(engine.duration_s(), 0.25 * carried / p));

        let leaf = solve(&from, &turned(&from, 4), 0.0).unwrap();
        assert!(close(leaf.duration_s(), 0.25 * energy(&get(&from, 4)) / p));

        let reparented = with(&from, 4, |p| p.placement.as_mut().unwrap().parent = PartId(2));
        assert_eq!(changes(&solve(&from, &reparented, 0.0).unwrap()), [(4, Change::Move)]);
        assert_eq!(sorted(&solve(&from, &reparented, 0.0).unwrap().at(1.0e30).form), sorted(&reparented));
    }

    #[test]
    fn moves_go_root_first() {
        // The child has the lower id, so id order alone would move it first.
        let from = Form {
            parts: vec![
                Part::mind(PartId(0), B.min_part_m3),
                part(2, Kind::Drone, ROD, 1.0, 0),
                part(5, Kind::Living, ROD, 0.5, 7),
                part(7, Kind::Engine, NOZZLE, 1.0, 0),
            ],
        };
        let turn = |p: &mut Part| p.placement.as_mut().unwrap().twist = 1.0;
        let target = with(&with(&from, 5, turn), 7, turn);
        let plan = solve(&from, &target, 0.0).unwrap();
        assert_eq!(changes(&plan), [(7, Change::Move), (5, Change::Move)]);
    }

    #[test]
    fn drones_built_first_speed_up_what_follows() {
        let from = ship();
        let engine = part(5, Kind::Engine, NOZZLE, 1.0, 1);
        let target = adding(&with(&from, 2, |p| p.volume_m3 = 3.0 * B.slot_volume_m3), &[engine]);
        let plan = solve(&from, &target, capacity(&from)).unwrap();
        assert_eq!(changes(&plan), [(2, Change::Grow), (5, Change::Add)]);
        let [grow, add] = [plan.steps()[0], plan.steps()[1]];
        assert!(close(grow.duration_s, grow.gross_j / power(&from)));
        // Three times the drones, a third of the time.
        assert!(close(add.duration_s, energy(&engine) / (3.0 * power(&from))));
    }

    #[test]
    fn drones_go_last_when_taken_apart() {
        let from = ship();
        let target = with(&without(&from, &[3, 4]), 2, |p| p.volume_m3 = 0.25 * B.slot_volume_m3);
        let plan = solve(&from, &target, 0.0).unwrap();
        assert_eq!(changes(&plan), [(3, Change::Remove), (4, Change::Remove), (2, Change::Shrink)]);
        assert!(close(plan.steps()[1].duration_s, energy(&get(&from, 4)) / power(&from)));
        assert!(close(plan.steps()[2].duration_s, plan.steps()[2].gross_j / power(&from)));
    }

    #[test]
    fn data_takes_its_work_factor_longer() {
        let from = ship();
        let data = part(5, Kind::Data, ROD, 1.0, 1);
        let plan = solve(&from, &adding(&from, &[data]), capacity(&from)).unwrap();
        assert!(close(plan.duration_s(), 3.0 * energy(&data) / power(&from)));
        let apart = solve(&adding(&from, &[data]), &from, 0.0).unwrap();
        assert!(close(apart.duration_s(), 3.0 * energy(&data) / power(&from)));
        // A move carrying it takes the same factor.
        let carried = with(&adding(&from, &[part(5, Kind::Data, ROD, 1.0, 3)]), 3, |p| {
            p.placement.as_mut().unwrap().twist = 0.5;
        });
        let moved = solve(&adding(&from, &[part(5, Kind::Data, ROD, 1.0, 3)]), &carried, 0.0).unwrap();
        let want = 0.25 * (energy(&get(&from, 3)) + energy(&get(&from, 4)) + 3.0 * energy(&data)) / power(&from);
        assert!(close(moved.duration_s(), want));
    }

    #[test]
    fn progress_is_linear_within_a_step_and_ends_at_the_target() {
        let from = ship();
        let engine = part(5, Kind::Engine, NOZZLE, 1.0, 1);
        let target = adding(&without(&from, &[4]), &[engine]);
        let stored = 0.5 * capacity(&from);
        let plan = solve(&from, &target, stored).unwrap();
        let build = plan.steps()[1];
        let mid = plan.at(100.0 + build.begins_s + 0.25 * build.duration_s);
        assert_eq!(mid.finished, 1);
        assert_eq!(mid.current, Some((1, 0.25)));
        let after_first = stored + plan.steps()[0].stored_j;
        assert!(close(mid.stored_j, after_first - 0.25 * build.gross_j));
        assert!(close(mid.in_hand_kg * C2, 0.25 * build.gross_j));
        assert!(!mid.form.parts.iter().any(|p| p.id == PartId(4) || p.id == PartId(5)));

        let taking = plan.at(100.0 + 0.5 * plan.steps()[0].duration_s);
        // Half the living space is gone: 95% of it into the store, 5% radiated.
        assert!(close(taking.stored_j - stored, 0.5 * 0.95 * plan.steps()[0].gross_j));
        assert!(close(-taking.in_hand_kg * C2, 0.5 * plan.steps()[0].gross_j));

        let end = plan.at(100.0 + plan.duration_s());
        assert_eq!(end.finished, 2);
        assert_eq!(sorted(&end.form), sorted(&target));
        assert!(close(end.stored_j, stored - plan.net_j()));
    }

    #[test]
    fn canceling_keeps_finished_steps_and_reverses_the_one_under_way() {
        let from = ship();
        let engine = part(5, Kind::Engine, NOZZLE, 1.0, 1);
        let living = part(6, Kind::Living, ROD, 0.5, 1);
        let target = adding(&with(&from, 2, |p| p.volume_m3 = 2.0 * B.slot_volume_m3), &[engine, living]);
        let plan = solve(&from, &target, capacity(&from)).unwrap();
        assert_eq!(changes(&plan), [(2, Change::Grow), (5, Change::Add), (6, Change::Add)]);
        let step = plan.steps()[1];
        let now = 100.0 + step.begins_s + 0.5 * step.duration_s;
        let ledger = plan.at(now).stored_j;
        let canceled = plan.cancel(now, ledger);
        assert_eq!(get(&canceled.form, 2).volume_m3, 2.0 * B.slot_volume_m3);
        assert!(!canceled.form.parts.iter().any(|p| p.id == PartId(5) || p.id == PartId(6)));
        assert!(close(canceled.stored_j, ledger + 0.95 * 0.5 * energy(&engine)));
        assert_eq!(canceled.vented_j, 0.0);

        // With the store refilled meanwhile, the return has nowhere to go.
        let full = capacity(&from);
        let canceled = plan.cancel(now, full);
        assert_eq!(canceled.stored_j, full);
        assert!(close(canceled.vented_j, 0.95 * 0.5 * energy(&engine)));

        // A dismantling under way is put back, and gives back what it had returned.
        let apart = solve(&from, &without(&from, &[4]), 0.0).unwrap();
        let now = 100.0 + 0.5 * apart.duration_s();
        let canceled = apart.cancel(now, apart.at(now).stored_j);
        assert_eq!(sorted(&canceled.form), sorted(&from));
        assert!(canceled.stored_j.abs() < 1.0e-12 * full);

        // Between steps there is nothing to reverse.
        let between = plan.cancel(100.0 + step.begins_s, 1.0);
        assert_eq!(between.stored_j, 1.0);
        assert_eq!(get(&between.form, 2).volume_m3, 2.0 * B.slot_volume_m3);
    }

    #[test]
    fn a_target_that_breaks_the_rules_is_refused() {
        let from = ship();
        let twice = adding(&from, &[part(3, Kind::Engine, NOZZLE, 1.0, 1)]);
        assert_eq!(solve(&from, &twice, 0.0).unwrap_err(), Refusal::Form(FormError::DuplicateId(PartId(3))));

        let starved = with(&from, 2, |p| p.volume_m3 = 5_000.0);
        assert_eq!(solve(&from, &starved, 0.0).unwrap_err(), Refusal::TooFewDrones { drone_m3: 5_000.0 });

        let usurped = adding(&without(&from, &[0]), &[Part::mind(PartId(9), B.min_part_m3)]);
        let usurped = Form {
            parts: usurped
                .parts
                .iter()
                .map(|p| match p.placement {
                    Some(mut place) if place.parent == PartId(0) => {
                        place.parent = PartId(9);
                        Part { placement: Some(place), ..*p }
                    }
                    _ => *p,
                })
                .collect(),
        };
        assert_eq!(solve(&from, &usurped, 0.0).unwrap_err(), Refusal::Mind(PartId(9)));

        // Reshaping the only drone leaves nothing to build it again with.
        let reshaped = with(&from, 2, |p| p.primitive = Primitive::Cylinder { length: 2.0 });
        assert_eq!(solve(&from, &reshaped, capacity(&from)).unwrap_err(), Refusal::NoDrones { part: PartId(2) });
    }

    #[test]
    fn the_minds_stored_shape_costs_nothing_to_change() {
        let from = ship();
        let target = with(&from, 0, |p| p.volume_m3 = 8.0e6);
        let plan = solve(&from, &target, 0.0).unwrap();
        assert!(plan.steps().is_empty());
        assert_eq!(sorted(&plan.at(100.0).form), sorted(&target));
    }
}
