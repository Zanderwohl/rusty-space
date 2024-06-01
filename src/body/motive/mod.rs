use std::collections::HashMap;
use std::sync::Arc;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use fixed::FixedMotive;
use flat_kepler::FlatKepler;
use linear::LinearMotive;
use stupid_circle::StupidCircle;
use crate::util::time_map::TimeMap;

pub mod fixed;
pub mod linear;
pub mod stupid_circle;
mod flat_kepler;

pub(crate) trait Motive {
    fn defined_primary(&self) -> Option<u32>;
    fn cached_trajectory(&self, start_time: f64, end_time: f64) -> TimeMap<DVec3>;
    fn calculate_trajectory(&mut self, start_time: f64, end_time: f64, time_step: f64) -> TimeMap<DVec3>;
    fn cached_local_position_at_time(&self, time: f64) -> Option<DVec3>;
    fn calculate_local_position_at_time(&mut self, time: f64) -> DVec3;
}

/// A Motive is a method by which a body can move.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum MotiveTypes {
    Fixed(FixedMotive),
    Linear(LinearMotive),
    StupidCircle(StupidCircle),
    FlatKepler(FlatKepler),
}
