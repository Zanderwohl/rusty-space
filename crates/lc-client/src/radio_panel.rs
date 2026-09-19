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

use lc_world::sky::StarId;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::{ask, duration};
use crate::ui::Channel;

/// Somebody talking. The one colour in the interface that means a person rather than a reading.
pub(crate) const RADIO: egui::Color32 = egui::Color32::from_rgb(120, 220, 140);

/// The face a log is set in: Geo, and only here.
///
/// What crossed the gap is set apart from the window showing it, the way a colour already sets
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
/// capitals are 0.56 of theirs against egui's own face at 0.69, so the same number draws a
/// visibly smaller line. Between matching the capitals (1.24) and matching the x-height (1.18),
/// because a log is mostly lowercase and the names in it are not.
const LOG_SCALE: f32 = 1.2;

/// And two points on top of that, which is a choice rather than a measurement: a transmission
/// is the one thing in this window that came from outside it, and it is read rather than
/// scanned.
const LOG_BUMP: f32 = 2.0;

/// Where a transmission is pointed, as the window offers it.
///
/// A mirror of [`lc_proto::Aim`] and not the type itself, because the third choice is "at
/// whatever star the telescope is on" — which is a thing the interface knows and the protocol
/// does not: on the wire it is already a catalogue identifier.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Aimed {
    #[default]
    Omni,
    AtThem,
    AtTheSelectedStar,
}

/// How wide the list of craft is, and how tall the window's body stays.
///
/// Fixed, both of them. A window that resizes itself as messages arrive is a window whose
/// buttons move under the cursor, and everything in here arrives without being asked for.
const LIST_WIDTH: f32 = 130.0;
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
    out: &mut MessageWriter<Requested>,
) {
    // Everyone this ship has talked to, most recent first, then everyone in sight it has not.
    // The second half is how a conversation starts at all.
    let mut parties: Vec<(lc_proto::ShipId, String)> =
        uplink.chat.conversations().into_iter().map(|(id, c)| (id, c.name.clone())).collect();
    for contact in &uplink.contacts {
        if !parties.iter().any(|(id, _)| *id == contact.ship_id) {
            parties.push((contact.ship_id, contact.name.clone()));
        }
    }

    // What this ship is called, for its own lines. The account's name as the broker knows it,
    // which is what every other craft sees on its contact list.
    let own = uplink.joined().map(|j| j.name.clone()).unwrap_or_else(|| "this ship".into());
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
            egui::ScrollArea::vertical()
                .id_salt("chat_parties")
                .max_height(BODY_HEIGHT)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                ui.set_min_width(LIST_WIDTH - 8.0);
                // Justified, so every entry is the width of the column rather than the width
                // of its own name. A list whose click targets are each a different size reads
                // as a pile of labels; one where they line up reads as a list.
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                if ui
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
                for (id, name) in &parties {
                    let in_sight = uplink.contacts.iter().any(|c| c.ship_id == *id);
                    let label = ui
                        .selectable_label(showing == Channel::With(*id), name.as_str())
                        .on_hover_text(match in_sight {
                            true => "in sight",
                            // Said, because it decides whether a beam can be aimed and whether
                            // a reply is years or minutes away.
                            false => "out of sight; still worth writing to",
                        });
                    if label.clicked() {
                        ask(out, Action::ChatWith(Channel::With(*id)));
                    }
                }
                if parties.is_empty() {
                    ui.weak(match game.remote {
                        true => "nobody yet",
                        false => "no server",
                    });
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
            // Two lines: who was talking to whom, and then what crossed. Not the conversation
            // template, because this is not a conversation — there is no "you" in it, nothing
            // here is addressed to this ship, and there is nothing to acknowledge. No reply
            // button either: the thing to reply to is the craft, not the remark, and the list
            // on the left is where a craft is chosen.
            let from = loose.from_name.as_str();
            let to = loose.to.map(|to| uplink.name_of(to));
            let face = logged(ui);
            ui.label(
                egui::RichText::new(match &to {
                    Some(to) => format!("{from} -> {to}"),
                    // Unreachable while `overheard` means "addressed to somebody else", and
                    // cheaper to render than to prove.
                    None => from.to_string(),
                })
                .color(RADIO)
                .font(face.clone()),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                match loose.line.body.as_deref() {
                    Some(body) => {
                        ui.label(egui::RichText::new(body).font(face.clone()));
                    }
                    // Fixed-length noise, and the same noise every frame. There is nothing in
                    // it to decode because there is nothing in it.
                    None => {
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
    let conversation = uplink.chat.get(with);
    log_area(ui, "chat_log", BODY_HEIGHT - COMPOSER_HEIGHT, |ui| {
        let Some(conversation) = conversation else {
            ui.weak("Nothing said yet.");
            return;
        };
        for line in &conversation.lines {
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
                if line.mine && !line.key {
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
                                body: line.body.clone().unwrap_or_default(),
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
pub(crate) fn clickable_name(ui: &mut egui::Ui, text: &str, colour: egui::Color32) -> egui::Response {
    let face = logged(ui);
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).color(colour).font(face))
            .sense(egui::Sense::click()),
    );
    if response.hovered() {
        // Painted after the text rather than under it: an underline is a line below the
        // baseline, so there is nothing for it to cover.
        let rect = response.rect;
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(1.0_f32, colour),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Who said it. Named rather than arrowed: the default font has no U+2192 and draws a tofu box
/// for it, and a name reads better in a log than a direction does.
fn speaker(ui: &mut egui::Ui, name: &str, mine: bool) {
    let colour = match mine {
        true => egui::Color32::from_rgb(170, 190, 200),
        false => RADIO,
    };
    let face = logged(ui);
    ui.label(egui::RichText::new(format!("{name}:")).color(colour).font(face));
}

/// What a message's tooltip says: when this ship learnt of it, and how loud it was.
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
    let colour = match mine {
        true => egui::Color32::from_rgb(170, 190, 200),
        false => RADIO,
    };
    let face = logged(ui);
    let said = |text: String| egui::RichText::new(text).font(face.clone());
    match (&line.body, line.key) {
        (_, true) if mine => ui.label(
            said(match to {
                Some(to) => format!("sent key to {to}"),
                None => "sent key".to_string(),
            })
            .weak(),
        ),
        (_, true) => ui.label(said("sent this ship its key".to_string()).color(RADIO)),
        (Some(body), _) => ui.label(said(body.clone()).color(colour)),
        // Heard and unreadable, which is worth showing rather than hiding: a player can see
        // that somebody in earshot is talking in private.
        (None, _) => ui.label(said("(encrypted, and not for this ship)".to_string()).weak()),
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
    // The bar and dot of an exclamation mark, in the window's own background colour so the
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
            let name = star
                .and_then(|id| game.star(id))
                .and_then(|s| s.name.clone())
                .unwrap_or_else(|| "the selected star".into());
            ui.selectable_value(aimed, Aimed::AtTheSelectedStar, "beam star").on_hover_text(
                format!("{name}: the whole system, for when you do not know where in it they are"),
            );
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
        if ui
            .button("send key")
            .on_hover_text(match with {
                Some(_) => "so they can encrypt messages back; it travels at c like anything else",
                // The reason to put a key on the public channel at all: it is the only way
                // somebody you have never spoken to can open a private conversation with you.
                None => "to whoever hears it, so anyone in range can answer in private",
            })
            .clicked()
        {
            ask(out, Action::OfferKey { to: with, aim });
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
