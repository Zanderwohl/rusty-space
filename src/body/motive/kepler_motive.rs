use bevy::math::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use bevy_egui::egui::Ui;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::sim::{BodySelection, CalculateTrajectory, SimTime};
use crate::body::universe::save::{UniversePhysics, ViewSettings};
use crate::foundations::kepler::{angular_motion, apoapsis, eccentric_anomaly, eccentricity, local, mean_anomaly, periapsis, period, semi_latus_rectum, semi_major_axis, semi_minor_axis, semi_parameter, true_anomaly};
use crate::foundations::time::{Includes, Instant, TimeDelta, TimeLength};
use crate::util::{mappings};
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Component, Clone)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
    /// Optional override for gravitational parameter (mu = G*M) in m^3/s^2.
    /// When set, this value is used instead of computing G * primary_mass.
    /// Useful for barycentric orbits where each body has a different effective mu.
    #[serde(default)]
    pub gravitational_parameter: Option<f64>,
}

#[inline(always)]
pub fn expansion_iterations(eccentricity: f64) -> usize {
    if eccentricity < 0.3 {
        4
    } else if eccentricity < 0.6 {
        8
    } else {
        10
    }
}

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

    /// Returns true if this orbit has precessing orbital elements (nodal or apsidal precession).
    pub fn is_precessing(&self) -> bool {
        matches!(self.rotation, KeplerRotation::PrecessingEulerAngles(_))
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
        let eccentricity = self.shape.eccentricity();
        true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), eccentricity, expansion_iterations(eccentricity))
    }

    pub fn radius_from_primary_at_time(&self, time: Instant, gravitational_parameter: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), ecc, expansion_iterations(ecc));
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)
    }

    pub fn radius_from_primary_at_true_anomaly(&self, true_anomaly: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)
    }

    pub fn eccentric_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        let eccentricity = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), eccentricity, expansion_iterations(eccentricity));
        eccentric_anomaly::from_true_anomaly(self.shape.eccentricity(), ta)
    }

    /// Perifocal Frame
    /// +P (+x) points to periapsis
    /// +Q (+y) points toward motion at periapsis, normal to P
    /// +W (+z) normal to the other 2 according to RHR
    pub fn displacement_pqw(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let ta = true_anomaly::fourier_expansion(self.mean_anomaly(time, gravitational_parameter), ecc, expansion_iterations(ecc));
        let rad = local::radius::from_elements2(self.shape.semi_major_axis(), ecc, ta)?;
        let (sin_ta, cos_ta) = ta.sin_cos();
        Some(DVec3::new(rad * cos_ta, rad * sin_ta, 0.0))
    }

    pub fn displacement(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let perifocal_displacement = self.displacement_pqw(time, gravitational_parameter)?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);

        Some(rotated)
    }

    /// Computes displacement at a given true anomaly, using precession angles for `precession_time`.
    /// This allows sampling the orbit shape while keeping a consistent orbital plane orientation.
    /// Use this for trajectory visualization where all points should share the current precession state.
    pub fn displacement_at_true_anomaly_with_precession(
        &self,
        true_anomaly: f64,
        precession_time: Instant,
    ) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let rad = local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)?;
        let (sin_ta, cos_ta) = true_anomaly.sin_cos();
        let perifocal_displacement = DVec3::new(rad * cos_ta, rad * sin_ta, 0.0);
        let rotated = self.perifocal_to_reference(perifocal_displacement, precession_time);
        Some(rotated)
    }

    /// Computes displacement from a precomputed true anomaly for non-precessing trajectory points.
    pub fn displacement_from_true_anomaly(&self, true_anomaly: f64, time: Instant) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let rad = local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)?;
        let (sin_ta, cos_ta) = true_anomaly.sin_cos();
        let perifocal_displacement = DVec3::new(rad * cos_ta, rad * sin_ta, 0.0);
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);
        Some(rotated)
    }

    /// Computes displacement from true anomaly using a precomputed rotation matrix.
    /// Use this in loops where the rotation matrix is constant across all sample points.
    #[inline]
    pub fn displacement_with_cached_rotation(
        &self,
        true_anomaly: f64,
        rotation_matrix: &DMat3,
    ) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let rad = local::radius::from_elements2(self.shape.semi_major_axis(), ecc, true_anomaly)?;
        let (sin_ta, cos_ta) = true_anomaly.sin_cos();
        let perifocal_displacement = DVec3::new(rad * cos_ta, rad * sin_ta, 0.0);
        Some(*rotation_matrix * perifocal_displacement)
    }

    /// Computes displacement from true anomaly using precomputed shape constants and rotation matrix.
    /// Most efficient variant for tight loops with all invariants cached.
    #[inline]
    pub fn displacement_fully_cached(
        sma: f64,
        ecc: f64,
        true_anomaly: f64,
        rotation_matrix: &DMat3,
    ) -> Option<DVec3> {
        let rad = local::radius::from_elements2(sma, ecc, true_anomaly)?;
        let (sin_ta, cos_ta) = true_anomaly.sin_cos();
        let perifocal_displacement = DVec3::new(rad * cos_ta, rad * sin_ta, 0.0);
        Some(*rotation_matrix * perifocal_displacement)
    }

    fn perifocal_to_reference(&self, perifocal_displacement: DVec3, time: Instant) -> DVec3 {
        let rotation_matrix = self.perifocal_to_reference_matrix(time);
        rotation_matrix * perifocal_displacement
    }

    /// Computes the combined rotation matrix for transforming perifocal coordinates to reference frame.
    /// Cache this matrix when computing multiple points at the same precession time.
    pub fn perifocal_to_reference_matrix(&self, time: Instant) -> DMat3 {
        let rot_arg_peri = DMat3::from_rotation_z(self.argument_of_periapsis(time).to_radians());
        let rot_inc = DMat3::from_rotation_x(self.inclination().to_radians());
        let rot_long_asc_node = DMat3::from_rotation_z(self.longitude_of_ascending_node_infallible(time).to_radians());

        rot_long_asc_node * rot_inc * rot_arg_peri
    }

    pub fn display(&self, ui: &mut Ui) {
        ui.label("Shape");

        ui.label("Rotation");

        ui.label("Epoch");
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
        self.inclination() < f64::EPSILON
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
    pub fn apsidal_precession_deg(&self, time_since_epoch: TimeDelta) -> f64 {
        (time_since_epoch.to_seconds() / self.apsidal_precession_period.to_seconds()) * 360.0
    }

    /// Degrees of nodal precession accumulated since epoch.
    /// Positive period = prograde (Ω advances), negative = retrograde (Ω regresses).
    pub fn nodal_precession_deg(&self, time_since_epoch: TimeDelta) -> f64 {
        (time_since_epoch.to_seconds() / self.nodal_precession_period.to_seconds()) * 360.0
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

pub fn calculate_trajectory(
    mut calcs: MessageReader<CalculateTrajectory>,
    mut bodies: Query<(&mut BodyState, &BodyInfo, &crate::body::motive::Motive)>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    physics_graph: Res<crate::body::motive::calculate_body_positions::PhysicsGraph>,
) {
    if calcs.is_empty() { return; }

    // First collect all body masses into a HashMap
    let mut body_masses: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for (_, info, _) in bodies.iter() {
        body_masses.insert(info.id.clone(), info.mass);
    }

    let current_time = sim_time.time;

    for calc in calcs.read() {
        // For IDs selection, use direct entity lookup (O(1) per ID) instead of scanning all bodies
        if let BodySelection::IDs(ids) = &calc.selection {
            for id in ids {
                let Some(entity) = physics_graph.id_to_entity.get(id) else {
                    continue;
                };
                let Ok((mut state, _info, motive)) = bodies.get_mut(*entity) else {
                    continue;
                };
                calculate_trajectory_for_body(
                    &mut state, motive, &body_masses, 
                    current_time, &physics, &view_settings,
                );
            }
            continue;
        }
        
        // For All and Tag selections, iterate all bodies
        for (mut state, info, motive) in bodies.iter_mut() {
            let do_this = match &calc.selection {
                BodySelection::All => true,
                BodySelection::Tag(tag) => info.tags.contains(tag),
                BodySelection::IDs(_) => unreachable!(), // Handled above
            };
            if !do_this { continue; }

            calculate_trajectory_for_body(
                &mut state, motive, &body_masses, 
                current_time, &physics, &view_settings,
            );
        }
    }
}

/// Helper function to calculate trajectory for a single body.
fn calculate_trajectory_for_body(
    state: &mut BodyState,
    motive: &crate::body::motive::Motive,
    body_masses: &std::collections::HashMap<String, f64>,
    current_time: Instant,
    physics: &UniversePhysics,
    view_settings: &ViewSettings,
) {
    // Get the current motive selection
    let (_, selection) = motive.motive_at(current_time);
    
    // Only calculate trajectories for Keplerian bodies
    let kepler_motive = match selection {
        crate::body::motive::MotiveSelection::Keplerian(k) => k,
        _ => return,
    };

    let primary_mass = body_masses.get(&kepler_motive.primary_id)
        .copied()
        .expect("Missing primary body mass");
    // Honor explicit mu overrides (e.g. barycentric orbits like Pluto/Charon).
    let mu = kepler_motive
        .gravitational_parameter
        .unwrap_or(physics.gravitational_constant * primary_mass);

    let period = kepler_motive.period(mu);
    let periapsis_time = kepler_motive.time_at_periapsis_passage(mu);

    // Preallocate trajectory map with known capacity
    let num_points = view_settings.trajectory_resolution + 1;
    state.trajectory = Some(TimeMap::with_capacity(num_points));
    let map = state.trajectory.as_mut().unwrap();

    if !kepler_motive.is_open() {
        map.set_periodicity(periapsis_time, period);
    }

    // Precompute orbit invariants outside the sample loop
    let period_s = period.to_seconds();
    let periapsis_j2000 = periapsis_time.to_j2000_seconds();
    let resolution = view_settings.trajectory_resolution as f64;
    let ecc = kepler_motive.eccentricity();
    let sma = kepler_motive.semi_major_axis();
    let mean_anomaly_at_epoch_rad = kepler_motive.epoch.mean_anomaly_at_epoch().to_radians();
    let epoch_j2000 = kepler_motive.epoch.epoch().to_j2000_seconds();
    let iterations = expansion_iterations(ecc);
    
    // Precompute mean motion: n = sqrt(mu / a^3)
    let mean_motion = f64::sqrt(mu / (sma * sma * sma));

    // Cache the rotation matrix - it's constant for the entire trajectory pass.
    // For precessing orbits, we use current_time for all points (per design).
    // For non-precessing orbits, angles are constant so any time gives the same matrix.
    let rotation_matrix = kepler_motive.perifocal_to_reference_matrix(current_time);

    // Precompute Bessel/Fourier coefficients for true anomaly calculation
    let fourier_coeffs = true_anomaly::precompute_coefficients(ecc, iterations);

    for i in 0..=view_settings.trajectory_resolution {
        let relative_time = (i as f64 / resolution) * period_s;
        let absolute_j2000 = periapsis_j2000 + relative_time;
        
        // Inline mean anomaly calculation with precomputed values
        let mean_anomaly = mean_anomaly_at_epoch_rad + mean_motion * (absolute_j2000 - epoch_j2000);
        let ta = true_anomaly::fourier_expansion_with_precompute(mean_anomaly, ecc, &fourier_coeffs);
        
        // Use fully cached displacement calculation with precomputed shape constants and rotation matrix
        let displacement = KeplerMotive::displacement_fully_cached(sma, ecc, ta, &rotation_matrix);
        
        if let Some(displacement) = displacement {
            map.insert_unordered(relative_time, displacement);
        }
    }
    
    // Finalize trajectory map by sorting time keys
    map.finalize_unordered();
}
