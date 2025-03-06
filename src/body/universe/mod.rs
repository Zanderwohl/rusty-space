use std::default::Default;
use std::path::PathBuf;
use bevy::prelude::{Commands, Component, Resource, Transform};
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::save::{SomeBody, UniverseFile};
use crate::gui::planetarium::time::SimTime;
use crate::util::kepler::mean_anomaly::kepler;

pub mod save;
pub mod solar_system;

#[derive(Resource)]
pub struct Universe {
    path: Option<PathBuf>,
    id_to_name: HashMap<String, String>,
    name_to_id: HashMap<String, String>,
}

#[derive(Component)]
pub struct Major;

#[derive(Component)]
pub struct Minor;

impl Universe {
    pub fn from_file(
        file: UniverseFile,
        mut commands: &mut Commands,
    ) -> (Self, SimTime) {
        let universe = Self {
            path: file.file.clone(),
            id_to_name: HashMap::new(),
            name_to_id: HashMap::new(),
        };

        let time = SimTime {
            time: file.contents.time.time,
            ..SimTime::default()
        };

        for body in file.contents.bodies.into_iter() {
            match body {
                SomeBody::FixedEntry(fixed) => fixed.spawn(commands),
                SomeBody::NewtonEntry(newton) => newton.spawn(commands),
                SomeBody::KeplerEntry(kepler) => kepler.spawn(commands),
                SomeBody::CompoundEntry(compound) => compound.spawn(commands),
            };
        }

        (universe, time)
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
}
