use bevy::math::{DVec2, DVec3, DMat3};
use serde::{Deserialize, Serialize};
use bevy::prelude::{Component};
use crate::util::kepler::{angular_motion, apoapsis, eccentric_anomaly, eccentricity, local, mean_anomaly, periapsis, period, semi_latus_rectum, semi_major_axis, semi_minor_axis, semi_parameter, true_anomaly};

#[derive(Serialize, Deserialize, Component)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
}

const J2000_JD: f64 = 2451545.0;
const EXPANSION_ITERATIONS: usize = 10;

impl KeplerMotive {
    pub fn semi_major_axis(&self) -> f64 {
        self.shape.semi_major_axis()
    }

    pub fn semi_minor_axis(&self) -> f64 {
        self.shape.semi_minor_axis()
    }

    pub fn eccentricity(&self) -> f64 {
        self.shape.eccentricity()
    }

    pub fn periapsis(&self) -> f64 {
        self.shape.periapsis()
    }

    pub fn semi_latus_rectum(&self) -> f64 {
        self.shape.semi_latus_rectum()
    }

    pub fn semi_parameter(&self) -> f64 {
        self.shape.semi_parameter()
    }

    pub fn apoapsis(&self) -> f64 {
        self.shape.apoapsis()
    }

    pub fn periapsis_vec(&self) -> DVec3 { todo!() }
    pub fn apoapsis_vec(&self) -> DVec3 { todo!() }

    pub fn inclination(&self) -> f64 {
        self.rotation.inclination()
    }

    /// For earth satellites, the equator.
    /// For solar satellites, the ecliptic
    pub fn is_coplanar(&self) -> bool {
        self.rotation.no_inclination()
    }

    pub fn longitude_of_ascending_node(&self) -> Option<f64> {
        self.rotation.longitude_of_ascending_node()
    }

    /// Defines 0.0 inclination case as having 0.0 long of asc node
    pub fn longitude_of_ascending_node_infallible(&self) -> f64 {
        self.rotation.longitude_of_ascending_node().unwrap_or(0.0)
    }

    pub fn longitude_of_periapsis(&self) -> f64 {
        self.rotation.longitude_of_periapsis()
    }

    pub fn argument_of_periapsis(&self) -> f64 {
        self.rotation.argument_of_periapsis()
    }

    pub fn period(&self, gravitational_parameter: f64) -> f64 {
        period::third_law(self.semi_major_axis(), gravitational_parameter)
    }

    pub fn mean_angular_motion(&self, gravitational_parameter: f64) -> f64 {
        angular_motion::mean(gravitational_parameter, self.semi_major_axis())
    }

    pub fn mean_anomaly(&self, time: f64, gravitational_parameter: f64) -> f64 {
        let mean_anomaly_at_epoch = self.epoch.mean_anomaly_at_epoch();
        let sma = self.shape.semi_major_axis();
        let epoch_time = self.epoch.epoch();
        mean_anomaly::definition(mean_anomaly_at_epoch, gravitational_parameter, sma, epoch_time, time)
    }

    pub fn true_anomaly(&self, time: f64, gravitational_parameter: f64) -> f64 {
        true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS)
    }

    pub fn radius_from_primary(&self, time: f64, gravitational_parameter: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), ecc, EXPANSION_ITERATIONS);
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)
    }

    pub fn eccentric_anomaly(&self, time: f64, gravitational_parameter: f64) -> f64 {
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS);
        eccentric_anomaly::from_true_anomaly(self.shape.eccentricity(), ta)
    }

    /// Perifocal Frame
    /// +P (+x) points to periapsis
    /// +Q (+y) points toward motion at periapsis, normal to P
    /// +W (+z) normal to the other 2 according to RHR
    pub fn displacement_pqw(&self, time: f64, gravitational_parameter: f64) -> Option<DVec3> {
        let rad = self.radius_from_primary(time, gravitational_parameter)?;
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS);
        Some(DVec3::new(rad * ta.cos(), rad * ta.sin(), 0.0))
    }

    pub fn displacement(&self, time: f64, gravitational_parameter: f64) -> Option<DVec3> {
        let perifocal_displacement = self.displacement_pqw(time, gravitational_parameter)?;

        let rot_arg_peri = DMat3::from_rotation_z(self.argument_of_periapsis().to_radians());
        let rot_inc = DMat3::from_rotation_x(self.inclination().to_radians());
        let rot_long_asc_node = DMat3::from_rotation_z(self.longitude_of_ascending_node_infallible());

        Some(rot_long_asc_node * rot_inc * rot_arg_peri * perifocal_displacement)
    }
}

#[derive(Serialize, Deserialize)]
pub enum KeplerShape {
    EccentricitySMA(EccentricitySMA),
    Apsides(Apsides),
}

impl KeplerShape {
    fn semi_major_axis(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                esma.semi_major_axis
            }
            KeplerShape::Apsides(apsides) => {
                semi_major_axis::radii(apsides.periapsis, apsides.apoapsis)
            }
        }
    }

    fn semi_minor_axis(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                semi_minor_axis::conic_definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(apsides) => {
                let sma = semi_major_axis::radii(apsides.periapsis, apsides.apoapsis);
                let ecc = eccentricity::radii(apsides.periapsis, apsides.apoapsis);
                semi_minor_axis::conic_definition(sma, ecc)
            }
        }
    }

    fn eccentricity(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => esma.eccentricity,
            KeplerShape::Apsides(apsides) => {
                eccentricity::radii(apsides.periapsis, apsides.apoapsis)
            }
        }
    }

    fn periapsis(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                periapsis::definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(apsides) => apsides.periapsis,
        }
    }

    fn apoapsis(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                apoapsis::definition(esma.semi_major_axis, esma.eccentricity).unwrap_or(f64::INFINITY)
            }
            KeplerShape::Apsides(apsides) => apsides.apoapsis,
        }
    }

    fn semi_parameter(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                semi_parameter::definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(apsides) => {
                let sma = self.semi_major_axis();
                let ecc = self.eccentricity();
                semi_parameter::definition(sma, ecc)
            }
        }
    }

    fn semi_latus_rectum(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                semi_latus_rectum::conic_definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(apsides) => {
                let sma = self.semi_major_axis();
                let ecc = self.eccentricity();
                semi_latus_rectum::conic_definition(sma, ecc)
            }
        }
    }
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
    FlatAngles(KeplerFlatAngles),
}

impl KeplerRotation {
    pub fn inclination(&self) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.inclination,
            KeplerRotation::FlatAngles(flat) => 0.0,
        }
    }

    pub fn no_inclination(&self) -> bool {
        self.inclination() < f64::EPSILON
    }

    pub fn longitude_of_ascending_node(&self) -> Option<f64> {
        if self.no_inclination() { return None; }
        match self {
            KeplerRotation::EulerAngles(ea) => Some(ea.longitude_of_ascending_node),
            KeplerRotation::FlatAngles(_) => None,
        }
    }

    pub fn longitude_of_periapsis(&self) -> f64 {
        self.longitude_of_ascending_node().unwrap_or(0.0) + self.argument_of_periapsis()
    }

    pub fn argument_of_periapsis(&self) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.argument_of_periapsis,
            KeplerRotation::FlatAngles(flat) => flat.longitude_of_periapsis,
        }
    }


}

#[derive(Serialize, Deserialize)]
pub struct KeplerEulerAngles {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerFlatAngles {
    pub longitude_of_periapsis: f64,
}

#[derive(Serialize, Deserialize)]
pub enum KeplerEpoch {
    MeanAnomaly(MeanAnomalyAtEpoch),
    TimeAtPeriapsisPassage(PeriapsisTime),
    TrueAnomaly(TrueAnomalyAtEpoch),
    J2000(MeanAnomalyAtJ2000),
}

impl KeplerEpoch {
    pub fn epoch(&self) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(maae) => {
                maae.epoch
            }
            KeplerEpoch::TimeAtPeriapsisPassage(_) => {
                todo!()
            }
            KeplerEpoch::TrueAnomaly(taae) => {
                taae.epoch
            }
            KeplerEpoch::J2000(_) => { 0.0 }
        }
    }

    pub fn mean_anomaly_at_epoch(&self) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => {
                mean_anomaly.mean_anomaly
            }
            KeplerEpoch::TimeAtPeriapsisPassage(_) => {
                todo!()
            }
            KeplerEpoch::TrueAnomaly(_) => { todo!() }
            KeplerEpoch::J2000(j2000) => {
                j2000.mean_anomaly
            }
        }
    }
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
