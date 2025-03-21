use std::path::PathBuf;
use bevy::math::DVec3;
use bevy::prelude::{info, Assets, Commands, Image, Mesh, ResMut, Resource, StandardMaterial, Transform};
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};
use crate::body::appearance::Appearance;
use crate::body::appearance::AssetCache;
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::SimulationObject;
use crate::body::universe::{Major, Minor};


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

#[derive(Serialize, Deserialize, Resource)]
pub struct ViewSettings {
    pub distance_scale: f64,
    pub body_scale: f64,
    pub show_labels: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            distance_scale: 1e-9,
            body_scale: 1e-9,
            show_labels: true,
        }
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
    ) {
        match self {
            SomeBody::FixedEntry(entry) => entry.spawn(commands, cache, meshes, materials, images),
            SomeBody::NewtonEntry(entry) => entry.spawn(commands, cache, meshes, materials, images),
            SomeBody::KeplerEntry(entry) => entry.spawn(commands, cache, meshes, materials, images),
            SomeBody::CompoundEntry(entry) => entry.spawn(commands, cache, meshes, materials, images),
        }
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
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub appearance: Appearance,
}

impl FixedEntry {
    pub fn spawn(
        self,
        mut commands: &mut Commands,
        mut cache: &mut ResMut<AssetCache>,
        mut meshes: &mut ResMut<Assets<Mesh>>,
        mut materials: &mut ResMut<Assets<StandardMaterial>>,
        mut images: &mut ResMut<Assets<Image>>,
    ) {
        let info = self.info;
        let motive = FixedMotive {
            position: self.position,
        };
        let (mesh, material) = self.appearance.pbr_bundle(&mut cache, &mut meshes, &mut materials, images);

        if info.major {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
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
    pub appearance: Appearance,
}

impl NewtonEntry {
    pub fn spawn(
        self,
        mut commands: &mut Commands,
        mut cache: &mut ResMut<AssetCache>,
        mut meshes: &mut ResMut<Assets<Mesh>>,
        mut materials: &mut ResMut<Assets<StandardMaterial>>,
        mut images: &mut ResMut<Assets<Image>>,
    ) {
        let info = self.info;
        let motive = NewtonMotive {
            position: self.position,
            velocity: self.velocity,
        };
        let (mesh, material) = self.appearance.pbr_bundle(&mut cache, &mut meshes, &mut materials, images);

        if info.major {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
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
    pub appearance: Appearance,
}

impl KeplerEntry {
    pub fn spawn(
        self,
        mut commands: &mut Commands,
        mut cache: &mut ResMut<AssetCache>,
        mut meshes: &mut ResMut<Assets<Mesh>>,
        mut materials: &mut ResMut<Assets<StandardMaterial>>,
        mut images: &mut ResMut<Assets<Image>>,
    ) {
        info!("Spawning KeplerEntry {:?}", self.info.name);
        let info = self.info;
        let motive = self.params;
        let (mesh, material) = self.appearance.pbr_bundle(&mut cache, &mut meshes, &mut materials, &mut images);

        if info.major {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
                    info,
                    motive,
                    Major,
                ));
        } else {
            commands
                .spawn((
                    SimulationObject,
                    Transform::default(),
                    mesh,
                    material,
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
    pub appearance: Appearance,
}

impl PatchedConicsEntry {
    pub fn spawn(
        self,
        mut commands: &mut Commands,
        mut cache: &mut ResMut<AssetCache>,
        mut meshes: &mut ResMut<Assets<Mesh>>,
        mut materials: &mut ResMut<Assets<StandardMaterial>>,
        mut images: &mut ResMut<Assets<Image>>,
    ) {
        todo!()
    }
}
