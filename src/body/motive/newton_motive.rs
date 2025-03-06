use bevy::prelude::Component;
use bevy::math::DVec3;

#[derive(Component)]
pub struct NewtonMotive {
    pub position: DVec3,
    pub velocity: DVec3,
}