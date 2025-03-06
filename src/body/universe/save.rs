use std::path::PathBuf;
use bevy::math::DVec3;
use bevy::prelude::{Commands, Transform};
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::{Major, Minor};

pub struct UniverseFile {
    pub file: Option<PathBuf>,
    pub contents: UniverseFileContents,
}

#[derive(Debug)]
pub enum UniverseWriteError {
    TOML(toml::ser::Error),
    IO(std::io::Error),
}

impl UniverseFile {
    pub fn has_file(&self) -> bool {
        self.file.is_some()
    }

    pub fn save(&self) -> Result<(), UniverseWriteError> {
        if self.file.is_none() {
            return Err(UniverseWriteError::IO(std::io::Error::new(std::io::ErrorKind::Other, "File not found.")));
        }
        let contents = toml::to_string_pretty(&self.contents);
        if contents.is_err() {
            return Err(UniverseWriteError::TOML(contents.unwrap_err()));
        }
        let contents = contents.unwrap();
        let file_path = self.file.as_ref().unwrap();
        let file_result = std::fs::write(file_path, contents);
        if file_result.is_err() {
            let err = file_result.unwrap_err();
            return Err(UniverseWriteError::IO(err));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileContents {
    pub version: String,
    pub time: UniverseFileTime,
    pub bodies: Vec<SomeBody>,
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileTime {
    pub time: f64,
}

#[derive(Serialize, Deserialize)]
pub enum SomeBody {
    FixedEntry(FixedEntry),
    NewtonEntry(NewtonEntry),
    KeplerEntry(KeplerEntry),
    CompoundEntry(PatchedConicsEntry),
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    pub info: BodyInfo,
    pub position: DVec3,
}

impl FixedEntry {
    pub fn spawn(self, mut commands: &mut Commands) {
        let info = self.info;
        let motive = FixedMotive {
            position: self.position,
        };
        if info.major {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Minor,
                ));
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NewtonEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub velocity: DVec3,
}

impl NewtonEntry {
    pub fn spawn(self, mut commands: &mut Commands) {
        let info = self.info;
        let motive = NewtonMotive {
            position: self.position,
            velocity: self.velocity,
        };
        if info.major {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Minor,
                ));
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEntry {
    pub info: BodyInfo,
    pub params: KeplerMotive,
}

impl KeplerEntry {
    pub fn spawn(self, mut commands: &mut Commands) {
        let info = self.info;
        let motive = self.params;
        if info.major {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    Transform::default(),
                    // TODO: Mesh
                    info,
                    motive,
                    Major,
                ));
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct PatchedConicsEntry {
    pub info: BodyInfo,
    pub route: HashMap<u64, KeplerMotive>,
}

impl PatchedConicsEntry {
    pub fn spawn(self, mut commands: &mut Commands) {
        todo!()
    }
}
