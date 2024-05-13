use glam::DVec3;
use serde::{Serialize, Deserialize};

/// A Motive is a method by which a body can move.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum Motive {
    Fixed(FixedMotive),
    Linear(LinearMotive),
    StupidCircle(StupidCircle),
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct FixedMotive {
    pub(crate) local_position: DVec3,
}

impl Default for FixedMotive {
    fn default() -> Self {
        FixedMotive {
            local_position: DVec3::ZERO,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct LinearMotive {
    pub(crate) local_position: DVec3,
    pub(crate) local_velocity: DVec3,
}

impl Default for LinearMotive {
    fn default() -> Self {
        LinearMotive {
            local_position: DVec3::ZERO,
            local_velocity: DVec3::new(1.0, 0.0, 0.0),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct StupidCircle {
    pub(crate) radius: f64,
}

impl Default for StupidCircle {
    fn default() -> Self {
        StupidCircle {
            radius: 1.0,
        }
    }
}
