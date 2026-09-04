//! Universe file I/O and spawning.
//!
//! The data model lives in `em_sim::universe`; this handles paths, formats, the SQLite
//! backend, and turning `SomeBody` entries into Bevy entities.

use std::path::PathBuf;
use std::ffi::OsStr;
use bevy::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use crate::body::appearance::{AssetCache, PbrBundle};
use crate::body::motive::info::{BodyInfo, BodyRotation, BodyState};
use crate::body::motive::Motive;
use crate::body::appearance::Appearance;
use bevy::math::DVec3;
use crate::sim::SimulationObject;
use crate::body::universe::{Major, Minor};
use crate::body::universe::save_sqlite;

/// Data types re-exported so `crate::body::universe::save::…` paths keep resolving.
pub use em_sim::universe::{
    TagState, UniverseFileContents, UniverseFileTime, UniversePhysics, ViewSettings,
    SomeBody, FixedEntry, NewtonEntry, KeplerEntry, PatchedConicsEntry, CompoundMotiveEntry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    /// TOML format (.toml)
    Toml,
    /// SQLite format (.em - Exotic Matters)
    Sqlite,
}

impl SaveFormat {
    /// Detect format from file extension
    pub fn from_path(path: &PathBuf) -> Option<Self> {
        match path.extension().and_then(OsStr::to_str) {
            Some("toml") => Some(SaveFormat::Toml),
            Some("em") => Some(SaveFormat::Sqlite),
            _ => None,
        }
    }

    /// Get the default extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            SaveFormat::Toml => "toml",
            SaveFormat::Sqlite => "em",
        }
    }
}

pub struct UniverseFile {
    pub(crate) file: Option<PathBuf>,
    pub contents: UniverseFileContents,
}

impl UniverseFile {
    /// Load from any supported format (auto-detected from extension)
    pub fn load_from_path(path: &PathBuf) -> Option<Self> {
        let format = SaveFormat::from_path(path)?;
        match format {
            SaveFormat::Toml => Self::load_from_path_toml(path),
            SaveFormat::Sqlite => Self::load_from_path_sqlite(path),
        }
    }

    /// Load from TOML format
    pub fn load_from_path_toml(path: &PathBuf) -> Option<Self> {
        let file_path = path.clone();
        let string = std::fs::read_to_string(path).ok()?;
        let contents: UniverseFileContents = toml::from_str(&string).ok()?;
        Some(Self {
            file: Some(file_path),
            contents,
        })
    }

    /// Load from SQLite (.em) format
    pub fn load_from_path_sqlite(path: &PathBuf) -> Option<Self> {
        let file_path = path.clone();
        let contents = save_sqlite::load_from_em(path).ok()?;
        Some(Self {
            file: Some(file_path),
            contents,
        })
    }
}

#[derive(Debug)]
pub enum UniverseWriteError {
    Toml(toml::ser::Error),
    Sqlite(save_sqlite::SqliteSaveError),
    IO(std::io::Error),
    UnknownFormat,
}

impl From<save_sqlite::SqliteSaveError> for UniverseWriteError {
    fn from(e: save_sqlite::SqliteSaveError) -> Self {
        UniverseWriteError::Sqlite(e)
    }
}

impl UniverseFile {
    pub fn has_file(&self) -> bool {
        self.file.is_some()
    }

    /// Save to the file (format auto-detected from extension)
    pub fn save(&self) -> Result<(), UniverseWriteError> {
        let path = self.file.as_ref()
            .ok_or_else(|| UniverseWriteError::IO(std::io::Error::new(
                std::io::ErrorKind::Other, 
                "No file path set"
            )))?;
        
        let format = SaveFormat::from_path(path)
            .ok_or(UniverseWriteError::UnknownFormat)?;
        
        match format {
            SaveFormat::Toml => self.save_toml(),
            SaveFormat::Sqlite => self.save_sqlite(),
        }
    }

    /// Save to TOML format
    pub fn save_toml(&self) -> Result<(), UniverseWriteError> {
        let path = self.file.as_ref()
            .ok_or_else(|| UniverseWriteError::IO(std::io::Error::new(
                std::io::ErrorKind::Other, 
                "No file path set"
            )))?;
        
        let contents = toml::to_string_pretty(&self.contents)
            .map_err(UniverseWriteError::Toml)?;
        
        std::fs::write(path, contents)
            .map_err(UniverseWriteError::IO)?;
        
        Ok(())
    }

    /// Save to SQLite (.em) format
    pub fn save_sqlite(&self) -> Result<(), UniverseWriteError> {
        let path = self.file.as_ref()
            .ok_or_else(|| UniverseWriteError::IO(std::io::Error::new(
                std::io::ErrorKind::Other, 
                "No file path set"
            )))?;
        
        save_sqlite::save_to_em(path, &self.contents)?;
        Ok(())
    }

    /// Save to a specific path with the given format
    pub fn save_as(&mut self, path: PathBuf, format: SaveFormat) -> Result<(), UniverseWriteError> {
        // Update the path with the correct extension if needed
        let path = if path.extension().and_then(OsStr::to_str) != Some(format.extension()) {
            path.with_extension(format.extension())
        } else {
            path
        };
        
        self.file = Some(path);
        self.save()
    }
}


/// Turns a saved body entry into a live entity. Was `SomeBody::spawn`; a trait now,
/// because `SomeBody` lives in `em-sim` and inherent impls on foreign types are not
/// allowed.
pub trait SpawnBody {
    fn spawn(
        self,
        commands: &mut Commands,
        cache: &mut ResMut<AssetCache>,
        meshes: &mut ResMut<Assets<Mesh>>,
        materials: &mut ResMut<Assets<StandardMaterial>>,
        images: &mut ResMut<Assets<Image>>,
    ) -> Entity;
}

impl SpawnBody for SomeBody {
    fn spawn(
        self,
        commands: &mut Commands,
        cache: &mut ResMut<AssetCache>,
        meshes: &mut ResMut<Assets<Mesh>>,
        materials: &mut ResMut<Assets<StandardMaterial>>,
        images: &mut ResMut<Assets<Image>>,
    )  -> Entity {
        let mut entity = commands.spawn((
            SimulationObject,
            Transform::default(),
            BodyState::default(),
        ));

        let (info, appearance, motive, rotation) = match self {
            SomeBody::FixedEntry(entry) => {
                // Convert legacy FixedEntry to Motive with single Fixed entry at Epoch
                let motive = Motive::fixed(entry.position);
                (entry.info, entry.appearance, motive, entry.rotation)
            },
            SomeBody::NewtonEntry(entry) => {
                // Convert legacy NewtonEntry to Motive with single Newtonian entry at Epoch
                let motive = Motive::newtonian(entry.position, entry.velocity);
                (entry.info, entry.appearance, motive, entry.rotation)
            },
            SomeBody::KeplerEntry(entry) => {
                // Convert legacy KeplerEntry to Motive with single Keplerian entry at Epoch
                let motive = Motive::keplerian(
                    entry.params.primary_id.clone(),
                    entry.params.shape,
                    entry.params.rotation,
                    entry.params.epoch,
                );
                (entry.info, entry.appearance, motive, entry.rotation)
            },
            SomeBody::CompoundEntry(entry) => {
                // Legacy patched conics - create empty motive for now
                // TODO: Convert old route HashMap to new Motive format if needed
                let motive = Motive::fixed(DVec3::ZERO);
                (entry.info, entry.appearance, motive, None)
            },
            SomeBody::CompoundMotiveEntry(entry) => {
                // New compound motive format - use directly
                (entry.info, entry.appearance, entry.motive, entry.rotation)
            },
        };

        // Insert the compound motive
        entity.insert(motive);

        // Insert body rotation if present
        if let Some(body_rotation) = rotation {
            entity.insert(body_rotation);
        }

        if info.major {
            entity.insert(Major);
        } else {
            entity.insert(Minor);
        }
        entity.insert(info);

        match &appearance {
            Appearance::Empty => {}
            Appearance::DebugBall(debug_ball) => {
                let (mesh, material) = debug_ball.pbr_bundle(cache, meshes, materials, images);
                entity.insert(mesh);
                entity.insert(material);
            }
            Appearance::Star(star_ball) => {
                let (mesh, material, light) = star_ball.pbr_bundle(cache, meshes, materials, images);
                entity.insert(mesh);
                entity.insert(material);
                entity.insert(light);
                entity.insert(NoFrustumCulling);
            }
        }
        entity.insert(appearance);

        entity.id()
    }

}
