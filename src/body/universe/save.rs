use std::path::PathBuf;
use std::ffi::OsStr;
use bevy::math::DVec3;
use bevy::prelude::*;
use std::collections::HashMap;
use bevy::camera::visibility::NoFrustumCulling;
use serde::{Deserialize, Serialize};
use crate::body::appearance::Appearance;
use crate::body::appearance::AssetCache;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::Motive;
use crate::body::SimulationObject;
use crate::body::universe::{Major, Minor};
use crate::body::universe::save_sqlite;
use crate::gui::menu::TagState;
use crate::util::mappings;

/// Supported save file formats
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

#[derive(Serialize, Deserialize)]
pub struct UniverseFileContents {
    pub version: String,
    pub time: UniverseFileTime,
    pub view: ViewSettings,
    pub physics: UniversePhysics,
    pub bodies: Vec<SomeBody>,
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileTime {
    pub time_julian_days: f64, // In Julian Days
}

#[derive(Resource, Serialize, Deserialize)]
pub struct UniversePhysics {
    pub gravitational_constant: f64,
}

impl Default for UniversePhysics {
    fn default() -> Self {
        Self {
            gravitational_constant: 6.6743015e-11, // Standard G in m³ kg⁻¹ s⁻²
        }
    }
}

#[derive(Serialize, Deserialize, Resource, Debug)]
pub struct ViewSettings {
    pub distance_scale: f64,
    pub logarithmic_distance_scale: bool,
    pub logarithmic_distance_base: f64,
    pub body_scale: f64,
    pub logarithmic_body_scale: bool,
    pub logarithmic_body_base: f64,
    pub show_labels: bool,
    pub show_trajectories: bool,
    pub tags: HashMap<String, TagState>,
    pub trajectory_resolution: usize,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            distance_scale: 1e-9,
            logarithmic_body_scale: false,
            logarithmic_body_base: 10.0,
            body_scale: 1e-9,
            logarithmic_distance_scale: false,
            logarithmic_distance_base: 10.0,
            show_labels: true,
            show_trajectories: true,
            tags: HashMap::new(),
            trajectory_resolution: 120,
        }
    }
}

impl ViewSettings {
    pub fn body_in_any_visible_tag<T:AsRef<str> + ToString>(&self, body_id: T) -> bool {
        for tag in self.tags.values() {
            if tag.shown && tag.members.contains(&body_id.to_string()) {
                return true;
            }
        }
        false
    }

    pub fn body_in_any_trajectory_tag<T:AsRef<str> + ToString>(&self, body_id: T) -> bool {
        for tag in self.tags.values() {
            if tag.trajectory && tag.members.contains(&body_id.to_string()) {
                return true;
            }
        }
        false
    }
    
    pub fn distance_factor(&self) -> f64 {
        if self.logarithmic_distance_scale {
            mappings::log_scale(self.distance_scale, self.logarithmic_distance_base)
        } else {
            self.distance_scale
        }
    }
    
    pub fn body_scale_factor(&self, radius: f64) -> f32 {
        let n = if self.logarithmic_body_scale {
            mappings::log_scale(radius, self.logarithmic_body_base) * self.body_scale
        } else {
            radius * self.body_scale
        } as f32;
        n
    }
}

#[derive(Serialize, Deserialize)]
pub enum SomeBody {
    /// Legacy fixed motive - loaded as Motive with single Fixed entry
    FixedEntry(FixedEntry),
    /// Legacy newton motive - loaded as Motive with single Newtonian entry
    NewtonEntry(NewtonEntry),
    /// Legacy kepler motive - loaded as Motive with single Keplerian entry
    KeplerEntry(KeplerEntry),
    /// Legacy patched conics - deprecated, kept for backward compatibility
    CompoundEntry(PatchedConicsEntry),
    /// New compound motive format - supports multiple motive types with transitions
    CompoundMotiveEntry(CompoundMotiveEntry),
}

impl SomeBody {
    pub fn spawn(
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

        let (info, appearance, motive) = match self {
            SomeBody::FixedEntry(entry) => {
                // Convert legacy FixedEntry to Motive with single Fixed entry at Epoch
                let motive = Motive::fixed(entry.position);
                (entry.info, entry.appearance, motive)
            },
            SomeBody::NewtonEntry(entry) => {
                // Convert legacy NewtonEntry to Motive with single Newtonian entry at Epoch
                let motive = Motive::newtonian(entry.position, entry.velocity);
                (entry.info, entry.appearance, motive)
            },
            SomeBody::KeplerEntry(entry) => {
                // Convert legacy KeplerEntry to Motive with single Keplerian entry at Epoch
                let motive = Motive::keplerian(
                    entry.params.primary_id.clone(),
                    entry.params.shape,
                    entry.params.rotation,
                    entry.params.epoch,
                );
                (entry.info, entry.appearance, motive)
            },
            SomeBody::CompoundEntry(entry) => {
                // Legacy patched conics - create empty motive for now
                // TODO: Convert old route HashMap to new Motive format if needed
                let motive = Motive::fixed(DVec3::ZERO);
                (entry.info, entry.appearance, motive)
            },
            SomeBody::CompoundMotiveEntry(entry) => {
                // New compound motive format - use directly
                (entry.info, entry.appearance, entry.motive)
            },
        };

        // Insert the compound motive
        entity.insert(motive);

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

    pub fn id(&self) -> String {
        match self {
            SomeBody::FixedEntry(entry) => (&entry.info.id).clone(),
            SomeBody::NewtonEntry(entry) => (&entry.info.id).clone(),
            SomeBody::KeplerEntry(entry) => (&entry.info.id).clone(),
            SomeBody::CompoundEntry(entry) => (&entry.info.id).clone(),
            SomeBody::CompoundMotiveEntry(entry) => (&entry.info.id).clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            SomeBody::FixedEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::NewtonEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::KeplerEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::CompoundEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::CompoundMotiveEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
        }
    }

    pub fn tags(&self) -> &Vec<String> {
        match self {
            SomeBody::FixedEntry(entry) => &entry.info.tags,
            SomeBody::NewtonEntry(entry) => &entry.info.tags,
            SomeBody::KeplerEntry(entry) => &entry.info.tags,
            SomeBody::CompoundEntry(entry) => &entry.info.tags,
            SomeBody::CompoundMotiveEntry(entry) => &entry.info.tags,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub appearance: Appearance,
}

#[derive(Serialize, Deserialize)]
pub struct NewtonEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub velocity: DVec3,
    pub appearance: Appearance,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEntry {
    pub info: BodyInfo,
    pub params: KeplerMotive,
    pub appearance: Appearance,
}

/// Legacy format - use CompoundMotiveEntry for new saves
#[derive(Serialize, Deserialize)]
pub struct PatchedConicsEntry {
    pub info: BodyInfo,
    pub route: HashMap<u64, KeplerMotive>,
    pub appearance: Appearance,
}

/// The new compound motive format that supports motive transitions over time
#[derive(Serialize, Deserialize)]
pub struct CompoundMotiveEntry {
    pub info: BodyInfo,
    pub motive: Motive,
    pub appearance: Appearance,
}
