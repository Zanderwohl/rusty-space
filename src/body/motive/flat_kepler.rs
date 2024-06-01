use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::body::motive::Motive;
use crate::util::gravity::G;
use crate::util::kepler;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct FlatKepler {
    pub(crate) semi_major_axis: f64,
    pub(crate) mean_anomaly_at_epoch: f64,
    pub(crate) eccentricity: f64,
    pub(crate) longitude_of_periapsis: f64,
    primary: u32,
    #[serde(skip)]
    primary_mass: f64, // This MUST be instantiated at some point.
    #[serde(skip)]
    trajectory_cache: TimeMap<DVec3>,
}

impl FlatKepler {
    fn new(semi_major_axis: f64,
           mean_anomaly_at_epoch: f64,
           eccentricity: f64,
           longitude_of_periapsis: f64,
           primary_id: u32,
           primary_mass: f64) -> Self {
        FlatKepler {
            semi_major_axis,
            mean_anomaly_at_epoch,
            eccentricity,
            longitude_of_periapsis,
            primary: primary_id,
            primary_mass,
            trajectory_cache: TimeMap::new(),
        }
    }
}

impl FlatKepler {
    pub fn period(&self) -> f64 {
        let mu = G * self.primary_mass;
        kepler::period::third_law(self.semi_major_axis, mu)
    }
}

impl Motive for FlatKepler {
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
            None => None,
            Some(value) => {
                Some(*value)
            }
        }
    }

    fn calculate_local_position_at_time(&mut self, time: f64) -> DVec3 {
        let mu = G * self.primary_mass;
        let local_r = kepler::in_plane::displacement(time, mu, self.mean_anomaly_at_epoch, self.semi_major_axis, self.eccentricity, self.longitude_of_periapsis);
        self.trajectory_cache.insert(time, local_r);
        local_r
    }
}
