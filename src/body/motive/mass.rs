use bevy::math::DVec3;
use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize)]
pub struct Mass {
    kg: f64,
}
