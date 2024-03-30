use bevy::prelude::Component;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};

/// A body that moves in a straight line for debug purposes.
#[derive(Component)]
pub struct LinearBody {
    pub(crate) global_position: DVec3,
    pub(crate) properties: BodyProperties,
    pub(crate) velocity: DVec3,
}

impl Body for LinearBody {
    fn global_position(&self) -> DVec3 {
        self.global_position
    }

    fn global_position_after_time(&self, delta: f64) -> DVec3 {
        self.global_position + (self.velocity * delta)
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}
