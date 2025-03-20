use bevy::math::DVec3;
use bevy::prelude::{Commands, Component, Entity, Query, Resource, With};
use crate::gui::common::despawn_entities_with;

pub mod motive;
pub mod universe;
pub mod appearance;

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}

#[derive(Component)]
pub struct SimulationObject;

pub fn unload_simulation_objects(
    mut commands: Commands,
    simulation_objects: Query<Entity, With<SimulationObject>>,
) {
    despawn_entities_with::<SimulationObject>(simulation_objects, commands);
}
