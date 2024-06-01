use glam::DVec3;
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::motive::Motive;
use crate::util::bitfutz;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct FixedMotive {
    pub(crate) local_position: DVec3,
    primary: Option<u32>,
    #[serde(skip)]
    position_cache: TimeMap<DVec3>,
}

impl FixedMotive {
    pub fn new(position: DVec3, primary: Option<u32>) -> Self {
        let mut position_cache = Self::calc_position_cache(position);
        Self {
            local_position: position,
            primary,
            position_cache,
        }
    }

    fn calc_position_cache(position: DVec3) -> TimeMap<DVec3> {
        let mut time_map= TimeMap::new();
        time_map.insert(0.0, position.clone());
        time_map
    }
}

impl Default for FixedMotive {
    fn default() -> Self {
        Self {
            local_position: DVec3::ZERO,
            primary: None,
            position_cache: Self::calc_position_cache(DVec3::ZERO)
        }
    }
}

impl Motive for FixedMotive {
    fn defined_primary(&self) -> Option<u32> {
        self.primary
    }

    fn cached_trajectory(&self, _start_time: f64, _end_time: f64) -> TimeMap<DVec3> {
        self.position_cache.clone()
    }

    fn calculate_trajectory(&mut self, _start_time: f64, _end_time: f64, _time_step: f64) -> TimeMap<DVec3> {
        self.position_cache.clone()
    }

    fn cached_local_position_at_time(&self, _time: f64) -> Option<DVec3> {
        Some(self.local_position)
    }

    fn calculate_local_position_at_time(&mut self, _time: f64) -> DVec3 {
        self.local_position
    }
}
