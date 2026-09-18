//! The bookshelf: what there is to read, and how to find it.
//!
//! **Not a window.** It is what [`crate::reader`]'s window shows when no book is open — the same
//! case, the same page, the same two bezels. A shelf and a book are one object a player picks up,
//! and two windows for it was two things to arrange on a screen and two things to close.

use bevy::prelude::*;
use bevy_egui::egui;
use egui::{Align, CornerRadius, Rect, Stroke, Vec2};
use lc_books::catalogue::{Entry, Order, shelve};

use crate::action::Action;
use crate::input::Requested;
use crate::library::Shelf;
use crate::panels::ask;
use crate::reader::{FAINT, INK, LABEL, PAPER, RULE, Setting, engraved, key};

/// Air above and below a book in the list.
const ROW_GAP: f32 = 9.0;
/// What a row is tinted when the cursor is over it. Paper does not glow, so this is a shadow.
const ROW_HOT: egui::Color32 = egui::Color32::from_rgb(232, 228, 217);

/// The foot of the case while the shelf is showing: how many books, and how they are ordered.
pub(crate) fn keys(
    ui: &mut egui::Ui,
    found: usize,
    total: usize,
    order: &mut Order,
) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let words =
            if found == total { format!("{total} books") } else { format!("{found} of {total}") };
        ui.add(engraved(words, 10.0, FAINT));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            // Reversed, because a right-to-left layout places the first thing rightmost and the
            // row is meant to read "title author year recent".
            for by in Order::ALL.iter().rev() {
                let response = key(ui, by.label(), "order the shelf by this");
                if response.clicked() {
                    *order = *by;
                }
                if *order == *by {
                    // Underlined rather than lit: a lit button on a dark bezel reads as one that
                    // is about to do something, not one that already has.
                    let rect = response.rect;
                    ui.painter().line_segment(
                        [
                            rect.left_bottom() + Vec2::new(2.0, 1.0),
                            rect.right_bottom() + Vec2::new(-2.0, 1.0),
                        ],
                        Stroke::new(1.0_f32, LABEL),
                    );
                }
            }
        });
    });
}

/// The page side of the shelf: the search line, and the books that answer to it.
pub(crate) fn paper(
    ui: &mut egui::Ui,
    shelf: &Shelf,
    setting: &Setting,
    query: &mut String,
    order: &mut Order,
    out: &mut MessageWriter<Requested>,
) -> usize {
    search(ui, setting, query);
    ui.add_space(12.0);

    if shelf.catalogue.books.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new("the shelf is empty").font(setting.body.clone()).color(FAINT),
            )
            .selectable(false),
        );
        return 0;
    }

    let found = shelve(&shelf.catalogue, query, *order, &shelf.recent());
    if found.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new("nothing here answers to that")
                    .font(setting.body.clone())
                    .color(FAINT),
            )
            .selectable(false),
        );
        return 0;
    }

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, entry) in found.iter().enumerate() {
            if i > 0 {
                ui.add_space(ROW_GAP);
                let y = ui.cursor().top() - ROW_GAP / 2.0;
                ui.painter().line_segment(
                    [
                        egui::pos2(ui.max_rect().left(), y),
                        egui::pos2(ui.max_rect().right(), y),
                    ],
                    Stroke::new(1.0_f32, RULE),
                );
            }
            if book(ui, entry, setting, shelf).clicked() {
                ask(out, Action::OpenBook(entry.id.clone()));
                // Opened where it was left. Two messages rather than one, because where a book
                // opens is a different fact from which book it is, and a client with no shard
                // has the first and not the second.
                if let Some(mark) = shelf.mark_for(&entry.id) {
                    ask(out, Action::GoTo(mark.spine as usize, mark.char_offset as usize));
                }
            }
        }
    });
    found.len()
}

/// One book on the shelf.
fn book(ui: &mut egui::Ui, entry: &Entry, setting: &Setting, shelf: &Shelf) -> egui::Response {
    // Reserved before the text is drawn, because the tint goes behind it and egui paints in the
    // order it is told.
    let tint = ui.painter().add(egui::Shape::Noop);

    let reading = shelf.open_id() == Some(entry.id.as_str());
    let laid = ui.scope(|ui| {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&entry.title).font(setting.listing()).color(INK),
                )
                .selectable(false),
            );
        });
        let mut line = entry.by_line();
        if let Some(year) = entry.year {
            if !line.is_empty() {
                line.push_str("  ·  ");
            }
            line.push_str(&year.to_string());
        }
        // How far in, when the shard has been keeping a place. A percentage rather than a
        // location, because the shelf is where someone decides what to pick up and "34%" is
        // what that decision wants.
        if let Some(mark) = shelf.mark_for(&entry.id)
            && mark.locations > 0
        {
            let through = (mark.location as f32 / mark.locations as f32 * 100.0).round();
            line.push_str(&format!("  ·  {through:.0}%"));
        } else if reading {
            line.push_str("  ·  open");
        }
        if !line.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(line).font(setting.small.clone()).color(FAINT),
                )
                .selectable(false),
            );
        }
        ui.add_space(4.0);
    });

    let row = Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), laid.response.rect.top()),
        egui::pos2(ui.max_rect().right(), laid.response.rect.bottom()),
    );
    let response = ui.interact(row, ui.id().with(&entry.id), egui::Sense::click());
    if response.hovered() || reading {
        ui.painter().set(
            tint,
            egui::Shape::rect_filled(row.expand2(Vec2::new(6.0, 0.0)), CornerRadius::same(3), ROW_HOT),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// The search line. One field, and a rule under it rather than a box around it.
fn search(ui: &mut egui::Ui, setting: &Setting, query: &mut String) {
    let visuals = ui.visuals_mut();
    visuals.override_text_color = Some(INK);
    visuals.extreme_bg_color = PAPER;
    visuals.selection.bg_fill = ROW_HOT;
    visuals.selection.stroke.color = INK;
    visuals.text_cursor.stroke.color = INK;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.bg_stroke = Stroke::NONE;

    let field = egui::TextEdit::singleline(query)
        .font(setting.body.clone())
        .hint_text(egui::RichText::new("search").color(FAINT))
        .desired_width(f32::INFINITY)
        .frame(false);
    let response = ui.add(field);
    let y = response.rect.bottom() + 3.0;
    ui.painter().line_segment(
        [egui::pos2(response.rect.left(), y), egui::pos2(response.rect.right(), y)],
        Stroke::new(1.0_f32, RULE),
    );
    ui.add_space(4.0);
}
