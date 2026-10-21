//! A subject's names and notes, and the fields that add to them. What the thread is lives in
//! `lc_world::knowledge::thread`; see `lightcone/docs/23-factions.md` §Notes.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_proto::{Scope, ShipId, Writing};
use lc_world::knowledge::Subject;
use lc_world::knowledge::thread::Held;

use crate::action::Action;
use crate::app::Game;
use crate::input::Requested;
use crate::panels::ask;
use crate::ui::{Channel, Panel};

/// What is being written, and where each may go. Both start private.
#[derive(Default)]
pub struct Draft {
    pub name: String,
    pub name_scope: Scope,
    pub note: String,
    pub note_scope: Scope,
}

/// How tall the list of notes may grow before it scrolls.
const NOTES_HEIGHT: f32 = 140.0;
const FIELD_WIDTH: f32 = 180.0;

/// The names and notes for one subject. With `writable`, the fields that add to them.
pub(crate) fn naming_and_notes(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    subject: Subject,
    writable: bool,
    draft: &mut Draft,
    out: &mut MessageWriter<Requested>,
) {
    let writable = writable && game.nameable(subject);
    names(ui, game, uplink, subject, writable, draft, out);
    notes(ui, game, uplink, subject, writable, draft, out);
}

/// Every name that stands, brightest first, each with whose word it is on; then a field to give
/// one of this ship's own.
pub(crate) fn names(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    subject: Subject,
    writable: bool,
    draft: &mut Draft,
    out: &mut MessageWriter<Requested>,
) {
    let owner = game.knowledge.owner;
    let file = game.knowledge.file(subject);
    let mut standing: Vec<&Held> =
        file.map(|f| f.thread().standing_names().into_iter().map(|(_, h)| h).collect()).unwrap_or_default();
    // The one this ship goes by first.
    let goes_by = file.and_then(|f| f.thread().standing(owner)).map(|(id, _)| id);
    standing.sort_by_key(|h| Some(h.entry.id()) != goes_by);
    let bright = ui.visuals().strong_text_color();
    for held in &standing {
        let Writing::Name(name) = &held.entry.writing else { continue };
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(name).strong().color(bright));
            source(ui, game, uplink, held, out);
        });
    }
    // What a rule assigned, under whatever was chosen, or as the name when nothing was.
    let designation = game.knowledge.designation_of(subject);
    match (standing.is_empty(), designation) {
        (true, Some(designation)) => {
            ui.label(egui::RichText::new(designation).strong().color(bright));
        }
        (false, Some(designation)) => {
            ui.weak(designation);
        }
        (_, None) => {}
    }
    if !writable {
        return;
    }
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(FIELD_WIDTH).hint_text("name"));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        scope_picker(ui, uplink, ("name scope", subject), &mut draft.name_scope);
        if (ui.button("rename").clicked() || entered) && !draft.name.trim().is_empty() {
            let writing = Writing::Name(std::mem::take(&mut draft.name));
            ask(out, Action::Write { subject, writing, scope: draft.name_scope });
        }
    });
}

/// The notes, parents before replies, each with who wrote it, where it may go, and what it was
/// written without knowing of; then a field to add one.
pub(crate) fn notes(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    subject: Subject,
    writable: bool,
    draft: &mut Draft,
    out: &mut MessageWriter<Requested>,
) {
    let owner = game.knowledge.owner;
    if let Some(thread) = game.knowledge.file(subject).map(|f| f.thread()) {
        let ordered: Vec<_> =
            thread.ordered().into_iter().filter(|(_, h)| matches!(h.entry.writing, Writing::Note(_))).collect();
        if !ordered.is_empty() {
            egui::ScrollArea::vertical().id_salt(("notes", subject)).max_height(NOTES_HEIGHT).show(ui, |ui| {
                for (at, (id, held)) in ordered.iter().enumerate() {
                    let Writing::Note(text) = &held.entry.writing else { continue };
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(format!("{}:", author(uplink, owner, held)));
                        ui.label(text);
                        source(ui, game, uplink, held, out);
                    });
                    // Who it was written without knowing of: the one thing this order cannot show.
                    let unaware: Vec<String> = ordered[..at]
                        .iter()
                        .filter(|(earlier, _)| thread.concurrent(*id, *earlier))
                        .map(|(_, h)| match h.entry.author == owner {
                            true => "yours".to_string(),
                            false => author(uplink, owner, h),
                        })
                        .collect();
                    if !unaware.is_empty() {
                        ui.weak(format!("written without knowing of {}", unaware.join(", ")));
                    }
                    if held.entry.parents.iter().any(|p| thread.get(*p).is_none()) {
                        ui.weak("replies to something not held here");
                    }
                }
            });
        }
    }
    if !writable {
        return;
    }
    ui.add(egui::TextEdit::multiline(&mut draft.note).desired_width(f32::INFINITY).desired_rows(3).hint_text("note"));
    ui.horizontal(|ui| {
        scope_picker(ui, uplink, ("note scope", subject), &mut draft.note_scope);
        if ui.button("note").clicked() && !draft.note.trim().is_empty() {
            let writing = Writing::Note(std::mem::take(&mut draft.note));
            ask(out, Action::Write { subject, writing, scope: draft.note_scope });
        }
    });
}

/// Whose word an entry is on: a faction's, as a link to its chat; this ship's; or somebody's in
/// the open.
fn source(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    held: &Held,
    out: &mut MessageWriter<Requested>,
) {
    match held.entry.scope {
        Scope::Faction(key) => {
            if quiet_link(ui, &scope_label(uplink, Scope::Faction(key))).on_hover_text("open its chat").clicked() {
                ask(out, Action::OpenPanel(Panel::Chat));
                ask(out, Action::ChatWith(Channel::Faction(key)));
            }
        }
        Scope::Own => quiet(ui, "yours"),
        Scope::Open if held.entry.author == game.knowledge.owner => quiet(ui, "yours, public"),
        Scope::Open => quiet(ui, &format!("public, {}", author(uplink, game.knowledge.owner, held))),
    }
}

/// Whose word something is on: small and weak, beside what it is about.
fn quiet(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(ui.visuals().weak_text_color()));
}

/// A link that stays in the background: small and weak, underlined only under the cursor.
fn quiet_link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let color = ui.visuals().weak_text_color();
    let response = ui.add(egui::Label::new(egui::RichText::new(text).small().color(color)).sense(egui::Sense::click()));
    if response.hovered() {
        let rect = response.rect;
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0_f32, color));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn author(uplink: &crate::uplink::Uplink, owner: lc_world::knowledge::Witness, held: &Held) -> String {
    match held.entry.author == owner {
        true => uplink.own_name(),
        false => uplink.name_of(ShipId(held.entry.author.0 as i64)),
    }
}

fn scope_picker(ui: &mut egui::Ui, uplink: &crate::uplink::Uplink, id: (&str, Subject), scope: &mut Scope) {
    egui::ComboBox::from_id_salt(id).selected_text(scope_label(uplink, *scope)).show_ui(ui, |ui| {
        for choice in scopes(uplink) {
            ui.selectable_value(scope, choice, scope_label(uplink, choice));
        }
    });
}

/// For me, Public, then every faction key held.
fn scopes(uplink: &crate::uplink::Uplink) -> Vec<Scope> {
    let mut out = vec![Scope::Own, Scope::Open];
    out.extend(uplink.chat.factions().into_iter().map(|(key, _)| Scope::Faction(key)));
    out
}

fn scope_label(uplink: &crate::uplink::Uplink, scope: Scope) -> String {
    match scope {
        Scope::Own => "for me".into(),
        Scope::Open => "public".into(),
        Scope::Faction(key) => match uplink.chat.faction(key) {
            Some(chat) => crate::radio_panel::faction_label(uplink, key, chat, false),
            None => crate::chat::key_mark(key),
        },
    }
}
