use bevy::prelude::*;

#[derive(Resource)]
pub struct SimTime {
    pub time_seconds: f64,
    pub previous_time: f64,
    pub step: f64,
    pub gui_speed: f64,
    pub playing: bool,
    pub seconds_only: bool,
}

impl Default for SimTime {
    fn default() -> Self {
        Self {
            time_seconds: 0.0,
            previous_time: 0.0,
            step: 0.1,
            gui_speed: 1.0,
            playing: false,
            seconds_only: false,
        }
    }
}
