use glam::Vec3;
use crate::body::body::{Body, BodyProperties};

pub struct KeplerBody {
    properties: BodyProperties,
    elements: KeplerElements,
}

impl Body for KeplerBody {
    fn global_position(&self) -> Vec3 {
        todo!()
    }

    fn time_step(&mut self, delta: i128) {
        todo!()
    }
}

struct KeplerElements {
    time_element: KeplerTimeElement,
    eccentricity: i64,
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
