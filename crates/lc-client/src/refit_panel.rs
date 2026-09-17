//! The refit panel and the development actions panel.
//!
//! What a refit would do is [`preview`], a pure function over the ship, so it is tested with no
//! window. The panels draw it and ask; neither decides anything. See
//! `lightcone/docs/19-ship-fitting.md`.

use bevy::prelude::*;
use bevy_egui::egui;
use std::ops::RangeInclusive;

use lc_world::craft::Craft;
use lc_world::fitting::{Balance, Loadout, Module};
use lc_world::refit::{Shortage, Step};

use crate::action::Action;
use crate::input::Requested;
use crate::panels::ask;
use crate::session::Session;
use crate::ui::UiState;

/// What applying a draft would do, as of now.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub target: Loadout,
    pub stored_j: f64,
    /// Stored energy, plus what dismantling returns, less what building costs. Before drain.
    pub available_j: f64,
    pub capacity_after_j: f64,
    /// Dry mass of the draft plus the energy it would end with, before drain.
    pub mass_after_kg: f64,
    /// Steps and how long they take, or why it cannot be done.
    pub planned: Result<(usize, f64), Shortage>,
    /// Why Apply cannot be pressed, when it cannot.
    pub blocked: Option<&'static str>,
}

/// What `draft` would take this ship through. `None` for a ship with no fitting.
pub fn preview(ship: &Craft, draft: Loadout, remote: bool, now_s: f64) -> Option<Preview> {
    let fitting = ship.fitting()?;
    let balance = fitting.balance;
    let current = fitting.loadout_at(now_s);
    let stored_j = fitting.stored_j_at(&ship.motion, now_s);
    let available_j = budget_j(&balance, current, draft, stored_j);

    let planned = lc_world::refit::Order { from: current, target: draft, stored_j, start_s: now_s }
        .solve(&balance)
        .map(|refit| (refit.steps().count(), refit.duration_s()));
    let blocked = if !remote {
        Some("no server: refits are the shard's to run")
    } else if ship.is_refitting(now_s) {
        Some("a refit is already running")
    } else if ship.motion.is_under_way() {
        Some("under way: cut the drive before refitting")
    } else if draft == current {
        Some("nothing to change")
    } else if let Err(short) = planned {
        Some(shortfall(short))
    } else {
        None
    };
    Some(Preview {
        target: draft,
        stored_j,
        available_j,
        capacity_after_j: balance.capacity_j(&draft),
        mass_after_kg: balance.dry_mass_kg(&draft) + available_j.max(0.0) / lc_world::fitting::C2,
        planned,
        blocked,
    })
}

/// Stored energy, plus what dismantling returns, less what building costs, if `draft` were built
/// from `current`. Negative when it cannot be paid for.
fn budget_j(balance: &Balance, current: Loadout, draft: Loadout, stored_j: f64) -> f64 {
    let module_j = balance.module_energy_j();
    let mut budget = stored_j;
    for module in Module::ALL {
        budget += moved(current.count(module), draft.count(module), module_j, balance.recovery);
    }
    budget + moved(current.slots, draft.slots, balance.slot_energy_j(), balance.recovery)
}

/// Whether a loadout could be ended at: every module has a slot, the builds are paid for, and
/// what is left fits in the storage it ends with.
fn ends_well(balance: &Balance, current: Loadout, draft: Loadout, stored_j: f64) -> bool {
    let budget = budget_j(balance, current, draft, stored_j);
    draft.modules() <= draft.slots && budget >= 0.0 && budget <= balance.capacity_j(&draft)
}

/// A slider on the refit panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    Module(Module),
    Slots,
}

impl Knob {
    fn get(self, loadout: &Loadout) -> u32 {
        match self {
            Knob::Module(module) => loadout.count(module),
            Knob::Slots => loadout.slots,
        }
    }

    fn with(self, loadout: Loadout, value: u32) -> Loadout {
        let mut out = loadout;
        match self {
            Knob::Module(module) => *out.count_mut(module) = value,
            Knob::Slots => out.slots = value,
        }
        out
    }
}

/// How far a slider may move from where the draft has it without leaving a loadout that could
/// not be ended at, within `limits`.
///
/// Walked out one step at a time from the draft's own value, so the range is always the
/// contiguous run the player can actually reach. A draft that is already out of bounds — the
/// ship's own loadout on a ship holding more than it could — is given the whole of `limits`, or
/// no slider could be moved at all.
pub fn reach(
    balance: &Balance,
    current: Loadout,
    draft: Loadout,
    stored_j: f64,
    knob: Knob,
    limits: RangeInclusive<u32>,
) -> RangeInclusive<u32> {
    let ok = |value: u32| ends_well(balance, current, knob.with(draft, value), stored_j);
    let at = knob.get(&draft).clamp(*limits.start(), *limits.end());
    if !ok(at) {
        return limits;
    }
    let (mut lo, mut hi) = (at, at);
    while lo > *limits.start() && ok(lo - 1) {
        lo -= 1;
    }
    while hi < *limits.end() && ok(hi + 1) {
        hi += 1;
    }
    lo..=hi
}

/// Energy a change of count returns, positive, or costs, negative.
fn moved(from: u32, to: u32, each_j: f64, recovery: f64) -> f64 {
    if to < from {
        (from - to) as f64 * each_j * recovery
    } else {
        -((to - from) as f64) * each_j
    }
}

/// What a shortage reads as. Something to act on, per `lightcone/docs/18-ui-style.md`.
pub fn shortfall(short: Shortage) -> &'static str {
    match short {
        Shortage::Unbuildable => "more modules than slots, or no drone left to build with",
        Shortage::Energy => "not enough energy, even taking apart what is not wanted",
        Shortage::Capacity => "a dismantling would return more than storage can hold",
        Shortage::NoDrones => "no drones to do the work",
    }
}

/// Joules as module-energies, the unit everything here is argued in.
pub fn me(joules: f64, module_j: f64) -> String {
    format!("{:.2} ME", joules / module_j)
}

fn step_name(step: Step) -> String {
    match step {
        Step::Build(module) => format!("building {}", module.name()),
        Step::Dismantle(module) => format!("taking apart {}", module.name()),
        Step::Grow => "growing the hull".into(),
        Step::Shrink => "shrinking the hull".into(),
    }
}

/// A hull length: metres, or kilometres once there are thousands of them.
pub fn length(metres: f64) -> String {
    if metres < 1.0e4 { format!("{metres:.0} m") } else { format!("{:.2} km", metres / 1.0e3) }
}

/// A duration a refit is measured in: days, or years past a few hundred of them.
fn span(seconds: f64) -> String {
    let days = seconds / 86_400.0;
    if days < 400.0 { format!("{days:.1} days") } else { format!("{:.1} years", days / 365.25) }
}

pub fn refit(ui: &mut egui::Ui, state: &UiState, game: &Session, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    let ship = &game.ship;
    let Some(fitting) = ship.fitting() else {
        ui.label("This ship has no modules. They arrive from a server.");
        return;
    };
    let module_j = fitting.balance.module_energy_j();
    let stored = fitting.stored_j_at(&ship.motion, now);
    let capacity = fitting.capacity_j_at(now);
    ui.label(format!("stored {} of {}", me(stored, module_j), me(capacity, module_j)));
    if capacity > 0.0 {
        ui.add(egui::ProgressBar::new((stored / capacity) as f32));
    }
    egui::Grid::new("refit-stats").num_columns(2).show(ui, |ui| {
        ui.label("mass");
        ui.label(format!("{:.3e} kg", ship.mass_kg_at(now)));
        ui.end_row();
        ui.label("accel");
        ui.label(format!("{:.1} g", fitting.rated_g_at(&ship.motion, now)));
        ui.end_row();
        ui.label("length");
        ui.label(length(ship.length_m));
        ui.end_row();
    });
    ui.separator();

    // While the drones work there is nothing to draft: the panel is the refit's progress.
    if let Some(running) = fitting.refit().filter(|_| ship.is_refitting(now)) {
        let progress = running.at(now);
        let total = running.steps().count();
        ui.label(format!("refit: step {} of {total}", (progress.finished + 1).min(total)));
        if let Some((step, fraction)) = progress.current {
            ui.add(egui::ProgressBar::new(fraction as f32).text(step_name(step)));
        }
        let left = running.order().start_s + running.duration_s() - now;
        ui.weak(format!("{} to go", span(left.max(0.0))));
        if ui.button("Cancel refit").on_hover_text("the step under way is reversed").clicked() {
            ask(out, Action::CancelRefit);
        }
        return;
    }

    let current = fitting.loadout_at(now);
    let draft = state.refit_draft.unwrap_or(current);
    let balance = fitting.balance;
    // Each slider ends where the budget does, and a drag below the lowest loadout the ship could
    // end at stops there. The left end stays put, so a handle does not jump as the range moves.
    ui.strong("Plan");
    let mut changed = draft;
    let knob = |ui: &mut egui::Ui, knob: Knob, floor: u32, most: u32, value: &mut u32, name: &str| {
        let range = reach(&balance, current, draft, stored, knob, floor..=most);
        ui.add(egui::Slider::new(value, floor..=*range.end()).text(name));
        *value = (*value).clamp(*range.start(), *range.end());
    };
    for module in Module::ALL {
        let floor = if module == Module::Drone { 1 } else { 0 };
        let most = draft.slots.max(floor);
        knob(ui, Knob::Module(module), floor, most, changed.count_mut(module), module.name());
    }
    let most = (current.slots * 2).max(40);
    knob(ui, Knob::Slots, 1, most, &mut changed.slots, "hull slots");
    if changed != draft {
        ask(out, Action::DraftRefit(changed));
    }

    let Some(view) = preview(ship, draft, game.remote, now) else { return };
    ui.separator();
    ui.strong("After");
    egui::Grid::new("refit-after").num_columns(2).show(ui, |ui| {
        let mut row = |key: &str, value: String| {
            ui.label(key);
            ui.label(value);
            ui.end_row();
        };
        row("free slots", draft.free_slots().to_string());
        row("length", length(balance.length_m(draft.slots)));
        row(
            "energy",
            format!("{} of {}", me(view.available_j, module_j), me(view.capacity_after_j, module_j)),
        );
        row("mass", format!("{:.3e} kg", view.mass_after_kg));
        let (steps, days) = match view.planned {
            Ok((steps, duration_s)) => (steps.to_string(), format!("{:.1}", duration_s / 86_400.0)),
            Err(_) => ("—".into(), "—".into()),
        };
        row("steps", steps);
        row("days", days);
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(view.blocked.is_none(), egui::Button::new("Apply")).clicked() {
            ask(out, Action::ApplyRefit);
        }
        if ui.button("Reset").clicked() {
            ask(out, Action::ResetRefitDraft);
        }
    });
    if let Some(why) = view.blocked {
        ui.weak(why);
    }
}

pub fn dev_actions(ui: &mut egui::Ui, game: &Session, out: &mut MessageWriter<Requested>) {
    ui.label("Development only. A shard takes these only from an admin.");
    ui.separator();
    let Some(fitting) = game.ship.fitting() else {
        ui.label("This ship has no modules, so nothing to fill.");
        return;
    };
    let module_j = fitting.balance.module_energy_j();
    ui.horizontal(|ui| {
        for count in [1.0, 10.0] {
            if ui.button(format!("+{count:.0} ME")).clicked() {
                ask(out, Action::GrantEnergy(count * module_j));
            }
        }
        if ui.button("Fill storage").clicked() {
            ask(out, Action::FillStorage);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use lc_world::craft::{CraftId, Kind};
    use lc_world::fitting::{Balance, Fitting};

    fn ship() -> Craft {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.fit(Some(Fitting::full(Loadout::STARTING, Balance::DEFAULT, 0.0)));
        craft
    }

    #[test]
    fn available_energy_counts_what_dismantling_returns() {
        let b = Balance::DEFAULT;
        // Two thirds full, so the refunds have somewhere to go.
        let mut craft = ship();
        let account = lc_world::fitting::Account {
            stored_j: 20.0 * b.module_energy_j(),
            ..craft.fitting().unwrap().account()
        };
        craft.fit(Some(Fitting::from_account(&account, b)));
        let draft = Loadout { living: 0, engines: 6, ..Loadout::STARTING };
        let view = preview(&craft, draft, true, 0.0).unwrap();
        let expected = view.stored_j + 2.0 * 0.95 * b.module_energy_j() - b.module_energy_j();
        assert!((view.available_j / expected - 1.0).abs() < 1.0e-12);
        assert!(matches!(view.planned, Ok((3, _))), "{:?}", view.planned);
        // Two taken apart and one built: the ship is lighter by the 5% of two modules radiated.
        let mass_before = b.dry_mass_kg(&Loadout::STARTING) + view.stored_j / lc_world::fitting::C2;
        let lost = mass_before - view.mass_after_kg;
        assert!((lost / (2.0 * 0.05 * b.module_mass_kg()) - 1.0).abs() < 1.0e-9, "{lost}");
        assert_eq!(view.blocked, None);
    }

    #[test]
    fn apply_says_why_it_cannot_be_pressed() {
        let full = Loadout { storage: 5, ..Loadout::STARTING };
        let view = preview(&ship(), full, true, 0.0).unwrap();
        assert_eq!(view.blocked, Some(shortfall(Shortage::Capacity)));
        let same = preview(&ship(), Loadout::STARTING, true, 0.0).unwrap();
        assert_eq!(same.blocked, Some("nothing to change"));
        let offline = preview(&ship(), Loadout { engines: 6, ..Loadout::STARTING }, false, 0.0);
        assert!(offline.unwrap().blocked.unwrap().starts_with("no server"));
    }

    /// **The sliders cannot reach an unaffordable loadout.** Checked against the budget worked
    /// out by hand, not against `ends_well`, which is what is being tested.
    #[test]
    fn a_slider_stops_where_the_energy_runs_out() {
        let b = Balance::DEFAULT;
        let me = b.module_energy_j();
        let start = Loadout::STARTING;
        let engines = Knob::Module(Module::Engine);

        // Three module-energies stored and five slots free: three more engines, not five.
        let range = reach(&b, start, start, 3.0 * me, engines, 0..=20);
        assert_eq!(*range.end(), 8);
        // And every engine can come out: storage has room for all five refunds.
        assert_eq!(*range.start(), 0);

        // With plenty stored it is the free slots that stop it.
        assert_eq!(*reach(&b, start, start, 25.0 * me, engines, 0..=20).end(), 10);

        // Full, so living space cannot be taken apart — its refund has nowhere to go — but the
        // five free slots can all be filled.
        let living = Knob::Module(Module::Living);
        assert_eq!(reach(&b, start, start, 30.0 * me, living, 0..=20), 2..=7);

        // And the hull cannot shrink below its modules, or grow past what it can pay for.
        let slots = reach(&b, start, start, 0.5 * me, Knob::Slots, 1..=40);
        assert_eq!(*slots.start(), 15);
        let growth = (0.5 / (b.slot_energy_j() / me)).floor() as u32;
        assert_eq!(*slots.end(), 20 + growth);

        // Nothing inside the range is a loadout the budget cannot pay for.
        for n in range_of(reach(&b, start, start, 3.0 * me, engines, 0..=20)) {
            let draft = Loadout { engines: n, ..start };
            assert!(budget_j(&b, start, draft, 3.0 * me) >= 0.0, "{n} engines");
        }
    }

    #[test]
    fn a_length_reads_in_metres_until_it_is_kilometres() {
        assert_eq!(length(500.0), "500 m");
        assert_eq!(length(Balance::DEFAULT.length_m(22)), "516 m");
        assert_eq!(length(50_000.0), "50.00 km");
    }

    fn range_of(range: RangeInclusive<u32>) -> Vec<u32> {
        range.collect()
    }

    #[test]
    fn a_ship_with_no_modules_has_nothing_to_preview() {
        let bare = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        assert!(preview(&bare, Loadout::STARTING, true, 0.0).is_none());
    }
}
