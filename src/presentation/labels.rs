//! Body label rendering system.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use crate::body::universe::save::ViewSettings;
use crate::camera::PlanetariumCamera;
use crate::gui::planetarium::{FocusedBodyState, HoverState};
use crate::sim::world::{BodyRef, SimSystem};
use crate::sim::SimulationObject;

/// Renders body name labels in screen-space via egui.
/// Labels are positioned above the body in screen space to avoid floating-point
/// precision issues with nearby bodies.
pub fn label_bodies(
    view_settings: Res<ViewSettings>,
    focused_body_state: Res<FocusedBodyState>,
    hover_state: Res<HoverState>,
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &Camera3d, &PlanetariumCamera, &Projection, &Transform), Without<SimulationObject>>,
    bodies: Query<(&BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("body_labels")));

    for (camera, _, _, projection, camera_transform) in &cameras {
        // Get viewport size for angular size calculations
        let Some(viewport_size) = camera.logical_viewport_size() else {
            continue;
        };

        // Get vertical FOV for perspective projection
        let fov_y = match projection {
            Projection::Perspective(persp) => persp.fov,
            _ => 1.0, // fallback for orthographic/custom
        };

        for (body, transform) in bodies.iter() {
            let Some(index) = system.0.index_of(body.0) else { continue };
            let body_info = system.0.info(index);
            let selected_match = focused_body_state.is_focused(&body_info.id);
            let should_show = view_settings.show_labels
                || view_settings.body_in_any_visible_tag(&body_info.id)
                || (selected_match && view_settings.show_selected_labels)
                || hover_state.is_body_hovered(&body_info.id);
            if !should_show {
                continue;
            }

            // Project body center to screen space using the camera's current-frame
            // Transform, not the stale GlobalTransform (only propagated in PostUpdate).
            let body_center = transform.translation;
            let fresh_camera_gt = GlobalTransform::from(*camera_transform);
            let Ok(center_screen) = camera.world_to_viewport(&fresh_camera_gt, body_center) else {
                continue;
            };

            // Calculate distance from camera to body
            let camera_pos = fresh_camera_gt.translation();
            let distance = (body_center - camera_pos).length();

            // Calculate projected radius in pixels using angular size
            // angular_size = 2 * atan(radius / distance)
            // For small angles: angular_size ≈ radius / distance
            // Screen pixels = angular_size / fov_y * viewport_height
            let radius = transform.scale.x;
            let angular_radius = (radius / distance).min(1.0); // clamp to avoid issues at very close range
            let screen_radius = angular_radius / (fov_y * 0.5) * viewport_size.y * 0.5;

            // Position label 1.2 radii above body center in screen space
            let screen_offset = screen_radius * 1.2;
            let label_pos = egui::pos2(center_screen.x, center_screen.y - screen_offset);

            painter.text(
                label_pos,
                egui::Align2::CENTER_BOTTOM,
                body_info.display_name(),
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(90, 237, 175),
            );
        }
    }
}
