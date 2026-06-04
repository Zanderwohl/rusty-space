//! Body position mapping system.

use bevy::math::DVec3;
use bevy::prelude::*;
use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::universe::save::ViewSettings;
use crate::camera::{PlanetariumCamera, Freecam};
use crate::sim::SimulationObject;
use crate::util::bevystuff::GlamVec;
use crate::util::mappings;

/// Maps simulation positions (in meters) to Bevy transforms with view scaling.
pub fn position_bodies(
    mut bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo, &BodyState, &Appearance)>,
    camera: Query<&Freecam, With<PlanetariumCamera>>,
    view_settings: Res<ViewSettings>,
) {
    let distance_scale = view_settings.distance_factor();

    let freecam = camera.single().unwrap();

    for (_, mut transform, _, state, appearance) in bodies.iter_mut() {
        let global_position: DVec3 = if !view_settings.logarithmic_distance_scale || state.current_local_position.is_none() || state.current_primary_position.is_none() {
            state.current_position
        } else {
            let local_position = state.current_local_position.unwrap();
            let primary_position = state.current_primary_position.unwrap();
            primary_position + local_position
        };
        transform.translation = global_position.as_bevy_scaled_cheated(distance_scale, freecam.bevy_pos);

        let body_scale = if view_settings.logarithmic_body_scale {
            mappings::log_scale(appearance.radius(), view_settings.logarithmic_body_base) * view_settings.body_scale
        } else {
            appearance.radius() * view_settings.body_scale
        } as f32;
        transform.scale = Vec3::splat(body_scale);
    }
}
