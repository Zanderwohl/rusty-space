use glam::DVec3;
use serde::{Deserialize, Serialize};

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
