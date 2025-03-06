use serde::{Deserialize, Serialize};
use bevy::prelude::Component;

#[derive(Serialize, Deserialize, Component)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerShape {
    EccentricitySMA(EccentricitySMA),
    Apsides(Apsides),
}

#[derive(Serialize, Deserialize)]
pub struct EccentricitySMA {
    pub eccentricity: f64,
    pub semi_major_axis: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Apsides {
    pub periapsis: f64,
    pub apoapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerRotation {
    EulerAngles(KeplerEulerAngles),
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEulerAngles {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerEpoch {
    MeanAnomaly(MeanAnomalyAtEpoch),
    TimeAtPeriapsisPassage(PeriapsisTime),
    TrueAnomaly(TrueAnomalyAtEpoch),
    J2000(MeanAnomalyAtJ2000),
}

#[derive(Serialize, Deserialize)]
pub struct MeanAnomalyAtEpoch {
    pub epoch: f64,
    pub mean_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct PeriapsisTime {
    pub time: f64,
}

#[derive(Serialize, Deserialize)]
pub struct TrueAnomalyAtEpoch {
    pub epoch: f64,
    pub true_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct MeanAnomalyAtJ2000 {
    pub mean_anomaly: f64,
}
