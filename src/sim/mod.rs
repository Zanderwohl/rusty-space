//! Simulation infrastructure: clock, events, and ECS helpers.
//!
//! This module owns simulation-related types that need to be shared between
//! `body` (domain/physics) and `gui` (presentation). By placing them here,
//! we avoid a bidirectional dependency between those modules.

use bevy::prelude::*;

pub mod world;


// The clock and the message types live in `em-sim`; re-exported here so existing
// `crate::sim::…` paths keep resolving. What remains below is pure ECS glue.
pub use em_sim::time::{SimTime, PreviousTimes, PreviousTimesIter};
pub use em_sim::events::{CalculateTrajectory, BodySelection};

/// Marker component for entities that belong to the current simulation.
/// Used to despawn all simulation entities when unloading a universe.
#[derive(Component)]
pub struct SimulationObject;

/// System to despawn all simulation objects when exiting the planetarium.
pub fn unload_simulation_objects(
    commands: Commands,
    simulation_objects: Query<Entity, With<SimulationObject>>,
) {
    despawn_entities_with::<SimulationObject>(simulation_objects, commands);
}

/// Generic helper to despawn all entities with a given component.
pub fn despawn_entities_with<T: Component>(to_despawn: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &to_despawn {
        commands.entity(entity).despawn();
    }
}

/// Generic helper to despawn all entities with a given component (recursive variant).
pub fn despawn_recursive_entities_with<T: Component>(
    mut commands: Commands,
    query: Query<Entity, With<T>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}
