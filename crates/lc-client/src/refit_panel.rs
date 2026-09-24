//! The refit panel and the development actions panel.
//!
//! What a refit would do is [`preview`], a pure function over the ship, so it is tested with no
//! window. The panels draw it and ask; neither decides anything. See
//! `lightcone/docs/19-ship-fitting.md`.

use bevy::prelude::*;
use bevy_egui::egui;

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
    /// What the plan throws away for want of room in storage, joules.
    pub vented_j: f64,
    pub capacity_after_j: f64,
    /// Dry mass of the draft plus the energy it would end with, before drain.
    pub mass_after_kg: f64,
    /// What the draft's engines pull with storage empty, and with it full.
    pub g_dry: f64,
    pub g_wet: f64,
    /// What a change of velocity of one kilometer a second costs the draft at `mass_after_kg`.
    pub energy_per_km_s_j: f64,
    /// What the draft's hull would collect here, holding still, and what its living space drains.
    pub solar_after_w: f64,
    pub drain_after_w: f64,
    /// Steps and how long they take, or why it cannot be done.
    pub planned: Result<(usize, f64), Shortage>,
    /// Why Apply cannot be pressed, when it cannot.
    pub blocked: Option<String>,
    /// A cost the player may not want. Does not block Apply.
    pub warning: Option<String>,
}

/// What `draft` would take this ship through. `None` for a ship with no fitting.
pub fn preview(ship: &Craft, draft: Loadout, remote: bool, now_s: f64) -> Option<Preview> {
    let fitting = ship.fitting()?;
    let balance = fitting.balance;
    let current = fitting.loadout_at(now_s);
    let stored_j = fitting.stored_j_at(&ship.motion, now_s);
    let refit = lc_world::refit::Order { from: current, target: draft, stored_j, start_s: now_s }.solve(&balance);
    // The plan knows what storage cannot keep; the arithmetic only says how far short a failed
    // plan is.
    let (available_j, vented_j) = match &refit {
        Ok(refit) => (stored_j - refit.net_j(), refit.vented_j()),
        Err(_) => (budget_j(&balance, current, draft, stored_j), 0.0),
    };
    let dry_kg = balance.dry_mass_kg(&draft);
    let mass_after_kg = dry_kg + available_j.max(0.0) / lc_world::fitting::C2;
    let planned = refit.map(|refit| (refit.steps().count(), refit.duration_s()));
    let blocked = if !remote {
        Some("no server: refits are the shard's to run".into())
    } else if ship.is_refitting(now_s) {
        Some("a refit is already running".into())
    } else if ship.motion.is_under_way() {
        Some("under way: cut the drive before refitting".into())
    } else if draft == current {
        Some("nothing to change".into())
    } else if let Err(short) = planned {
        Some(shortfall(short))
    } else {
        None
    };
    // A rounding error's worth of venting is not worth a line.
    let warning = (vented_j > 1.0e-6 * balance.module_energy_j()).then(|| {
        format!(
            "{} has no room in storage and would be thrown away; empty energy storage would keep it",
            me(vented_j, balance.module_energy_j())
        )
    });
    Some(Preview {
        target: draft,
        stored_j,
        available_j,
        vented_j,
        capacity_after_j: balance.capacity_j(&draft),
        mass_after_kg,
        g_dry: balance.accel_g(&draft, dry_kg),
        g_wet: balance.accel_g(&draft, dry_kg + balance.capacity_j(&draft) / lc_world::fitting::C2),
        energy_per_km_s_j: energy_per_km_s_j(&balance, mass_after_kg),
        solar_after_w: ship.solar_w_for(balance.length_m(draft.slots), now_s),
        drain_after_w: balance.drain_w(&draft),
        planned,
        blocked,
        warning,
    })
}

/// Stored energy, plus what dismantling returns, less what building costs, if `draft` were built
/// from `current`. Negative when it cannot be paid for; blind to what storage cannot keep.
fn budget_j(balance: &Balance, current: Loadout, draft: Loadout, stored_j: f64) -> f64 {
    let mut budget = stored_j;
    for module in Module::ALL {
        let each_j = balance.build_energy_j(module);
        budget += moved(current.count(module), draft.count(module), each_j, balance.recovery);
    }
    budget + moved(current.slots, draft.slots, balance.slot_energy_j(), balance.recovery)
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
pub fn shortfall(short: Shortage) -> String {
    match short {
        Shortage::Unbuildable => "more modules than slots, or no drone left to build with".into(),
        Shortage::Energy => "not enough energy, even taking apart what is not wanted".into(),
        Shortage::NoDrones => "no drones to do the work".into(),
        Shortage::CannotBuild(module) => format!("cannot build {}", module.name()),
        Shortage::CannotDismantle(module) => format!("cannot take apart {}", module.name()),
    }
}

/// What a refused form reads as, naming the part as 29 says a refusal does. The structural
/// cases word themselves as `lc_world::form::FormError` does.
pub fn form_fault(fault: lc_proto::FormFault) -> String {
    use lc_proto::FormFault as F;
    use lc_world::form::{Number, PartId};
    let part = |id: lc_proto::form::PartId| PartId::from(id);
    match fault {
        F::TooManyParts { found } => format!("{found} parts, at most {}", lc_world::form::MAX_PARTS),
        F::DuplicateId(id) => format!("{} appears twice", part(id)),
        F::NoMind => "no Mind".into(),
        F::SecondMind { first, second } => format!("{} is a second Mind beside {}", part(second), part(first)),
        F::Malformed { part: id, number } => format!("{} has a malformed {}", part(id), Number::from(number)),
        F::MindPlaced(id) => format!("the Mind, {}, has a parent", part(id)),
        F::Unplaced(id) => format!("{} has no parent", part(id)),
        F::MissingParent { part: id, parent } => {
            format!("{} hangs from {}, which does not exist", part(id), part(parent))
        }
        F::Cycle(id) => format!("{} is its own ancestor", part(id)),
        F::EngineOffAxis(id) => format!("{} must point fore or aft", part(id)),
        F::EngineBlocked(id) => format!("something is in {}'s exhaust cone", part(id)),
        F::BayBlocked(id) => format!("{}'s mouth is blocked", part(id)),
        F::Detached(id) => format!("{} does not touch its parent", part(id)),
        F::Uncontained(id) => format!("{} does not contain its parent", part(id)),
        F::Extent => "the ship is too long or too short".into(),
        F::TooSmall(id) => format!("{} is smaller than the smallest part", part(id)),
        F::TooFewDrones => "fewer drones than a ship may keep".into(),
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

pub fn energy_per_km_s_j(balance: &Balance, mass_kg: f64) -> f64 {
    let rapidity = lc_world::cost::rapidity_between(
        glam::DVec3::ZERO,
        glam::DVec3::X * 1.0e3 / lc_world::flight::C_M_S,
    );
    lc_world::cost::energy_j(mass_kg, rapidity, balance.drive_efficiency)
}

/// A rate of energy, in module-energies a game year, signed.
pub fn me_per_year(watts: f64, module_j: f64) -> String {
    let rate = watts * lc_world::flight::JULIAN_YEAR_S / module_j;
    if rate.abs() < 0.1 { format!("{rate:+.3} ME/yr") } else { format!("{rate:+.2} ME/yr") }
}

/// An energy small enough to need an exponent, in module-energies.
fn me_small(joules: f64, module_j: f64) -> String {
    format!("{:.3e} ME", joules / module_j)
}

/// A hull length: meters, or kilometers once there are thousands of them.
pub fn length(meters: f64) -> String {
    if meters < 1.0e4 { format!("{meters:.0} m") } else { format!("{:.2} km", meters / 1.0e3) }
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
        ui.label("energy / km/s");
        ui.label(me_small(energy_per_km_s_j(&fitting.balance, ship.mass_kg_at(now)), module_j));
        ui.end_row();
        let drain = fitting.balance.drain_w(&fitting.loadout_at(now));
        ui.label("solar");
        ui.label(me_per_year(fitting.solar_w(), module_j));
        ui.end_row();
        ui.label("net");
        ui.label(me_per_year(fitting.solar_w() - drain, module_j));
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
    // The sliders go anywhere a hull could hold. Whether the ship can get there is the planner's
    // to say, so there is one judge and Apply shows its reason.
    ui.strong("Plan");
    let mut changed = draft;
    for module in Module::ALL {
        let floor = if module == Module::Drone { 1 } else { 0 };
        let most = draft.slots.max(floor);
        ui.add(egui::Slider::new(changed.count_mut(module), floor..=most).text(module.name()));
    }
    let most = (current.slots * 2).max(40);
    ui.add(egui::Slider::new(&mut changed.slots, 1..=most).text("hull slots"));
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
        row("g dry", format!("{:.1} g", view.g_dry));
        row("g wet", format!("{:.1} g", view.g_wet));
        row("energy / km/s", me_small(view.energy_per_km_s_j, module_j));
        row("solar", me_per_year(view.solar_after_w, module_j));
        row("net", me_per_year(view.solar_after_w - view.drain_after_w, module_j));
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
        // Enabled whatever the planner says: skipping the energy is the point. The shard
        // decides who may.
        if game.remote
            && draft != current
            && ui.button("Magic").on_hover_text("build this now, free; admins only").clicked()
        {
            ask(out, Action::OpenPanel(crate::ui::Panel::Console));
            ask(out, Action::RunCommand(magic_line(draft)));
        }
    });
    if let Some(why) = &view.blocked {
        ui.weak(why);
    }
    if let Some(warning) = &view.warning {
        ui.colored_label(ui.visuals().warn_fg_color, warning);
    }
}

fn magic_line(draft: Loadout) -> String {
    let Loadout { storage, drones, living, engines, slots, data } = draft;
    format!("refit-magic storage:{storage} drones:{drones} living:{living} engines:{engines} data:{data} slots:{slots}")
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

    #[test]
    fn a_refused_form_names_the_part() {
        use lc_proto::form::{Number, PartId};
        let malformed = lc_proto::FormFault::Malformed { part: PartId(3), number: Number::Tilt };
        assert_eq!(form_fault(malformed), "part 3 has a malformed tilt");
        assert_eq!(form_fault(lc_proto::FormFault::EngineBlocked(PartId(2))), "something is in part 2's exhaust cone");
    }

    /// The button's line binds back to the draft. Native only: the browser build has no shard.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_magic_button_says_what_the_shard_reads() {
        use lc_server::ability::Level;
        use lc_server::command::{bind, find, parse};
        let draft = Loadout { storage: 3, drones: 1, living: 0, engines: 12, slots: 33, data: 2 };
        let parsed = parse(&magic_line(draft)).expect("it parses");
        let spec = find(&parsed.name, Level::DEBUG).expect("a command");
        let args = bind(spec, &parsed, Level::DEBUG).expect("it binds");
        let got = |n: &str| args.count(n).unwrap();
        assert_eq!(
            Loadout {
                storage: got("storage"),
                drones: got("drones"),
                living: got("living"),
                engines: got("engines"),
                slots: got("slots"),
                data: got("data"),
            },
            draft,
        );
    }

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
        let expected = view.stored_j + 0.95 * b.module_energy_j() - b.module_energy_j();
        assert!((view.available_j / expected - 1.0).abs() < 1.0e-12);
        assert!(matches!(view.planned, Ok((2, _))), "{:?}", view.planned);
        // One taken apart and one built: the ship is lighter by the 5% of a module radiated.
        let mass_before = b.dry_mass_kg(&Loadout::STARTING) + view.stored_j / lc_world::fitting::C2;
        let lost = mass_before - view.mass_after_kg;
        assert!((lost / (0.05 * b.module_mass_kg()) - 1.0).abs() < 1.0e-9, "{lost}");
        assert_eq!(view.blocked, None);
    }

    #[test]
    fn apply_says_why_it_cannot_be_pressed() {
        let unaffordable = Loadout { engines: 40, slots: 60, ..Loadout::STARTING };
        let view = preview(&ship(), unaffordable, true, 0.0).unwrap();
        assert_eq!(view.blocked, Some(shortfall(Shortage::Energy)));
        let same = preview(&ship(), Loadout::STARTING, true, 0.0).unwrap();
        assert_eq!(same.blocked.as_deref(), Some("nothing to change"));
        let offline = preview(&ship(), Loadout { engines: 6, ..Loadout::STARTING }, false, 0.0);
        assert!(offline.unwrap().blocked.unwrap().starts_with("no server"));
    }

    /// A full ship can take a drone apart, and is warned that the refund has nowhere to go.
    #[test]
    fn a_full_ship_is_warned_that_a_refund_would_be_thrown_away() {
        let fewer = Loadout { drones: 1, ..Loadout::STARTING };
        let view = preview(&ship(), fewer, true, 0.0).unwrap();
        assert_eq!(view.blocked, None);
        assert!(view.warning.as_deref().unwrap().starts_with("0.95 ME has no room"), "{:?}", view.warning);
        // Nothing kept, so the energy after is what is stored now.
        assert!((view.available_j / view.stored_j - 1.0).abs() < 1.0e-12);

        let b = Balance::DEFAULT;
        let mut craft = ship();
        let account = lc_world::fitting::Account {
            stored_j: 20.0 * b.module_energy_j(),
            ..craft.fitting().unwrap().account()
        };
        craft.fit(Some(Fitting::from_account(&account, b)));
        assert_eq!(preview(&craft, fewer, true, 0.0).unwrap().warning, None);
    }

    #[test]
    fn a_data_module_is_budgeted_at_half_a_module_energy() {
        let b = Balance::DEFAULT;
        let view = preview(&ship(), Loadout { data: 2, ..Loadout::STARTING }, true, 0.0).unwrap();
        let spent = (view.stored_j - view.available_j) / b.module_energy_j();
        assert!((spent - 0.5).abs() < 1.0e-12, "{spent}");
        assert_eq!(view.blocked, None);
    }

    /// Checked against the rocket law's low-speed form, `m Δv c / ε`, and the engine rating.
    #[test]
    fn the_after_table_rates_the_draft_dry_wet_and_per_km_s() {
        let b = Balance::DEFAULT;
        let view = preview(&ship(), Loadout::STARTING, true, 0.0).unwrap();
        assert!((view.g_wet - 5.0).abs() < 1.0e-9, "{}", view.g_wet);
        assert!((view.g_dry - 13.81).abs() < 0.01, "{}", view.g_dry);
        let linear = view.mass_after_kg * 1.0e3 * lc_world::flight::C_M_S / b.drive_efficiency;
        assert!((view.energy_per_km_s_j / linear - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn a_rate_reads_signed_in_module_energies_a_year() {
        let me = Balance::DEFAULT.module_energy_j();
        let per_year = me / lc_world::flight::JULIAN_YEAR_S;
        assert_eq!(me_per_year(4.2 * per_year, me), "+4.20 ME/yr");
        assert_eq!(me_per_year(-0.009 * per_year, me), "-0.009 ME/yr");
    }

    /// A ship between systems collects nothing, so After's solar is zero and its net is the drain.
    #[test]
    fn a_draft_between_systems_collects_nothing() {
        let b = Balance::DEFAULT;
        let view = preview(&ship(), Loadout { living: 3, ..Loadout::STARTING }, true, 0.0).unwrap();
        assert_eq!(view.solar_after_w, 0.0);
        assert_eq!(view.drain_after_w, 3.0 * b.living_drain_w);
    }

    #[test]
    fn a_length_reads_in_meters_until_it_is_kilometers() {
        assert_eq!(length(500.0), "500 m");
        assert_eq!(length(Balance::DEFAULT.length_m(22)), "516 m");
        assert_eq!(length(50_000.0), "50.00 km");
    }

    #[test]
    fn a_ship_with_no_modules_has_nothing_to_preview() {
        let bare = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        assert!(preview(&bare, Loadout::STARTING, true, 0.0).is_none());
    }
}
