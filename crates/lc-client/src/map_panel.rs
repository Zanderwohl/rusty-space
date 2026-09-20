//! The two surfaces the map is shown on, and the input they take.
//!
//! One rendered image inside an egui surface, a third case beside the two
//! `lightcone/docs/18-ui-style.md` names. The image is allocated with an explicit
//! [`egui::Sense`] rather than through `ui.image`, so egui wants the pointer and
//! `crate::input`'s wheel and cursor grab stand down on their own.

use bevy::prelude::*;
use glam::DVec3;
use bevy_egui::{EguiContexts, egui};

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::map::Map;
#[cfg(feature = "godview")]
use crate::map_source::Source;
use crate::panels::ask;
use crate::ui::Panel;

/// Radians of turn per point of drag. The same feel as the sky's own look.
const TURN_PER_POINT: f64 = 0.006;

/// A pan drag across the whole viewport moves the focus by this much of the stand-off.
const PAN_PER_VIEWPORT: f64 = 1.2;

/// How much of the surface the scale rule may take, and the least it is worth drawing at. A
/// fraction to stay proportionate on the minimap, and a ceiling so it does not stretch across
/// a wide panel.
const RULE_MAX_FRACTION: f32 = 0.4;
const RULE_MIN_FRACTION: f32 = 0.3;
const RULE_MAX_PX: f32 = 600.0;
const RULE_MIN_PX: f32 = 100.0;

/// Where the rule sits, inset from the bottom right of the surface.
const RULE_INSET: egui::Vec2 = egui::vec2(12.0, 10.0);

/// How tall the end caps are, and the divisions between them.
const RULE_CAP_PX: f32 = 5.0;
const RULE_TICK_PX: f32 = 3.0;

/// Where the plane is sampled for the scale, as a fraction of the way down the viewport.
///
/// The camera is perspective, so there is no one scale. The rule is drawn near the bottom, so
/// that is where the plane is asked how far a pixel goes.
const RULE_SAMPLE_NDC_Y: f64 = -0.75;

/// How far a label sits from its mark's edge, and the clearance kept between two labels. Two
/// names a pixel apart are one smear, so the clearance is generous and [`em_map::label`] drops
/// the second.
const LABEL_GAP_PX: f32 = 4.0;
/// How light a body may be and still be named, against the heaviest thing on screen.
///
/// The same number that decides how big its mark is drawn, because it is the same comparison:
/// see [`em_map::weight::FLOOR`] for where it comes from.
const LABEL_FLOOR: f64 = em_map::weight::FLOOR;
/// The amber a contact's name is written in, which is the amber its mark is drawn in.
const SHIP_LABEL: bevy::prelude::Color = bevy::prelude::Color::srgb(0.95, 0.70, 0.25);
const LABEL_CLEARANCE_PX: f32 = 6.0;

/// The minimap's side, in points.
const MINIMAP_SIDE: f32 = 190.0;

/// Bottom left. The event log takes bottom right with this same inset (`panels.rs`).
const MINIMAP_MARGIN: egui::Vec2 = egui::vec2(12.0, -12.0);

/// Draw whichever surface is in force, and turn pointer input on it into actions. The panel
/// and the minimap are never both up: the minimap is the map when the panel is closed.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    game: Res<Game>,
    mut map: ResMut<Map>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let open = ui_state.is_open(Panel::Map);
    map.shown = true;

    let response = match open {
        true => panel(ctx, &ui_state, &game, &mut map, &mut out),
        false => minimap(ctx, &mut map),
    };

    let Some((rect, response)) = response else {
        map.shown = false;
        return;
    };
    map.wanted = UVec2::new(
        (rect.width() * ctx.pixels_per_point()).round().max(1.0) as u32,
        (rect.height() * ctx.pixels_per_point()).round().max(1.0) as u32,
    );
    // One function for both surfaces, so they answer a drag alike.
    let painter = ctx.layer_painter(response.layer_id);
    scale_rule(&painter, rect, ui_state.map);
    labels(&painter, rect, ui_state.map, &map);
    read_input(ctx, &response, rect, ui_state.map, &map, &mut out);
    if !open && response.clicked() {
        // egui keeps a click and a drag apart, so this is the minimap's one extra gesture.
        ask(&mut out, Action::OpenPanel(Panel::Map));
    }
}

/// The names, over the image.
///
/// egui text rather than geometry on the layer, per `18-ui-style.md`. The work is in two
/// halves: this measures what each name would occupy, and [`em_map::label::lay_out`] decides
/// which of them fit.
fn labels(painter: &egui::Painter, rect: egui::Rect, view: crate::ui::MapView, map: &Map) {
    let Some(frame) = map.frame.as_ref() else { return };
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let aspect = (rect.width() / rect.height()) as f64;
    let font = painter.ctx().style_of(egui::Theme::Dark).text_styles[&egui::TextStyle::Small]
        .clone();

    let mut candidates = Vec::with_capacity(frame.placements.len());
    let mut galleys = Vec::with_capacity(frame.placements.len());
    for placement in &frame.placements {
        // Not the observer: the rings and spokes are centered on it already.
        if placement.kind == em_map::ItemKind::Observer || placement.label.is_empty() {
            continue;
        }
        let Some(ndc) =
            view.orbit.project(view.plane, placement.at.as_dvec3(), crate::map::MAP_FOV as f64,
                aspect)
        else {
            continue;
        };
        let galley = painter.layout_no_wrap(placement.label.clone(), font.clone(),
            label_color(placement.kind));
        let size = galley.size();
        candidates.push(em_map::label::Candidate {
            key: placement.key,
            weight: placement.weight,
            // egui counts pixels down from the top left; NDC runs up from the middle.
            at: glam::Vec2::new(
                (ndc.x as f32 + 1.0) * 0.5 * rect.width(),
                (1.0 - ndc.y as f32) * 0.5 * rect.height(),
            ),
            size: glam::Vec2::new(size.x, size.y),
        });
        galleys.push((placement.key, galley));
    }

    // Clear of the mark and centered on it. The mark is sized against the texture and the
    // text against the surface showing it, which differ on a display that scales.
    let points_per_pixel = match map.size.y {
        0 => 1.0,
        height => rect.height() / height as f32,
    };
    let layout = em_map::label::Layout {
        offset: glam::Vec2::new(
            map.symbol_px() * points_per_pixel * 0.5 + LABEL_GAP_PX,
            -font.size * 0.5,
        ),
        gap: LABEL_CLEARANCE_PX,
        floor: LABEL_FLOOR,
    };
    let viewport = glam::Vec2::new(rect.width(), rect.height());
    for placed in em_map::label::lay_out(candidates, viewport, layout) {
        let Some((_, galley)) = galleys.iter().find(|(key, _)| *key == placed.key) else {
            continue;
        };
        painter.galley(
            rect.min + egui::vec2(placed.at.x, placed.at.y),
            galley.clone(),
            egui::Color32::WHITE,
        );
    }
}

/// Amber for a contact, the interface's text color for everything else. Not the mark's color:
/// a body's mark takes the palette's dimmer greens, too dim to read text at.
fn label_color(kind: em_map::ItemKind) -> egui::Color32 {
    match kind {
        em_map::ItemKind::Ship | em_map::ItemKind::Station => color_of(SHIP_LABEL),
        _ => color_of(em_ui::vfd::TEXT),
    }
}

/// The full surface: the map, and the controls for what it is showing.
fn panel(
    ctx: &egui::Context,
    ui_state: &Ui,
    game: &Game,
    map: &mut Map,
    out: &mut MessageWriter<Requested>,
) -> Option<(egui::Rect, egui::Response)> {
    let mut open = true;
    let mut answer = None;
    egui::Window::new("Map")
        .open(&mut open)
        .default_size([640.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| {
            controls(ui, ui_state, game, out);
            ui.separator();
            let side = ui.available_size();
            answer = Some(image(ui, map, side, egui::Sense::click_and_drag()));
        });
    if !open {
        ask(out, Action::ClosePanel(Panel::Map));
    }
    answer
}

/// The corner surface: the panel's gestures, plus a click that opens the panel.
///
/// The cost is real and worth stating. `crate::input`'s wheel and cursor grab stand down while
/// egui wants the pointer, and this surface is always on screen, so hovering the corner stops
/// the ship's boom zooming and a right-press begun here pans the map. Every panel costs this;
/// the minimap is the only one never closed.
fn minimap(ctx: &egui::Context, map: &mut Map) -> Option<(egui::Rect, egui::Response)> {
    let mut answer = None;
    egui::Area::new("minimap".into())
        // Middle, not Foreground: an open window has to cover this.
        .order(egui::Order::Middle)
        .anchor(egui::Align2::LEFT_BOTTOM, MINIMAP_MARGIN)
        .show(ctx, |ui| {
            let side = egui::vec2(MINIMAP_SIDE, MINIMAP_SIDE);
            answer = Some(image(ui, map, side, egui::Sense::click_and_drag()));
        });
    answer
}

/// Put the rendered texture on screen, with a sense so egui claims the pointer over it.
fn image(ui: &mut egui::Ui, map: &Map, size: egui::Vec2, sense: egui::Sense)
    -> (egui::Rect, egui::Response) {
    let size = egui::vec2(size.x.max(64.0), size.y.max(64.0));
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if let Some(texture) = map.texture {
        ui.painter().image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    (rect, response)
}

/// Draw the scale rule into the bottom right of the map's surface.
fn scale_rule(painter: &egui::Painter, rect: egui::Rect, view: crate::ui::MapView) {
    let Some(per_point) = meters_per_point(rect, view) else { return };
    let max = (rect.width() * RULE_MAX_FRACTION).min(RULE_MAX_PX);
    let min = (rect.width() * RULE_MIN_FRACTION).min(RULE_MIN_PX);
    let Some(rule) = em_map::rule::choose((max * per_point) as f64, (min * per_point) as f64)
    else {
        return;
    };
    let length = (rule.meters / per_point as f64) as f32;
    if !length.is_finite() || length <= 0.0 || length > rect.width() {
        return;
    }

    let stroke = egui::Stroke::new(1.0, color_of(em_ui::vfd::TEXT_DIM));
    let right = rect.max.x - RULE_INSET.x;
    let y = rect.max.y - RULE_INSET.y;
    let left = right - length;
    painter.line_segment([egui::pos2(left, y), egui::pos2(right, y)], stroke);
    for (at, height) in [(left, RULE_CAP_PX), (right, RULE_CAP_PX)] {
        painter.line_segment([egui::pos2(at, y - height), egui::pos2(at, y)], stroke);
    }
    // The divisions, which are whole units of the label. See `em_map::rule::Rule::parts`.
    for i in 1..rule.parts {
        let at = left + length * i as f32 / rule.parts as f32;
        painter.line_segment([egui::pos2(at, y - RULE_TICK_PX), egui::pos2(at, y)], stroke);
    }
    painter.text(
        egui::pos2(right, y - RULE_CAP_PX - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        &rule.label,
        egui::FontId::proportional(11.0),
        color_of(em_ui::vfd::TEXT),
    );
}

/// How far a point on the surface reaches, in meters, where the rule is drawn. A ray cast at
/// the rule's own height meets the plane, and the scale is taken at that depth; when it meets
/// nothing the stand-off is the only answer left.
fn meters_per_point(rect: egui::Rect, view: crate::ui::MapView) -> Option<f32> {
    if rect.height() <= 0.0 {
        return None;
    }
    let rad_per_point = 2.0 * (crate::map::MAP_FOV * 0.5).tan() / rect.height();
    let (forward, ..) = view.orbit.view_basis(view.plane);
    let eye = view.orbit.eye_ly(view.plane);
    let aspect = (rect.width() / rect.height()) as f64;
    let direction =
        view.orbit.ray(view.plane, glam::DVec2::new(0.0, RULE_SAMPLE_NDC_Y),
            crate::map::MAP_FOV as f64, aspect);
    let depth_m = match view.plane.intersect(eye, direction, view.orbit.focus_ly) {
        Some(hit) => (hit - eye).dot(forward) * em_map::snapshot::M_PER_LY,
        None => view.orbit.distance_m(),
    };
    let per_point = depth_m as f32 * rad_per_point;
    (per_point.is_finite() && per_point > 0.0).then_some(per_point)
}

/// The interface's palette, converted at the toolkit boundary. One source, per
/// `lightcone/docs/18-ui-style.md`.
fn color_of(color: bevy::prelude::Color) -> egui::Color32 {
    let rgba = color.to_srgba();
    egui::Color32::from_rgb(
        (rgba.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn controls(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    ui.horizontal(|ui| {
        ui.label("plane");
        for plane in [em_map::Plane::Ecliptic, em_map::Plane::Galactic] {
            if ui.selectable_label(state.map.plane == plane, plane.label()).clicked() {
                ask(out, Action::SetMapPlane(plane));
            }
        }
        // Light delay is the premise everywhere in the client; the picture taken without it
        // is the one that has to say so.
        #[cfg(feature = "godview")]
        if state.map.source == Source::God {
            ui.separator();
            ui.weak("coordinate time, no light delay");
        }
    });

    #[cfg(feature = "godview")]
    ui.horizontal(|ui| {
        ui.label("source");
        for source in [Source::Observed, Source::God] {
            let allowed = source == Source::Observed || state.may_see_everything;
            let chosen = state.map.source == source;
            // A plain selectable label: `add_enabled` around one reports clicks nobody made.
            // A refused source is grayed and says why, because color is not the only signal.
            if !allowed {
                ui.weak(source.label()).on_hover_text("needs an administrative account");
                continue;
            }
            if ui.selectable_label(chosen, source.label()).clicked() {
                ask(out, Action::SetMapSource(source));
            }
        }
        if state.map.source == Source::God {
            ui.weak("— other ships are not drawn: their light has not arrived");
        }
    });

    ui.horizontal(|ui| {
        if ui.button("center on the ship").clicked() {
            ask(out, Action::FocusMap(crate::ui::MapFocus::Observer));
        }
        // "the star", not its name: a button whose label changes between systems has to be
        // read before it is pressed. The name is on the star itself.
        if let Some(system) = game.0.system.as_ref() {
            if ui.button("center on the star").clicked() {
                ask(out, Action::FocusMap(crate::ui::MapFocus::Item(
                    em_map::ItemKey::from_id("star", system.star.get()),
                )));
            }
        }
    });
}

/// Drag and wheel over the full surface: left turns, right pans.
///
/// Right is also the sky's look button, so a right-press that starts on either map surface
/// pans the map and not the view. `grab_cursor` stands down while egui wants the pointer, and
/// the drag belongs to the widget it began on.
fn read_input(
    ctx: &egui::Context,
    response: &egui::Response,
    rect: egui::Rect,
    view: crate::ui::MapView,
    map: &Map,
    out: &mut MessageWriter<Requested>,
) {
    if response.dragged() {
        let delta = response.drag_delta();
        // Right as well as middle: the minimap is too small to hold a modifier over.
        let panning = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || ctx.input(|i| i.modifiers.shift);
        if panning {
            let span = rect.width().max(rect.height()).max(1.0) as f64;
            ask(out, Action::PanMap {
                right: -delta.x as f64 / span * PAN_PER_VIEWPORT,
                ahead: delta.y as f64 / span * PAN_PER_VIEWPORT,
            });
        } else {
            ask(out, Action::TurnMap {
                azimuth: -delta.x as f64 * TURN_PER_POINT,
                elevation: delta.y as f64 * TURN_PER_POINT,
            });
        }
    }
    read_wheel_only(ctx, response, rect, view, map, out);
}

fn read_wheel_only(
    ctx: &egui::Context,
    response: &egui::Response,
    rect: egui::Rect,
    view: crate::ui::MapView,
    map: &Map,
    out: &mut MessageWriter<Requested>,
) {
    if !response.hovered() {
        return;
    }
    // egui reports pixels, and `input::notches` is the tested divider for a trackpad.
    let scrolled = ctx.input(|i| i.smooth_scroll_delta.y);
    if scrolled == 0.0 {
        return;
    }
    let notches =
        crate::input::notches(bevy::input::mouse::MouseScrollUnit::Pixel, scrolled);
    let anchor_ly = match zooms_to_cursor(view.focus) {
        true => under_cursor(ctx, rect, view, map),
        false => None,
    };
    ask(out, Action::ZoomMap { notches, anchor_ly });
}

/// Whether the wheel zooms toward the pointer rather than the center.
///
/// Only when nothing is locked. `Action::ZoomMap` gives the lock up the moment an anchor moves
/// the focus, so a held center must ask for no anchor.
fn zooms_to_cursor(focus: crate::ui::MapFocus) -> bool {
    matches!(focus, crate::ui::MapFocus::Free)
}

/// What the cursor is over, on the reference plane. `None` when the pointer is nowhere, the
/// ray runs along the plane, or the plane is behind the camera; the caller then zooms about
/// the middle.
fn under_cursor(
    ctx: &egui::Context,
    rect: egui::Rect,
    view: crate::ui::MapView,
    map: &Map,
) -> Option<DVec3> {
    let at = ctx.input(|i| i.pointer.hover_pos())?;
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    // NDC: `[-1, 1]` across the viewport with `+y` up, where egui counts down from the top.
    let ndc = glam::DVec2::new(
        ((at.x - rect.min.x) / rect.width() * 2.0 - 1.0) as f64,
        (1.0 - (at.y - rect.min.y) / rect.height() * 2.0) as f64,
    );
    let aspect = (rect.width() / rect.height()) as f64;
    let direction = view.orbit.ray(view.plane, ndc, crate::map::MAP_FOV as f64, aspect);
    view.plane.intersect(
        view.orbit.eye_ly(view.plane),
        direction,
        map.plane_origin_ly(view.orbit.focus_ly),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::MapFocus;
    use em_map::ItemKey;

    /// The wheel does not break a lock. Zooming toward the pointer moves the focus and
    /// `Action::ZoomMap` drops the lock when it does, so a held center asks for no anchor.
    #[test]
    fn only_a_free_camera_zooms_toward_the_pointer() {
        assert!(zooms_to_cursor(MapFocus::Free));
        assert!(!zooms_to_cursor(MapFocus::Observer), "centered on the ship");
        assert!(!zooms_to_cursor(MapFocus::Item(ItemKey::from_name("Sol"))), "on a body");
    }
}
