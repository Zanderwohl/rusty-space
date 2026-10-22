//! The card: one subject, whatever was last picked — a star, a body, a belt or a craft — with
//! what it is called and on whose word, where what is known of it came from, and what has been
//! said about it. See `lightcone/docs/13-client-shell.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_proto::ShipId;
use lc_world::knowledge::whereabouts::Kind;
use lc_world::knowledge::{Subject, Witness};

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
    if in_sight {
        ui.weak("in sight");
    }
    whereabouts(ui, game, uplink, id);
    if ui.button("conversation").clicked() {
        ask(out, Action::OpenPanel(Panel::Chat));
        ask(out, Action::ChatWith(Channel::With(id)));
    }
}

/// Where the craft was last known to be, how that is known and on whose word, and every stay
/// before it.
fn whereabouts(ui: &mut egui::Ui, game: &Game, uplink: &crate::uplink::Uplink, id: ShipId) {
    let Some(held) = game.knowledge.file(Subject::Craft(id.0)).map(|f| f.whereabouts()) else { return };
    let Some((stay, best)) = held.last_known() else { return };
    let now_s = game.coordinate_time_s();
    let here = game.system.as_ref().map(|s| s.star);
    let place = |system: Option<lc_world::sky::StarId>| match system {
        Some(star) => match game.knowledge.name_of(star) {
            Some(name) => name,
            None if Some(star) == here => "this system".to_string(),
            None => "an unnamed system".to_string(),
        },
        None => "between the stars".to_string(),
    };
    let how = |kind: Kind| match kind {
        Kind::Seen => "seen",
        Kind::Heard => "heard",
        Kind::Stated => "on its own word",
        Kind::Collapse => "collapse",
    };
    let ago = |s: &lc_world::knowledge::whereabouts::Sighting| crate::panels::duration(now_s - s.emitted_s);
    ui.label(format!("Last known: {}, {} ago, {}", place(stay.system), ago(best), how(best.kind)));
    // A bearing places nothing, so say where it was last placed as well.
    if let Some(placed) = held.last_placed().filter(|p| !std::ptr::eq(*p, best)) {
        ui.label(format!("Last placed: {}, {} ago, {}", place(placed.system), ago(placed), how(placed.kind)));
    }
    let owner = game.knowledge.owner;
    let lines: Vec<String> = best
        .witnesses
        .iter()
        .map(|w| {
            let who = |witness: Witness| match witness == owner {
                true => uplink.own_name(),
                false => uplink.name_of(ShipId(witness.0 as i64)),
            };
            let relays: Vec<String> = w.lineage.iter().skip(1).map(|hop| who(hop.from)).collect();
            match relays.is_empty() {
                true => who(w.witness),
                false => format!("{}, relayed by {}", who(w.witness), relays.join(", ")),
            }
        })
        .collect();
    sources(ui, lines);
    if held.stays().len() > 1 {
        egui::CollapsingHeader::new("Where it has been").default_open(false).show(ui, |ui| {
            for stay in held.stays().iter().rev() {
                let (first, last) = stay.span_s();
                ui.weak(format!(
                    "{}: {} to {} ago",
                    place(stay.system),
                    crate::panels::duration(now_s - first),
                    crate::panels::duration(now_s - last)
                ));
            }
        });
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
