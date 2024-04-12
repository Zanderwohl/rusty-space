use std::collections::HashMap;
use bevy::math::DVec3;
use bevy::prelude::{Component, Resource};
use crate::body::fixed::FixedBody;

#[derive(Resource)]
pub struct Universe {
    bodies: HashMap<u32, NewBody>,
    counter: u32,
}

impl Default for Universe {
    fn default() -> Self {
        Universe {
            bodies: HashMap::new(),
            counter: 0,
        }
    }
}

impl Universe {

    pub fn new() -> Universe {
        Universe::default()
    }

    pub fn get_body(&self, id: u32) -> Option<&NewBody> {
        self.bodies.get(&id)
    }

    fn next_id(&mut self) -> u32 {
        let id = self.counter;
        self.counter += 1;
        id
    }
}

pub struct NewBody {
    physics: Box<dyn Motive + Send + Sync>,
    name: String,
    mass: f64,
    radius: f64,
}

/// A Motive is a method by which a body can move.
pub trait Motive: Send + Sync + 'static {
    fn global_position_at_time(&self, time: f64) -> DVec3;
}

pub struct FixedMotive {

}

impl Motive for FixedMotive {
    fn global_position_at_time(&self, time: f64) -> DVec3 {
        todo!()
    }
}
