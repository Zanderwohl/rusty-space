use std::collections::HashMap;
use std::sync::Arc;
use glam::DVec3;
use bevy::prelude::{Asset, Resource, TypePath};
use serde::{Deserialize, Serialize};
use crate::body::body::Body;
use crate::body::motive::Motive;
use crate::util::{bitfutz, circular, kepler};

const DEBUG_G: f64 = 6.67430e-3;
const REAL_G: f64 = 6.67430e-11;
const G: f64 = DEBUG_G;

#[derive(Resource, Serialize, Deserialize, Debug, Asset, TypePath)]
pub struct Universe {
    pub(crate) bodies: HashMap<u32, Body>,
    counter: u32,
    #[serde(skip)]
    trajectory_cache: HashMap<u64, Arc<HashMap<u32, DVec3>>> // Am I going to regret this?
}

impl Default for Universe {
    fn default() -> Self {
        Universe {
            bodies: HashMap::new(),
            counter: 0,
            trajectory_cache: HashMap::new(),
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

    pub fn remove_body(&mut self, _id: u32) {
        todo!("Removing will need to give children to their primary's primary with ref frame changes")
    }

    pub fn get_local_position_at_time(&self, body_id: u32, time: f64) -> DVec3 {
        if let Some(time_slice) = self.trajectory_cache.get(&bitfutz::f64::to_u64(time)) {
            if let Some(position) = time_slice.get(&body_id) {
                *position
            } else {
                DVec3::ZERO
            }
        } else {
            DVec3::ZERO
        }
    }

    pub fn get_global_position_at_time(&self, body_id: u32, time: f64) -> DVec3 {
        if let Some(body) = self.bodies.get(&body_id) {
            let position = self.get_local_position_at_time(body_id, time);
            let primary_position = {
                match body.defined_primary {
                    None => {DVec3::ZERO}
                    Some(primary_id) => {self.get_global_position_at_time(primary_id, time)}
                }
            };
            primary_position + position
        } else {
            DVec3::ZERO
        }
    }

    pub fn calc_positions_at_time(&mut self, time: f64) -> Arc<HashMap<u32, DVec3>> {
        let mut positions: HashMap<u32, DVec3> = HashMap::new();
        for body_id in self.bodies.keys().cloned().collect::<Vec<u32>>() {
            let body_position = self.calc_local_position_at_time(body_id, time);
            positions.insert(body_id, body_position);
        }

        let shared = Arc::new(positions);
        self.trajectory_cache.insert(bitfutz::f64::to_u64(time), shared.clone());
        shared
    }

    pub(crate) fn calc_local_position_at_time(&mut self, body_id: u32, time: f64) -> DVec3 {
        let body = self.bodies.get(&body_id);
        if let Some(mut body) = body {
            match &body.physics {
                Motive::Fixed(fixed_motive) => {
                    fixed_motive.local_position
                },
                Motive::Linear(linear_motive) => {
                    linear_motive.local_velocity * time
                },
                Motive::StupidCircle(circular_motive) => {
                    let parent_id = body.defined_primary.unwrap(); // Uh oh! Bodies need something to orbit.
                    let parent = self.get_body(parent_id).unwrap();
                    let mu = G * parent.mass;
                    let nu = circular::true_anomaly::at_time(time, circular_motive.radius, mu);
                    let local_r = circular::position::from_true_anomaly(circular_motive.radius, nu);
                    local_r
                },
                Motive::FlatKepler(flat_kepler) => {
                    let id = body.id.unwrap_or(u32::MAX);
                    let message = format!("'{}' (ID {}) has no primary set. {:?}", body.get_name(), id, body);
                    let message_str = message.as_str();
                    let parent_id = body.defined_primary.expect(message_str);
                    let parent = self.get_body(parent_id).unwrap();
                    let mu = G * parent.mass;

                    let local_r = kepler::in_plane::displacement(time, mu, flat_kepler.mean_anomaly_at_epoch, flat_kepler.semi_major_axis, flat_kepler.eccentricity, flat_kepler.longitude_of_periapsis);
                    local_r
                }
                _ => { DVec3::ZERO }
            }
        } else {
            DVec3::ZERO
        }
    }

    pub fn get_trajectory_for(&self, body_id: u32, current_time: f64, mode: TrajectoryMode) -> Vec<DVec3> {
        let mut trajectory = Vec::new();
        let mut times: Vec<u64> = self.trajectory_cache.keys().cloned().collect::<Vec<u64>>();
        times.sort();
        for time in times {
            let time = bitfutz::u64::to_f64(time);
            let position = match mode {
                TrajectoryMode::Global => { self.get_global_position_at_time(body_id, time) }
                _ => {
                    let current_primary_position = match self.bodies.get(&body_id) {
                        None => { DVec3::ZERO }
                        Some(body) => match body.defined_primary {
                                None => { DVec3::ZERO }
                                Some(parent_id) => {
                                    self.get_global_position_at_time(parent_id, current_time)
                                }
                        }
                    };
                    self.get_local_position_at_time(body_id, time) + current_primary_position
                }
            };
            trajectory.push(position.clone())
        }
        trajectory
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrajectoryMode {
    Global,
    LocalToEachPrimary,
    LocalToCurrentPrimary,
}
