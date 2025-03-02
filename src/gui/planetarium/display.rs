use bevy::prelude::Resource;

#[derive(Resource)]
pub struct Display {
    distance_scale: f32,
    body_scale: f32,
    time_scale: f32,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            distance_scale: 1.0,
            body_scale: 1.0,
            time_scale: 1.0,
        }
    }
}
