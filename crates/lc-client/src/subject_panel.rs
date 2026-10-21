//! The card: one subject, whatever was last picked — a star, a body, a belt or a craft — with
//! what it is called and on whose word, where what is known of it came from, and what has been
//! said about it. See `lightcone/docs/13-client-shell.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_proto::ShipId;
use lc_world::knowledge::Subject;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::ask;
use crate::ui::{Channel, Panel};

const WIDTH: f32 = 340.0;

pub(crate) fn card(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    held: &crate::beliefs::Held,
    uplink: &crate::uplink::Uplink,
    draft: &mut crate::thread_view::Draft,
    out: &mut MessageWriter<Requested>,
) {
    ui.set_width(WIDTH);
    let Some(subject) = state.card else {
        ui.weak("Nothing picked.");
        return;
    };
    match subject {
        Subject::Star(id) => {
            ui.heading(game.name_of(id));
            ui.weak("star");
            crate::thread_view::names(ui, game, uplink, subject, true, draft, out);
            if let Some(belief) = game.belief(id) {
                ui.label(format!("Estimated range: {}", crate::range::short(Some(belief), game.ship.motion.position_ly)));
                sources(ui, crate::range::sources(belief, game.knowledge.owner));
            }
        }
        Subject::Body { body, .. } => {
            let belief = held.bodies.iter().find(|b| b.body == body);
            ui.heading(match belief {
                Some(belief) => game.called(belief),
                None => game.knowledge.name_of(subject).unwrap_or_else(|| "a body".into()),
            });
            if let Some(belief) = belief {
                ui.weak(crate::system_panel::type_text(belief, game));
            }
            crate::thread_view::names(ui, game, uplink, subject, true, draft, out);
            if let Some(belief) = belief {
                ui.label(format!("Distance: {}", crate::system_panel::distance_text(belief)));
                sources(ui, crate::system_panel::sources(belief, game.knowledge.owner));
            }
        }
        Subject::Population { index, .. } => {
            ui.heading(game.band_label(index as usize));
            ui.weak("belt");
            crate::thread_view::names(ui, game, uplink, subject, true, draft, out);
        }
        Subject::Craft(id) => craft(ui, game, uplink, ShipId(id), draft, out),
        // A place a stare was aimed at is nothing a player picks.
        Subject::Place(_) => {}
    }
    ui.separator();
    crate::thread_view::notes(ui, game, uplink, subject, true, draft, out);
}

fn craft(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    id: ShipId,
    draft: &mut crate::thread_view::Draft,
    out: &mut MessageWriter<Requested>,
) {
    let label = uplink.labels.get(&id);
    ui.heading(label.map_or_else(|| crate::craft_names::unnamed(id), |l| l.name.clone()));
    ui.horizontal(|ui| {
        ui.weak(format!("craft {}", id.tag()));
        if let Some(original) = label.and_then(|l| l.claimant_of) {
            ui.weak(format!("— claims the name of {}", uplink.name_of(original)));
        }
    });
    crate::thread_view::names(ui, game, uplink, Subject::Craft(id.0), true, draft, out);
    let in_sight = uplink.contacts.iter().any(|c| c.ship_id == id);
    ui.weak(match in_sight {
        true => "in sight",
        false => "out of sight",
    });
    if ui.button("conversation").clicked() {
        ask(out, Action::OpenPanel(Panel::Chat));
        ask(out, Action::ChatWith(Channel::With(id)));
    }
}

/// Where what is known came from.
fn sources(ui: &mut egui::Ui, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    egui::CollapsingHeader::new("Sources").default_open(false).show(ui, |ui| {
        for line in lines {
            ui.weak(line);
        }
    });
}
