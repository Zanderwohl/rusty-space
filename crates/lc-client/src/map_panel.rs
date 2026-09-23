//! The two surfaces the map is shown on, and the input they take.
//!
//! The map is a **mode of the main view**, not a window over it: it fills the screen under the
//! readout, with the world in the corner square, and the same square shows the map while the
//! world is the one being flown. A click on the square swaps them. Windows float over either.
//!
//! The image is allocated with an explicit [`egui::Sense`] rather than through `ui.image`, so
//! a drag on it belongs to the map.

use bevy::camera::Viewport;
use bevy::math::URect;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use glam::DVec3;
use bevy_egui::{EguiContexts, egui};

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::map::Map;
#[cfg(feature = "godview")]
use crate::map_source::Source;
use crate::panels::ask;
use crate::ui::ViewMode;

/// Radians of turn per point of drag. The same feel as the sky's own look.
const TURN_PER_POINT: f64 = 0.006;

/// A pan drag across the whole viewport moves the focus by this much of the stand-off.
const PAN_PER_VIEWPORT: f64 = 1.2;

/// How much of the surface the scale rule may take, and the least it is worth drawing at. A
/// fraction to stay proportionate in the corner square, and a ceiling so it does not stretch
/// across a whole screen.
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
const LABEL_CLEARANCE_PX: f32 = 6.0;

/// The corner square's side and its inset from the bottom left, in points. The event log
/// takes the bottom right with the same inset (`panels.rs`).
const CORNER_SIDE: f32 = 190.0;
const CORNER_INSET: f32 = 12.0;

/// The whole of a texture.
const WHOLE_TEXTURE: egui::Rect =
    egui::Rect { min: egui::pos2(0.0, 0.0), max: egui::pos2(1.0, 1.0) };

/// Where the world's camera draws, in physical pixels, or the whole window when the world is
/// what the screen is showing.
///
/// Written by the egui pass and spent by [`frame_world`] on the next frame's scene stage. The
/// square is laid out in points, and egui is the only thing here that knows what a point is
/// worth.
#[derive(Resource, Default)]
pub struct WorldInset(pub Option<URect>);

/// Draw the map on whichever surface it has, and turn pointer input on it into actions.
///
/// Both modes end in the same three calls: one rule, one set of names, one reading of the
/// pointer. A map that answered a drag differently depending on how big it was drawn would be
/// two maps.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    game: Res<Game>,
    mut map: ResMut<Map>,
    foot: Res<crate::panels::HudFoot>,
    mut world: ResMut<WorldInset>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mode = ui_state.view;
    let per_point = ctx.pixels_per_point();
    let square = corner(ctx.viewport_rect());
    map.shown = true;

    // What the world's camera is to draw into, which is the square it is the thumbnail in.
    world.0 = (mode == ViewMode::Map).then(|| pixels(square, per_point));

    // The square holds whichever mode is not in force, and a click swaps them.
    let swap = square_area(ctx, square, (mode == ViewMode::World).then_some(&*map));
    if swap.clicked() {
        ask(&mut out, Action::SetView(mode.other()));
    }

    let (rect, response) = match mode {
        ViewMode::Map => whole(ctx, foot.0, &ui_state, &game, &map, square, &mut out),
        ViewMode::World => (square, swap.clone()),
    };
    map.wanted = pixels(rect, per_point).size().max(UVec2::ONE);

    let painter = ctx.layer_painter(response.layer_id);
    // The square is the map's own, and in world mode it is the surface itself.
    let over = crate::pick::occupied_rects(ctx, &[corner_id()]);
    let hole = match mode {
        ViewMode::Map => square,
        ViewMode::World => egui::Rect::NOTHING,
    };
    // Before the names, because a mark carries its own and the layout has to leave that one
    // out. Nothing is picked off the corner square: 190 points is a thumbnail, not a surface.
    let picked = match mode {
        ViewMode::Map => crate::map_pick::survey(&response, rect, &ui_state, &map, &mut out),
        ViewMode::World => crate::map_pick::Picked::default(),
    };
    scale_rule(&painter, rect, ui_state.map, &over);
    labels(&painter, rect, hole, ui_state.map, &map, &picked.named);
    crate::map_pick::draw(&painter, rect, hole, &over, &picked);
    read_input(ctx, &response, rect, ViewMode::Map, ui_state.map, &map, &mut out);
    if mode == ViewMode::Map {
        // The square is showing the world, so it answers the world's own gesture.
        read_input(ctx, &swap, square, ViewMode::World, ui_state.map, &map, &mut out);
    }
}

/// Put the world's camera in the corner square, or give it the window back.
///
/// Clamped to the window: the square was laid out against a rect a frame old, and a viewport
/// reaching past the surface is a wgpu validation failure rather than a clipped picture.
pub fn frame_world(
    inset: Res<WorldInset>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Camera, With<crate::app::SkyCamera>>,
) {
    let wanted = inset.0.filter(|rect| !rect.is_empty()).map(|rect| {
        let mut viewport = Viewport {
            physical_position: rect.min,
            physical_size: rect.size(),
            ..default()
        };
        viewport.clamp_to_size(UVec2::new(window.physical_width(), window.physical_height()));
        viewport
    });
    // Written only when it moves. A camera marked changed every frame is a frame's worth of
    // render-world work for a viewport that did not budge.
    let same = |a: &Viewport, b: &Viewport| {
        a.physical_position == b.physical_position && a.physical_size == b.physical_size
    };
    let settled = match (&camera.viewport, &wanted) {
        (None, None) => true,
        (Some(held), Some(wanted)) => same(held, wanted),
        _ => false,
    };
    if !settled {
        camera.viewport = wanted;
    }
}

/// Give the window back on leaving the sky.
///
/// The browser build can be stranded from in game, and nothing outside the sky writes an
/// inset — so without this the world's camera would still be drawing into a corner of a screen
/// the map is no longer on.
pub fn release_world_frame(
    mut inset: ResMut<WorldInset>,
    mut camera: Query<&mut Camera, With<crate::app::SkyCamera>>,
) {
    inset.0 = None;
    if let Ok(mut camera) = camera.single_mut() {
        camera.viewport = None;
    }
}

/// The names, over the image.
///
/// egui text rather than geometry on the layer, per `18-ui-style.md`. The work is in two
/// halves: this measures what each name would occupy, and [`em_map::label::lay_out`] decides
/// which of them fit.
fn labels(
    painter: &egui::Painter,
    rect: egui::Rect,
    hole: egui::Rect,
    view: crate::ui::MapView,
    map: &Map,
    named: &[em_map::ItemKey],
) {
    let Some(frame) = map.frame.as_ref() else { return };
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let aspect = (rect.width() / rect.height()) as f64;
    let font = painter.ctx().style_of(egui::Theme::Dark).text_styles[&egui::TextStyle::Small]
        .clone();

    let mut candidates = Vec::with_capacity(frame.placements.len());
    // Keyed, because the layout hands back whichever names fit in whatever order it settled
    // them, and matching them up by scanning made that quadratic in the number of names.
    let mut galleys = std::collections::HashMap::with_capacity(frame.placements.len());
    for placement in &frame.placements {
        // A mark names what it is on; naming it here too would write it twice.
        if placement.label.is_empty() || named.contains(&placement.key) {
            continue;
        }
        let Some(ndc) =
            view.orbit.project(view.datum(), placement.at.as_dvec3(), crate::map::MAP_FOV as f64,
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
            first: placement.kind == em_map::ItemKind::Observer,
            // egui counts pixels down from the top left; NDC runs up from the middle.
            at: glam::Vec2::new(
                (ndc.x as f32 + 1.0) * 0.5 * rect.width(),
                (1.0 - ndc.y as f32) * 0.5 * rect.height(),
            ),
            size: glam::Vec2::new(size.x, size.y),
        });
        galleys.insert(placement.key, galley);
    }

    // Clear of the mark and centered on it. The mark is sized against the texture and the
    // text against the surface showing it, which differ on a display that scales.
    let points_per_pixel = map.points_per_pixel(rect.height());
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
        let Some(galley) = galleys.get(&placed.key) else {
            continue;
        };
        let at = rect.min + egui::vec2(placed.at.x, placed.at.y);
        // Not over the corner square. Nothing else of the map is drawn there, and a name alone
        // on the world reads as a name for the world.
        if egui::Rect::from_min_size(at, galley.size()).intersects(hole) {
            continue;
        }
        painter.galley(at, galley.clone(), egui::Color32::WHITE);
    }
}

/// Amber for a craft, this ship included, and the interface's text color for everything else.
/// The palette has two phosphors and no white.
///
/// Not the mark's color for a body: those take the palette's dimmer greens, which are too dim
/// to read text at.
fn label_color(kind: em_map::ItemKind) -> egui::Color32 {
    match kind {
        em_map::ItemKind::Ship | em_map::ItemKind::Station | em_map::ItemKind::Observer => {
            color_of(em_ui::vfd::AMBER)
        }
        _ => color_of(em_ui::vfd::TEXT),
    }
}

/// The map as the main view: a strip of controls under the readout, and the image under that.
///
/// In the background layer, which is what puts every window over it, and with a hole left
/// where the world's camera is drawing.
fn whole(
    ctx: &egui::Context,
    foot: f32,
    ui_state: &Ui,
    game: &Game,
    map: &Map,
    hole: egui::Rect,
    out: &mut MessageWriter<Requested>,
) -> (egui::Rect, egui::Response) {
    let mut under = ctx.viewport_rect();
    under.min.y = foot.clamp(under.min.y, under.max.y);
    let mut root = egui::Ui::new(
        ctx.clone(),
        "map mode".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(under),
    );
    egui::Panel::top("map controls")
        .show(&mut root, |ui| controls(ui, ui_state, game, map.primary, out));

    let rect = root.available_rect_before_wrap();
    let response = root.allocate_rect(rect, egui::Sense::click_and_drag());
    if let Some(texture) = map.texture {
        for piece in around(rect, hole) {
            root.painter().image(texture, piece, uv(rect, piece), egui::Color32::WHITE);
        }
    }
    (rect, response)
}

/// The corner square: the mode that is not in force, and the way into it.
///
/// It paints no image in map mode, because the world's own camera is drawing into exactly this
/// square and anything painted here would be painted over it.
///
/// The cost is real and worth stating. A drag begun on the square belongs to it wherever the
/// cursor goes afterwards, so a right-press begun in the corner turns the map instead of the
/// view. Every panel in the interface costs this; the square is the one never closed.
fn square_area(ctx: &egui::Context, square: egui::Rect, map: Option<&Map>) -> egui::Response {
    egui::Area::new(corner_id())
        // Middle, not Foreground: an open window has to cover this.
        .order(egui::Order::Middle)
        .fixed_pos(square.min)
        .show(ctx, |ui| {
            let response = ui.allocate_rect(square, egui::Sense::click_and_drag());
            if let Some(texture) = map.and_then(|map| map.texture) {
                ui.painter().image(texture, square, WHOLE_TEXTURE, egui::Color32::WHITE);
            }
            // A border, so the world in the corner reads as a frame within a frame rather than
            // as the map failing to draw there.
            ui.painter().rect_stroke(
                square,
                0.0,
                egui::Stroke::new(1.0, color_of(em_ui::vfd::TEXT_DIM)),
                egui::StrokeKind::Inside,
            );
            response
        })
        .inner
}

fn corner_id() -> egui::Id {
    egui::Id::new("map corner")
}

/// Where the square sits, in points: the same place in both modes, so the click that swaps
/// them does not move out from under the cursor that took it.
///
/// Shrunk to fit a window too small to hold it. Nothing here may leave the window: the world's
/// viewport is cut from this.
fn corner(viewport: egui::Rect) -> egui::Rect {
    let side = CORNER_SIDE
        .min(viewport.width() - 2.0 * CORNER_INSET)
        .min(viewport.height() - 2.0 * CORNER_INSET)
        .max(0.0);
    // The inset is clamped as well as the side: a window smaller than the inset itself would
    // otherwise put the square above the top of it.
    let min = egui::pos2(
        (viewport.min.x + CORNER_INSET).min(viewport.max.x),
        (viewport.max.y - CORNER_INSET - side).max(viewport.min.y),
    );
    egui::Rect::from_min_size(min, egui::vec2(side, side))
}

/// The pieces of `rect` left over around `hole`: one above, one below, and one to each side.
///
/// egui clips to rectangles, and a rectangle with a hole in it is not one. Empty pieces are
/// dropped, so a hole that touches nothing gives back the whole of `rect`.
fn around(rect: egui::Rect, hole: egui::Rect) -> Vec<egui::Rect> {
    let hole = hole.intersect(rect);
    if !hole.is_positive() {
        return vec![rect];
    }
    let mut pieces = Vec::with_capacity(4);
    for piece in [
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, hole.min.y)),
        egui::Rect::from_min_max(egui::pos2(rect.min.x, hole.max.y), rect.max),
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x, hole.min.y),
            egui::pos2(hole.min.x, hole.max.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(hole.max.x, hole.min.y),
            egui::pos2(rect.max.x, hole.max.y),
        ),
    ] {
        if piece.is_positive() {
            pieces.push(piece);
        }
    }
    pieces
}

/// Which part of the texture a piece of the surface shows. The image is drawn for the whole
/// surface, so a piece of it takes the matching piece of the texture.
fn uv(rect: egui::Rect, piece: egui::Rect) -> egui::Rect {
    let at = |p: egui::Pos2| {
        egui::pos2(
            (p.x - rect.min.x) / rect.width().max(f32::MIN_POSITIVE),
            (p.y - rect.min.y) / rect.height().max(f32::MIN_POSITIVE),
        )
    };
    egui::Rect::from_min_max(at(piece.min), at(piece.max))
}

/// A rect of the surface as one of the window, which is what a render target and a camera's
/// viewport are measured in.
fn pixels(rect: egui::Rect, per_point: f32) -> URect {
    let px = |v: f32| (v * per_point).round().max(0.0) as u32;
    let at = |p: egui::Pos2| UVec2::new(px(p.x), px(p.y));
    URect::from_corners(at(rect.min), at(rect.max))
}

/// Draw the scale rule into the bottom right of the map's surface.
fn scale_rule(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: crate::ui::MapView,
    over: &[egui::Rect],
) {
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
    let left = right - length;
    let y = rule_row(rect, left..right, over);
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

/// Which row the rule is drawn on: the bottom of the surface, lifted clear of anything
/// floating across it.
///
/// The events box takes the same corner with the same inset, and a scale rule under it is a
/// scale rule nobody can read. Only boxes that cross the row it would take count, and the lift
/// clears the highest of them.
fn rule_row(rect: egui::Rect, span: std::ops::Range<f32>, over: &[egui::Rect]) -> f32 {
    let row = rect.max.y - RULE_INSET.y;
    let lifted = over
        .iter()
        // The background layer is one of these and it is the whole window. A box covering the
        // surface is not floating over it; it is what the surface is drawn on.
        .filter(|box_| !box_.contains_rect(rect))
        .filter(|box_| box_.min.x < span.end && box_.max.x > span.start)
        .filter(|box_| box_.min.y <= row && box_.max.y >= row - RULE_CAP_PX)
        .fold(row, |row, box_| row.min(box_.min.y - RULE_INSET.y));
    // Nowhere to go is a reason to stay put. On the corner square a covered rule is better
    // than one drawn above the surface it measures.
    match lifted > rect.min.y + RULE_CAP_PX {
        true => lifted,
        false => row,
    }
}

/// How far a point on the surface reaches, in meters, where the rule is drawn. A ray cast at
/// the rule's own height meets the plane, and the scale is taken at that depth; when it meets
/// nothing the stand-off is the only answer left.
/// Why a plane cannot be laid rings in, or `None` when it can.
///
/// Galactic always can: it is the same frame everywhere and needs nothing solved. A system's
/// plane is a belief, and the two ways of not having one are worth distinguishing — nothing
/// observed at all reads differently from "watched from one place and it is not enough", which
/// is a hint about what to do next rather than a refusal.
fn unsolved(plane: em_map::Plane, believed: lc_world::knowledge::SystemPlane) -> Option<&'static str> {
    use lc_world::knowledge::SystemPlane;
    if plane != em_map::Plane::System {
        return None;
    }
    match believed {
        SystemPlane::Known { .. } => None,
        SystemPlane::Circle(_) => {
            Some("the transits seen from here put this system's pole somewhere on a circle; \
                  another craft watching from elsewhere would cross it")
        }
        SystemPlane::Unknown => Some("no orbit solved in this system yet"),
    }
}

fn meters_per_point(rect: egui::Rect, view: crate::ui::MapView) -> Option<f32> {
    if rect.height() <= 0.0 {
        return None;
    }
    let rad_per_point = 2.0 * (crate::map::MAP_FOV * 0.5).tan() / rect.height();
    let (forward, ..) = view.orbit.view_basis(view.datum());
    let eye = view.orbit.eye_ly(view.datum());
    let aspect = (rect.width() / rect.height()) as f64;
    let direction =
        view.orbit.ray(view.datum(), glam::DVec2::new(0.0, RULE_SAMPLE_NDC_Y),
            crate::map::MAP_FOV as f64, aspect);
    let depth_m = match view.datum().intersect(eye, direction, view.orbit.focus_ly) {
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

/// The map's own controls, in one strip under the readout.
///
/// One row and not three. It is a strip rather than a panel of settings: everything on it says
/// what the map is a map *of*, and a reader glances at it rather than working down it.
fn controls(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    primary: Option<em_map::ItemKey>,
    out: &mut MessageWriter<Requested>,
) {
    ui.horizontal(|ui| {
        for plane in [em_map::Plane::System, em_map::Plane::Galactic] {
            // A system's plane is something this craft solved, so it can be missing. Grayed
            // and saying why rather than absent, the same way a refused source is: the option
            // exists, and what is lacking is the observation.
            if let Some(why) = unsolved(plane, state.map.system_plane) {
                ui.weak(plane.label()).on_hover_text(why);
                continue;
            }
            if ui.selectable_label(state.map.plane == plane, plane.label()).clicked() {
                ask(out, Action::SetMapPlane(plane));
            }
        }

        #[cfg(feature = "godview")]
        {
            ui.separator();
            for source in [Source::Observed, Source::God] {
                let allowed = source == Source::Observed || state.may_see_everything;
                let chosen = state.map.source == source;
                // A plain selectable label: `add_enabled` around one reports clicks nobody
                // made. A refused source is grayed and says why, because color is not the only
                // signal.
                if !allowed {
                    ui.weak(source.label()).on_hover_text("needs an administrative account");
                    continue;
                }
                if ui.selectable_label(chosen, source.label()).clicked() {
                    ask(out, Action::SetMapSource(source));
                }
            }
            // Light delay is the premise everywhere in the client; the picture taken without it
            // is the one that has to say so.
            if state.map.source == Source::God {
                ui.weak("coordinate time, no light delay — other ships are not drawn");
            }
        }

        ui.separator();
        if ui.button("center on the ship").clicked() {
            ask(out, Action::FocusMap(crate::ui::MapFocus::Observer));
        }
        // Whatever holds the ship: a moon's planet, a planet's star. A mode, so it follows the
        // ship out of one sphere of influence and into the next.
        //
        // A pair rather than a button, because which frame is in force is a thing to read off
        // the strip: the same two bodies at the same zoom look alike, and only one of them is
        // turning with the ship.
        match primary.is_some() {
            true => {
                ui.label("center on the primary");
                for frame in [crate::ui::Frame::Fixed, crate::ui::Frame::Local] {
                    let held = state.map.focus == crate::ui::MapFocus::Primary(frame);
                    if ui.selectable_label(held, frame.label()).clicked() {
                        ask(out, Action::FocusMap(crate::ui::MapFocus::Primary(frame)));
                    }
                }
            }
            false => {
                ui.weak("center on the primary")
                    .on_hover_text("there is nothing here holding the ship");
            }
        }
        // "the star", not its name: a button whose label changes between systems has to be
        // read before it is pressed. The name is on the star itself.
        let star = game.0.system.as_ref().map(|system| system.star);
        if centering(ui, "center on the star", star.is_some())
            && let Some(star) = star
        {
            ask(out, Action::FocusMap(crate::ui::MapFocus::Item(
                em_map::ItemKey::from_id("star", star.get()),
            )));
        }
    });
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    /// The map's camera about its focus.
    Turn,
    /// The map's focus across the reference plane.
    Pan,
    /// The ship's own view, as a drag over the sky would.
    Look,
}

/// What a drag does on a surface showing `shown`, with the look button down or not.
///
/// **The right button turns whichever view is under it.** It is the sky's look button, and one
/// button meaning opposite things on the two modes of one screen is worse than either meaning —
/// so it turns the map over the map, and the ship's view over the world's corner square.
///
/// The button left over pans the map. Over the world it does nothing: there the left button
/// belongs to picking, and the corner is too small to pick in.
fn drag_of(shown: ViewMode, right: bool) -> Option<Drag> {
    match (shown, right) {
        (ViewMode::Map, true) => Some(Drag::Turn),
        (ViewMode::Map, false) => Some(Drag::Pan),
        (ViewMode::World, true) => Some(Drag::Look),
        (ViewMode::World, false) => None,
    }
}

/// One of the center buttons, grayed where there is nothing to center on.
///
/// Grayed rather than hidden: between the stars there is neither a primary nor a star, and a
/// control that vanishes takes the strip's other buttons out from under the cursor with it.
fn centering(ui: &mut egui::Ui, label: &str, exists: bool) -> bool {
    if !exists {
        ui.weak(label).on_hover_text("there is nothing here holding the ship");
        return false;
    }
    ui.button(label).clicked()
}

/// Drag and wheel over a surface, as actions.
///
/// A right-press that starts on either surface belongs to it wherever the cursor goes
/// afterwards, and `grab_cursor` stands down while egui wants the pointer — which is what lets
/// the corner square answer the look button while the map holds the rest of the screen.
fn read_input(
    ctx: &egui::Context,
    response: &egui::Response,
    rect: egui::Rect,
    shown: ViewMode,
    view: crate::ui::MapView,
    map: &Map,
    out: &mut MessageWriter<Requested>,
) {
    if response.dragged() {
        let delta = response.drag_delta();
        match drag_of(shown, response.dragged_by(egui::PointerButton::Secondary)) {
            Some(Drag::Pan) => {
                let span = rect.width().max(rect.height()).max(1.0) as f64;
                ask(out, Action::PanMap {
                    right: -delta.x as f64 / span * PAN_PER_VIEWPORT,
                    ahead: delta.y as f64 / span * PAN_PER_VIEWPORT,
                });
            }
            Some(Drag::Turn) => ask(out, Action::TurnMap {
                azimuth: -delta.x as f64 * TURN_PER_POINT,
                elevation: delta.y as f64 * TURN_PER_POINT,
            }),
            Some(Drag::Look) => {
                let (yaw, pitch) = crate::input::look_from(Vec2::new(delta.x, delta.y));
                ask(out, Action::Look { yaw, pitch });
            }
            None => {}
        }
    }
    // The wheel is the map's. Over the world's corner it is left to the ship, which is not
    // being flown from here.
    if shown == ViewMode::Map {
        read_wheel_only(ctx, response, rect, view, map, out);
    }
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
    let direction = view.orbit.ray(view.datum(), ndc, crate::map::MAP_FOV as f64, aspect);
    view.datum().intersect(
        view.orbit.eye_ly(view.datum()),
        direction,
        map.plane_origin_ly(view.orbit.focus_ly),
    )
}

#[cfg(test)]
mod tests {

    /// **Galactic is always offered; a system's plane has to have been solved.** And the two
    /// ways of not having one read differently, because one of them is a hint.
    #[test]
    fn the_system_plane_is_refused_until_it_is_solved() {
        use lc_world::knowledge::SystemPlane;
        let known = SystemPlane::Known { pole: glam::DVec3::Z, sigma_rad: 0.01, zero: glam::DVec3::X };

        assert_eq!(unsolved(em_map::Plane::System, known), None);
        assert!(unsolved(em_map::Plane::System, SystemPlane::Unknown).is_some());
        let circle = unsolved(em_map::Plane::System, SystemPlane::Circle(glam::DVec3::X));
        assert!(circle.is_some());
        assert_ne!(circle, unsolved(em_map::Plane::System, SystemPlane::Unknown), "one reason for both");

        for belief in [known, SystemPlane::Unknown, SystemPlane::Circle(glam::DVec3::X)] {
            assert_eq!(unsolved(em_map::Plane::Galactic, belief), None, "galactic needs nothing");
        }
    }
    use super::*;
    use crate::ui::MapFocus;
    use em_map::ItemKey;

    fn window() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0))
    }

    /// The square is the click that swaps the modes, so it is in one place and it is the
    /// bottom left corner.
    #[test]
    fn the_square_sits_in_the_corner_at_its_stated_size() {
        let square = corner(window());
        assert_eq!(square.width(), CORNER_SIDE);
        assert_eq!(square.height(), CORNER_SIDE);
        assert_eq!(square.min.x, CORNER_INSET);
        assert_eq!(square.max.y, 720.0 - CORNER_INSET);
    }

    /// **Nothing may leave the window**: the world's own viewport is cut from this, and one
    /// reaching past the surface is a wgpu validation failure rather than a clipped picture.
    #[test]
    fn a_window_too_small_shrinks_the_square_rather_than_spilling_out_of_it() {
        for size in [egui::vec2(120.0, 90.0), egui::vec2(20.0, 400.0), egui::vec2(1.0, 1.0)] {
            let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size);
            let square = corner(viewport);
            assert!(square.width() >= 0.0 && square.height() >= 0.0, "{size:?}");
            assert!(viewport.contains_rect(square), "{size:?} let the square out at {square:?}");
        }
    }

    /// The pieces are the surface less the hole: all of it, once each, and none of it over the
    /// square the world is drawing into.
    #[test]
    fn the_pieces_cover_everything_but_the_hole() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 30.0), egui::vec2(800.0, 500.0));
        let hole = corner(rect);
        let pieces = around(rect, hole);
        let area = |r: egui::Rect| r.width() * r.height();
        let covered: f32 = pieces.iter().copied().map(area).sum();
        assert!((covered - (area(rect) - area(hole))).abs() < 0.01, "{covered} covered");
        for piece in &pieces {
            assert!(rect.contains_rect(*piece), "{piece:?} is outside the surface");
            assert!(!piece.intersect(hole).is_positive(), "{piece:?} paints over the world");
        }
    }

    /// A hole outside the surface takes nothing from it, and one covering it leaves nothing.
    #[test]
    fn a_hole_that_touches_nothing_leaves_the_surface_whole() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0));
        let away = egui::Rect::from_min_size(egui::pos2(500.0, 500.0), egui::vec2(20.0, 20.0));
        assert_eq!(around(rect, away), vec![rect]);
        assert_eq!(around(rect, egui::Rect::NOTHING), vec![rect]);
        assert!(around(rect, rect).is_empty(), "a hole the size of the surface leaves none");
    }

    /// The rule goes under the events box otherwise, which takes the same corner with the same
    /// inset — and a scale rule nobody can read is not one.
    #[test]
    fn the_rule_is_lifted_clear_of_what_floats_over_it() {
        // The surface as the map's mode leaves it: the window, less the two strips at the top.
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 61.0), egui::pos2(1280.0, 720.0));
        let span = 885.0..1268.0;
        let bottom = rect.max.y - RULE_INSET.y;
        assert_eq!(rule_row(rect, span.clone(), &[]), bottom, "nothing in the way");

        let elsewhere = egui::Rect::from_min_size(egui::pos2(0.0, 620.0), egui::vec2(200.0, 100.0));
        assert_eq!(rule_row(rect, span.clone(), &[elsewhere]), bottom, "not in these columns");

        let events = egui::Rect::from_min_size(egui::pos2(954.0, 654.0), egui::vec2(314.0, 54.0));
        assert!(rule_row(rect, span.clone(), &[events]) < events.min.y, "still under the box");

        // **The background layer is one of these and it is the whole window.** Counted, it
        // reaches past the top of the surface, the lift has nowhere to go, and the rule stays
        // under the events box — which is what it did.
        let window = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0));
        let lifted = rule_row(rect, span.clone(), &[window, events]);
        assert!(lifted < events.min.y, "the whole window counted as a box over the surface");
        assert_eq!(rule_row(rect, span.clone(), &[window]), bottom, "and on its own it is not");

        // A box down the whole right-hand side, which leaves the rule nowhere to go.
        let tall = egui::Rect::from_min_max(egui::pos2(900.0, 61.0), egui::pos2(1280.0, 720.0));
        assert_eq!(rule_row(rect, span, &[tall]), bottom, "nowhere to go is a reason to stay");
    }

    /// A piece of the surface shows the matching piece of the texture. Get this wrong and the
    /// map is drawn four times over, once per piece.
    #[test]
    fn a_piece_takes_the_matching_piece_of_the_texture() {
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 20.0), egui::vec2(200.0, 100.0));
        assert_eq!(uv(rect, rect), WHOLE_TEXTURE);
        let quarter = egui::Rect::from_min_size(rect.min, rect.size() * 0.5);
        let taken = uv(rect, quarter);
        assert_eq!(taken.min, egui::pos2(0.0, 0.0));
        assert_eq!(taken.max, egui::pos2(0.5, 0.5));
    }

    /// **The look button means the same thing on both modes of the screen.** Both halves are
    /// pinned here, because the pair is the claim: move the sky's look to another button and
    /// this says so rather than leaving the map turning on the one the sky no longer uses.
    #[test]
    fn the_look_button_turns_whichever_view_is_under_it() {
        assert_eq!(crate::input::LOOK_BUTTON, bevy::input::mouse::MouseButton::Right);
        assert_eq!(drag_of(ViewMode::Map, true), Some(Drag::Turn), "egui's secondary");
        assert_eq!(drag_of(ViewMode::World, true), Some(Drag::Look), "the same one, over a ship");
        assert_eq!(drag_of(ViewMode::Map, false), Some(Drag::Pan));
        assert_eq!(drag_of(ViewMode::World, false), None, "the left button is picking's");
    }

    /// Every name is written in a palette color, this ship's included. Two phosphors, and no
    /// white: a craft is amber and everything else is the interface's own green.
    #[test]
    fn every_name_is_written_in_the_palette() {
        for kind in [em_map::ItemKind::Ship, em_map::ItemKind::Station, em_map::ItemKind::Observer]
        {
            assert_eq!(label_color(kind), color_of(em_ui::vfd::AMBER), "{kind:?}");
        }
        assert_eq!(label_color(em_map::ItemKind::Planet), color_of(em_ui::vfd::TEXT));
    }

    /// The wheel does not break a lock. Zooming toward the pointer moves the focus and
    /// `Action::ZoomMap` drops the lock when it does, so a held center asks for no anchor.
    #[test]
    fn only_a_free_camera_zooms_toward_the_pointer() {
        assert!(zooms_to_cursor(MapFocus::Free));
        assert!(!zooms_to_cursor(MapFocus::Observer), "centered on the ship");
        assert!(!zooms_to_cursor(MapFocus::Item(ItemKey::from_name("Sol"))), "on a body");
        assert!(
            !zooms_to_cursor(MapFocus::Primary(crate::ui::Frame::Fixed)),
            "on whatever holds the ship",
        );
    }
}
