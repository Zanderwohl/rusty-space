use bevy::math::DVec3;
use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize)]
pub struct NewtonMotive {
    position: DVec3,
    velocity: DVec3,
    acceleration: DVec3,
}
