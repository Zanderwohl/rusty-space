use bevy::prelude::Component;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};

/// A body that never moves, no matter what.
#[derive(Component)]
pub struct FixedBody {
    pub(crate) global_position: DVec3,
    pub(crate) properties: BodyProperties,
}

impl Body for FixedBody {
    fn global_position(&self) -> DVec3 {
        self.global_position
    }

    fn global_position_after_time(&self, _delta: f64) -> DVec3 {
        self.global_position
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}
