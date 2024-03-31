use bevy::prelude::Component;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};

#[derive(Component)]
pub struct CircularBody {
    pub(crate) properties: BodyProperties,
}

impl Body for CircularBody {
    fn global_position(&self) -> DVec3 {
        self.global_position_after_time(0.0)
    }

    fn global_position_after_time(&self, delta: f64) -> DVec3 {
        let granularity: f64 = 1.0;
        DVec3::ZERO
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}