use std::collections::HashMap;
use glam::DVec3;
use bevy::prelude::Resource;
use crate::body::body::{Body, Motive};
use crate::util::circular;

const G: f64 = 6.67430e-11;

#[derive(Resource)]
pub struct Universe {
    bodies: HashMap<u32, Body>,
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

    pub fn get_body(&self, id: u32) -> Option<&Body> {
        self.bodies.get(&id)
    }

    fn next_id(&mut self) -> u32 {
        let id = self.counter;
        self.counter += 1;
        id
    }

    pub fn add_body(&mut self, body: Body) -> u32 {
        let id = self.next_id();
        self.bodies.insert(id, body);
        id
    }

    pub fn remove_body(&mut self, id: u32) {
        self.bodies.remove(&id);
    }

    pub fn calc_positions_at_time(&self, time: f64) -> HashMap<u32, DVec3> {
        let mut positions: HashMap<u32, DVec3> = HashMap::new();
        for (&id, body) in self.bodies.iter() {
            let origin = self.calc_origin_at_time(time, body);
            let position = self.calc_position_at_time(time, body, origin);
            positions.insert(id, position);
        }
        positions
    }

    pub(crate) fn calc_origin_at_time(&self, time: f64, body: &Body) -> DVec3 {
        if let Some(parent_id) = body.parent {
            let parent = self.bodies.get(&parent_id).unwrap(); // If we crash here, then parent IDs aren't getting inserted/updated/deleted properly
            let parent_origin = self.calc_origin_at_time(time, parent);
            self.calc_position_at_time(time, parent, parent_origin)
        } else {
            DVec3::ZERO
        }
    }

    pub(crate) fn calc_position_at_time(&self, time: f64, body: &Body, origin: DVec3) -> DVec3 {
        match &body.physics {
            Motive::Fixed(fixed_motive) => {
                origin + fixed_motive.local_position
            },
            Motive::Linear(linear_motive) => {
                origin + linear_motive.local_velocity * time
            },
            Motive::StupidCircle(circular_motive) => {
                let parent_id = body.parent.unwrap(); // Uh oh! Bodies need something to orbit.
                let parent = self.get_body(parent_id).unwrap();
                let mu = G * parent.mass;
                let v = circular::true_anomaly::at_time(time, circular_motive.radius, mu);
                let local_p = circular::position::from_true_anomaly(circular_motive.radius, v);
                origin + local_p
            },
            _ => { origin }
        }
    }
}
