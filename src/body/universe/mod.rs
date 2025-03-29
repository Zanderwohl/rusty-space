use std::default::Default;
use std::path::PathBuf;
use bevy::prelude::{Component, Res, ResMut, Resource, Time};
use bevy::utils::hashbrown::hash_map::Iter;
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::universe::save::UniverseFile;
use crate::gui::planetarium::time::SimTime;
pub mod save;
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
    if sim_time.playing {
        sim_time.previous_time = sim_time.time_seconds;
        sim_time.time_seconds += sim_time.gui_speed * time.delta_secs_f64();
    }
}