//! A subject's thread, and the field that adds to it. What the thread is lives in
//! `lc_world::knowledge::thread`; see `lightcone/docs/23-factions.md` §Notes.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_proto::{Scope, ShipId, Writing};
use lc_world::knowledge::Subject;

use crate::action::Action;
use crate::app::Game;
use crate::input::Requested;
use crate::panels::ask;

/// What is being written, and where it may go. Starts private.
#[derive(Default)]
pub struct Draft {
    pub text: String,
    pub scope: Scope,
}

/// How tall the list of entries may grow before it scrolls.
const LIST_HEIGHT: f32 = 160.0;

pub(crate) fn thread(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    subject: Subject,
    draft: &mut Draft,
    out: &mut MessageWriter<Requested>,
) {
    let owner = game.knowledge.owner;
    if let Some(thread) = game.knowledge.file(subject).map(|f| f.thread()).filter(|t| !t.is_empty()) {
        let ordered = thread.ordered();
        egui::ScrollArea::vertical().id_salt(("thread", subject)).max_height(LIST_HEIGHT).show(ui, |ui| {
            for (at, (id, held)) in ordered.iter().enumerate() {
                let entry = &held.entry;
                let who = match entry.author == owner {
                    true => uplink.own_name(),
                    false => uplink.name_of(ShipId(entry.author.0 as i64)),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("{who}:"));
                    match &entry.writing {
                        Writing::Note(text) => ui.label(text),
                        Writing::Name(name) => ui.label(egui::RichText::new(format!("named it {name}")).italics()),
                    };
                    ui.weak(format!("({})", scope_label(uplink, entry.scope)));
                });
                // Who it was written without knowing of: the one thing this order cannot show.
                let unaware: Vec<String> = ordered[..at]
                    .iter()
                    .filter(|(earlier, _)| thread.concurrent(*id, *earlier))
                    .map(|(_, h)| match h.entry.author == owner {
                        true => "yours".to_string(),
                        false => uplink.name_of(ShipId(h.entry.author.0 as i64)),
                    })
                    .collect();
                if !unaware.is_empty() {
                    ui.weak(format!("written without knowing of {}", unaware.join(", ")));
                }
                if entry.parents.iter().any(|p| thread.get(*p).is_none()) {
                    ui.weak("replies to something not held here");
                }
            }
        });
    }
    if !game.nameable(subject) {
        return;
    }
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(150.0));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        egui::ComboBox::from_id_salt(("scope", subject))
            .selected_text(scope_label(uplink, draft.scope))
            .show_ui(ui, |ui| {
                for scope in scopes(uplink) {
                    ui.selectable_value(&mut draft.scope, scope, scope_label(uplink, scope));
                }
            });
        let note = ui.button("note").clicked() || entered;
        let rename = ui.button("rename").clicked();
        if (note || rename) && !draft.text.trim().is_empty() {
            let text = std::mem::take(&mut draft.text);
            let writing = match rename {
                true => Writing::Name(text),
                false => Writing::Note(text),
            };
            ask(out, Action::Write { subject, writing, scope: draft.scope });
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
