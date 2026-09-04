//! Simulation infrastructure shared between `body` and `gui`, so neither depends on the
//! other.

use bevy::prelude::*;

pub mod world;


// Re-exported from `em-sim` so `crate::sim::…` paths keep resolving.
pub use em_sim::time::{SimTime, PreviousTimes, PreviousTimesIter};
pub use em_sim::events::{CalculateTrajectory, BodySelection};

/// Marks entities belonging to the current simulation, for bulk despawn on unload.
#[derive(Component)]
pub struct SimulationObject;

/// Despawn all simulation objects when leaving the planetarium.
pub fn unload_simulation_objects(
    commands: Commands,
    simulation_objects: Query<Entity, With<SimulationObject>>,
) {
    despawn_entities_with::<SimulationObject>(simulation_objects, commands);
}

/// Despawn every entity carrying `T`.
pub fn despawn_entities_with<T: Component>(to_despawn: Query<Entity, With<T>>, mut commands: Commands) {
    for entity in &to_despawn {
        commands.entity(entity).despawn();
    }
}

/// Recursive variant of [`despawn_entities_with`].
pub fn despawn_recursive_entities_with<T: Component>(
    mut commands: Commands,
    query: Query<Entity, With<T>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
}
