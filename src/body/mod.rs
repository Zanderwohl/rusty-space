use bevy::math::DVec3;
use bevy::prelude::{Component, Resource};

pub mod motive;
pub mod universe;

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
pub struct SimulationSettings {
    pub gravity_constant: f64,
}
