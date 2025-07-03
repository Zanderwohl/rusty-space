use bevy::prelude::*;
use crate::gui::common::despawn_entities_with;

pub mod motive;
pub mod universe;
pub mod appearance;


#[derive(Resource, Debug, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            gravity_constant: scilib::constant::G,
        }
    }
}

#[derive(Component)]
pub struct SimulationObject;

pub fn unload_simulation_objects(
    commands: Commands,
    simulation_objects: Query<Entity, With<SimulationObject>>,
) {
    despawn_entities_with::<SimulationObject>(simulation_objects, commands);
}
