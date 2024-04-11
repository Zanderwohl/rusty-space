use bevy::prelude::Component;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::util::gravity::newton_gravity;

#[derive(Component)]
pub struct NewtonBody {
    pub(crate) global_position: DVec3,
    pub(crate) global_velocity: DVec3,
    pub(crate) properties: BodyProperties,
}

impl Body for NewtonBody {
    fn local_position(&self) -> DVec3 {
        self.global_position
    }

    fn local_position_after_time(&self, delta: f64) -> DVec3 {
        let granularity: f64 = 1.0;
        self.global_position
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }

    fn size(&self) -> f64 {
        self.properties.size
    }
}

impl NewtonBody {
    pub fn acceleration(&self, other: Box<dyn Body>) -> DVec3 {
        newton_gravity(self.mass(), other.mass(), self.global_position)
    }
}
