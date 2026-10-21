//! The radio window: a list of craft on the left, one log on the right.
//!
//! Deliberately a log with a selector rather than a messaging app. Everything in it is minutes
//! to years old and there is no typing indicator to be had, so an interface that pretended the
//! far end was present would be lying about the one thing this game is about.
//!
//! What a conversation *is* lives in [`crate::chat`], which has no engine in it. This draws one
//! and asks; it decides nothing. See `lightcone/docs/05-observation.md` for the mechanic and
//! `13-client-shell.md` for the window.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_proto::{Body, FactionId};
use lc_world::sky::StarId;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::{ask, duration};
use crate::ui::Channel;

/// Somebody talking. The one color in the interface that means a person rather than a reading.
pub(crate) const RADIO: egui::Color32 = egui::Color32::from_rgb(120, 220, 140);

/// The face a log is set in: Geo, and only here.
///
/// What crossed the gap is set apart from the window showing it, the way a color already sets
/// it apart — the list on the left, the composer and every marker the client adds stay in the
/// interface face, because those are this ship talking to its pilot rather than a ship talking
/// to another ship.
///
/// Falls back to the interface face, which is what a build whose assets did not load gets. It
/// takes neither the scale nor the bump: both are about Geo, and there is no Geo in that case.
fn logged(ui: &egui::Ui) -> egui::FontId {
    let family = egui::FontFamily::Name(crate::faces::RADIO.into());
    let body = egui::TextStyle::Body.resolve(ui.style());
    match ui.fonts(|f| f.families().contains(&family)) {
        true => egui::FontId::new(body.size * LOG_SCALE + LOG_BUMP, family),
        false => body,
    }
}

/// What Geo needs to read at the size the interface reads at beside it.
///
/// A point size is an em, and an em says nothing about how much of it the letters fill: Geo's
/// capitals are 0.56 of theirs against Quantico's 0.70, so the same number draws a visibly
/// smaller line. Between matching the capitals (1.25) and matching the x-height (1.14),
/// because a log is mostly lowercase and the names in it are not.
const LOG_SCALE: f32 = 1.2;

/// And four points on top of that, which is a choice rather than a measurement: a transmission
/// is the one thing in this window that came from outside it, and it is read rather than
/// scanned.
const LOG_BUMP: f32 = 4.0;

/// Where a transmission is pointed, as the window offers it.
///
/// A mirror of [`lc_proto::Aim`] and not the type itself, because the third choice is "at
/// whatever star the telescope is on" — which is a thing the interface knows and the protocol
/// does not: on the wire it is already a catalog identifier.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Aimed {
    #[default]
    Omni,
    AtThem,
    AtTheSelectedStar,
}

/// Which kinds of chat the list shows. Public goes with Group; Overheard is always shown.
pub struct Listing {
    pub group: bool,
    pub direct: bool,
}

impl Default for Listing {
    fn default() -> Self {
        Self { group: true, direct: true }
    }
}

/// How wide the list of craft is, and how tall the window's body stays.
///
/// Fixed, both of them. A window that resizes itself as messages arrive is a window whose
/// buttons move under the cursor, and everything in here arrives without being asked for.
const LIST_WIDTH: f32 = 130.0;
/// The Group, Direct and found row above the list.
const LIST_HEADER: f32 = 26.0;
/// The whole body: both columns, the composer included.
const BODY_HEIGHT: f32 = 260.0;
/// What the composer under a log needs — the aim row, the key row, the field.
const COMPOSER_HEIGHT: f32 = 92.0;
const PANEL_WIDTH: f32 = 460.0;

/// How wide the log and the composer under it are. The column, less its separator and padding.
const LOG_WIDTH: f32 = PANEL_WIDTH - LIST_WIDTH - 24.0;

/// The radio: a list of craft on the left, one log on the right.
///
/// Deliberately a log with a selector rather than a messaging app. Everything here is minutes
/// to years old and there is no typing indicator to be had, so an interface that pretended the
/// far end was present would be lying about the one thing this game is about.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chat(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    draft: &mut String,
    aimed: &mut Aimed,
    seal: &mut bool,
    listing: &mut Listing,
    out: &mut MessageWriter<Requested>,
) {
    // Faction chats and craft together, most recently heard first; then everyone in sight never
    // spoken to, which is how a conversation starts at all.
    let mut entries: Vec<(f64, Channel, String)> = Vec::new();
    if listing.group {
        for (key, faction) in uplink.chat.factions() {
            let label = faction_label(uplink, key, faction, forked(uplink, key));
            entries.push((faction.last_at(), Channel::Faction(key), label));
        }
    }
    if listing.direct {
        for (id, c) in uplink.chat.conversations() {
            entries.push((c.last_at(), Channel::With(id), c.name.clone()));
        }
        for contact in &uplink.contacts {
            if !entries.iter().any(|(_, ch, _)| *ch == Channel::With(contact.ship_id)) {
                entries.push((f64::NEG_INFINITY, Channel::With(contact.ship_id), contact.name.clone()));
            }
        }
    }
    entries.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.2.cmp(&b.2)));

    // What this ship is called, for its own lines. The account's name as the broker knows it,
    // which is what every other craft sees on its contact list.
    let own = uplink.own_name();
    let showing = state.0.chat_with;

    // **One allocation for the whole body.** Everything inside is bounded by it, so the window
    // is the same size with one message in it and with two hundred — which matters here more
    // than in most panels, because what fills it arrives without being asked for.
    ui.allocate_ui(egui::vec2(PANEL_WIDTH, BODY_HEIGHT), |ui| {
    ui.horizontal_top(|ui| {
        // `allocate_ui_with_layout`, not `allocate_ui`: a child of a horizontal layout keeps
        // that direction, so the plain form laid the whole list out left to right — one name
        // per column, with the separator between "Public" and the first craft standing on end.
        ui.allocate_ui_with_layout(
            egui::vec2(LIST_WIDTH, BODY_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
            ui.horizontal(|ui| {
                ui.toggle_value(&mut listing.group, "Group");
                ui.toggle_value(&mut listing.direct, "Direct");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(showing == Channel::Founding, "+").clicked() {
                        ask(out, Action::ChatWith(Channel::Founding));
                    }
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("chat_parties")
                .max_height(BODY_HEIGHT - LIST_HEADER)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                ui.set_min_width(LIST_WIDTH - 8.0);
                // Justified, so every entry is the width of the column rather than the width
                // of its own name. A list whose click targets are each a different size reads
                // as a pile of labels; one where they line up reads as a list.
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                if listing.group
                    && ui
                        .selectable_label(showing == Channel::Public, "Public")
                        .on_hover_text("broadcasts: everything said to nobody in particular")
                        .clicked()
                {
                    ask(out, Action::ChatWith(Channel::Public));
                }
                if ui
                    .selectable_label(showing == Channel::Overheard, "Overheard")
                    .on_hover_text(
                        "traffic between other craft that this ship was in range of. Open ones \
                         can be read; encrypted ones cannot",
                    )
                    .clicked()
                {
                    ask(out, Action::ChatWith(Channel::Overheard));
                }
                ui.separator();
                for (_, channel, label) in &entries {
                    let mut item = ui.selectable_label(showing == *channel, label.as_str());
                    if let Channel::With(id) = channel {
                        let in_sight = uplink.contacts.iter().any(|c| c.ship_id == *id);
                        item = item.on_hover_text(match in_sight {
                            true => "in sight",
                            // Said, because it decides whether a beam can be aimed and whether
                            // a reply is years or minutes away.
                            false => "out of sight; still worth writing to",
                        });
                    }
                    if item.clicked() {
                        ask(out, Action::ChatWith(*channel));
                    }
                }
                if entries.is_empty() && !game.remote {
                    ui.weak("no server");
                }
                });
            });
        },
        );
        ui.separator();
        ui.vertical(|ui| match showing {
            Channel::Overheard => overheard_log(ui, uplink),
            Channel::Public => {
                let star = state.0.selected;
                // No craft to aim at, so that choice is not offered: a beam at one ship is not
                // a broadcast, whatever the panel is showing.
                if *aimed == Aimed::AtThem || (*aimed == Aimed::AtTheSelectedStar && star.is_none())
                {
                    *aimed = Aimed::Omni;
                }
                public_log(ui, uplink, &own, out);
                compose(ui, game, uplink, None, star, false, aim_of(*aimed, None, star), draft, aimed, seal, out);
            }
            Channel::Faction(key) => {
                let star = state.0.selected;
                if *aimed == Aimed::AtThem || (*aimed == Aimed::AtTheSelectedStar && star.is_none()) {
                    *aimed = Aimed::Omni;
                }
                faction_log(ui, uplink, key, &own, out);
                compose_faction(ui, game, uplink, key, star, aim_of(*aimed, None, star), draft, aimed, out);
            }
            Channel::Founding => founding(ui, draft, out),
            Channel::Myself => myself(ui, uplink, draft, out),
            Channel::With(with) => {
                let star = state.0.selected;
                let in_sight = uplink.contacts.iter().any(|c| c.ship_id == with);
                // Corrected once, here, so the log's resend button and the composer's send
                // button cannot disagree about where a message is pointed. A choice the panel
                // offered and the world has since withdrawn — the contact left, or the
                // telescope moved — falls back to a shout rather than a beam at nothing.
                if (*aimed == Aimed::AtThem && !in_sight)
                    || (*aimed == Aimed::AtTheSelectedStar && star.is_none())
                {
                    *aimed = Aimed::Omni;
                }
                let aim = aim_of(*aimed, Some(with), star);
                conversation(ui, uplink, with, &own, aim, out);
                compose(
                    ui, game, uplink, Some(with), star, in_sight, aim, draft, aimed, seal, out,
                );
            }
        });
    });
    });
}

/// Broadcasts, sent and heard: everything said to nobody in particular.
fn public_log(
    ui: &mut egui::Ui,
    uplink: &crate::uplink::Uplink,
    own: &str,
    out: &mut MessageWriter<Requested>,
) {
    let lines = uplink.chat.public();
    log_area(ui, "chat_public", BODY_HEIGHT - COMPOSER_HEIGHT, |ui| {
        if lines.is_empty() {
            ui.weak("Nothing has been broadcast.");
            return;
        }
        for loose in &lines {
            if loose.line.notice {
                notice(ui, &loose.line);
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                match loose.from.filter(|_| !loose.line.mine) {
                    // Somebody else's, so the name is the way to them. The name and not a
                    // button beside it: a log is prose, and prose links are words.
                    Some(from) => {
                        if clickable_name(ui, &format!("{}:", loose.from_name), RADIO)
                            .on_hover_text("open this conversation")
                            .clicked()
                        {
                            ask(out, Action::ChatWith(Channel::With(from)));
                        }
                    }
                    None => speaker(ui, own, true),
                }
                body_of(ui, &loose.line, loose.line.mine, None);
            });
        }
    });
}

/// A faction's chat: everything heard under its key, and this ship's own.
fn faction_log(
    ui: &mut egui::Ui,
    uplink: &crate::uplink::Uplink,
    key: FactionId,
    own: &str,
    out: &mut MessageWriter<Requested>,
) {
    let lines = uplink.chat.faction(key).map(|f| f.lines.as_slice()).unwrap_or_default();
    let newer = uplink.chat.newer(key);
    // Outside the log, so scrolling cannot hide it, and a row reserved even when empty, so the
    // log does not jump when a key lands. One row per branch: a fork may be two loyal members
    // who could not know of each other, or a traitor cutting part of the faction off.
    let banner = BANNER_ROW * newer.len().max(1) as f32;
    ui.allocate_ui(egui::vec2(LOG_WIDTH, banner), |ui| {
        let amber = egui::Color32::from_rgb(210, 165, 60);
        for (k, chat) in &newer {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Superseded by").color(amber));
                if clickable_name(ui, &faction_label(uplink, *k, chat, true), amber).clicked() {
                    ask(out, Action::ChatWith(Channel::Faction(*k)));
                }
            });
        }
    });
    log_area(ui, "chat_faction", BODY_HEIGHT - COMPOSER_HEIGHT - banner, |ui| {
        if lines.is_empty() {
            ui.weak("Nothing said under this key.");
            return;
        }
        for loose in lines {
            if loose.line.notice {
                notice(ui, &loose.line);
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                match loose.from.filter(|_| !loose.line.mine) {
                    Some(from) => {
                        if clickable_name(ui, &format!("{}:", loose.from_name), RADIO).clicked() {
                            ask(out, Action::ChatWith(Channel::With(from)));
                        }
                    }
                    None => speaker(ui, own, true),
                }
                body_of(ui, &loose.line, loose.line.mine, None);
            });
        }
    });
}

/// Whether another key held shares this one's name and generation, which only a fork makes.
fn forked(uplink: &crate::uplink::Uplink, key: FactionId) -> bool {
    let Some(this) = uplink.chat.faction(key) else { return false };
    uplink.chat.factions().iter().any(|(k, other)| {
        *k != key && other.name == this.name && other.generation() == this.generation()
    })
}

/// Room for one line saying a chat's key has been superseded.
const BANNER_ROW: f32 = 22.0;

/// What a faction is shown as: its name, or a mark that tells keys apart until the name arrives,
/// and always its generation. With `founder`, who minted it, which tells a fork's branches apart.
pub(crate) fn faction_label(uplink: &crate::uplink::Uplink, key: FactionId, chat: &crate::chat::FactionChat, founder: bool) -> String {
    let name = match chat.name.is_empty() {
        true => crate::chat::key_mark(key),
        false => chat.name.clone(),
    };
    let by = chat.founder.filter(|_| founder).map(|f| match uplink.joined().is_some_and(|j| j.ship_id == f) {
        true => "you".to_string(),
        false => uplink.name_of(f),
    });
    let g = chat.generation();
    match by {
        None => format!("{name} (gen {g})"),
        Some(by) => format!("{name} (gen {g}, {by})"),
    }
}

/// The aim and the field a message to a faction is typed into. Always under the key.
#[allow(clippy::too_many_arguments)]
fn compose_faction(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    key: FactionId,
    star: Option<StarId>,
    aim: lc_proto::Aim,
    draft: &mut String,
    aimed: &mut Aimed,
    out: &mut MessageWriter<Requested>,
) {
    ui.separator();
    let secrecy = lc_proto::Secrecy::Faction(key);
    ui.horizontal(|ui| {
        ui.selectable_value(aimed, Aimed::Omni, "omni")
            .on_hover_text("every direction: heard by everyone in range, read by whoever holds the key");
        ui.add_enabled_ui(star.is_some(), |ui| {
            let name = star.map(|id| game.name_of(id)).unwrap_or_else(|| "the selected star".into());
            ui.selectable_value(aimed, Aimed::AtTheSelectedStar, "beam star").on_hover_text(name);
        });
        // The rarer actions, behind a menu at the row's end.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = hamburger(ui);
            egui::Popup::menu(&more).show(|ui| {
                if ui.button("send survey").clicked() {
                    ask(out, Action::SendReport { to: None, aim, secrecy });
                }
                // The next generation, under the same name. Nobody else holds it until it is given.
                if ui.button("re-key").clicked() {
                    let name = uplink.chat.faction(key).map(|f| f.name.clone()).unwrap_or_default();
                    ask(out, Action::Found { name, replaces: Some(key) });
                }
                if ui.button("rename this ship").clicked() {
                    ask(out, Action::ChatWith(Channel::Myself));
                }
            });
        });
    });
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(draft).desired_width(LOG_WIDTH - 52.0));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("send").clicked() || entered) && !draft.trim().is_empty() {
            ask(out, Action::Say { to: None, aim, secrecy, body: std::mem::take(draft), idem: None });
            field.request_focus();
        }
    });
}

/// The faction keys this ship holds, to send: a submenu, disabled while it holds none.
#[allow(clippy::too_many_arguments)]
fn faction_keys(
    ui: &mut egui::Ui,
    uplink: &crate::uplink::Uplink,
    to: Option<lc_proto::ShipId>,
    aim: lc_proto::Aim,
    secrecy: lc_proto::Secrecy,
    sealed: bool,
    out: &mut MessageWriter<Requested>,
) {
    let factions = uplink.chat.factions();
    ui.add_enabled_ui(!factions.is_empty(), |ui| {
        ui.menu_button("send faction key", |ui| {
            for (key, faction) in &factions {
                if ui.button(faction_label(uplink, *key, faction, forked(uplink, *key))).clicked() {
                    ask(out, Action::GiveKey { key: *key, to, aim, secrecy });
                    ui.close();
                }
            }
        })
        .response
        // Open, it hands the key to everyone it reaches.
        .on_hover_text(match sealed {
            true => "sealed: only they will hold it",
            false => "open: everyone it reaches will hold it",
        });
    });
}

/// A button of three bars. Painted, because the default font has no U+2630 and draws a box.
fn hamburger(ui: &mut egui::Ui) -> egui::Response {
    let response = ui.add(egui::Button::new("").min_size(egui::vec2(24.0, 18.0)));
    let ink = ui.visuals().widgets.style(&response).fg_stroke.color;
    let c = response.rect.center();
    for dy in [-4.0, 0.0, 4.0] {
        ui.painter().line_segment(
            [egui::pos2(c.x - 6.0, c.y + dy), egui::pos2(c.x + 6.0, c.y + dy)],
            egui::Stroke::new(1.5_f32, ink),
        );
    }
    response
}

/// A line this client added to a log rather than one somebody said.
fn notice(ui: &mut egui::Ui, line: &crate::chat::Line) {
    if let Body::Text(text) = &line.body {
        ui.label(egui::RichText::new(text).italics().weak()).on_hover_text(reception(line));
    }
}

/// What this ship calls itself: a claim every transmission carries from now on.
fn myself(ui: &mut egui::Ui, uplink: &crate::uplink::Uplink, draft: &mut String, out: &mut MessageWriter<Requested>) {
    let Some(me) = uplink.joined().map(|joined| joined.ship_id) else {
        ui.weak("no server");
        return;
    };
    ui.label(format!("This ship is {}.", uplink.own_name()));
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(draft).desired_width(LOG_WIDTH - 72.0).hint_text("name"));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("rename").clicked() || entered) && !draft.trim().is_empty() {
            ask(out, Action::Write {
                subject: lc_world::knowledge::Subject::Craft(me.0),
                writing: lc_proto::Writing::Name(std::mem::take(draft)),
                scope: lc_proto::Scope::Open,
            });
        }
    });
}

/// Naming a new faction. Founding puts nothing on the air; the key is held at once.
fn founding(ui: &mut egui::Ui, draft: &mut String, out: &mut MessageWriter<Requested>) {
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(draft).desired_width(LOG_WIDTH - 64.0).hint_text("name"));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("found").clicked() || entered) && !draft.trim().is_empty() {
            ask(out, Action::Found { name: std::mem::take(draft), replaces: None });
        }
    });
}

/// Other people's traffic, which this ship happened to be in range of.
///
/// **The encrypted ones are here too**, and that is deliberate: the fact of a signal is real
/// whether or not it can be read, and a run of traffic between two craft says something even
/// when none of it says anything.
fn overheard_log(
    ui: &mut egui::Ui,
    uplink: &crate::uplink::Uplink,
) {
    let lines = uplink.chat.overheard();
    log_area(ui, "chat_overheard", BODY_HEIGHT, |ui| {
        if lines.is_empty() {
            ui.weak("Nothing has been overheard.");
            return;
        }
        for loose in &lines {
            if loose.line.notice {
                notice(ui, &loose.line);
                continue;
            }
            // Two lines: who was talking to whom, and then what crossed. Not the conversation
            // template, because this is not a conversation — there is no "you" in it, nothing
            // here is addressed to this ship, and there is nothing to acknowledge. No reply
            // button either: the thing to reply to is the craft, not the remark, and the list
            // on the left is where a craft is chosen.
            let from = loose.from_name.as_str();
            let to = loose.to.map(|to| uplink.name_of(to));
            let face = logged(ui);
            ui.label(
                egui::RichText::new(match (&to, loose.under) {
                    (Some(to), _) => format!("{from} -> {to}"),
                    (None, Some(key)) => format!("{from} -> {}", crate::chat::key_mark(key)),
                    // Unreachable while `overheard` means "addressed to somebody else", and
                    // cheaper to render than to prove.
                    (None, None) => from.to_string(),
                })
                .color(RADIO)
                .font(face.clone()),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                match &loose.line.body {
                    // A report is logged as a notice, which is drawn above.
                    Body::Report { .. } => {}
                    Body::Text(body) => {
                        ui.label(egui::RichText::new(body).font(face.clone()));
                    }
                    Body::Key => {
                        ui.label(egui::RichText::new("offered its key").font(face.clone()).weak());
                    }
                    Body::Faction(faction) => {
                        let gave = format!("gave the key to {} (gen {})", faction.name, faction.generation());
                        ui.label(egui::RichText::new(gave).font(face.clone()).weak());
                    }
                    Body::Ack => {}
                    // Fixed-length noise, and the same noise every frame. There is nothing in
                    // it to decode because there is nothing in it.
                    Body::Unreadable => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(loose.line.ciphertext())
                                    .monospace()
                                    .color(egui::Color32::from_rgb(110, 120, 130)),
                            )
                            .truncate(),
                        )
                        .on_hover_text("encrypted, and not for this ship");
                    }
                }
            })
            .response
            .on_hover_text(reception(&loose.line));
        }
    });
}


/// One craft's conversation, both halves.
fn conversation(
    ui: &mut egui::Ui,
    uplink: &crate::uplink::Uplink,
    with: lc_proto::ShipId,
    own: &str,
    aim: lc_proto::Aim,
    out: &mut MessageWriter<Requested>,
) {
    // A craft borrowing another's name says whose, and leads to the one it borrows from.
    if let Some(original) = uplink.labels.get(&with).and_then(|l| l.claimant_of) {
        let name = uplink.labels.get(&with).map(|l| l.name.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).color(RADIO));
            if clickable_name(ui, &format!("(claimant {})", with.tag()), egui::Color32::from_rgb(210, 165, 60))
                .on_hover_text(format!("not the {} you know", uplink.name_of(original)))
                .clicked()
            {
                ask(out, Action::ChatWith(Channel::With(original)));
            }
        });
    }
    let conversation = uplink.chat.get(with);
    log_area(ui, "chat_log", BODY_HEIGHT - COMPOSER_HEIGHT, |ui| {
        let Some(conversation) = conversation else {
            ui.weak("Nothing said yet.");
            return;
        };
        for line in &conversation.lines {
            if line.notice {
                notice(ui, line);
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                let said_by = match line.mine {
                    true => own,
                    false => conversation.name.as_str(),
                };
                speaker(ui, said_by, line.mine);
                body_of(ui, line, line.mine, Some(conversation.name.as_str()));
                if line.sealed {
                    ui.weak("encrypted");
                }
                // Only for something this ship said, and never for a key offer: a key is not
                // a thing anyone acknowledges — the server keeps it out of the window on
                // purpose — so marking one unacknowledged would be a warning that can never
                // clear.
                if let (true, Body::Text(text)) = (line.mine, &line.body) {
                    // The only delivery report there is. Silence is not a failure — it is a
                    // reply that has not been composed yet, or one still crossing.
                    match conversation.delivered(line) {
                        true => {
                            ui.colored_label(RADIO, "ack").on_hover_text(
                                "they named this message in something they sent back",
                            );
                        }
                        false if unacknowledged(ui, line.event_ids.len()).clicked() => {
                            ask(out, Action::Say {
                                to: Some(with),
                                aim,
                                // The original's, never the panel's. A message sent encrypted
                                // must not become one sent in the open by a second click.
                                secrecy: match line.sealed {
                                    true => lc_proto::Secrecy::Sealed,
                                    false => lc_proto::Secrecy::Open,
                                },
                                body: text.clone(),
                                // What makes this the same message rather than a second one.
                                idem: Some(line.idem),
                            });
                        }
                        false => {}
                    }
                }
            });
        }
    });
}

/// A scrolling region of a fixed height, so the window does not breathe as messages land.
fn log_area(ui: &mut egui::Ui, id: &str, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    let width = LOG_WIDTH;
    // Allocated to an exact size and then scrolled inside it. `max_height` alone only stops it
    // growing: a log with one line in it would still be one line tall, and the composer under
    // it would sit at a different height in every conversation.
    ui.allocate_ui(egui::vec2(width, height), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(width);
                add(ui);
            });
    });
}

/// A name that opens a conversation when it is clicked.
///
/// **Not a button.** A button's frame in the middle of a line of prose reads as a control that
/// does something to the line, and a log is prose. This is the name itself, underlined while
/// the cursor is on it — which is the oldest affordance there is for "this goes somewhere" and
/// is the one thing a label can do without becoming a widget.
pub(crate) fn clickable_name(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    let face = logged(ui);
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).color(color).font(face))
            .sense(egui::Sense::click()),
    );
    if response.hovered() {
        // Painted after the text rather than under it: an underline is a line below the
        // baseline, so there is nothing for it to cover.
        let rect = response.rect;
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(1.0_f32, color),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Who said it. Named rather than arrowed: the default font has no U+2192 and draws a tofu box
/// for it, and a name reads better in a log than a direction does.
fn speaker(ui: &mut egui::Ui, name: &str, mine: bool) {
    let color = match mine {
        true => egui::Color32::from_rgb(170, 190, 200),
        false => RADIO,
    };
    let face = logged(ui);
    ui.label(egui::RichText::new(format!("{name}:")).color(color).font(face));
}

/// What a message's tooltip says: when this ship learned of it, and how loud it was.
///
/// Two facts and no more. Both are about the *reception* rather than the message — a signal is
/// something that arrived somewhere at some strength, and everything else on the line is what
/// it said.
fn reception(line: &crate::chat::Line) -> String {
    let Some(arrived) = line.arrive_s else {
        return "Sent. Nothing here can see it land.".to_string();
    };
    let when = format!("Received T + {:.2} years", arrived / crate::flight::JULIAN_YEAR_S);
    match line.decibels() {
        Some(db) => format!("{when}\n{db:.1} dB, after {} in flight", duration((arrived - line.sent_s).max(0.0))),
        // Only a message recorded before the store kept the reading, now that it does.
        None => format!("{when}\nsignal strength not recorded"),
    }
}

/// The message itself. `to` names who it went to, for a line this ship sent to somebody — a
/// broadcast has nobody to name, and one that arrived came *from* the name already shown.
fn body_of(ui: &mut egui::Ui, line: &crate::chat::Line, mine: bool, to: Option<&str>) {
    let color = match mine {
        true => egui::Color32::from_rgb(170, 190, 200),
        false => RADIO,
    };
    let face = logged(ui);
    let said = |text: String| egui::RichText::new(text).font(face.clone());
    match &line.body {
        // Logged as a notice, never as a line somebody said.
        Body::Report { .. } => ui.weak(""),
        Body::Key if mine => ui.label(
            said(match to {
                Some(to) => format!("sent key to {to}"),
                None => "sent key".to_string(),
            })
            .weak(),
        ),
        Body::Key => ui.label(said("sent this ship its key".to_string()).color(RADIO)),
        Body::Faction(faction) if mine => ui.label(
            said(match to {
                Some(to) => format!("gave {to} the key to {} (gen {})", faction.name, faction.generation()),
                None => format!("gave out the key to {} (gen {})", faction.name, faction.generation()),
            })
            .weak(),
        ),
        Body::Faction(faction) => {
            ui.label(said(format!("gave the key to {} (gen {})", faction.name, faction.generation())).color(RADIO))
        }
        Body::Text(body) => ui.label(said(body.clone()).color(color)),
        Body::Ack => ui.label(said("(acknowledgment)".to_string()).weak()),
        // Heard and unreadable, which is worth showing rather than hiding: a player can see
        // that somebody in earshot is talking in private.
        Body::Unreadable => {
            ui.label(said("(encrypted, and not for this ship)".to_string()).weak())
        }
    }
    .on_hover_text(reception(line));
}

/// The warning on a message nothing has acknowledged, which is also the button that resends it.
///
/// **Painted rather than typed.** The obvious glyph for this is U+26A0, and the default font
/// draws a tofu box for it — the trap that has already cost this interface a close button and a
/// pair of arrows. A triangle is four lines of geometry and cannot be missing.
fn unacknowledged(ui: &mut egui::Ui, sends: usize) -> egui::Response {
    let size = egui::vec2(14.0, 14.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let amber = match response.hovered() {
        true => egui::Color32::from_rgb(255, 210, 90),
        false => egui::Color32::from_rgb(210, 165, 60),
    };
    let c = rect.center();
    let h = rect.height() * 0.42;
    let w = rect.width() * 0.46;
    let points = vec![
        egui::pos2(c.x, c.y - h),
        egui::pos2(c.x + w, c.y + h),
        egui::pos2(c.x - w, c.y + h),
    ];
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(points, amber, egui::Stroke::NONE));
    // The bar and dot of an exclamation mark, in the window's own background color so the
    // triangle reads as a warning sign rather than a plain arrowhead.
    let ink = ui.visuals().window_fill;
    painter.line_segment(
        [egui::pos2(c.x, c.y - h * 0.35), egui::pos2(c.x, c.y + h * 0.35)],
        egui::Stroke::new(1.6_f32, ink),
    );
    painter.circle_filled(egui::pos2(c.x, c.y + h * 0.65), 0.9, ink);
    response.on_hover_text(match sends {
        // A resend is a second pulse of light, not a retry of a failed one: nothing failed, and
        // nothing at either end can tell a message that missed from one still crossing.
        1 => "Not ACKed. Click to re-send.".to_string(),
        n => format!("Not ACKed. Click to re-send. (sent {n} times)"),
    })
}

/// The aim, the encryption and the field a message is typed into.
#[allow(clippy::too_many_arguments)]
fn compose(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    // `None` on the public channel: addressed to nobody, and nobody's key to encrypt with.
    with: Option<lc_proto::ShipId>,
    star: Option<StarId>,
    in_sight: bool,
    aim: lc_proto::Aim,
    draft: &mut String,
    aimed: &mut Aimed,
    seal: &mut bool,
    out: &mut MessageWriter<Requested>,
) {
    ui.separator();
    let holds_key = with.is_some_and(|with| uplink.chat.holds_key(with));

    ui.horizontal(|ui| {
        ui.selectable_value(aimed, Aimed::Omni, "omni")
            .on_hover_text("every direction: heard by everyone in range, and it says where you are");
        // Not offered at all on the public channel: aiming a broadcast at one ship is not a
        // broadcast, and an option that contradicts the channel is worse than a missing one.
        if with.is_some() {
            ui.add_enabled_ui(in_sight, |ui| {
                ui.selectable_value(aimed, Aimed::AtThem, "beam").on_hover_text(match in_sight {
                    true => "aimed where they are predicted to be; a craft under thrust is missed",
                    false => "nothing in sight to aim at",
                });
            });
        }
        ui.add_enabled_ui(star.is_some(), |ui| {
            let name = star.map(|id| game.name_of(id)).unwrap_or_else(|| "the selected star".into());
            ui.selectable_value(aimed, Aimed::AtTheSelectedStar, "beam star").on_hover_text(
                format!("{name}: the whole system, for when you do not know where in it they are"),
            );
        });
        // Sealed or not as the encrypt box says, for everything sent from here but a key offer.
        let secrecy = match *seal {
            true => lc_proto::Secrecy::Sealed,
            false => lc_proto::Secrecy::Open,
        };
        // The rarer transmissions are behind a menu at the row's end.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = hamburger(ui);
            egui::Popup::menu(&more).show(|ui| {
                // On the public channel, to whoever hears it, so anyone in range can answer in
                // private: the only way somebody never spoken to can open a private conversation.
                if ui.button("send personal key").clicked() {
                    ask(out, Action::OfferKey { to: with, aim });
                }
                faction_keys(ui, uplink, with, aim, secrecy, *seal, out);
                if ui.button("send survey").clicked() {
                    ask(out, Action::SendReport { to: with, aim, secrecy });
                }
                if ui.button("rename this ship").clicked() {
                    ask(out, Action::ChatWith(Channel::Myself));
                }
            });
        });
    });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(holds_key, |ui| {
            ui.checkbox(seal, "encrypt").on_hover_text(match (with, holds_key) {
                (_, true) => "Encrypt message to prevent others from understanding message.",
                (Some(_), false) => "Can't encrypt; don't have key",
                // Kept apart from the missing-key wording, because on a broadcast it would be
                // false: there is no key to be had, not a key this ship is short of. Nobody is
                // the addressee, so there is nobody to encrypt it for — and the server refuses
                // the combination rather than quietly sending it in the open.
                (None, false) => "Can't encrypt a broadcast; it is addressed to nobody",
            });
        });
        if !holds_key {
            *seal = false;
        }
        // One craft, one standing answer. Never on the public channel: a ship that answered
        // every broadcast it heard would announce its position to everything in range.
        if let Some(with) = with {
            let mut on = uplink.chat.auto_acks(with);
            if ui
                .checkbox(&mut on, "auto-ack")
                .on_hover_text("Automatically acknowledge messages from this ship upon receipt")
                .changed()
            {
                ask(out, Action::AutoAck { with, on });
            }
        }
    });

    ui.horizontal(|ui| {
        // From the column rather than from `available_width`, which inside a fixed allocation
        // is the whole remaining window and pushed the send button out past the panel's edge.
        let field = ui.add(
            egui::TextEdit::singleline(draft)
                .desired_width(LOG_WIDTH - 52.0)
                .hint_text("say something"),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("send").clicked() || entered) && !draft.trim().is_empty() {
            ask(out, Action::Say {
                to: with,
                aim,
                secrecy: match *seal {
                    true => lc_proto::Secrecy::Sealed,
                    false => lc_proto::Secrecy::Open,
                },
                body: std::mem::take(draft),
                idem: None,
            });
            field.request_focus();
        }
    });
    let left = lc_proto::MESSAGE_LIMIT.saturating_sub(draft.len());
    if left < 80 {
        ui.weak(format!("{left} characters left"));
    }
}

fn aim_of(aimed: Aimed, with: Option<lc_proto::ShipId>, star: Option<StarId>) -> lc_proto::Aim {
    match (aimed, star) {
        (Aimed::Omni, _) => lc_proto::Aim::Omni,
        (Aimed::AtThem, _) => match with {
            Some(with) => lc_proto::Aim::Ship(with),
            // Nobody to aim at, which the public channel never offers.
            None => lc_proto::Aim::Omni,
        },
        (Aimed::AtTheSelectedStar, Some(star)) => lc_proto::Aim::Star(star.get()),
        // The star went away between the choice and the click. Shouting is the safe fallback:
        // a beam with nothing to aim at is refused, and being refused is worse than being loud.
        (Aimed::AtTheSelectedStar, None) => lc_proto::Aim::Omni,
    }
}
