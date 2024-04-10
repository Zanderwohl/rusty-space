use bevy::prelude::{Component, Resource};

pub mod body;
pub mod kepler;
pub mod newton;
pub mod fixed;
pub mod linear;
pub mod circular;

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}
