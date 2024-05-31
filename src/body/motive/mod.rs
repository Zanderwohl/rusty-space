use std::collections::HashMap;
use std::sync::Arc;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use fixed::FixedMotive;
use flat_kepler::FlatKepler;
use linear::LinearMotive;
use stupid_circle::StupidCircle;

pub mod fixed;
pub mod linear;
pub mod stupid_circle;
mod flat_kepler;

pub(crate) trait Motive {
    fn cached_trajectory(&self, start_time: f64, end_time: f64) -> Arc<HashMap<u64, DVec3>>;
}

/// A Motive is a method by which a body can move.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum MotiveTypes {
    Fixed(FixedMotive),
    Linear(LinearMotive),
    StupidCircle(StupidCircle),
    FlatKepler(FlatKepler),
}
