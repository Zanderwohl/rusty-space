//! Rebuilding a ship: from one loadout to another, a module at a time.
//!
//! The order is the recipe — where it started, where it is going, what it had, and when — and
//! both ends run [`Order::solve`] over it, as they do a course. The plan is a list of steps, each
//! moving one module's energy (or one slot's) at the drones' combined power, so progress is a
//! closed form in time. See `lightcone/docs/19-ship-fitting.md`.

use crate::fitting::{Balance, C2, Loadout, Module};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Build(Module),
    Dismantle(Module),
    Grow,
    Shrink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortage {
    /// A module without a slot, or no drone left to build with.
    Unbuildable,
    /// Nothing left to dismantle and still not enough to build.
    Energy,
    NoDrones,
    CannotBuild(Module),
    CannotDismantle(Module),
}

/// Drones first, so every later step is faster for them. The hull grows after the drones.
pub const BUILD_ORDER: [Module; 5] = [Module::Drone, Module::Storage, Module::Engine, Module::Living, Module::Data];
/// Drones last, so there is always one left to finish the job.
pub const DISMANTLE_ORDER: [Module; 5] =
    [Module::Living, Module::Data, Module::Engine, Module::Storage, Module::Drone];

/// The first module `more` has more of than `fewer` that `order` has no place for.
fn unlisted(order: &[Module], fewer: &Loadout, more: &Loadout) -> Option<Module> {
    Module::ALL.into_iter().find(|&m| fewer.count(m) < more.count(m) && !order.contains(&m))
}

/// A refit as the arguments it is planned from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Order {
    pub from: Loadout,
    pub target: Loadout,
    /// Stored energy when it began, joules.
    pub stored_j: f64,
    /// Coordinate seconds it began.
    pub start_s: f64,
}

impl Order {
    pub fn solve(&self, balance: &Balance) -> Result<Refit, Shortage> {
        Refit::plan(*self, balance)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Planned {
    step: Step,
    /// Seconds after the refit began.
    begins_s: f64,
    duration_s: f64,
    /// Energy the step moves, joules: what a build takes, or what a dismantled thing was worth.
    gross_j: f64,
    /// What the step does to stored energy, joules, before drain. Less than a refund, or negative,
    /// when storage has no room.
    stored_j: f64,
    after: Loadout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refit {
    order: Order,
    recovery: f64,
    steps: Vec<Planned>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Counting only finished steps.
    pub loadout: Loadout,
    /// Stored energy the refit has taken so far, net of what it has returned, joules.
    pub consumed_j: f64,
    /// Mass in the step under way that is in neither the loadout nor the store: a module half
    /// built, or (negative) the half of one already taken apart.
    pub in_hand_kg: f64,
    /// What canceling now would return to storage, joules. Negative for a dismantling under way,
    /// whose kept refund goes back into the module.
    pub reversal_j: f64,
    /// The step under way, and how far through it is.
    pub current: Option<(Step, f64)>,
    pub finished: usize,
}

impl Refit {
    fn plan(order: Order, balance: &Balance) -> Result<Self, Shortage> {
        let target = order.target;
        if !target.is_buildable() {
            return Err(Shortage::Unbuildable);
        }
        if let Some(module) = unlisted(&BUILD_ORDER, &order.from, &target) {
            return Err(Shortage::CannotBuild(module));
        }
        if let Some(module) = unlisted(&DISMANTLE_ORDER, &target, &order.from) {
            return Err(Shortage::CannotDismantle(module));
        }
        let mut current = order.from;
        let mut stored = order.stored_j;
        let mut elapsed = 0.0;
        let mut steps = Vec::new();

        while current != target {
            if current.drones == 0 {
                return Err(Shortage::NoDrones);
            }
            let power = balance.refit_power_w(&current);
            let drain = balance.drain_w(&current);
            let slot_j = balance.slot_energy_j();
            // Builds pay as they go, so the drain over the step has to be affordable too.
            let affords = |gross: f64, duration_s: f64| stored - gross - drain * duration_s >= 0.0;
            let wants = |module: Module, loadout: &Loadout| loadout.count(module) < target.count(module);
            let spares = |module: Module, loadout: &Loadout| loadout.count(module) > target.count(module);

            let mut chosen: Option<(Step, f64, f64)> = None;
            let room = current.free_slots() > 0;
            for module in BUILD_ORDER {
                let (gross, duration_s) = (balance.build_energy_j(module), balance.build_s(module, power));
                if chosen.is_none() && wants(module, &current) && room && affords(gross, duration_s) {
                    chosen = Some((Step::Build(module), gross, duration_s));
                }
                if module == Module::Drone
                    && chosen.is_none()
                    && current.slots < target.slots
                    && affords(slot_j, slot_j / power)
                {
                    chosen = Some((Step::Grow, slot_j, slot_j / power));
                }
            }
            if chosen.is_none() && current.slots > target.slots && room {
                chosen = Some((Step::Shrink, slot_j, slot_j / power));
            }
            if chosen.is_none() {
                chosen = DISMANTLE_ORDER.into_iter().find(|&m| spares(m, &current)).map(|module| {
                    (Step::Dismantle(module), balance.build_energy_j(module), balance.build_s(module, power))
                });
            }
            let Some((step, gross_j, duration_s)) = chosen else {
                return Err(Shortage::Energy);
            };

            let mut after = current;
            match step {
                Step::Build(module) => *after.count_mut(module) += 1,
                Step::Dismantle(module) => *after.count_mut(module) -= 1,
                Step::Grow => after.slots += 1,
                Step::Shrink => after.slots -= 1,
            }
            let stored_j = match step {
                Step::Build(_) | Step::Grow => -gross_j,
                Step::Dismantle(_) | Step::Shrink => {
                    (stored + balance.recovery * gross_j).min(balance.capacity_j(&after)) - stored
                }
            };
            stored = (stored + stored_j - drain * duration_s).max(0.0);
            steps.push(Planned { step, begins_s: elapsed, duration_s, gross_j, stored_j, after });
            elapsed += duration_s;
            current = after;
        }
        Ok(Self { order, recovery: balance.recovery, steps })
    }

    pub fn order(&self) -> Order {
        self.order
    }

    pub fn target(&self) -> Loadout {
        self.order.target
    }

    pub fn steps(&self) -> impl Iterator<Item = Step> + '_ {
        self.steps.iter().map(|p| p.step)
    }

    pub fn duration_s(&self) -> f64 {
        self.steps.last().map_or(0.0, |p| p.begins_s + p.duration_s)
    }

    pub fn is_done(&self, now_s: f64) -> bool {
        now_s >= self.order.start_s + self.duration_s()
    }

    /// Energy taken from storage over the whole refit, before drain, joules.
    pub fn net_j(&self) -> f64 {
        -self.steps.iter().map(|p| p.stored_j).sum::<f64>()
    }

    /// Joules thrown away for want of room in storage, not counting the 5% a dismantling radiates.
    pub fn vented_j(&self) -> f64 {
        self.steps
            .iter()
            .filter(|p| matches!(p.step, Step::Dismantle(_) | Step::Shrink))
            .map(|p| self.recovery * p.gross_j - p.stored_j)
            .sum()
    }

    pub fn at(&self, now_s: f64) -> Progress {
        let since = now_s - self.order.start_s;
        let mut progress = Progress {
            loadout: self.order.from,
            consumed_j: 0.0,
            in_hand_kg: 0.0,
            reversal_j: 0.0,
            current: None,
            finished: 0,
        };
        for planned in &self.steps {
            if since >= planned.begins_s + planned.duration_s {
                progress.loadout = planned.after;
                progress.consumed_j -= planned.stored_j;
                progress.finished += 1;
                continue;
            }
            if since > planned.begins_s {
                let fraction = (since - planned.begins_s) / planned.duration_s;
                let moved = planned.gross_j * fraction;
                progress.consumed_j -= planned.stored_j * fraction;
                progress.current = Some((planned.step, fraction));
                match planned.step {
                    Step::Build(_) | Step::Grow => {
                        progress.in_hand_kg = moved / C2;
                        progress.reversal_j = self.recovery * moved;
                    }
                    // What was thrown away is not given back.
                    Step::Dismantle(_) | Step::Shrink => {
                        progress.in_hand_kg = -moved / C2;
                        progress.reversal_j = -planned.stored_j.max(0.0) * fraction;
                    }
                }
            }
            break;
        }
        progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn order(from: Loadout, target: Loadout, stored_me: f64) -> Order {
        Order { from, target, stored_j: stored_me * B.module_energy_j(), start_s: 100.0 }
    }

    fn plan(from: Loadout, target: Loadout, stored_me: f64) -> Result<Refit, Shortage> {
        order(from, target, stored_me).solve(&B)
    }

    /// The loadout the expected numbers below were worked out by hand against.
    const WORKED: Loadout = Loadout { living: 2, data: 0, ..Loadout::STARTING };

    #[test]
    fn nothing_to_do_is_done_at_once() {
        let refit = plan(Loadout::STARTING, Loadout::STARTING, 30.0).unwrap();
        assert_eq!(refit.steps().count(), 0);
        assert!(refit.is_done(100.0));
    }

    #[test]
    fn short_of_energy_it_takes_apart_before_it_builds() {
        let from = WORKED;
        let target = Loadout { living: 0, engines: 6, ..from };
        let refit = plan(from, target, 0.0).unwrap();
        let steps: Vec<_> = refit.steps().collect();
        assert_eq!(steps[0], Step::Dismantle(Module::Living));
        assert_eq!(refit.at(1.0e30).loadout, target);
        // Two taken apart pay for one built, and a little less than they were worth.
        let net = refit.net_j() / B.module_energy_j();
        assert!((net - (1.0 - 2.0 * 0.95)).abs() < 1.0e-12, "{net}");
    }

    #[test]
    fn with_energy_to_spare_it_builds_first() {
        let from = WORKED;
        let target = Loadout { living: 1, engines: 6, ..from };
        let steps: Vec<_> = plan(from, target, 30.0).unwrap().steps().collect();
        assert_eq!(steps, [Step::Build(Module::Engine), Step::Dismantle(Module::Living)]);
    }

    #[test]
    fn short_of_slots_it_takes_apart_before_it_builds() {
        let from = Loadout { storage: 6, drones: 2, living: 2, engines: 10, slots: 20, data: 0 };
        let target = Loadout { living: 0, engines: 12, ..from };
        let steps: Vec<_> = plan(from, target, 20.0).unwrap().steps().collect();
        assert_eq!(steps[0], Step::Dismantle(Module::Living));
    }

    #[test]
    fn drones_come_first_and_go_last() {
        let from = WORKED;
        let more = plan(from, Loadout { drones: 4, engines: 7, ..from }, 30.0).unwrap();
        let steps: Vec<_> = more.steps().collect();
        assert_eq!(&steps[..2], [Step::Build(Module::Drone); 2]);
        // And the engines after them are built twice as fast.
        let first = more.steps[0].duration_s;
        assert!((more.steps[2].duration_s / first - 0.5).abs() < 1.0e-12);

        let fewer = plan(from, Loadout { drones: 1, engines: 4, ..from }, 20.0).unwrap();
        let steps: Vec<_> = fewer.steps().collect();
        assert_eq!(steps.last(), Some(&Step::Dismantle(Module::Drone)));
    }

    #[test]
    fn a_refund_storage_has_no_room_for_is_thrown_away() {
        let me = B.module_energy_j();
        // Full, and asked to take apart two engines: both refunds go.
        let from = WORKED;
        let full = plan(from, Loadout { engines: 3, ..from }, 30.0).unwrap();
        // Less the little room the living drain makes during the first.
        assert!((full.vented_j() / me - 2.0 * 0.95).abs() < 1.0e-3);
        assert!(full.net_j().abs() < 1.0e-3 * me);
        // With a little room, nothing is.
        assert!(plan(from, Loadout { engines: 3, ..from }, 27.0).unwrap().vented_j() < 1.0e-9 * me);
        // A full storage module taken apart loses what it held as well as its refund.
        let storage = plan(from, Loadout { storage: 5, ..from }, 30.0).unwrap();
        assert!((storage.vented_j() / me - (5.0 + 0.95)).abs() < 1.0e-12);
        assert!((storage.net_j() / me - 5.0).abs() < 1.0e-12);
        // And canceling it halfway does not give back what was thrown away.
        let half = storage.at(100.0 + 0.5 * storage.duration_s());
        assert!(half.reversal_j.abs() < 1.0e-12 * me);
        assert!((half.consumed_j / me - 2.5).abs() < 1.0e-12);
    }

    #[test]
    fn an_impossible_target_says_what_it_is_short_of() {
        let from = WORKED;
        assert_eq!(
            plan(from, Loadout { engines: 20, ..from }, 30.0).unwrap_err(),
            Shortage::Unbuildable
        );
        assert_eq!(plan(from, Loadout { drones: 0, ..from }, 30.0).unwrap_err(), Shortage::Unbuildable);
        let bare = Loadout { storage: 0, drones: 1, living: 0, engines: 0, slots: 20, data: 0 };
        assert_eq!(plan(bare, Loadout { engines: 1, ..bare }, 0.0).unwrap_err(), Shortage::Energy);
    }

    #[test]
    fn a_data_module_costs_half_and_takes_three_times_as_long() {
        let from = Loadout { storage: 15, slots: 30, ..Loadout::STARTING };
        let refit = plan(from, Loadout { data: 2, ..from }, 75.0).unwrap();
        assert_eq!(refit.steps().collect::<Vec<_>>(), [Step::Build(Module::Data)]);
        assert!((refit.net_j() / B.module_energy_j() - 0.5).abs() < 1.0e-12);
        let engine = plan(from, Loadout { engines: 6, ..from }, 75.0).unwrap();
        assert!((refit.duration_s() / engine.duration_s() - 3.0).abs() < 1.0e-12);

        let apart = plan(from, Loadout { data: 0, ..from }, 70.0).unwrap();
        assert_eq!(apart.steps().collect::<Vec<_>>(), [Step::Dismantle(Module::Data)]);
        assert!((apart.net_j() / B.module_energy_j() + 0.95 * 0.5).abs() < 1.0e-12);
        assert!((apart.duration_s() / engine.duration_s() - 3.0).abs() < 1.0e-12);
    }

    #[test]
    fn every_module_has_a_place_to_be_built_and_taken_apart() {
        for module in Module::ALL {
            assert!(BUILD_ORDER.contains(&module), "{module:?} cannot be built");
            assert!(DISMANTLE_ORDER.contains(&module), "{module:?} cannot be taken apart");
        }
    }

    #[test]
    fn a_module_the_planner_has_no_place_for_is_named() {
        let from = WORKED;
        let more = Loadout { engines: 6, ..from };
        assert_eq!(unlisted(&[Module::Drone], &from, &more), Some(Module::Engine));
        assert_eq!(unlisted(&[Module::Drone], &more, &from), None);
        assert_eq!(unlisted(&BUILD_ORDER, &from, &more), None);
    }

    #[test]
    fn the_hull_grows_before_it_is_filled_and_shrinks_once_emptied() {
        let from = WORKED;
        let bigger = Loadout { engines: 12, slots: 22, ..from };
        let steps: Vec<_> = plan(from, bigger, 30.0).unwrap().steps().collect();
        assert_eq!(&steps[..2], [Step::Grow, Step::Grow]);
        let smaller = Loadout { slots: 15, ..from };
        let steps: Vec<_> = plan(from, smaller, 20.0).unwrap().steps().collect();
        assert_eq!(steps, [Step::Shrink; 5]);
    }

    #[test]
    fn the_timing_adds_up() {
        let from = WORKED;
        let refit = plan(from, Loadout { engines: 6, living: 3, ..from }, 30.0).unwrap();
        let week = 7.0 * 86_400.0;
        // Two drones, two modules: a week in all.
        assert!((refit.duration_s() / week - 1.0).abs() < 1.0e-12);
        let halfway = refit.at(100.0 + week * 0.25);
        assert_eq!(halfway.finished, 0);
        assert_eq!(halfway.current.map(|(s, _)| s), Some(Step::Build(Module::Engine)));
        assert!((halfway.consumed_j / (0.5 * B.module_energy_j()) - 1.0).abs() < 1.0e-12);
        assert!((halfway.in_hand_kg * C2 - halfway.consumed_j).abs() < 1.0e-3 * halfway.consumed_j);
        let done = refit.at(100.0 + week);
        assert_eq!(done.finished, 2);
        assert!((done.consumed_j / (2.0 * B.module_energy_j()) - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn canceling_keeps_what_finished_and_returns_most_of_what_did_not() {
        let from = WORKED;
        let refit = plan(from, Loadout { engines: 7, ..from }, 30.0).unwrap();
        let half_week = 3.5 * 86_400.0;
        let at = refit.at(100.0 + half_week * 1.5);
        assert_eq!(at.loadout.engines, 6);
        let (step, fraction) = at.current.unwrap();
        assert_eq!(step, Step::Build(Module::Engine));
        assert!((fraction - 0.5).abs() < 1.0e-12);
        assert!((at.reversal_j / (0.5 * 0.95 * B.module_energy_j()) - 1.0).abs() < 1.0e-12);
    }
}
