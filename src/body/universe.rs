use std::collections::HashMap;
use std::sync::Arc;
use bevy::log::info;
use glam::DVec3;
use bevy::prelude::{Asset, Resource, TypePath};
use serde::{Deserialize, Serialize};
use crate::body::body::Body;
use crate::body::motive::{Motive, MotiveTypes};
use crate::util::overlapping_chunks::last_n_items;
use crate::util::time_map::TimeMap;


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

    pub fn give_motives_info(&mut self) {
        let body_info: HashMap<u32, f64> = self.bodies.iter().map(|(id, body)| {
            (*id, body.mass)
        }).collect();
        self.bodies.iter_mut().for_each(|(id, body)| {
            if let Some(primary) = body.motive_ref().defined_primary() {
                match &mut body.physics {
                    MotiveTypes::StupidCircle(ref mut circle) => {
                        if let Some(mass) = body_info.get(&primary) {
                            circle.primary_mass = *mass;
                        }
                    }
                    MotiveTypes::FlatKepler(ref mut flat_kepler) => {
                        if let Some(mass) = body_info.get(&primary) {
                            flat_kepler.primary_mass = *mass;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    pub fn remove_body(&mut self, _id: u32) {
        todo!("Removing will need to give children to their primary's primary with ref frame changes")
    }

    ///
    pub fn cached_local_position_at_time(&self, body_id: u32, time: f64) -> DVec3 {
        if let Some(body) = self.bodies.get(&body_id) {
            if let Some(position) = body.motive_ref().cached_local_position_at_time(time) {
                position
            } else {
                DVec3::ZERO
            }
        } else {
            DVec3::ZERO
        }
    }

    ///
    pub fn cached_global_position_at_time(&self, body_id: u32, time: f64) -> DVec3 {
        if let Some(body) = self.bodies.get(&body_id) {
            let position = self.cached_local_position_at_time(body_id, time);
            let primary_position = {
                match body.motive_ref().defined_primary() {
                    None => {DVec3::ZERO}
                    Some(primary_id) => {self.cached_global_position_at_time(primary_id, time)}
                }
            };
            primary_position + position
        } else {
            DVec3::ZERO
        }
    }

    ///
    pub fn calc_compound_positions_span(&mut self, start_time: f64, end_time: f64, time_step: f64) -> HashMap<u32, TimeMap<DVec3>> {
        self.bodies.iter_mut().map(|(id, body)| {
            let mut motive = body.motive_mut();
            let trajectory = motive.calculate_trajectory(start_time, end_time, time_step);
            (*id, trajectory)
        }).collect()
    }

    ///
    pub fn calc_positions_at_time(&mut  self, time: f64) -> HashMap<u32, DVec3> {
        self.bodies.iter_mut().map(|(id, body)| {
            let mut motive = body.motive_mut();
            let position = motive.calculate_local_position_at_time(time);
            (*id, position)
        }).collect()
    }

    ///
    pub(crate) fn calc_local_position_at_time(&mut self, body_id: u32, time: f64) -> DVec3 {
        let body = self.bodies.get_mut(&body_id);
        if let Some(body) = body {
            let mut motive = body.motive_mut();
            motive.calculate_local_position_at_time(time)
        } else {
            DVec3::ZERO
        }
    }

    ///
    pub fn get_trajectory_for(&self, body_id: u32, current_time: f64, mode: TrajectoryMode) -> Vec<DVec3> {
        match self.bodies.get(&body_id) {
            None => Vec::new(),
            Some(_body) => {
                // let motive = Self::motive_ref(body);
                Vec::new()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrajectoryMode {
    Global,
    LocalToEachPrimary,
    LocalToCurrentPrimary,
}
