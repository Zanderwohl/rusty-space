use glam::DVec3;
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::motive::Motive;
use crate::util::bitfutz;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct FixedMotive {
    pub(crate) local_position: DVec3,
    #[serde(skip)]
    position_cache: Arc<HashMap<u64, DVec3>>
}

impl FixedMotive {
    pub fn new(position: DVec3) -> Self {
        let mut position_cache= HashMap::new();
        position_cache.insert(bitfutz::f64::to_u64(0.0), position.clone());
        Self {
            local_position: position,
            position_cache: Arc::new(position_cache),
        }
    }
}

impl Default for FixedMotive {
    fn default() -> Self {
        FixedMotive {
            local_position: DVec3::ZERO,
            position_cache: Arc::new(HashMap::new())
        }
    }
}

impl Motive for FixedMotive {
    fn cached_trajectory(&self, _start_time: f64, _end_time: f64) -> Arc<HashMap<u64, DVec3>> {
        self.position_cache.clone()
    }
}
