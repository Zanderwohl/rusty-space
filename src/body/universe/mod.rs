use std::collections::hash_map::Iter;
use std::default::Default;
use std::path::PathBuf;
use bevy::prelude::*;
use std::collections::HashMap;
use crate::body::universe::save::UniverseFile;
use crate::foundations::time::Instant;
use crate::sim::SimTime;

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
            time: Instant::from_julian_day(file.contents.time.time_julian_days),
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
        // Take the id out first: removing from `name_to_id` before the lookup would
        // leave the `id_to_name` half of the bimap orphaned.
        if let Some(id) = self.name_to_id.remove(name.as_ref()) {
            self.id_to_name.remove(&id);
        }
    }

    pub fn remove_by_id<T: AsRef<str>>(&mut self, id: T) {
        if let Some(name) = self.id_to_name.remove(id.as_ref()) {
            self.name_to_id.remove(&name);
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