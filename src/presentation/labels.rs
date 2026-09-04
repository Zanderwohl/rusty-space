//! Body label rendering system.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use crate::body::universe::save::ViewSettings;
use crate::camera::PlanetariumCamera;
use crate::sim::world::{BodyRef, SimSystem};

/// Renders body name labels in screen-space via egui.
pub fn label_bodies(
    view_settings: Res<ViewSettings>,
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &Camera3d, &PlanetariumCamera, &GlobalTransform)>,
    bodies: Query<(&BodyRef, &Transform)>,
    system: Res<SimSystem>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("body_labels")));

    for (camera, _, _, camera_transform) in &cameras {
        for (body, transform) in bodies.iter() {
            let Some(i) = system.0.index_of(body.0) else { continue };
            let info = system.0.info(i);
            if !view_settings.show_labels && !view_settings.body_in_any_visible_tag(&info.id) {
                continue;
            }

            let position = transform.translation;
            let view_pos = camera.world_to_viewport(camera_transform, position);
            match view_pos {
                Ok(pos) => {
                    painter.text(
                        egui::pos2(pos.x, pos.y),
                        egui::Align2::CENTER_BOTTOM,
                        info.display_name(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }
                Err(_) => {}
            }
        }
    }
}
