use std::collections::HashMap;
use glam::DVec3;
use bevy::prelude::{Asset, Resource, TypePath};
use serde::{Deserialize, Serialize};
use crate::body::body::Body;
use crate::body::motive::Motive;
use crate::util::{circular, kepler};

const DEBUG_G: f64 = 6.67430e-3;
const REAL_G: f64 = 6.67430e-11;
const G: f64 = DEBUG_G;

#[derive(Resource, Serialize, Deserialize, Debug, Asset, TypePath)]
pub struct Universe {
    pub(crate) bodies: HashMap<u32, Body>,
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

    pub fn add_body(&mut self, mut body: Body) -> u32 {
        let id = self.next_id();
        body.id = Some(id);
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
        if let Some(parent_id) = body.defined_primary {
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
                let parent_id = body.defined_primary.unwrap(); // Uh oh! Bodies need something to orbit.
                let parent = self.get_body(parent_id).unwrap();
                let mu = G * parent.mass;
                let nu = circular::true_anomaly::at_time(time, circular_motive.radius, mu);
                let local_r = circular::position::from_true_anomaly(circular_motive.radius, nu);
                origin + local_r
            },
            Motive::FlatKepler(flat_kepler) => {
                let id = body.id.unwrap_or(u32::MAX);
                let message = format!("'{}' (ID {}) has no primary set. {:?}", body.get_name(), id, body);
                let message_str = message.as_str();
                let parent_id = body.defined_primary.expect(message_str);
                let parent = self.get_body(parent_id).unwrap();
                let mu = G * parent.mass;

                let local_r = kepler::in_plane::displacement(time, mu, flat_kepler.mean_anomaly_at_epoch, flat_kepler.semi_major_axis, flat_kepler.eccentricity, flat_kepler.longitude_of_periapsis);
                origin + local_r
            }
            _ => { origin }
        }
    }
}
