//! Keplerian orbital elements and the position they imply.
//!
//! Pure data plus the functions over it. Systems that drive this from an ECS live in
//! the app crate.

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use em_foundations::kepler::{angular_motion, apoapsis, eccentric_anomaly, eccentricity, local, mean_anomaly, periapsis, period, semi_latus_rectum, semi_major_axis, semi_minor_axis, semi_parameter, true_anomaly};
use em_foundations::time::{Includes, Instant, TimeDelta, TimeLength};
use em_foundations::mappings;

#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[derive(Serialize, Deserialize, Clone)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
}

const EXPANSION_ITERATIONS: usize = 10;

/// Inclinations below this (in degrees) are treated as coplanar.
/// `f64::EPSILON` (2.2e-16) is meaningless as an angular tolerance.
const ANGLE_EPSILON_DEG: f64 = 1e-9;

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

    pub fn is_open(&self) -> bool {
        self.eccentricity() >= 1.0
    }

    pub fn periapsis(&self) -> f64 {
        self.shape.periapsis()
    }

    pub fn time_at_periapsis_passage(&self, gravitational_parameter: f64) -> Instant {
        let period = self.period(gravitational_parameter);
        self.epoch.time_at_periapsis_passage(period)
    }

    pub fn semi_latus_rectum(&self) -> f64 {
        self.shape.semi_latus_rectum()
    }

    pub fn semi_parameter(&self) -> f64 {
        self.shape.semi_parameter()
    }

    pub fn apoapsis(&self) -> Option<f64> {
        self.shape.apoapsis()
    }

    pub fn periapsis_vec_pqw(&self) -> DVec3 {
        let rad = self.shape.periapsis();
        DVec3::new(rad, 0.0, 0.0)
    }

    pub fn apoapsis_vec_pqw(&self) -> Option<DVec3> {
        self.shape.apoapsis().map(|apoapsis| DVec3::new(-apoapsis, 0.0, 0.0))
    }

    pub fn periapsis_vec(&self, time: Instant) -> DVec3 {
        let perifocal_displacement = self.periapsis_vec_pqw();
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);

        rotated
    }

    pub fn apoapsis_vec(&self, time: Instant) -> Option<DVec3> {
        let perifocal_displacement = self.apoapsis_vec_pqw()?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);

        Some(rotated)
    }

    pub fn inclination(&self) -> f64 {
        self.rotation.inclination()
    }

    /// For earth satellites, the equator.
    /// For solar satellites, the ecliptic
    pub fn is_coplanar(&self) -> bool {
        self.rotation.no_inclination()
    }

    pub fn longitude_of_ascending_node(&self, time: Instant) -> Option<f64> {
        let time_since_epoch = self.time_since_epoch(time);
        self.rotation.longitude_of_ascending_node(time_since_epoch)
    }

    /// Lets 0.0 inclination case have long of asc node
    pub fn longitude_of_ascending_node_infallible(&self, time: Instant) -> f64 {
        let time_since_epoch = self.time_since_epoch(time);
        self.rotation.longitude_of_ascending_node_infallible(time_since_epoch)
    }

    pub fn longitude_of_periapsis(&self, time: Instant) -> f64 {
        let time_since_epoch = self.time_since_epoch(time);
        self.rotation.longitude_of_periapsis(time_since_epoch)
    }

    pub fn argument_of_periapsis(&self, time: Instant) -> f64 {
        let time_since_epoch = self.time_since_epoch(time);
        self.rotation.argument_of_periapsis(time_since_epoch)
    }
    
    pub fn time_since_epoch(&self, time: Instant) -> TimeDelta {
        time - self.epoch.epoch()
    }

    pub fn period(&self, gravitational_parameter: f64) -> TimeLength {
        TimeLength::from_seconds(period::third_law(self.semi_major_axis(), gravitational_parameter), Includes::Beginning)
    }

    pub fn mean_angular_motion(&self, gravitational_parameter: f64) -> f64 {
        angular_motion::mean(gravitational_parameter, self.semi_major_axis())
    }

    /// Returns mean anomaly at the given time in radians.
    pub fn mean_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        // mean_anomaly_at_epoch is stored in degrees; convert to radians for the math
        let mean_anomaly_at_epoch_rad = self.epoch.mean_anomaly_at_epoch().to_radians();
        let sma = self.shape.semi_major_axis();
        let epoch_time = self.epoch.epoch();
        mean_anomaly::definition(mean_anomaly_at_epoch_rad, gravitational_parameter, sma, epoch_time.to_j2000_seconds(), time.to_j2000_seconds())
    }

    pub fn true_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS)
    }

    pub fn radius_from_primary_at_time(&self, time: Instant, gravitational_parameter: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), ecc, EXPANSION_ITERATIONS);
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)
    }

    pub fn radius_from_primary_at_true_anomaly(&self, true_anomaly: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)
    }

    pub fn eccentric_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS);
        eccentric_anomaly::from_true_anomaly(self.shape.eccentricity(), ta)
    }

    /// Perifocal Frame
    /// +P (+x) points to periapsis
    /// +Q (+y) points toward motion at periapsis, normal to P
    /// +W (+z) normal to the other 2 according to RHR
    pub fn displacement_pqw(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), ecc, EXPANSION_ITERATIONS);
        let rad = local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)?;

        Some(DVec3::new(rad * ta.cos(), rad * ta.sin(), 0.0))
    }

    pub fn displacement(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let perifocal_displacement = self.displacement_pqw(time, gravitational_parameter)?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);

        Some(rotated)
    }

    fn perifocal_to_reference(&self, perifocal_displacement: DVec3, time: Instant) -> DVec3 {
        let rot_arg_peri = DMat3::from_rotation_z(self.argument_of_periapsis(time).to_radians());
        let rot_inc = DMat3::from_rotation_x(self.inclination().to_radians());
        let rot_long_asc_node = DMat3::from_rotation_z(self.longitude_of_ascending_node_infallible(time).to_radians());

        rot_long_asc_node * rot_inc * rot_arg_peri * perifocal_displacement
    }

}

#[derive(Serialize, Deserialize, Clone)]
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

    fn apoapsis(&self) -> Option<f64> {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                apoapsis::definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(apsides) => Some(apsides.apoapsis),
        }
    }

    fn semi_parameter(&self) -> f64 {
        match self {
            KeplerShape::EccentricitySMA(esma) => {
                semi_parameter::definition(esma.semi_major_axis, esma.eccentricity)
            }
            KeplerShape::Apsides(_) => {
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
            KeplerShape::Apsides(_) => {
                let sma = self.semi_major_axis();
                let ecc = self.eccentricity();
                semi_latus_rectum::conic_definition(sma, ecc)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct EccentricitySMA {
    pub eccentricity: f64,
    pub semi_major_axis: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Apsides {
    pub periapsis: f64,
    pub apoapsis: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum KeplerRotation {
    EulerAngles(KeplerEulerAngles),
    FlatAngles(KeplerFlatAngles),
    PrecessingEulerAngles(KeplerPrecessingEulerAngles),
}

impl KeplerRotation {
    pub fn inclination(&self) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.inclination,
            KeplerRotation::FlatAngles(_) => 0.0,
            KeplerRotation::PrecessingEulerAngles(pea) => pea.inclination,
        }
    }

    pub fn no_inclination(&self) -> bool {
        self.inclination().abs() < ANGLE_EPSILON_DEG
    }

    pub fn longitude_of_ascending_node_infallible(&self, time_since_epoch: TimeDelta) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.longitude_of_ascending_node,
            KeplerRotation::FlatAngles(_) => 0.0,
            KeplerRotation::PrecessingEulerAngles(pea) => {
                let deg = pea.nodal_precession_deg(time_since_epoch);
                let long = mappings::bound_circle(pea.longitude_of_ascending_node + deg, 360.0);
                long
            }
        }
    }

    pub fn longitude_of_ascending_node(&self, time_since_epoch: TimeDelta) -> Option<f64> {
        if self.no_inclination() { return None; }
        match self {
            KeplerRotation::EulerAngles(ea) => Some(ea.longitude_of_ascending_node),
            KeplerRotation::FlatAngles(_) => None,
            KeplerRotation::PrecessingEulerAngles(pea) => {
                let deg = pea.nodal_precession_deg(time_since_epoch);
                let long = mappings::bound_circle(pea.longitude_of_ascending_node + deg, 360.0);
                Some(long)
            }
        }
    }

    pub fn longitude_of_periapsis(&self, time_since_epoch: TimeDelta) -> f64 {
        self.longitude_of_ascending_node(time_since_epoch).unwrap_or(0.0) + self.argument_of_periapsis(time_since_epoch)
    }

    pub fn argument_of_periapsis(&self, time_since_epoch: TimeDelta) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.argument_of_periapsis,
            KeplerRotation::FlatAngles(flat) => flat.longitude_of_periapsis,
            KeplerRotation::PrecessingEulerAngles(pea) => {
                let deg = pea.apsidal_precession_deg(time_since_epoch);
                mappings::bound_circle(pea.argument_of_periapsis + deg, 360.0)
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KeplerEulerAngles {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KeplerPrecessingEulerAngles {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
    pub apsidal_precession_period: TimeLength, // Julian Days
    pub nodal_precession_period: TimeLength, // Julian Days
}

impl KeplerPrecessingEulerAngles {
    /// Degrees of apsidal precession accumulated since epoch.
    /// Positive period = prograde (ω advances), negative = retrograde.
    /// A zero period means "no precession" and yields 0, rather than inf/NaN.
    pub fn apsidal_precession_deg(&self, time_since_epoch: TimeDelta) -> f64 {
        let period = self.apsidal_precession_period.to_seconds();
        if period == 0.0 { return 0.0; }
        (time_since_epoch.to_seconds() / period) * 360.0
    }

    /// Degrees of nodal precession accumulated since epoch.
    /// Positive period = prograde (Ω advances), negative = retrograde (Ω regresses).
    /// A zero period means "no precession" and yields 0, rather than inf/NaN.
    pub fn nodal_precession_deg(&self, time_since_epoch: TimeDelta) -> f64 {
        let period = self.nodal_precession_period.to_seconds();
        if period == 0.0 { return 0.0; }
        (time_since_epoch.to_seconds() / period) * 360.0
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KeplerFlatAngles {
    pub longitude_of_periapsis: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum KeplerEpoch {
    MeanAnomaly(MeanAnomalyAtEpoch),
    TimeAtPeriapsisPassage(Instant),
    TrueAnomaly(TrueAnomalyAtEpoch),
    J2000(MeanAnomalyAtJ2000),
}

impl KeplerEpoch {
    pub fn epoch(&self) -> Instant {
        match self {
            KeplerEpoch::MeanAnomaly(maae) => maae.epoch,
            KeplerEpoch::TimeAtPeriapsisPassage(tapp) => *tapp,
            KeplerEpoch::TrueAnomaly(taae) => taae.epoch,
            KeplerEpoch::J2000(_) => Instant::J2000,
        }
    }

    /// This refers to the internal epoch of this particular orbit description.
    /// Most orbits should share the same epoch, but they might not.
    pub fn mean_anomaly_at_epoch(&self) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => mean_anomaly.mean_anomaly,
            KeplerEpoch::TimeAtPeriapsisPassage(_) => 0.0,
            KeplerEpoch::TrueAnomaly(_) => { todo!() }
            KeplerEpoch::J2000(j2000) => j2000.mean_anomaly,
        }
    }

    pub fn time_at_periapsis_passage(&self, period: TimeLength) -> Instant {
        let period_seconds = period.to_seconds();
        let raw_time = match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => {
                // mean_anomaly is stored in degrees; convert to radians for the division by TAU
                let mean_anomaly_rad = mean_anomaly.mean_anomaly.to_radians();
                mean_anomaly.epoch.to_j2000_seconds() - period_seconds * (mean_anomaly_rad / std::f64::consts::TAU)
            }
            KeplerEpoch::TimeAtPeriapsisPassage(tapp) => tapp.to_j2000_seconds(),
            KeplerEpoch::TrueAnomaly(_) => { todo!() }
            KeplerEpoch::J2000(j2000) => {
                // mean_anomaly is stored in degrees; convert to radians for the division by TAU
                let mean_anomaly_rad = j2000.mean_anomaly.to_radians();
                -period_seconds * (mean_anomaly_rad / std::f64::consts::TAU)
            }
        };
        
        // Ensure we return the first periapsis passage at or after J2000 (>= 0.0)
        let val = if raw_time < 0.0 {
            let periods_to_add = (-raw_time / period_seconds).ceil();
            raw_time + (periods_to_add * period_seconds)
        } else {
            raw_time
        };
        Instant::from_seconds_since_j2000(val)
    }
}

/// Mean anomaly at a specified epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct MeanAnomalyAtEpoch {
    pub epoch: Instant,
    /// Mean anomaly in **degrees** (converted to radians at math boundaries).
    pub mean_anomaly: f64,
}

/// True anomaly at a specified epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct TrueAnomalyAtEpoch {
    pub epoch: Instant,
    /// True anomaly in **degrees** (converted to radians at math boundaries).
    pub true_anomaly: f64,
}

/// Mean anomaly at the J2000 epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct MeanAnomalyAtJ2000 {
    /// Mean anomaly in **degrees** (converted to radians at math boundaries).
    pub mean_anomaly: f64,
}
