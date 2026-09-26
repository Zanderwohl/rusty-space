//! The refit panel and the development actions panel.
//!
//! The refit panel is the ledger: what the ship holds and what it is doing. A refit is begun from
//! the editor, whose Apply C5 builds. See `lightcone/docs/29-ship-form.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_world::fitting::Balance;
use lc_world::refit::rounds::{Change, Step};

use crate::action::Action;
use crate::input::Requested;
use crate::panels::ask;
use crate::session::Session;
use crate::ui::UiState;

/// What a shortage reads as. Something to act on, per `lightcone/docs/18-ui-style.md`.
pub fn shortfall(short: lc_proto::Shortfall) -> String {
    use lc_proto::Shortfall;
    match short {
        Shortfall::Energy => "not enough energy, even taking apart what is not wanted".into(),
        Shortfall::NoDrones(id) => {
            format!("no drones left to work on {}: keep some until the build", lc_world::form::PartId::from(id))
        }
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
        F::OtherMind(id) => format!("{} is not this ship's Mind, which it keeps", part(id)),
    }
}

/// Joules as module-energies, the unit everything here is argued in.
pub fn me(joules: f64, module_j: f64) -> String {
    format!("{:.2} ME", joules / module_j)
}

fn step_name(step: &Step) -> String {
    let doing = match step.change {
        Change::Grow => "growing",
        Change::Shrink => "shrinking",
        Change::Add => "building",
        Change::Remove => "taking apart",
        Change::Move => "moving",
    };
    format!("{doing} {} ({:?})", step.part, step.kind)
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

pub fn refit(ui: &mut egui::Ui, _state: &UiState, game: &Session, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    let ship = &game.ship;
    let Some(fitting) = ship.fitting() else {
        ui.label("This ship has no form. It arrives from a server.");
        return;
    };
    let module_j = fitting.balance().module_energy_j();
    let stored = fitting.stored_j_at(&ship.motion, now);
    let capacities = fitting.capacities_at(now);
    let capacity = capacities.storage_j;
    ui.label(format!("stored {} of {}", me(stored, module_j), me(capacity, module_j)));
    if capacity > 0.0 {
        ui.add(egui::ProgressBar::new((stored / capacity) as f32));
    }
    egui::Grid::new("refit-stats").num_columns(2).show(ui, |ui| {
        let mut row = |key: &str, value: String| {
            ui.label(key);
            ui.label(value);
            ui.end_row();
        };
        row("mass", format!("{:.3e} kg", ship.mass_kg_at(now)));
        row("accel", format!("{:.1} g", fitting.rated_g_at(&ship.motion, now)));
        row("length", length(ship.length_m));
        row("turn", format!("{:.0} s a flip", lc_world::attitude::flip_time_s(ship.slew_rate_rad_s())));
        row("energy / km/s", me_small(energy_per_km_s_j(fitting.balance(), ship.mass_kg_at(now)), module_j));
        row("building", me_per_year(capacities.building_w, module_j));
        row("data", format!("{:.2} MB", capacities.data_b / 1.0e6));
        row("solar", me_per_year(fitting.solar_w(), module_j));
        row("net", me_per_year(fitting.solar_w() - capacities.drain_w, module_j));
    });
    ui.separator();

    if let Some(running) = fitting.refit().filter(|_| ship.is_refitting(now)) {
        let progress = running.at(now);
        let total = running.steps().len();
        ui.label(format!("refit: step {} of {total}", (progress.finished + 1).min(total)));
        if let Some((step, fraction)) = progress.current {
            ui.add(egui::ProgressBar::new(fraction as f32).text(step_name(&running.steps()[step])));
        }
        let left = running.round().start_s + running.duration_s() - now;
        ui.weak(format!("{} to go", span(left.max(0.0))));
        if ui.button("Cancel refit").on_hover_text("the step under way is undone").clicked() {
            ask(out, Action::CancelRefit);
        }
        return;
    }
    ui.weak("A ship is its parts now. Refits return with the form editor.");
}

pub fn dev_actions(ui: &mut egui::Ui, game: &Session, out: &mut MessageWriter<Requested>) {
    ui.label("Development only. A shard takes these only from an admin.");
    ui.separator();
    let Some(fitting) = game.ship.fitting() else {
        ui.label("This ship has no storage, so nothing to fill.");
        return;
    };
    let module_j = fitting.balance().module_energy_j();
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

    #[test]
    fn a_refused_form_names_the_part() {
        use lc_proto::form::{Number, PartId};
        let malformed = lc_proto::FormFault::Malformed { part: PartId(3), number: Number::Tilt };
        assert_eq!(form_fault(malformed), "part 3 has a malformed tilt");
        assert_eq!(form_fault(lc_proto::FormFault::EngineBlocked(PartId(2))), "something is in part 2's exhaust cone");
    }

    #[test]
    fn a_shortfall_says_what_is_short() {
        let short = lc_proto::Shortfall::NoDrones(lc_proto::form::PartId(3));
        assert_eq!(shortfall(short), "no drones left to work on part 3: keep some until the build");
    }

    /// The rocket law's low-speed form, `m Δv c / ε`.
    #[test]
    fn a_kilometer_a_second_costs_its_momentum_times_c() {
        let b = Balance::DEFAULT;
        let linear = 7.3e9 * 1.0e3 * lc_world::flight::C_M_S / b.drive_efficiency;
        assert!((energy_per_km_s_j(&b, 7.3e9) / linear - 1.0).abs() < 1.0e-5);
    }

    #[test]
    fn a_rate_reads_signed_in_module_energies_a_year() {
        let me = Balance::DEFAULT.module_energy_j();
        let per_year = me / lc_world::flight::JULIAN_YEAR_S;
        assert_eq!(me_per_year(4.2 * per_year, me), "+4.20 ME/yr");
        assert_eq!(me_per_year(-0.009 * per_year, me), "-0.009 ME/yr");
    }

    #[test]
    fn a_length_reads_in_meters_until_it_is_kilometers() {
        assert_eq!(length(500.0), "500 m");
        assert_eq!(length(570.6), "571 m");
        assert_eq!(length(50_000.0), "50.00 km");
    }
}
