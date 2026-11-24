//! The ordered feature list, with suppression, the rollback bar and the unread rows in place.

use bevy_egui::egui::{self, Color32, RichText, Sense};
use lc_galaxy::{Row, Unread};

use crate::doc::Doc;
use crate::palette;
use crate::params_panel;

pub fn show(ui: &mut egui::Ui, doc: &mut Doc) {
    ui.horizontal(|ui| {
        ui.heading("Timeline");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Add", |ui| {
                let kinds: Vec<&'static str> = doc.registry.kinds().collect();
                for kind in kinds {
                    if ui.button(kind).clicked() {
                        add(doc, kind);
                        ui.close();
                    }
                }
            });
        });
    });
    ui.separator();

    let problems = doc.result.as_ref().map(|r| (r.compiled.problems.clone(), r.compiled.held.clone()));
    let cut = doc.cut();
    let mut row_rects = Vec::with_capacity(doc.timeline.len());
    let mut action = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for index in 0..doc.timeline.len() {
            if index == cut {
                rollback_bar(ui, &mut action);
            }
            let rolled_back = index >= cut;
            let selected = doc.selected == Some(index);
            let response = ui
                .scope(|ui| {
                    if rolled_back {
                        ui.multiply_opacity(0.4);
                    }
                    row(ui, doc, index, selected, problems.as_ref(), &mut action)
                })
                .inner;
            row_rects.push(response.rect);
            if response.clicked() {
                doc.selected = Some(index);
            }
            response.context_menu(|ui| context_menu(ui, doc, index, &mut action));
        }
        if cut >= doc.timeline.len() {
            rollback_bar(ui, &mut action);
        }
    });

    if let Some(Action::DragCut(y)) = action {
        let to = row_rects.iter().position(|r| y < r.center().y).unwrap_or(row_rects.len());
        doc.cut = if to >= doc.timeline.len() { None } else { Some(to) };
    }
    if let Some(Action::Run(f)) = action {
        f(doc);
    }
}

enum Action {
    DragCut(f32),
    Run(Box<dyn FnOnce(&mut Doc)>),
}

fn rollback_bar(ui: &mut egui::Ui, action: &mut Option<Action>) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::drag());
    let color = if response.hovered() || response.dragged() { palette::ROLLBACK_HOT } else { palette::ROLLBACK };
    ui.painter().rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 2.0, color);
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        *action = Some(Action::DragCut(p.y));
    }
}

fn row(
    ui: &mut egui::Ui,
    doc: &Doc,
    index: usize,
    selected: bool,
    problems: Option<&(std::collections::BTreeMap<lc_galaxy::FeatureId, lc_galaxy::Problem>, std::collections::BTreeSet<lc_galaxy::FeatureId>)>,
    action: &mut Option<Action>,
) -> egui::Response {
    let frame = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(4, 3))
        .corner_radius(3.0)
        .fill(if selected { palette::SELECTED } else { Color32::TRANSPARENT });
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        match &doc.timeline.rows()[index] {
            Row::Feature(f) => {
                let (id, kind, label, summary, enabled) =
                    (f.id, f.params.kind(), f.label(), f.params.summary(), f.enabled);
                ui.horizontal(|ui| {
                    let mut on = enabled;
                    if ui.checkbox(&mut on, "").changed() {
                        *action = Some(Action::Run(Box::new(move |d: &mut Doc| {
                            if let Some(f) = d.timeline.feature_mut(id) {
                                f.enabled = on;
                            }
                        })));
                    }
                    ui.label(RichText::new(kind).small().color(palette::KIND));
                    let text = RichText::new(&label);
                    ui.label(if enabled { text } else { text.strikethrough() });
                    if let Some((problems, held)) = problems
                        && let Some(problem) = problems.get(&id)
                    {
                        let mark = if held.contains(&id) { "held" } else { "problem" };
                        ui.label(RichText::new(mark).small().color(palette::PROBLEM)).on_hover_text(problem.to_string());
                    }
                });
                if f.name.is_some() {
                    ui.label(RichText::new(summary).small().weak());
                }
            }
            Row::Unread(u) => {
                let (tag, detail) = match u {
                    Unread::Unknown { kind, version, .. } => ("unknown", format!("{kind} v{version}")),
                    Unread::Unsupported { kind, name, fields, .. } => (
                        "unsupported",
                        format!("{} {kind}: {}", name.as_deref().unwrap_or(""), fields.join(", ")),
                    ),
                    Unread::Damaged { error, .. } => ("damaged", error.clone()),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tag).small().color(palette::UNREAD));
                    ui.label(RichText::new(detail).small());
                });
            }
        }
    });
    ui.interact(inner.response.rect, ui.id().with(("row", index)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .union(inner.response)
}

fn context_menu(ui: &mut egui::Ui, doc: &Doc, index: usize, action: &mut Option<Action>) {
    let legal = doc.timeline.legal_range(index);
    let mut run = |ui: &mut egui::Ui, label: &str, enabled: bool, f: Box<dyn FnOnce(&mut Doc)>| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            *action = Some(Action::Run(f));
            ui.close();
        }
    };
    run(ui, "Move up", index > 0 && legal.contains(&(index - 1)), Box::new(move |d| {
        if d.timeline.move_row(index, index - 1).is_ok() {
            d.selected = Some(index - 1);
        }
    }));
    run(ui, "Move down", legal.contains(&(index + 1)), Box::new(move |d| {
        if d.timeline.move_row(index, index + 1).is_ok() {
            d.selected = Some(index + 1);
        }
    }));
    run(ui, "Roll back to here", true, Box::new(move |d| d.cut = Some(index + 1)));
    ui.separator();
    run(ui, "Delete", true, Box::new(move |d| delete(d, index)));
}

pub fn delete(doc: &mut Doc, index: usize) {
    if index >= doc.timeline.len() {
        return;
    }
    doc.timeline.remove(index);
    doc.selected = None;
    if let Some(c) = doc.cut
        && c > index
    {
        doc.cut = Some(c - 1);
    }
}

fn add(doc: &mut Doc, kind: &str) {
    let Some(mut params) = doc.registry.create(kind) else { return };
    let at = doc.selected.map_or(doc.timeline.len(), |i| i + 1);
    params_panel::seed_references(params.as_mut(), &doc.timeline, at);
    doc.timeline.insert(at, params, None);
    doc.selected = Some(at);
    if let Some(c) = doc.cut
        && c >= at
    {
        doc.cut = Some(c + 1);
    }
}
