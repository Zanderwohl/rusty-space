use bevy::prelude::Component;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};

#[derive(Component)]
pub struct KeplerBody {
    properties: BodyProperties,
    elements: KeplerElements,
}

impl Body for KeplerBody {
    fn global_position(&self) -> DVec3 {
        todo!()
    }

    fn global_position_after_time(&self, delta: f64) -> DVec3 {
        todo!()
    }

    fn mass(&self) -> f64 {
        self.properties.mass
    }

    fn name(&self) -> &String {
        &self.properties.name
    }
}

struct KeplerElements {
    time_element: KeplerTimeElement,
    eccentricity: f64,
    semi_major_axis_millimeters: u128,
    inclination: i64,
    longitude_of_ascending_node: i64,
    argument_of_periapsis: i64,
}

enum KeplerTimeElement {
    TrueAnomalyAtEpoch(i128),
    TimeOfPeriapsisPassage(i128),
    ArgumentOfLongitudeAtEpoch(i128),
}
