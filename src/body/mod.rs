use bevy::prelude::{Component, Resource};

pub mod universe;
pub mod body;
pub mod motive;
pub mod appearance;

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}
