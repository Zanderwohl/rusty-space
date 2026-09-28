//! The field bar in the top readout: Black, Clear and Auto, the bar, and Auto's markers on it.
//! What it shows is [`hud::Field`]; see `lightcone/docs/30-the-field.md` §The field bar.

use bevy::prelude::MessageWriter;
use bevy_egui::egui;
use lc_proto::FieldMode;
use lc_world::fitting::Setting;

use crate::action::Action;
use crate::hud::{self, Marker};
use crate::input::Requested;
use crate::panels::ask;

/// The energy bar's, so the two read as a pair.
pub const WIDTH: f32 = 80.0;

pub fn draw(ui: &mut egui::Ui, field: &hud::Field, text: Option<&str>, out: &mut MessageWriter<Requested>) {
    let auto = matches!(field.setting, Setting::Auto(_));
    let buttons = [
        ("Black", FieldMode::Black, field.setting == Setting::Black),
        ("Clear", FieldMode::Clear, field.setting == Setting::Clear),
        ("Auto", Setting::Auto(field.auto).into(), auto),
    ];
    for (label, mode, on) in buttons {
        if ui.selectable_label(on, label).clicked() && !on {
            ask(out, Action::SetField(mode));
        }
    }
    if let Some(shade) = &field.shade {
        ui.small(shade);
    }

    let blue = ui.visuals().selection.bg_fill;
    let lit = field.brightness(ui.input(|i| i.time));
    let [r, g, b] = field.color([blue.r(), blue.g(), blue.b()].map(|c| f32::from(c) / 255.0));
    let byte = |c: f32| (c * lit * 255.0).round().clamp(0.0, 255.0) as u8;
    let bar = ui.add(egui::ProgressBar::new(field.fraction).desired_width(WIDTH).fill(egui::Color32::from_rgb(byte(r), byte(g), byte(b))));
    let rect = bar.rect;
    let x_of = |fraction: f64| rect.left() + fraction as f32 * rect.width();
    let ink = ui.visuals().strong_text_color();
    let painter = ui.painter_at(rect.expand(1.0));
    let tick = x_of(f64::from(field.heading)).min(rect.right() - 1.0);
    painter.line_segment([egui::pos2(tick, rect.top()), egui::pos2(tick, rect.bottom())], egui::Stroke::new(2.0, ink));

    if let Setting::Auto(held) = field.setting {
        let now_s = ui.input(|i| i.time);
        let pending = bar.id.with("dropped");
        let dropped = ui.data(|d| d.get_temp::<hud::Dropped>(pending)).filter(|d| d.holds(field, now_s));
        let thresholds = dropped.map_or(held, |d| d.thresholds);
        let shown_field = hud::Field { setting: Setting::Auto(thresholds), ..field.clone() };
        for marker in [Marker::ClearAbove, Marker::BlackBelow] {
            let at = marker.of(&thresholds);
            let hit = egui::Rect::from_center_size(egui::pos2(x_of(at), rect.center().y), egui::vec2(10.0, rect.height()));
            let grip = ui.interact(hit, bar.id.with(marker as u8), egui::Sense::drag());
            let dropped_at = |pos: egui::Pos2| f64::from((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let shown = match grip.interact_pointer_pos().filter(|_| grip.dragged()) {
                Some(pos) => marker.of(&marker.moved(thresholds, dropped_at(pos))),
                None => at,
            };
            notch(&painter, marker, x_of(shown), rect, ink);
            if grip.drag_stopped()
                && let Some(action) = grip.interact_pointer_pos().and_then(|pos| hud::drop_marker(&shown_field, marker, dropped_at(pos)))
            {
                // A switch under way refuses it before it is sent, so there is nothing to wait for.
                if let (false, Action::SetField(mode)) = (field.switching, &action)
                    && let Setting::Auto(ordered) = Setting::from(*mode)
                {
                    ui.data_mut(|d| d.insert_temp(pending, hud::Dropped { thresholds: ordered, at_s: now_s }));
                }
                ask(out, action);
            }
            grip.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        }
    }

    if let Some(text) = text {
        ui.label(text);
    }
    if text != Some(field.text.as_str()) {
        bar.on_hover_text(&field.text);
    }
}

/// A marker as a notch into the bar: Clear's from the top edge, Black's from the bottom, so the
/// two are told apart by shape.
fn notch(painter: &egui::Painter, marker: Marker, x: f32, rect: egui::Rect, ink: egui::Color32) {
    let (edge, inward) = match marker {
        Marker::ClearAbove => (rect.top(), 1.0),
        Marker::BlackBelow => (rect.bottom(), -1.0),
    };
    let depth = 0.45 * rect.height() * inward;
    let points = vec![egui::pos2(x - 4.0, edge), egui::pos2(x + 4.0, edge), egui::pos2(x, edge + depth)];
    painter.add(egui::Shape::convex_polygon(points, ink, egui::Stroke::NONE));
}
