use std::path::PathBuf;
use bevy::math::DVec3;
use serde::{Deserialize, Serialize};

pub struct UniverseFile {
    file: Option<PathBuf>,
    contents: UniverseFileContents,
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileContents {
    time: UniverseFileTime,
    bodies: Vec<SomeBody>
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileTime {
    time: f64,
}

#[derive(Serialize, Deserialize)]
pub enum SomeBody {
    FixedEntry(FixedEntry),
    MajorNewtonEntry(NewtonEntry),
    MajorKeplerEntry(KeplerEntry),
}

#[derive(Serialize, Deserialize)]
pub struct BodyInfo {
    name: Option<String>,
    id: String,
    mass: f64,
    major: bool,
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    info: BodyInfo,
    position: DVec3,
}

#[derive(Serialize, Deserialize)]
pub struct NewtonEntry {
    info: BodyInfo,
    position: DVec3,
    velocity: DVec3,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEntry {
    info: BodyInfo,
    primary_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerParams {
    pub shape: KeplerShapeParams,
    pub rotation: KeplerRotationParams,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerShapeParams {
    Standard(StandardKeplerShapeParams),
}

#[derive(Serialize, Deserialize)]
pub struct StandardKeplerShapeParams {
    eccentricity: f64,
    semi_major_axis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerRotationParams {
    Standard(StandardKeplerRotationParams),
}

#[derive(Serialize, Deserialize)]
pub struct StandardKeplerRotationParams {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64,
    pub argument_of_periapsis: f64,
}
