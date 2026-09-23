//! The System window: what a craft believes is here, and where it can be sent.
//!
//! Split out of [`crate::panels`] when the body list stopped reading the generator: a list of
//! beliefs needs a detail section with provenance behind it, which is a panel's worth of its
//! own. See `lightcone/docs/25-system-knowledge.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::{SystemTab, ask, range_to, ships, span, span_m, station};
use lc_world::navigation::Target;

/// The system window: what is here, and where the ship can be sent.
///
/// Two sections. The inventory runs outward from the star with each body's satellites behind
/// it; picking one opens its courses. Nothing is flown until Go, so a player can read the
/// options without committing to one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn system(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    tab: &mut SystemTab,
    show_all: &mut bool,
    revealed: &mut Option<Target>,
    out: &mut MessageWriter<Requested>,
) {
    let Some(system) = game.system.as_ref() else {
        ui.label("Between systems. There is nothing local to go to.");
        return;
    };
    ui.horizontal(|ui| {
        ui.selectable_value(tab, SystemTab::Bodies, format!("{} bodies", system.len()));
        // Counted in the tab, because whether anyone is here at all is the first thing worth
        // knowing and opening the other list to find out would be one click too many.
        ui.selectable_value(tab, SystemTab::Ships, match uplink.contacts.len() {
            0 => "no ships".to_string(),
            1 => "1 ship".to_string(),
            n => format!("{n} ships"),
        });
    });
    station(ui, state, game, out);
    ui.separator();
    if *tab == SystemTab::Ships {
        ships(ui, game, uplink, out);
        return;
    }
    let labels = game.home_labels();
    let called = |target: &lc_world::navigation::Target, fallback: &str| match target {
        lc_world::navigation::Target::Body(key) => labels.of(key),
        // A band's designation is made from its shape, not from anybody's name.
        lc_world::navigation::Target::Band(_) => fallback.to_string(),
    };
    ui.horizontal(|ui| {
        ui.label(game.name_of(system.star));
        ui.checkbox(show_all, "all");
    });

    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for entry in system.inventory().iter().filter(|e| *show_all || e.major) {
            let picked = state.focus.as_ref() == Some(&entry.target);
            ui.horizontal(|ui| {
                ui.add_space(entry.depth as f32 * 12.0);
                let row = ui.selectable_label(picked, called(&entry.target, &entry.designation));
                if row.clicked() {
                    let next = (!picked).then(|| entry.target.clone());
                    ask(out, Action::FocusTarget(next));
                }
                // Once, when the focus changes. Every frame would fight the player's own
                // scrolling, and never would hide a body picked from anywhere but this list.
                if picked && revealed.as_ref() != Some(&entry.target) {
                    *revealed = Some(entry.target.clone());
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                ui.weak(span_m(entry.orbit_radius_m));
            });
        }
    });
    ui.separator();

    let Some(target) = state.focus.as_ref() else {
        ui.label("Pick something to go to.");
        return;
    };
    let Some(entry) = system.inventory().iter().find(|e| &e.target == target) else { return };
    ui.heading(called(&entry.target, &entry.designation));
    ui.weak(match entry.orbit_radius_m {
        r if r > 0.0 => format!(
            "{} — {} out, {} away",
            entry.kind.label(),
            span_m(r),
            span(range_to(game, system, target)),
        ),
        _ => format!("{} — {} away", entry.kind.label(), span(range_to(game, system, target))),
    });

    for (label, course) in crate::navigation::options_for(system, target) {
        let armed = state.course.as_ref() == Some(&course);
        if ui.selectable_label(armed, &label).clicked() {
            ask(out, Action::ChooseCourse((!armed).then_some(course)));
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        let ready = state.course.is_some();
        if ui.add_enabled(ready, egui::Button::new("Go")).clicked() {
            if let Some(course) = state.course.clone() {
                ask(out, Action::SetCourse(course));
            }
        }
        ui.weak(format!("brachistochrone at {:.0} g", game.ship.motion.drive.accel_g));
    });
}
