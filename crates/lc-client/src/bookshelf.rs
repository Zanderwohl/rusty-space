//! The bookshelf: what there is to read, and how to find it.
//!
//! The same case as [`crate::reader`], because it is the same object — one is the shelf and the
//! other is the page, and a player should not have to learn two things. Everything visual comes
//! from there; what is here is the list and the search.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use egui::{Align, CornerRadius, Margin, Rect, Stroke, Vec2};
use lc_books::catalogue::{Entry, Order, shelve};

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::library::{FontFace, Shelf};
use crate::panels::ask;
use crate::reader::{
    FAINT, INK, KEYS_HEIGHT, LABEL, PAPER, RULE, SCREEN_MARGIN, Setting, case, engraved, key,
    setting_for,
};
use crate::ui::Panel;

/// Air above and below a book in the list.
const ROW_GAP: f32 = 9.0;
/// What a row is tinted when the cursor is over it. Paper does not glow, so this is a shadow.
const ROW_HOT: egui::Color32 = egui::Color32::from_rgb(232, 228, 217);

#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut contexts: EguiContexts,
    state: Res<Ui>,
    mut shelf: ResMut<Shelf>,
    faces: Res<Assets<FontFace>>,
    assets: Res<AssetServer>,
    mut out: MessageWriter<Requested>,
    mut query: Local<String>,
    mut order: Local<Order>,
) {
    if !state.is_open(Panel::Bookshelf) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    crate::reader::settle_face_for_shelf(ctx, &mut shelf, &assets, &faces);
    let setting = setting_for(ctx);

    let view = ctx.content_rect();
    let width = 440.0_f32.min(view.width() - 48.0);
    let height = 560.0_f32.min(view.height() - 48.0);
    let mut open = true;

    egui::Window::new("bookshelf")
        .title_bar(false)
        .resizable(true)
        .default_size([width, height])
        // Off to the side rather than over the middle: the shelf and the book it opens are
        // meant to be used together, and a shelf that lands on top of the page is one you have
        // to close to read anything.
        .default_pos(egui::pos2(view.left() + 24.0, view.center().y - height / 2.0))
        .max_height(view.height() - 24.0)
        .min_width(300.0)
        .min_height(240.0)
        .frame(case())
        .show(ctx, |ui| {
            let inside = ui.available_height();
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.add(engraved("BOOKSHELF", 10.0, crate::reader::LABEL));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if key(ui, "×", "put the shelf away").clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(8.0);
            let taken = ui.min_rect().height();
            let paper = (inside - taken - KEYS_HEIGHT - SCREEN_MARGIN.top as f32
                - SCREEN_MARGIN.bottom as f32)
                .max(80.0);

            let screen = egui::Frame {
                inner_margin: Margin::symmetric(22, 20),
                fill: PAPER,
                stroke: Stroke::new(1.0_f32, RULE),
                corner_radius: CornerRadius::same(3),
                ..Default::default()
            };
            let found = screen
                .show(ui, |ui| {
                    ui.set_height(paper);
                    ui.set_width(ui.available_width());
                    paper_side(ui, &shelf, &setting, &mut query, &mut order, &mut out)
                })
                .inner;

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let total = shelf.catalogue.books.len();
                let words = if found == total {
                    format!("{total} books")
                } else {
                    format!("{found} of {total}")
                };
                ui.add(engraved(words, 10.0, FAINT));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    // Reversed, because a right-to-left layout places the first thing rightmost
                    // and the row is meant to read "title author year".
                    for by in Order::ALL.iter().rev() {
                        let response = key(ui, by.label(), "order the shelf by this");
                        if response.clicked() {
                            *order = *by;
                        }
                        if *order == *by {
                            // Underlined rather than lit: a lit button on a dark bezel reads as
                            // one that is about to do something, not one that already has.
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
        });

    if !open {
        ask(&mut out, Action::ClosePanel(Panel::Bookshelf));
    }
}

/// The page side of the shelf: the search line, and the books that answer to it.
fn paper_side(
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

    let found = shelve(&shelf.catalogue, query, *order);
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
                    egui::RichText::new(&entry.title).font(setting.body.clone()).color(INK),
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
        if reading {
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
        .hint_text(egui::RichText::new("title, author, subject").color(FAINT))
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
