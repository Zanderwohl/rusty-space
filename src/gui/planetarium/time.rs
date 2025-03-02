use bevy::prelude::Resource;

#[derive(Resource)]
pub struct SimTime {
    pub time: f64,
    pub previous_time: f64,
    pub step: f64,
    pub gui_speed: f64,
    pub playing: bool,
}

impl Default for SimTime {
    fn default() -> Self {
        Self {
            time: 0.0,
            previous_time: 0.0,
            step: 0.1,
            gui_speed: 1.0,
            playing: false,
        }
    }
}
