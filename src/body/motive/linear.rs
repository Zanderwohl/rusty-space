use std::collections::HashMap;
use std::sync::Arc;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::body::motive::Motive;
use crate::util::bitfutz;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct LinearMotive {
    pub(crate) local_position: DVec3,
    pub(crate) local_velocity: DVec3,
    primary: Option<u32>,
}

impl LinearMotive {
    pub fn new(local_position: DVec3, local_velocity: DVec3, primary: Option<u32>) -> Self {
        Self {
            local_position,
            local_velocity,
            primary,
        }
    }

    fn backing_position(&self, time: f64) -> DVec3 {
        self.local_position + self.local_velocity * time
    }

    fn backing_trajectory(&self, start_time: f64, end_time: f64) -> TimeMap<DVec3> {
        let start_point = self.backing_position(start_time);
        let end_point = self.backing_position(end_time);
        let mut map = TimeMap::new();
        map.insert(start_time, start_point);
        map.insert(end_time, end_point);
        map
    }
}

impl Default for LinearMotive {
    fn default() -> Self {
        Self {
            local_position: DVec3::ZERO,
            local_velocity: DVec3::new(0.0, 0.0, 0.0),
            primary: None,
        }
    }
}

impl Motive for LinearMotive {
    fn defined_primary(&self) -> Option<u32> {
        return self.primary
    }
    
    fn cached_trajectory(&self, start_time: f64, end_time: f64) -> TimeMap<DVec3> {
        self.backing_trajectory(start_time, end_time)
    }

    fn calculate_trajectory(&mut self, start_time: f64, end_time: f64, _time_step: f64) -> TimeMap<DVec3> {
        self.backing_trajectory(start_time, end_time)
    }

    fn cached_local_position_at_time(&self, time: f64) -> Option<DVec3> {
        Some(self.backing_position(time))
    }

    fn calculate_local_position_at_time(&mut self, time: f64) -> DVec3 {
        self.backing_position(time)
    }
}
