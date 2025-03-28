use std::path::PathBuf;
use bevy::math::DVec3;
use bevy::prelude::{info, Assets, Commands, Entity, Image, Mesh, ResMut, Resource, StandardMaterial, Transform};
use bevy::render::view::NoFrustumCulling;
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::appearance::Appearance;
use crate::body::appearance::AssetCache;
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::SimulationObject;
use crate::body::universe::{Major, Minor};
use crate::gui::menu::TagState;

pub struct UniverseFile {
    pub(crate) file: Option<PathBuf>,
    pub contents: UniverseFileContents,
}

impl UniverseFile {
    pub(crate) fn load_from_path(path: &PathBuf) -> Option<Self> {
        let file_path = path.clone();
        let string = std::fs::read_to_string(path).unwrap(); // TODO: Handle errors? Or do we just assume we'd never call this with a bath path..?
        let contents: UniverseFileContents = toml::from_str(&string).unwrap(); // TODO: Handle errors.
        Some(Self {
            file: Some(file_path),
            contents,
        })
    }
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
    pub tags: HashMap<String, TagState>,
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
            tags: HashMap::new(),
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
}

#[derive(Serialize, Deserialize)]
pub enum SomeBody {
    FixedEntry(FixedEntry),
    NewtonEntry(NewtonEntry),
    KeplerEntry(KeplerEntry),
    CompoundEntry(PatchedConicsEntry),
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

        let (info, appearance) = match self {
            SomeBody::FixedEntry(entry) => {
                entity.insert(FixedMotive {
                    position: entry.position,
                });
                (entry.info, entry.appearance)
            },
            SomeBody::NewtonEntry(entry) => {
                entity.insert(NewtonMotive {
                    position: entry.position,
                    velocity: entry.velocity,
                });
                (entry.info, entry.appearance)
            },
            SomeBody::KeplerEntry(entry) => {
                entity.insert(entry.params);
                (entry.info, entry.appearance)
            },
            SomeBody::CompoundEntry(entry) => {
                // TODO: Insert when patched conics are implemented.
                (entry.info, entry.appearance)
            },
        };

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
        }
    }

    pub fn name(&self) -> String {
        match self {
            SomeBody::FixedEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::NewtonEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::KeplerEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::CompoundEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
        }
    }

    pub fn tags(&self) -> &Vec<String> {
        match self {
            SomeBody::FixedEntry(entry) => &entry.info.tags,
            SomeBody::NewtonEntry(entry) => &entry.info.tags,
            SomeBody::KeplerEntry(entry) => &entry.info.tags,
            SomeBody::CompoundEntry(entry) => &entry.info.tags,
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

#[derive(Serialize, Deserialize)]
pub struct PatchedConicsEntry {
    pub info: BodyInfo,
    pub route: HashMap<u64, KeplerMotive>,
    pub appearance: Appearance,
}
