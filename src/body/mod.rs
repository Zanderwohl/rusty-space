use bevy::prelude::{Component, Resource};

pub mod motive;

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}
