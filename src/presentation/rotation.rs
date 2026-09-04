//! Body rotation and axis gizmo systems.

use bevy::prelude::*;
use bevy::color::Srgba;
use crate::body::motive::info::{BodyInfo, BodyRotation, BodyState, RotationMode};
use crate::body::universe::save::ViewSettings;
use crate::sim::SimTime;
use crate::presentation::render_space::{ToRender, ToRenderRotation};

/// Updates body Transform rotations based on their BodyRotation component.
///
/// Computes the current orientation at simulation time, converts from
/// simulation Z-up to Bevy Y-up coordinates, and applies to transform.rotation.
/// For tidally locked bodies, orients them to face their primary.
pub fn orient_bodies(
    mut bodies: Query<(&mut Transform, &BodyRotation, &BodyState)>,
    all_bodies: Query<(&BodyInfo, &BodyState)>,
    sim_time: Res<SimTime>,
) {
    for (mut transform, rotation, state) in bodies.iter_mut() {
        let current_orientation = match &rotation.mode {
            RotationMode::Spinning { .. } => {
                rotation.orientation_at(sim_time.time)
            }
            RotationMode::TidallyLocked { primary_id, .. } => {
                // Find the primary body's position
                let primary_pos = all_bodies.iter()
                    .find(|(info, _)| &info.id == primary_id)
                    .map(|(_, primary_state)| primary_state.current_position);
                
                if let Some(primary_pos) = primary_pos {
                    rotation.orientation_tidally_locked(state.current_position, primary_pos)
                } else {
                    None
                }
            }
        };
        
        if let Some(orientation) = current_orientation {
            transform.rotation = orientation.to_render_rotation();
        }
    }
}

/// Renders debug axis gizmos for each body's rotation.
///
/// Draws:
/// - Red line through the rotation pole (like an "olive spear"), 3x radius above and below
/// - Blue line pointing out of the prime meridian (local +X), 3x radius outward
pub fn render_axes(
    bodies: Query<(&Transform, &BodyRotation)>,
    mut gizmos: Gizmos,
    view_settings: Res<ViewSettings>,
) {
    if !view_settings.show_axes {
        return;
    }

    let pole_color = Srgba::new(1.0, 0.0, 0.0, 1.0); // Red for pole axis
    let meridian_color = Srgba::new(0.0, 0.0, 1.0, 1.0); // Blue for prime meridian

    for (transform, rotation) in bodies.iter() {
        // Use transform values set by position_bodies and orient_bodies
        let center = transform.translation;
        let length = transform.scale.x * 3.0;
        
        // Pole axis is a simulation-space direction; convert through the one funnel
        // rather than open-coding the swizzle, so it cannot drift from the quaternion path.
        let pole_bevy = rotation.pole_axis().to_render().normalize();
        
        // Red pole axis line (through the body)
        let pole_start = center - pole_bevy * length;
        let pole_end = center + pole_bevy * length;
        gizmos.line(pole_start, pole_end, pole_color);
        
        // Green prime meridian line (local +X direction, already in Bevy space via orient_bodies)
        let forward_bevy = transform.rotation * Vec3::X;
        let meridian_end = center + forward_bevy * length;
        gizmos.line(center, meridian_end, meridian_color);
    }
}
