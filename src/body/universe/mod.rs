use std::collections::hash_map::Iter;
use std::default::Default;
use std::path::PathBuf;
use bevy::prelude::*;
use std::collections::HashMap;
use crate::body::universe::save::UniverseFile;
use crate::gui::planetarium::time::SimTime;

pub mod save;
pub mod save_sqlite;
pub mod migrations;
pub mod solar_system;

#[derive(Resource)]
pub struct Universe {
    pub path: Option<PathBuf>,
    id_to_name: HashMap<String, String>,
    name_to_id: HashMap<String, String>,
}

impl Default for Universe {
    fn default() -> Self {
        Self {
            path: None,
            id_to_name: HashMap::new(),
            name_to_id: HashMap::new(),
        }
    }
}

#[derive(Component)]
pub struct Major;

#[derive(Component)]
pub struct Minor;

impl Universe {
    pub fn from_file(
        file: &UniverseFile,
    ) -> (Self, SimTime) {
        let universe = Self {
            path: file.file.clone(),
            id_to_name: HashMap::new(),
            name_to_id: HashMap::new(),
        };

        let time = SimTime {
            time_seconds: file.contents.time.time_julian_days,
            step: file.contents.time.step,
            gui_speed: file.contents.time.gui_speed,
            max_frame_time: file.contents.time.max_frame_time,
            ..SimTime::default()
        };

        (universe, time)
    }

    pub fn clear_all(&mut self) {
        self.id_to_name = HashMap::new();
        self.name_to_id = HashMap::new();
    }

    pub fn insert<T: AsRef<str> + Clone>(&mut self, name: T, id: T) {
        let id = id.as_ref().to_string();
        let name = name.as_ref().to_string();
        self.id_to_name.insert(id.clone(), name.clone());
        self.name_to_id.insert(name, id);
    }

    pub fn remove_by_name<T: AsRef<str>>(&mut self, name: T) {
        self.name_to_id.remove(name.as_ref());
        if let Some(id) = self.name_to_id.get(name.as_ref()) {
            self.id_to_name.remove(id);
        }
    }

    pub fn remove_by_id<T: AsRef<str>>(&mut self, id: T) {
        self.id_to_name.remove(id.as_ref());
        if let Some(name) = self.id_to_name.get(id.as_ref()) {
            self.name_to_id.remove(name);
        }
    }

    pub fn id_to_name_iter(&self) -> Iter<'_, String, String> {
        self.id_to_name.iter()
    }

    pub fn get_by_id<T: AsRef<str>>(&self, id: T) -> Option<&String> {
        self.id_to_name.get(id.as_ref())
    }

    pub fn get_by_name<T: AsRef<str>>(&self, name: T) -> Option<&String> {
        self.name_to_id.get(name.as_ref())
    }
}

pub fn advance_time(mut sim_time: ResMut<SimTime>, time: Res<Time>) {
    if !sim_time.playing {
        return;
    }
    
    let real_delta = time.delta_secs_f64();
    let step = sim_time.step;
    
    // Calculate how much sim time we WANT to advance based on gui_speed
    let desired_sim_delta = sim_time.gui_speed * real_delta;
    
    // Accumulate the desired time
    sim_time.accumulated_time += desired_sim_delta;
    
    // Calculate how many FULL steps we can queue from accumulated time
    // This prevents overshoot when step > desired_sim_delta
    let full_steps = (sim_time.accumulated_time / step).floor() as usize;
    
    if full_steps == 0 {
        // Not enough accumulated time for a full step yet - wait until next frame
        return;
    }
    
    // Subtract the time we're actually queuing from accumulated
    sim_time.accumulated_time -= full_steps as f64 * step;
    
    // How many steps are already queued (unprocessed from last frame)?
    let already_queued = sim_time.previous_times.len();
    
    // Only add new steps if we need more than what's queued
    // This handles the case where we're falling behind
    let steps_to_add = full_steps.saturating_sub(already_queued);
    
    if steps_to_add == 0 {
        return;
    }
    
    // Get the last queued time (or current time if queue is empty)
    let last_queued_time = sim_time.previous_times.last()
        .unwrap_or(sim_time.time_seconds);
    
    // Expand the queue by adding more steps at the end
    sim_time.previous_times.expand(last_queued_time + step, steps_to_add, step);
    
    // NOTE: We do NOT update time_seconds here.
    // time_seconds is updated by calculate_body_positions to reflect
    // what was actually processed, not what we're trying to reach.
}