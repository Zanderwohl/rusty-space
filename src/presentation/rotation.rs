//! Body rotation and axis gizmo systems.

use bevy::prelude::*;
use bevy::color::Srgba;
use crate::body::universe::save::ViewSettings;
use crate::presentation::render_space::ToRender;
use crate::sim::world::{BodyRef, SimSystem};

// Orientation itself is applied by `sim::world::sync_rotations`, which reads the arena.
// What remains here is the debug gizmo.

/// Renders debug axis gizmos for each body's rotation.
///
/// Draws:
/// - Red line through the rotation pole (like an "olive spear"), 3x radius above and below
/// - Blue line pointing out of the prime meridian (local +X), 3x radius outward
pub fn render_axes(
    bodies: Query<(&BodyRef, &Transform)>,
    system: Res<SimSystem>,
    mut gizmos: Gizmos,
    view_settings: Res<ViewSettings>,
) {
    if !view_settings.show_axes {
        return;
    }

    let pole_color = Srgba::new(1.0, 0.0, 0.0, 1.0); // Red for pole axis
    let meridian_color = Srgba::new(0.0, 0.0, 1.0, 1.0); // Blue for prime meridian

    for (body, transform) in bodies.iter() {
        let Some(i) = system.0.index_of(body.0) else { continue };
        let Some(rotation) = system.0.rotation(i) else { continue };
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
