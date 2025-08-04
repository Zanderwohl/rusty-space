use bevy::math::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use std::collections::HashMap;
use bevy_egui::egui::Ui;
use num_traits::FloatConst;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::SimulationObject;
use crate::body::universe::save::{UniversePhysics, ViewSettings};
use crate::gui::planetarium::time::SimTime;
use crate::util::kepler::{angular_motion, apoapsis, eccentric_anomaly, eccentricity, local, mean_anomaly, periapsis, period, semi_latus_rectum, semi_major_axis, semi_minor_axis, semi_parameter, true_anomaly};
use crate::util::mappings;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Component)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
}

const J2000_JD: f64 = 2451545.0;
const JD_SECONDS: f64 = 24.0 * 60. * 60.0;
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

    pub fn periapsis_vec(&self, time_seconds: f64) -> DVec3 {
        let perifocal_displacement = self.periapsis_vec_pqw();
        let rotated = self.perifocal_to_reference(perifocal_displacement, time_seconds);

        rotated
    }
    pub fn apoapsis_vec(&self, time_seconds: f64) -> Option<DVec3> {
        let perifocal_displacement = self.apoapsis_vec_pqw()?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time_seconds);

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

    pub fn longitude_of_ascending_node(&self, time_seconds: f64) -> Option<f64> {
        self.rotation.longitude_of_ascending_node((time_seconds / JD_SECONDS) - self.epoch.epoch_julian_day())
    }

    /// Defines 0.0 inclination case as having 0.0 long of asc node
    pub fn longitude_of_ascending_node_infallible(&self, time_seconds: f64) -> f64 {
        self.rotation.longitude_of_ascending_node((time_seconds / JD_SECONDS) - self.epoch.epoch_julian_day()).unwrap_or(0.0)
    }

    pub fn longitude_of_periapsis(&self, time_seconds: f64) -> f64 {
        self.rotation.longitude_of_periapsis((time_seconds / JD_SECONDS) - self.epoch.epoch_julian_day())
    }

    pub fn argument_of_periapsis(&self, time_seconds: f64) -> f64 {
        self.rotation.argument_of_periapsis((time_seconds / JD_SECONDS) - self.epoch.epoch_julian_day())
    }

    pub fn period(&self, gravitational_parameter: f64) -> f64 {
        period::third_law(self.semi_major_axis(), gravitational_parameter)
    }

    pub fn mean_angular_motion(&self, gravitational_parameter: f64) -> f64 {
        angular_motion::mean(gravitational_parameter, self.semi_major_axis())
    }

    pub fn mean_anomaly(&self, time_seconds: f64, gravitational_parameter: f64) -> f64 {
        let mean_anomaly_at_epoch = self.epoch.mean_anomaly_at_epoch();
        let sma = self.shape.semi_major_axis();
        let epoch_time = self.epoch.epoch_seconds_since_j2000();
        mean_anomaly::definition(mean_anomaly_at_epoch, gravitational_parameter, sma, epoch_time, time_seconds)
    }

    pub fn true_anomaly(&self, time_seconds: f64, gravitational_parameter: f64) -> f64 {
        true_anomaly::fourier_expansion(self.mean_anomaly(time_seconds, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS)
    }

    pub fn radius_from_primary_at_time(&self, time_seconds: f64, gravitational_parameter: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time_seconds, gravitational_parameter), ecc, EXPANSION_ITERATIONS);
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)
    }

    pub fn radius_from_primary_at_true_anomaly(&self, true_anomaly: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)
    }

    pub fn eccentric_anomaly(&self, time_seconds: f64, gravitational_parameter: f64) -> f64 {
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time_seconds, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS);
        eccentric_anomaly::from_true_anomaly(self.shape.eccentricity(), ta)
    }

    /// Perifocal Frame
    /// +P (+x) points to periapsis
    /// +Q (+y) points toward motion at periapsis, normal to P
    /// +W (+z) normal to the other 2 according to RHR
    pub fn displacement_pqw(&self, time_seconds: f64, gravitational_parameter: f64) -> Option<DVec3> {
        let rad = self.radius_from_primary_at_time(time_seconds, gravitational_parameter)?;
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time_seconds, gravitational_parameter), self.shape.eccentricity(), EXPANSION_ITERATIONS);
        Some(DVec3::new(rad * ta.cos(), rad * ta.sin(), 0.0))
    }

    pub fn displacement(&self, time_seconds: f64, gravitational_parameter: f64) -> Option<DVec3> {
        let perifocal_displacement = self.displacement_pqw(time_seconds, gravitational_parameter)?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time_seconds);

        Some(rotated)
    }

    fn perifocal_to_reference(&self, perifocal_displacement: DVec3, time_seconds: f64) -> DVec3 {
        let rot_arg_peri = DMat3::from_rotation_z(self.argument_of_periapsis(time_seconds).to_radians());
        let rot_inc = DMat3::from_rotation_x(self.inclination().to_radians());
        let rot_long_asc_node = DMat3::from_rotation_z(self.longitude_of_ascending_node_infallible((time_seconds / JD_SECONDS) - self.epoch.epoch_julian_day()));

        rot_long_asc_node * rot_inc * rot_arg_peri * perifocal_displacement
    }

    pub fn display(&self, ui: &mut Ui) {
        ui.label("Shape");

        ui.label("Rotation");

        ui.label("Epoch");
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
        self.inclination() < f64::EPSILON
    }

    pub fn longitude_of_ascending_node(&self, time_since_epoch_jd: f64) -> Option<f64> {
        if self.no_inclination() { return None; }
        match self {
            KeplerRotation::EulerAngles(ea) => Some(ea.longitude_of_ascending_node),
            KeplerRotation::FlatAngles(_) => None,
            KeplerRotation::PrecessingEulerAngles(pea) => {
                let deg = pea.nodal_precession_deg(time_since_epoch_jd);
                let long = mappings::bound_circle(pea.longitude_of_ascending_node + deg, 360.0);
                Some(long)
            }
        }
    }

    pub fn longitude_of_periapsis(&self, time_since_epoch_jd: f64) -> f64 {
        self.longitude_of_ascending_node(time_since_epoch_jd).unwrap_or(0.0) + self.argument_of_periapsis(time_since_epoch_jd)
    }

    pub fn argument_of_periapsis(&self, time_since_epoch_jd: f64) -> f64 {
        match self {
            KeplerRotation::EulerAngles(ea) => ea.argument_of_periapsis,
            KeplerRotation::FlatAngles(flat) => flat.longitude_of_periapsis,
            KeplerRotation::PrecessingEulerAngles(pea) => {
                let deg = pea.apsidal_precession_deg(time_since_epoch_jd);
                mappings::bound_circle(pea.argument_of_periapsis + deg, 360.0)
            }
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
pub struct KeplerPrecessingEulerAngles {
    pub inclination: f64,
    pub longitude_of_ascending_node: f64, // "Right ascension of ascending node"
    pub argument_of_periapsis: f64,
    pub apsidal_precession_period: f64, // Julian Days
    pub nodal_precession_period: f64, // Julian Days
}

impl KeplerPrecessingEulerAngles {
    pub fn apsidal_precession_deg(&self, julian_days_since_epoch: f64) -> f64 {
        let bound_days = mappings::bound_circle(julian_days_since_epoch, self.apsidal_precession_period);
        (bound_days / self.apsidal_precession_period) / 360.0
    }

    pub fn nodal_precession_deg(&self, julian_days_since_epoch: f64) -> f64 {
         let bound_days = mappings::bound_circle(julian_days_since_epoch, self.apsidal_precession_period);
        (bound_days / self.nodal_precession_period) / 360.0
    }
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
    pub fn epoch_julian_day(&self) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(maae) => {
                maae.epoch_julian_day
            }
            KeplerEpoch::TimeAtPeriapsisPassage(tapp) => {
                tapp.time_julian_day
            }
            KeplerEpoch::TrueAnomaly(taae) => {
                taae.epoch_julian_day
            }
            KeplerEpoch::J2000(_) => { 2451544.500000 }
        }
    }

    pub fn epoch_seconds_since_j2000(&self) -> f64 {
        let epoch_jd = self.epoch_julian_day();
        (epoch_jd - J2000_JD) * 86400.0  // Convert Julian days to seconds
    }

    pub fn mean_anomaly_at_epoch(&self) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => {
                mean_anomaly.mean_anomaly
            }
            KeplerEpoch::TimeAtPeriapsisPassage(_) => {
                0.0
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
    pub epoch_julian_day: f64,
    pub mean_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct PeriapsisTime {
    pub time_julian_day: f64,
}

#[derive(Serialize, Deserialize)]
pub struct TrueAnomalyAtEpoch {
    pub epoch_julian_day: f64,
    pub true_anomaly: f64,
}

#[derive(Serialize, Deserialize)]
pub struct MeanAnomalyAtJ2000 {
    pub mean_anomaly: f64,
}

pub fn calculate(
    mut sim_time: ResMut<SimTime>,
    mut kepler_bodies: Query<(&mut KeplerMotive, &BodyInfo, &mut BodyState)>,
    fixed_bodies: Query<(&SimulationObject, &BodyInfo, &BodyState), Without<KeplerMotive>>,
    physics: Res<UniversePhysics>,
) {
    // First collect all body IDs and masses into a HashMap to avoid borrow conflicts
    let mut bodies_prev_frame: std::collections::HashMap<String, (f64, DVec3)> = std::collections::HashMap::new();
    for (_, info, state) in fixed_bodies.iter() {
        bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
    }
    for (_, info, state) in kepler_bodies.iter() {
        bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
    }

    let time = sim_time.time_seconds;
    for (mut motive, _, mut state) in kepler_bodies.iter_mut() {
        let (primary_mass, primary_position) = bodies_prev_frame.get(&motive.primary_id)
            .copied()
            .expect("Missing body info");

        let mu = physics.gravitational_constant * primary_mass;
        let position = motive.displacement(time, mu);
        if let Some(position) = position {
            state.current_position = primary_position + position;
            state.current_local_position = Some(position);
            state.current_primary_position = Some(primary_position);
        }
    }
}

pub fn calculate_trajectory(
    mut kepler_bodies: Query<(&mut BodyState, &BodyInfo, &KeplerMotive),
        Or<(Changed<KeplerMotive>, Added<KeplerMotive>)>>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
) {
    for (mut state, info, motive) in kepler_bodies.iter_mut() {
        state.trajectory = Some(TimeMap::new());
        let period = motive.period(physics.gravitational_constant);
        info!("Caching trajectory for {}", info.display_name());
        for i in 0..=view_settings.trajectory_resolution {
            let time = (i as f64 / view_settings.trajectory_resolution as f64) * period;
            let displacement = motive.displacement(time, physics.gravitational_constant);
            if let Some(displacement) = displacement && let Some(map) = state.trajectory.as_mut() {
                map.insert(time, displacement);
            }
        }
    }
}
