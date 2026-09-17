//! The refit panel and the development actions panel.
//!
//! What a refit would do is [`preview`], a pure function over the ship, so it is tested with no
//! window. The panels draw it and ask; neither decides anything. See
//! `lightcone/docs/19-ship-fitting.md`.

use bevy::prelude::*;
use bevy_egui::egui;
use lc_world::craft::Craft;
use lc_world::fitting::{Loadout, Module};
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
    let module_j = balance.module_energy_j();
    let mut available_j = stored_j;
    for module in Module::ALL {
        available_j += moved(current.count(module), draft.count(module), module_j, balance.recovery);
    }
    available_j += moved(current.slots, draft.slots, balance.slot_energy_j(), balance.recovery);

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
        planned,
        blocked,
    })
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
    ui.label(format!(
        "mass {:.3e} kg — engines give {:.1} g",
        ship.mass_kg_at(now),
        fitting.rated_g_at(&ship.motion, now)
    ));
    ui.separator();

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
        ui.separator();
    }

    let current = fitting.loadout_at(now);
    let draft = state.refit_draft.unwrap_or(current);
    let mut changed = draft;
    for module in Module::ALL {
        let floor = if module == Module::Drone { 1 } else { 0 };
        let count = changed.count_mut(module);
        ui.add(egui::Slider::new(count, floor..=draft.slots.max(floor)).text(module.name()));
    }
    let most = (current.slots * 2).max(40);
    let fewest = changed.modules().max(1);
    ui.add(egui::Slider::new(&mut changed.slots, fewest..=most).text("hull slots"));
    changed.slots = changed.slots.max(changed.modules());
    if changed != draft {
        ask(out, Action::DraftRefit(changed));
    }

    let Some(view) = preview(ship, draft, game.remote, now) else { return };
    ui.separator();
    ui.label(format!("free slots after: {}", draft.free_slots()));
    ui.label(format!(
        "energy after: {} of {}",
        me(view.available_j, module_j),
        me(view.capacity_after_j, module_j)
    ));
    if let Ok((steps, duration_s)) = view.planned {
        ui.label(format!("{steps} steps, {}", span(duration_s)));
    }
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
    ui.label("Development only. A shard refuses all of these.");
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

    #[test]
    fn a_ship_with_no_modules_has_nothing_to_preview() {
        let bare = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        assert!(preview(&bare, Loadout::STARTING, true, 0.0).is_none());
    }
}
