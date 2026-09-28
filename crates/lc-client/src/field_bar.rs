//! The field bar in the top readout: Black, Clear and Auto, the bar, and Auto's markers on it.
//! What it shows is [`hud::Field`]; see `lightcone/docs/30-the-field.md` §The field bar.

use bevy_egui::egui;
use lc_proto::FieldMode;
use lc_world::field::Mode;
use lc_world::fitting::Setting;

use crate::action::Action;
use crate::hud::{self, Marker};

/// The energy bar's, so the two read as a pair.
pub const WIDTH: f32 = 80.0;

/// Fixed rather than derived from the bar's auto id, which moves when anything before it in the
/// readout comes or goes: a dropped marker's thresholds would be lost with it.
const ID: &str = "field bar";

/// Returns what was asked of it.
pub fn draw(ui: &mut egui::Ui, field: &hud::Field, text: Option<&str>) -> Vec<Action> {
    let mut asked = Vec::new();
    let auto = matches!(field.setting, Setting::Auto(_));
    // Lit is what was chosen; underlined is what the field is in, which in Auto is the thermostat's.
    let buttons = [
        ("Black", FieldMode::Black, field.setting == Setting::Black, field.shade == Mode::Black),
        ("Clear", FieldMode::Clear, field.setting == Setting::Clear, field.shade == Mode::Clear),
        ("Auto", Setting::Auto(field.auto).into(), auto, false),
    ];
    for (label, mode, on, underlined) in buttons {
        let text = egui::RichText::new(label);
        let text = if underlined { text.underline() } else { text };
        if ui.selectable_label(on, text).clicked() && !on {
            asked.push(Action::SetField(mode));
        }
    }
    if let Some(switch) = &field.switch {
        ui.small(switch);
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
        let pending = egui::Id::new(ID).with("dropped");
        let dropped = ui.data(|d| d.get_temp::<hud::Dropped>(pending)).filter(|d| d.holds(field, now_s));
        let thresholds = dropped.map_or(held, |d| d.thresholds);
        let shown_field = hud::Field { setting: Setting::Auto(thresholds), ..field.clone() };
        for marker in [Marker::ClearAbove, Marker::BlackBelow] {
            let at = marker.of(&thresholds);
            let hit = egui::Rect::from_center_size(egui::pos2(x_of(at), rect.center().y), egui::vec2(10.0, rect.height()));
            let grip = ui.interact(hit, egui::Id::new(ID).with(marker as u8), egui::Sense::drag());
            let dropped_at = |pos: egui::Pos2| f64::from((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            // `dragged` is false on the release frame, which drew the old threshold for a frame.
            let pointer = grip.interact_pointer_pos().filter(|_| grip.dragged() || grip.drag_stopped());
            let shown = pointer.map_or(at, |pos| marker.of(&marker.moved(thresholds, dropped_at(pos))));
            notch(&painter, marker, x_of(shown), rect, ink);
            if grip.drag_stopped()
                && let Some(action) = pointer.and_then(|pos| hud::drop_marker(&shown_field, marker, dropped_at(pos)))
            {
                // A switch under way refuses it before it is sent, so there is nothing to wait for.
                if let (false, Action::SetField(mode)) = (field.switch.is_some(), &action)
                    && let Setting::Auto(ordered) = Setting::from(*mode)
                {
                    ui.data_mut(|d| d.insert_temp(pending, hud::Dropped { thresholds: ordered, at_s: now_s }));
                }
                asked.push(action);
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
    asked
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

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, RawInput, Shape};
    use lc_world::fitting::{Balance, Thresholds};

    use super::*;

    fn field() -> hud::Field {
        hud::Field {
            fraction: 0.2,
            heading: 0.3,
            kelvin: 2_400.0,
            blackbody: [1.0, 0.5, 0.2],
            stress: 0.0,
            text: "2 400 K steady".into(),
            countdown: None,
            setting: Setting::Auto(Thresholds::of(&Balance::DEFAULT)),
            auto: Thresholds::of(&Balance::DEFAULT),
            shade: Mode::Clear,
            switch: None,
        }
    }

    /// One frame at `t_s`: where Clear's notch was painted, and what was asked.
    fn frame(ctx: &egui::Context, t_s: f64, events: Vec<Event>) -> (f32, Vec<Action>) {
        let input = RawInput { time: Some(t_s), events, ..Default::default() };
        let mut asked = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            ui.horizontal(|ui| asked = draw(ui, &field(), None));
        });
        out.textures_delta.clear();
        // Clear's notch hangs from the top edge, so its apex is the lower of its points.
        let apex = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Path(path) if path.points.len() == 3 && path.points[0].y < path.points[2].y => Some(path.points[2].x),
                _ => None,
            })
            .next()
            .expect("Clear's notch is drawn");
        (apex, asked)
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Breaks if any frame from the release on draws the old threshold before the answer is overdue.
    #[test]
    fn a_dropped_marker_never_shows_its_old_place_while_in_flight() {
        let ctx = egui::Context::default();
        let (x0, _) = frame(&ctx, 0.0, vec![]);
        let (at, to) = (Pos2::new(x0, 8.0), Pos2::new(x0 + 0.2 * WIDTH, 8.0));
        frame(&ctx, 0.1, vec![Event::PointerMoved(at)]);
        frame(&ctx, 0.2, vec![press(at, true)]);
        let mut t = 0.3;
        for step in 1..=4 {
            let x = x0 + 0.05 * step as f32 * WIDTH;
            frame(&ctx, t, vec![Event::PointerMoved(Pos2::new(x, 8.0))]);
            t += 0.05;
        }
        let (released, asked) = frame(&ctx, t, vec![press(to, false)]);
        assert!((released - to.x).abs() < 1.0, "on release: {released}, dropped at {}", to.x);
        let [Action::SetField(lc_proto::FieldMode::Auto { clear_above, .. })] = asked.as_slice() else {
            panic!("one order: {asked:?}");
        };
        assert!((clear_above - 0.7).abs() < 1.0e-9, "{clear_above}");
        for k in 1..=5 {
            let (x, _) = frame(&ctx, t + 0.5 * f64::from(k), vec![]);
            assert!((x - to.x).abs() < 1.0, "{k} frames on: {x}");
        }
        let (x, _) = frame(&ctx, t + 10.0, vec![]);
        assert!((x - x0).abs() < 1.0, "no answer, so back: {x}");
    }
}
