//! Marking what is hovered and what is focused, and pointing at it when it is off screen.
//!
//! The shapes are [`em_ui::reticle`], shared with the other product. What is here is the part
//! that knows an Exotic Matters body from anything else.
//!
//! Edge arrows are new. They could not be built before because [`labels`](super::labels) — and
//! everything else that placed something on screen — goes through `Camera::world_to_viewport`,
//! which **errors** for anything behind the camera. An arrow pointing at what is behind you
//! cannot be built from a function that refuses to answer for the half of the sky the arrow is
//! for. This works in clip space and keeps `w`.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use em_ui::reticle::{self, Frame, Marker};

use crate::body::universe::save::ViewSettings;
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::planetarium::{FocusedBodyState, HoverState};
use crate::presentation::render_space::ToRender;
use crate::sim::world::SimSystem;

/// What the cursor is over.
const HOVER: egui::Color32 = egui::Color32::from_rgb(150, 170, 190);
/// What is focused. The same amber the interface uses for a selection elsewhere.
const FOCUSED: egui::Color32 = egui::Color32::from_rgb(235, 200, 120);

const MIN_RING_PX: f32 = 12.0;
const BRACKET_ARM_PX: f32 = 8.0;
const ARROW_PX: f32 = 11.0;
const LABEL_SIZE: f32 = 12.0;

/// Draw the marks.
///
/// Ordered after the panels in the interface pass: `available_rect` is built up as panels are
/// added, so an overlay that runs first sees the whole window and puts its arrows under them.
pub fn draw_reticle(
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
    hover: Res<HoverState>,
    focused: Res<FocusedBodyState>,
) {
    let Ok(context) = contexts.ctx_mut() else { return };
    let Ok((camera, Projection::Perspective(perspective), camera_at, freecam)) = cameras.single()
    else {
        return;
    };
    let Some(viewport) = camera.logical_viewport_size() else { return };

    let available = context.available_rect();
    let inset = reticle::EDGE_INSET_PX;
    let safe = bevy::math::Rect::from_corners(
        Vec2::new(available.min.x + inset, available.min.y + inset),
        Vec2::new(available.max.x - inset, available.max.y - inset),
    );
    // What the interface has taken. `available_rect` accounts for docked panels and nothing
    // else, so every floating window has to be named here or a mark lands on one. The
    // overlay's own layer is skipped, or it would exclude itself.
    let occupied: Vec<bevy::math::Rect> = context.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .iter()
            .filter(|layer| layer.order < egui::Order::Foreground)
            .filter_map(|layer| memory.area_rect(layer.id))
            .map(|r| {
                bevy::math::Rect::from_corners(
                    Vec2::new(r.min.x, r.min.y),
                    Vec2::new(r.max.x, r.max.y),
                )
            })
            .collect()
    });
    let frame = Frame::with(safe, &occupied, ARROW_PX);

    // Foreground, not background: panels paint in `Order::Background` and floating windows in
    // `Order::Middle`, so a mark on either is covered by whatever happens to be open. Keeping
    // the marks inside `available` is what stops this from drawing over a docked panel.
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("em_reticle"),
    ));

    let camera_gt = GlobalTransform::from(*camera_at);
    let view_from_world = camera_gt.to_matrix().inverse();
    let clip_from_view = {
        use bevy::camera::CameraProjection;
        perspective.get_clip_from_view()
    };
    let rad_per_px = 2.0 * (perspective.fov * 0.5).tan() / viewport.y.max(1.0);
    let distance_scale = view_settings.distance_factor();

    let mark = |id: &str, colour: egui::Color32, bracketed: bool| {
        let Some(index) = system.0.by_name(id) else { return };
        let at = system.0.position(index).to_render_relative(distance_scale, freecam.bevy_pos);
        // Clip space, not a viewport position: the whole point is to be able to say where
        // something behind the camera is.
        let clip: Vec4 = clip_from_view * (view_from_world * at.extend(1.0));
        let distance = at.distance(camera_at.translation);
        let radius_px = if distance > 0.0 {
            (view_settings.body_scale_factor(system.0.radius(index)) / distance) / rad_per_px
        } else {
            0.0
        };
        paint(&painter, clip, radius_px, id, viewport, frame, colour, bracketed);
    };

    if let Some(id) = &focused.current_body_id {
        mark(id, FOCUSED, true);
    }
    if let Some(id) = &hover.hovered_body_id
        && Some(id) != focused.current_body_id.as_ref()
    {
        mark(id, HOVER, false);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    painter: &egui::Painter,
    clip: Vec4,
    radius_px: f32,
    label: &str,
    viewport: Vec2,
    frame: Frame<'_>,
    colour: egui::Color32,
    bracketed: bool,
) {
    let stroke = egui::Stroke::new(1.0_f32, colour);
    let (anchor, reach, preferred, name_it) = match reticle::place(clip, radius_px, viewport, frame)
    {
        Marker::On { at, radius_px } => {
            let radius = radius_px.max(MIN_RING_PX);
            let segments = if bracketed {
                reticle::brackets(at, radius, BRACKET_ARM_PX)
            } else {
                reticle::ring(at, radius, 40)
            };
            draw(painter, &segments, stroke);
            // `labels` already names everything on screen; a second copy would double-print.
            (at, radius, None, false)
        }
        Marker::Off { at, direction } => {
            draw(painter, &reticle::arrow(at, direction, ARROW_PX), stroke);
            // Off screen it has no label from anywhere else, and an unlabelled arrow says
            // only that *something* is out there.
            (at, ARROW_PX, Some(-direction), true)
        }
    };

    if !name_it {
        return;
    }
    // Measured before it is placed, because where it fits depends on how wide it is.
    let galley = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(LABEL_SIZE), colour);
    let size = Vec2::new(galley.size().x, galley.size().y);
    let centre = reticle::place_label(anchor, reach, size, frame, reticle::LABEL_GAP_PX, preferred);
    painter.galley(egui::pos2(centre.x - size.x * 0.5, centre.y - size.y * 0.5), galley, colour);
}

fn draw(painter: &egui::Painter, segments: &[[Vec2; 2]], stroke: egui::Stroke) {
    for [a, b] in segments {
        painter.line_segment([egui::pos2(a.x, a.y), egui::pos2(b.x, b.y)], stroke);
    }
}
