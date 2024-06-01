use std::collections::HashMap;
use std::sync::Arc;
use bevy::log::info;
use glam::DVec3;
use bevy::prelude::{Asset, Resource, TypePath};
use serde::{Deserialize, Serialize};
use crate::body::body::Body;
use crate::body::motive::{Motive, MotiveTypes};
use crate::util::overlapping_chunks::last_n_items;

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

    fn motive_ref(body: &Body) -> Box<&dyn Motive> {
        match &body.physics {
            MotiveTypes::Fixed(fixed_motive) => Box::new(fixed_motive),
            MotiveTypes::Linear(linear_motive) => Box::new(linear_motive),
            MotiveTypes::StupidCircle(stupid_circle) => Box::new(stupid_circle),
            MotiveTypes::FlatKepler(flat_kepler) => Box::new(flat_kepler),
        }
    }

    fn motive_mut(body: &mut Body) -> Box<&mut dyn Motive> {
        match &mut body.physics {
            MotiveTypes::Fixed(ref mut fixed_motive) => Box::new(fixed_motive),
            MotiveTypes::Linear(ref mut linear_motive) => Box::new(linear_motive),
            MotiveTypes::StupidCircle(ref mut stupid_circle) => Box::new(stupid_circle),
            MotiveTypes::FlatKepler(ref mut flat_kepler) => Box::new(flat_kepler),
        }
    }

    ///
    pub fn cached_local_position_at_time(&self, body_id: u32, time: f64) -> DVec3 {
        if let Some(body) = self.bodies.get(&body_id) {
            if let Some(position) = Self::motive_ref(body).cached_local_position_at_time(time) {
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
                match Self::motive_ref(body).defined_primary() {
                    None => {DVec3::ZERO}
                    Some(primary_id) => {self.cached_global_position_at_time(primary_id, time)}
                }
            };
            primary_position + position
        } else {
            DVec3::ZERO
        }
    }

    
    pub fn calc_compound_positions_span(&mut self, start_time: f64, end_time: f64, time_step: f64) {
        let mut current_time = start_time;
        let temp = self.trajectory_cache.len();
        while current_time< end_time {
            self.calc_positions_at_time(current_time);
            current_time += time_step;
        }
        info!("{}\t->\t{}\t{}", temp, self.trajectory_cache.len(), self.trajectory_cache.len() - temp);
        if self.trajectory_cache.len() > 5000 {
            let times: Vec<u64> = self.trajectory_cache.keys().cloned().collect::<Vec<u64>>();
            let times = last_n_items(times, 3000);
            let mut new_map = HashMap::new();
            for time in times {
                if let Some(item) = self.trajectory_cache.remove(&time) {
                    new_map.insert(time, item);
                }
            }
            self.trajectory_cache = new_map;
        }
        self.calc_positions_at_time(end_time);
    }

    ///
    pub fn calc_positions_at_time(&mut  self, time: f64) -> HashMap<u32, DVec3> {
        self.bodies.iter_mut().map(|(id, body)| {
            let mut motive = Self::motive_mut(body);
            let position = motive.calculate_local_position_at_time(time);
            (*id, position)
        }).collect()
    }

    ///
    pub(crate) fn calc_local_position_at_time(&mut self, body_id: u32, time: f64) -> DVec3 {
        let body = self.bodies.get_mut(&body_id);
        if let Some(body) = body {
            let mut motive = Self::motive_mut(body);
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
