use bevy::prelude::Component;
use bevy::math::DVec3;

#[derive(Component)]
pub struct FixedMotive {
    pub position: DVec3,
}
