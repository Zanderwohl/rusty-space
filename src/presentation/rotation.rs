//! Body rotation system.

use bevy::prelude::*;
use crate::body::motive::info::{BodyRotation, BodyState, RotationMode};
use crate::body::motive::calculate_body_positions::PhysicsGraph;
use crate::sim::SimTime;
use crate::util::bevystuff::GlamQuat;

/// Updates body Transform rotations based on their BodyRotation component.
///
/// Computes the current orientation at simulation time, converts from
/// simulation Z-up to Bevy Y-up coordinates, and applies to transform.rotation.
/// For tidally locked bodies, orients them to face their primary.
pub fn orient_bodies(
    mut bodies: Query<(&mut Transform, &BodyRotation, &BodyState)>,
    all_body_states: Query<&BodyState>,
    physics_graph: Res<PhysicsGraph>,
    sim_time: Res<SimTime>,
) {
    for (mut transform, rotation, state) in bodies.iter_mut() {
        let current_orientation = match &rotation.mode {
            RotationMode::Spinning { .. } => {
                rotation.orientation_at(sim_time.time)
            }
            RotationMode::TidallyLocked { primary_id, .. } => {
                // Find the primary body's position using O(1) lookup via PhysicsGraph
                let primary_pos = physics_graph.id_to_entity.get(primary_id)
                    .and_then(|entity| all_body_states.get(*entity).ok())
                    .map(|primary_state| primary_state.current_position);
                
                if let Some(primary_pos) = primary_pos {
                    rotation.orientation_tidally_locked(state.current_position, primary_pos)
                } else {
                    None
                }
            }
        };
        
        if let Some(orientation) = current_orientation {
            transform.rotation = orientation.as_bevy();
        }
    }
}
