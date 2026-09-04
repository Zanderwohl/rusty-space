//! Star light adjustment system.

use bevy::light::PointLight;
use bevy::prelude::*;
use crate::body::appearance::Appearance;
use crate::body::universe::save::ViewSettings;
use crate::sim::world::{BodyRef, SimSystem};

/// Adjusts star point-light intensity and range based on view scale.
pub fn adjust_lights(
    mut lights: Query<(&BodyRef, &mut PointLight)>,
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
) {
    if !view_settings.is_changed() {
        return;
    }

    let distance_scale = view_settings.distance_factor();

    // Calculate the scaled solar system edge distance (1e14m * distance_scale)
    let scaled_solar_system_edge = 1e14 * distance_scale;
    
    for (body, mut light) in lights.iter_mut() {
        let Some(i) = system.0.index_of(body.0) else { continue };
        match system.0.appearance(i) {
            Appearance::Star(star_ball) => {
                // Set range to reach the scaled solar system edge
                light.range = scaled_solar_system_edge as f32;
                
                // Scale intensity to maintain consistent illumination at the solar system edge
                // Using inverse square law: to maintain same illumination when distance scales by factor S,
                // intensity must scale by S^2
                let intensity_scale_factor = distance_scale * distance_scale;
                light.intensity = star_ball.intensity() * (intensity_scale_factor as f32);
            }
            _ => {}
        }
    }
}
