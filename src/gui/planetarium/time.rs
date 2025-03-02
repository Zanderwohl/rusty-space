use bevy::prelude::Resource;

#[derive(Resource)]
pub struct Time {
    pub time: f64,
    pub step: f64,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            time: 0.0,
            step: 0.1,
        }
    }
}
