use std::path::PathBuf;
use bevy::math::DVec3;
use bevy::utils::HashMap;
use serde::{Deserialize, Serialize};

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
pub struct BodyInfo {
    pub name: Option<String>,
    pub id: String,
    pub mass: f64,
    pub major: bool,
    pub designation: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    pub info: BodyInfo,
    pub position: DVec3,
}

#[derive(Serialize, Deserialize)]
pub struct NewtonEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub velocity: DVec3,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEntry {
    pub info: BodyInfo,
    pub params: KeplerParams,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerParams {
    pub primary_id: String,
    pub shape: KeplerShapeParams,
    pub rotation: KeplerRotationParams,
    pub epoch: KeplerEpochParams,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerShapeParams {
    EccentricitySMA(EccentricitySMAParams),
    Apsides(ApsidesParams),
}

#[derive(Serialize, Deserialize)]
pub struct EccentricitySMAParams {
    pub eccentricity: f64,
    pub semi_major_axis: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ApsidesParams {
    pub periapsis: f64,
    pub apoapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerRotationParams {
    EulerAngles(KeplerEulerAngleParams),
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEulerAngleParams {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerEpochParams {
    MeanAnomaly(TrueAnomalyAtEpochParams),
    TimeAtPeriapsisPassage(PeriapsisTimeParams),
    TrueAnomaly(TrueAnomalyAtEpochParams),
    J2000(MeanAnomalyAtJ2000),
}

#[derive(Serialize, Deserialize)]
pub struct MeanAnomalyAtEpochParams {
    pub epoch: f64,
    pub mean_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct PeriapsisTimeParams {
    pub time: f64,
}

#[derive(Serialize, Deserialize)]
pub struct TrueAnomalyAtEpochParams {
    pub epoch: f64,
    pub mean_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct MeanAnomalyAtJ2000 {
    pub mean_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct PatchedConicsEntry {
    info: BodyInfo,
    route: HashMap<u64, KeplerParams>,
}
