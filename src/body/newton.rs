use glam::DVec3;
use crate::body::body::{Body, BodyProperties};


pub struct NewtonBody {
    global_position: DVec3,
    properties: BodyProperties,
}

impl Body for NewtonBody {
    fn global_position(&self) -> DVec3 {
        self.global_position
    }

    fn global_position_after_time(&self, delta: f64) -> DVec3 {
        let granularity: f64 = 1.0;
        todo!()
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}