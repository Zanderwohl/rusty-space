use std::sync::Arc;
use bevy::prelude::{Component, Entity, Parent};
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::util;

#[derive(Component)]
pub struct CircularBody {
    pub(crate) properties: BodyProperties,
    pub radius: f64,
}

impl Body for CircularBody {
    fn global_position(&self) -> DVec3 {
        self.global_position_after_time(0.0)
    }

    fn global_position_after_time(&self, delta: f64) -> DVec3 {
        // let parent_location = self.parent.global_position_after_time(delta);
        // let mu = self.parent.mu();
        let parent_location = DVec3::ZERO;
        let mu = 0.1;
        let v = util::circular::true_anomaly::at_time(delta, self.radius, mu);
        let local_pos = util::circular::position::from_true_anomaly(self.radius, v);
        // println!("{}", parent_location + local_pos);
        parent_location + local_pos
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}
