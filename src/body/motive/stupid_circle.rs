use bevy::log::info;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::body::motive::{Motive};
use crate::util::circular;
use crate::util::gravity::G;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct StupidCircle {
    pub(crate) radius: f64,
    primary: u32,
    #[serde(skip)]
    pub(crate) primary_mass: f64, // This MUST be instantiated at some point.
    #[serde(skip)]
    trajectory_cache: TimeMap<DVec3>,
}

impl StupidCircle {
    pub fn new(radius: f64, primary_id: u32, primary_mass: f64) -> Self {
        Self {
            radius,
            primary: primary_id,
            primary_mass,
            trajectory_cache: TimeMap::new(),
        }
    }

    pub fn period(&self) -> f64 {
        let mu = G * self.primary_mass;
        circular::period::definition(self.radius, mu)
    }

    fn calculate_local_position_at_time_helper(&self, time: f64) -> DVec3 {
        let mu = G * self.primary_mass;
        let nu = circular::true_anomaly::at_time(time, self.radius, mu);
        let local_r = circular::position::from_true_anomaly(self.radius, nu);
        local_r
    }
}

impl Motive for StupidCircle {
    fn defined_primary(&self) -> Option<u32> {
        Some(self.primary)
    }

    fn cached_trajectory(&self, start_time: f64, end_time: f64) -> TimeMap<DVec3> {
        self.trajectory_cache.range(start_time, end_time)
    }

    fn calculate_trajectory(&mut self, start_time: f64, end_time: f64, time_step: f64) -> TimeMap<DVec3> {
        let mut current_time = start_time;
        while current_time + time_step <= end_time {
            self.calculate_local_position_at_time(current_time);
            current_time += time_step
        }

        self.cached_trajectory(start_time, end_time)
    }

    fn cached_local_position_at_time(&self, time: f64) -> Option<DVec3> {
        match self.trajectory_cache.get(time) {
            None => {
                let position = self.calculate_local_position_at_time_helper(time);
                Some(position)
            },
            Some(value) => {
                Some(*value)
            }
        }
    }

    fn calculate_local_position_at_time(&mut self, time: f64) -> DVec3 {
        if let Some(local_r) = self.trajectory_cache.get(time) {
            return local_r.clone()
        }
        let local_r = self.calculate_local_position_at_time_helper(time);
        self.trajectory_cache.insert(time, local_r);
        local_r
    }
}
