use bevy::math::DVec3;
use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize)]
pub struct FixedMotive {
    origin_id: Option<String>,
    position: DVec3,
}
