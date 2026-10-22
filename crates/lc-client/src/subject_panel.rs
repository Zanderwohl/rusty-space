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
        Subject::Collapse(_) => collapse(ui, game, uplink, subject, draft, out),
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
    // Collapses this ship holds were this craft, on whosever word stands.
    let owner = game.knowledge.owner;
    for (key, file) in game.knowledge.collapses() {
        if file.thread().was(owner).is_some_and(|(craft, _)| craft == Some(id))
            && let Some((_, best)) = file.whereabouts().last_known()
        {
            let line = format!("Collapsed {} ago", crate::panels::duration(game.coordinate_time_s() - best.emitted_s));
            if crate::radio_panel::clickable_name(ui, &line, ui.visuals().text_color()).clicked() {
                ask(out, Action::Card(Subject::Collapse(key)));
            }
        }
    }
    if ui.button("conversation").clicked() {
        ask(out, Action::OpenPanel(Panel::Chat));
        ask(out, Action::ChatWith(Channel::With(id)));
    }
}

/// A collapse seen by its light: where and when, who saw it, and whose it was on whosever word
/// stands, with what this ship says of that.
fn collapse(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    subject: Subject,
    draft: &mut crate::thread_view::Draft,
    out: &mut MessageWriter<Requested>,
) {
    ui.heading("Collapse");
    let Some(file) = game.knowledge.file(subject) else { return };
    if let Some((stay, best)) = file.whereabouts().last_known() {
        let ago = crate::panels::duration(game.coordinate_time_s() - best.emitted_s);
        ui.label(format!("In {}, {ago} ago", place(game, stay.system)));
        sources(ui, witnesses(game, uplink, best));
    }
    let owner = game.knowledge.owner;
    ui.horizontal_wrapped(|ui| {
        match file.thread().was(owner) {
            Some((craft, held)) => {
                let whose = craft.map_or_else(|| "nobody known".to_string(), |c| uplink.name_of(c));
                ui.label(egui::RichText::new(whose).strong().color(ui.visuals().strong_text_color()));
                crate::thread_view::source(ui, game, uplink, held, out);
            }
            None => {
                ui.weak("whose: unknown");
            }
        }
    });
    ui.horizontal(|ui| {
        let label = |c: Option<ShipId>| c.map_or_else(|| "nobody known".to_string(), |c| uplink.name_of(c));
        egui::ComboBox::from_id_salt(("was", subject)).selected_text(label(draft.was)).show_ui(ui, |ui| {
            ui.selectable_value(&mut draft.was, None, label(None));
            for (id, _) in game.knowledge.roster() {
                ui.selectable_value(&mut draft.was, Some(ShipId(id)), label(Some(ShipId(id))));
            }
        });
        crate::thread_view::scope_picker(ui, uplink, ("was scope", subject), &mut draft.was_scope);
        if ui.button("was").clicked() {
            ask(out, Action::Write { subject, writing: lc_proto::Writing::Was(draft.was), scope: draft.was_scope });
        }
    });
}

fn place(game: &Game, system: Option<lc_world::sky::StarId>) -> String {
    let here = game.system.as_ref().map(|s| s.star);
    match system {
        Some(star) => match game.knowledge.name_of(star) {
            Some(name) => name,
            None if Some(star) == here => "this system".to_string(),
            None => "an unnamed system".to_string(),
        },
        None => "between the stars".to_string(),
    }
}

/// Each witness of a sighting, with whoever relayed it.
fn witnesses(game: &Game, uplink: &crate::uplink::Uplink, sighting: &lc_world::knowledge::whereabouts::Sighting) -> Vec<String> {
    let owner = game.knowledge.owner;
    let who = |witness: Witness| match witness == owner {
        true => uplink.own_name(),
        false => uplink.name_of(ShipId(witness.0 as i64)),
    };
    sighting
        .witnesses
        .iter()
        .map(|w| {
            let relays: Vec<String> = w.lineage.iter().skip(1).map(|hop| who(hop.from)).collect();
            match relays.is_empty() {
                true => who(w.witness),
                false => format!("{}, relayed by {}", who(w.witness), relays.join(", ")),
            }
        })
        .collect()
}

/// Where the craft was last known to be, how that is known and on whose word, and every stay
/// before it.
fn whereabouts(ui: &mut egui::Ui, game: &Game, uplink: &crate::uplink::Uplink, id: ShipId) {
    let Some(held) = game.knowledge.file(Subject::Craft(id.0)).map(|f| f.whereabouts()) else { return };
    let Some((stay, best)) = held.last_known() else { return };
    let now_s = game.coordinate_time_s();
    let place = |system: Option<lc_world::sky::StarId>| place(game, system);
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
    sources(ui, witnesses(game, uplink, best));
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
