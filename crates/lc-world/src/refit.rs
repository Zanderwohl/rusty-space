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

/// Why a target cannot be reached from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortage {
    /// A module without a slot, or no drone left to build with.
    Unbuildable,
    /// Nothing left to dismantle and still not enough to build.
    Energy,
    /// A dismantling would return more than storage could hold.
    Capacity,
    /// No drones to do the work.
    NoDrones,
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
    /// The loadout once this step is done.
    after: Loadout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refit {
    order: Order,
    recovery: f64,
    steps: Vec<Planned>,
}

/// Where a refit has got to at an instant.
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
    /// whose refund so far goes back into the module.
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
            let module_j = balance.module_energy_j();
            // Builds pay as they go, so the drain over the step has to be affordable too.
            let affords = |gross: f64| stored - gross - drain * gross / power >= 0.0;
            // A refund has to fit in what storage there will be once the step is done.
            let holds = |refund: f64, after: &Loadout| {
                stored + refund <= balance.capacity_j(after) * (1.0 + 1.0e-12)
            };
            let wants = |module: Module, loadout: &Loadout| loadout.count(module) < target.count(module);
            let spares = |module: Module, loadout: &Loadout| loadout.count(module) > target.count(module);

            let mut chosen: Option<(Step, f64)> = None;
            let room = current.free_slots() > 0;
            for module in [Module::Drone, Module::Storage, Module::Engine, Module::Living] {
                if chosen.is_none() && wants(module, &current) && room && affords(module_j) {
                    chosen = Some((Step::Build(module), module_j));
                }
                // Drones before growing, the rest after.
                if module == Module::Drone
                    && chosen.is_none()
                    && current.slots < target.slots
                    && affords(balance.slot_energy_j())
                {
                    chosen = Some((Step::Grow, balance.slot_energy_j()));
                }
            }
            if chosen.is_none() && current.slots > target.slots && room {
                let mut after = current;
                after.slots -= 1;
                if holds(balance.recovery * balance.slot_energy_j(), &after) {
                    chosen = Some((Step::Shrink, balance.slot_energy_j()));
                }
            }
            if chosen.is_none() {
                for module in [Module::Living, Module::Engine, Module::Storage, Module::Drone] {
                    if chosen.is_some() || !spares(module, &current) {
                        continue;
                    }
                    let mut after = current;
                    *after.count_mut(module) -= 1;
                    if holds(balance.recovery * module_j, &after) {
                        chosen = Some((Step::Dismantle(module), module_j));
                    }
                }
            }
            let Some((step, gross_j)) = chosen else {
                let blocked_by_capacity = Module::ALL.iter().any(|m| spares(*m, &current));
                return Err(if blocked_by_capacity { Shortage::Capacity } else { Shortage::Energy });
            };

            let duration_s = gross_j / power;
            let mut after = current;
            match step {
                Step::Build(module) => {
                    *after.count_mut(module) += 1;
                    stored -= gross_j;
                }
                Step::Dismantle(module) => {
                    *after.count_mut(module) -= 1;
                    stored += balance.recovery * gross_j;
                }
                Step::Grow => {
                    after.slots += 1;
                    stored -= gross_j;
                }
                Step::Shrink => {
                    after.slots -= 1;
                    stored += balance.recovery * gross_j;
                }
            }
            stored = (stored - drain * duration_s).max(0.0);
            steps.push(Planned { step, begins_s: elapsed, duration_s, gross_j, after });
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

    /// Stored energy at the end, before drain, joules.
    pub fn net_j(&self) -> f64 {
        self.steps.iter().map(|p| self.signed(p)).sum()
    }

    fn signed(&self, planned: &Planned) -> f64 {
        match planned.step {
            Step::Build(_) | Step::Grow => planned.gross_j,
            Step::Dismantle(_) | Step::Shrink => -self.recovery * planned.gross_j,
        }
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
                progress.consumed_j += self.signed(planned);
                progress.finished += 1;
                continue;
            }
            if since > planned.begins_s {
                let fraction = (since - planned.begins_s) / planned.duration_s;
                let moved = planned.gross_j * fraction;
                progress.consumed_j += self.signed(planned) * fraction;
                progress.current = Some((planned.step, fraction));
                match planned.step {
                    Step::Build(_) | Step::Grow => {
                        progress.in_hand_kg = moved / C2;
                        progress.reversal_j = self.recovery * moved;
                    }
                    Step::Dismantle(_) | Step::Shrink => {
                        progress.in_hand_kg = -moved / C2;
                        progress.reversal_j = -self.recovery * moved;
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

    #[test]
    fn nothing_to_do_is_done_at_once() {
        let refit = plan(Loadout::STARTING, Loadout::STARTING, 30.0).unwrap();
        assert_eq!(refit.steps().count(), 0);
        assert!(refit.is_done(100.0));
    }

    #[test]
    fn short_of_energy_it_takes_apart_before_it_builds() {
        let from = Loadout::STARTING;
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
        let from = Loadout::STARTING;
        let target = Loadout { living: 1, engines: 6, ..from };
        let steps: Vec<_> = plan(from, target, 30.0).unwrap().steps().collect();
        assert_eq!(steps, [Step::Build(Module::Engine), Step::Dismantle(Module::Living)]);
    }

    #[test]
    fn short_of_slots_it_takes_apart_before_it_builds() {
        let from = Loadout { storage: 6, drones: 2, living: 2, engines: 10, slots: 20 };
        let target = Loadout { living: 0, engines: 12, ..from };
        let steps: Vec<_> = plan(from, target, 20.0).unwrap().steps().collect();
        assert_eq!(steps[0], Step::Dismantle(Module::Living));
    }

    #[test]
    fn drones_come_first_and_go_last() {
        let from = Loadout::STARTING;
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
    fn no_refund_overflows_storage() {
        // Full, and asked to take apart engines: the refund has nowhere to go.
        let from = Loadout::STARTING;
        let refused = plan(from, Loadout { engines: 3, ..from }, 30.0);
        assert_eq!(refused.unwrap_err(), Shortage::Capacity);
        // With a little room it goes through.
        assert!(plan(from, Loadout { engines: 3, ..from }, 27.0).is_ok());
        // Taking apart storage while full of energy cannot be done either.
        assert_eq!(plan(from, Loadout { storage: 5, ..from }, 30.0).unwrap_err(), Shortage::Capacity);
    }

    #[test]
    fn an_impossible_target_says_what_it_is_short_of() {
        let from = Loadout::STARTING;
        assert_eq!(
            plan(from, Loadout { engines: 20, ..from }, 30.0).unwrap_err(),
            Shortage::Unbuildable
        );
        assert_eq!(plan(from, Loadout { drones: 0, ..from }, 30.0).unwrap_err(), Shortage::Unbuildable);
        let bare = Loadout { storage: 0, drones: 1, living: 0, engines: 0, slots: 20 };
        assert_eq!(plan(bare, Loadout { engines: 1, ..bare }, 0.0).unwrap_err(), Shortage::Energy);
    }

    #[test]
    fn the_hull_grows_before_it_is_filled_and_shrinks_once_emptied() {
        let from = Loadout::STARTING;
        let bigger = Loadout { engines: 12, slots: 22, ..from };
        let steps: Vec<_> = plan(from, bigger, 30.0).unwrap().steps().collect();
        assert_eq!(&steps[..2], [Step::Grow, Step::Grow]);
        let smaller = Loadout { slots: 15, ..from };
        let steps: Vec<_> = plan(from, smaller, 20.0).unwrap().steps().collect();
        assert_eq!(steps, [Step::Shrink; 5]);
    }

    #[test]
    fn the_timing_adds_up() {
        let from = Loadout::STARTING;
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
        let from = Loadout::STARTING;
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
