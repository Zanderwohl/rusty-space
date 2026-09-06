//! Keplerian orbital elements and the position they imply. Angles are stored in degrees.

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use em_foundations::kepler::{anomaly, state, angular_motion, apoapsis, eccentric_anomaly, eccentricity, local, periapsis, period, semi_latus_rectum, semi_major_axis, semi_minor_axis, semi_parameter, true_anomaly};
use em_foundations::time::{Instant, TimeDelta};
use em_foundations::mappings;

#[derive(Serialize, Deserialize, Clone)]
pub struct KeplerMotive {
    pub primary_id: String,
    pub shape: KeplerShape,
    pub rotation: KeplerRotation,
    pub epoch: KeplerEpoch,
    /// Anomalistic period: one full turn of the mean anomaly from periapsis. `None` derives
    /// the rate from the semi-major axis by Kepler's third law, exact only for an ideal
    /// two-body orbit — precessing periapsis splits the anomalistic from the sidereal rate
    /// (Luna 27.5545 d against 27.3217 d) and an oblate primary shifts it (Saturn's J2 moves
    /// Pan's ~1.5%). Dropping it forces `a` to absorb the difference; fitted elements set it.
    #[serde(default)]
    pub anomalistic_period: Option<TimeDelta>,
    /// Explicit mu, m^3/s^2, overriding `G * (M_primary + m)`. Needed for a body orbiting a
    /// barycentre, where effective mu depends on the *other* body's mass (Pluto and Charon).
    /// Dropping it silently changes the orbit.
    #[serde(default)]
    pub gravitational_parameter: Option<f64>,
}

/// Terms for [`true_anomaly::fourier_expansion`], no longer the default route from mean to
/// true anomaly.
const EXPANSION_ITERATIONS: usize = 10;

/// Inclinations below this, in degrees, are treated as coplanar. `f64::EPSILON` is
/// meaningless as an angular tolerance.
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
        self.epoch.time_at_periapsis_passage(
            self.mean_angular_motion(gravitational_parameter),
            self.eccentricity(),
        )
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

    /// Coplanar with the reference plane: the equator for Earth satellites, the ecliptic
    /// for solar ones.
    pub fn is_coplanar(&self) -> bool {
        self.rotation.no_inclination()
    }

    pub fn longitude_of_ascending_node(&self, time: Instant) -> Option<f64> {
        let time_since_epoch = self.time_since_epoch(time);
        self.rotation.longitude_of_ascending_node(time_since_epoch)
    }

    /// Zero inclination yields 0 rather than `None`.
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

    /// Whether the elements precess, nodally or apsidally.
    pub fn is_precessing(&self) -> bool {
        matches!(self.rotation, KeplerRotation::PrecessingEulerAngles(_))
    }
    
    pub fn time_since_epoch(&self, time: Instant) -> TimeDelta {
        time - self.epoch.epoch()
    }

    pub fn period(&self, gravitational_parameter: f64) -> TimeDelta {
        TimeDelta::from_seconds(period::third_law(self.semi_major_axis(), gravitational_parameter))
    }

    /// Rate at which the mean anomaly advances, rad/s. Prefers `anomalistic_period`;
    /// otherwise Kepler's third law off `a`, giving the sidereal rate. The two differ once
    /// periapsis precesses.
    pub fn mean_angular_motion(&self, gravitational_parameter: f64) -> f64 {
        match self.anomalistic_period {
            Some(period) if period.to_seconds() != 0.0 => {
                std::f64::consts::TAU / period.to_seconds()
            }
            _ => angular_motion::mean(gravitational_parameter, self.semi_major_axis()),
        }
    }

    /// Mean anomaly at `time`, in radians.
    pub fn mean_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        // Stored in degrees.
        let mean_anomaly_at_epoch_rad = self.epoch.mean_anomaly_at_epoch(self.eccentricity()).to_radians();
        let n = self.mean_angular_motion(gravitational_parameter);
        let dt = (time - self.epoch.epoch()).to_seconds();
        mean_anomaly_at_epoch_rad + n * dt
    }

    pub fn true_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter))
    }

    /// True anomaly from mean, solved rather than expanded. Falls back to the series only
    /// for `e == 1`, which has no mean anomaly here.
    fn true_anomaly_at(&self, mean_anomaly: f64) -> f64 {
        let ecc = self.shape.eccentricity();
        anomaly::true_from_mean(mean_anomaly, ecc)
            .unwrap_or_else(|| true_anomaly::fourier_expansion(mean_anomaly, ecc, EXPANSION_ITERATIONS))
    }

    /// True anomaly via the Bessel expansion. Diverges past the Laplace limit `e ~ 0.6627`.
    pub fn true_anomaly_series(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        true_anomaly::fourier_expansion(
            self.mean_anomaly(time, gravitational_parameter),
            self.shape.eccentricity(),
            EXPANSION_ITERATIONS,
        )
    }

    pub fn radius_from_primary_at_time(&self, time: Instant, gravitational_parameter: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        let ta = self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter));
        local::radius::from_semi_major_axis(self.shape.semi_major_axis(), ecc, ta)
    }

    pub fn radius_from_primary_at_true_anomaly(&self, true_anomaly: f64) -> Option<f64> {
        let ecc = self.shape.eccentricity();
        local::radius::from_semi_major_axis(self.shape.semi_major_axis(), ecc, true_anomaly)
    }

    pub fn eccentric_anomaly(&self, time: Instant, gravitational_parameter: f64) -> f64 {
        let ta = self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter));
        eccentric_anomaly::from_true_anomaly(self.shape.eccentricity(), ta)
    }

    /// Displacement in the perifocal frame: +P at periapsis, +Q along motion there,
    /// +W = P × Q.
    pub fn displacement_pqw(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let ta = self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter));
        let rad = local::radius::from_semi_major_axis(self.shape.semi_major_axis(), ecc, ta)?;

        let (sin_ta, cos_ta) = ta.sin_cos();
        Some(DVec3::new(rad * cos_ta, rad * sin_ta, 0.0))
    }

    /// Velocity in the perifocal frame, m/s.
    pub fn velocity_pqw(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let ecc = self.shape.eccentricity();
        let p = self.shape.semi_latus_rectum();
        if p <= 0.0 || !p.is_finite() {
            return None;
        }
        let ta = self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter));
        Some(state::perifocal_velocity(gravitational_parameter, p, ecc, ta))
    }

    /// Velocity relative to the primary, in the reference frame, m/s. Along the osculating
    /// ellipse only: a precessing frame's own rotation is not included.
    pub fn velocity(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let v_pqw = self.velocity_pqw(time, gravitational_parameter)?;
        Some(self.perifocal_to_reference(v_pqw, time))
    }

    /// Position and velocity relative to the primary, in the reference frame.
    ///
    /// This is the propagation hot path, so it does not go through [`Self::displacement`]
    /// and [`Self::velocity`]: those each solve Kepler's equation and each build the
    /// perifocal-to-reference rotation, and both results are shared here. Same answer,
    /// half the work.
    pub fn state_vectors(
        &self,
        time: Instant,
        gravitational_parameter: f64,
    ) -> Option<(DVec3, DVec3)> {
        let ecc = self.shape.eccentricity();
        let p = self.shape.semi_latus_rectum();
        // Matches `velocity_pqw`: a degenerate semi-latus rectum has no state vector.
        if p <= 0.0 || !p.is_finite() {
            return None;
        }
        let ta = self.true_anomaly_at(self.mean_anomaly(time, gravitational_parameter));
        let rad = local::radius::from_semi_major_axis(self.shape.semi_major_axis(), ecc, ta)?;

        let (sin_ta, cos_ta) = ta.sin_cos();
        let r_pqw = DVec3::new(rad * cos_ta, rad * sin_ta, 0.0);
        let v_pqw = state::perifocal_velocity(gravitational_parameter, p, ecc, ta);

        let rotation = self.perifocal_to_reference_matrix(time);
        Some((rotation * r_pqw, rotation * v_pqw))
    }

    /// These elements in radians, true anomaly resolved for `time`.
    pub fn elements_at(&self, time: Instant, gravitational_parameter: f64) -> state::Elements {
        state::Elements {
            semi_major_axis: self.semi_major_axis(),
            eccentricity: self.eccentricity(),
            inclination: self.inclination().to_radians(),
            longitude_of_ascending_node: self
                .longitude_of_ascending_node_infallible(time)
                .to_radians(),
            argument_of_periapsis: self.argument_of_periapsis(time).to_radians(),
            true_anomaly: self.true_anomaly(time, gravitational_parameter),
        }
    }

    pub fn displacement(&self, time: Instant, gravitational_parameter: f64) -> Option<DVec3> {
        let perifocal_displacement = self.displacement_pqw(time, gravitational_parameter)?;
        let rotated = self.perifocal_to_reference(perifocal_displacement, time);

        Some(rotated)
    }

    /// Rotate a perifocal vector into the reference frame at `time`.
    fn perifocal_to_reference(&self, perifocal: DVec3, time: Instant) -> DVec3 {
        self.perifocal_to_reference_matrix(time) * perifocal
    }

    /// The perifocal-to-reference rotation at `time`: the 3-1-3 sequence.
    pub fn perifocal_to_reference_matrix(&self, time: Instant) -> DMat3 {
        state::perifocal_to_inertial(
            self.longitude_of_ascending_node_infallible(time).to_radians(),
            self.inclination().to_radians(),
            self.argument_of_periapsis(time).to_radians(),
        )
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
    /// One full turn of the argument of periapsis; positive is prograde. Measured from the
    /// regressing node, so not the ~8.85 yr precession of the longitude of perihelion.
    #[serde(deserialize_with = "legacy_time_length")]
    pub apsidal_precession_period: TimeDelta,
    /// One full turn of the longitude of the ascending node; negative (retrograde) is usual.
    #[serde(deserialize_with = "legacy_time_length")]
    pub nodal_precession_period: TimeDelta,
}

impl KeplerPrecessingEulerAngles {
    /// Degrees of apsidal precession since epoch. A zero period yields 0, not inf/NaN.
    pub fn apsidal_precession_deg(&self, time_since_epoch: TimeDelta) -> f64 {
        let period = self.apsidal_precession_period.to_seconds();
        if period == 0.0 { return 0.0; }
        (time_since_epoch.to_seconds() / period) * 360.0
    }

    /// Degrees of nodal precession since epoch. A zero period yields 0, not inf/NaN.
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

/// Mean anomaly from true anomaly, both in **degrees**. Closed orbits go via the eccentric
/// anomaly, open ones via the hyperbolic. A true anomaly outside a hyperbola's asymptotes
/// has no mean anomaly; the input is returned rather than a NaN that would spread.
fn mean_from_true_degrees(true_anomaly_deg: f64, eccentricity: f64) -> f64 {
    let nu = true_anomaly_deg.to_radians();
    let mean = if eccentricity < 1.0 {
        anomaly::mean_from_eccentric(anomaly::eccentric_from_true(nu, eccentricity), eccentricity)
    } else {
        match anomaly::hyperbolic_from_true(nu, eccentricity) {
            Some(h) => anomaly::mean_from_hyperbolic(h, eccentricity),
            None => return true_anomaly_deg,
        }
    };
    mean.to_degrees()
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

    /// Mean anomaly at this orbit's own epoch, in **degrees**. Needs the eccentricity
    /// because the `TrueAnomaly` form converts via the eccentric or hyperbolic anomaly.
    pub fn mean_anomaly_at_epoch(&self, eccentricity: f64) -> f64 {
        match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => mean_anomaly.mean_anomaly,
            KeplerEpoch::TimeAtPeriapsisPassage(_) => 0.0,
            KeplerEpoch::TrueAnomaly(taae) => {
                mean_from_true_degrees(taae.true_anomaly, eccentricity)
            }
            KeplerEpoch::J2000(j2000) => j2000.mean_anomaly,
        }
    }

    /// Needs the eccentricity for the same reason as
    /// [`KeplerEpoch::mean_anomaly_at_epoch`].
    /// When the body last passed periapsis, from the mean anomaly at this epoch.
    ///
    /// Takes the mean angular motion rather than the period, because `t = epoch - M / n` is
    /// the general form and a hyperbola has no period — expressed through one, every
    /// capture arc came out NaN. It is also the rate propagation actually advances the mean
    /// anomaly at, which a period derived from the semi-major axis is not when an
    /// anomalistic period is recorded.
    pub fn time_at_periapsis_passage(&self, mean_angular_motion: f64, eccentricity: f64) -> Instant {
        let n = mean_angular_motion;
        let raw_time = match self {
            KeplerEpoch::MeanAnomaly(mean_anomaly) => {
                // Stored in degrees.
                mean_anomaly.epoch.to_j2000_seconds() - mean_anomaly.mean_anomaly.to_radians() / n
            }
            KeplerEpoch::TimeAtPeriapsisPassage(tapp) => tapp.to_j2000_seconds(),
            KeplerEpoch::TrueAnomaly(taae) => {
                let mean_anomaly_rad =
                    mean_from_true_degrees(taae.true_anomaly, eccentricity).to_radians();
                taae.epoch.to_j2000_seconds() - mean_anomaly_rad / n
            }
            KeplerEpoch::J2000(j2000) => {
                // Stored in degrees.
                -j2000.mean_anomaly.to_radians() / n
            }
        };

        // Normalise to the first periapsis passage at or after J2000. Only a repeating
        // orbit has more than one to choose from; a hyperbola passes periapsis once, and
        // sliding that passage forward by a "period" would move the whole arc.
        if eccentricity < 1.0 && n > 0.0 && raw_time < 0.0 && raw_time.is_finite() {
            let period_seconds = std::f64::consts::TAU / n;
            let periods_to_add = (-raw_time / period_seconds).ceil();
            return Instant::from_seconds_since_j2000(raw_time + periods_to_add * period_seconds);
        }
        Instant::from_seconds_since_j2000(raw_time)
    }
}

/// Mean anomaly at a specified epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct MeanAnomalyAtEpoch {
    pub epoch: Instant,
    /// **Degrees**.
    pub mean_anomaly: f64,
}

/// True anomaly at a specified epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct TrueAnomalyAtEpoch {
    pub epoch: Instant,
    /// **Degrees**.
    pub true_anomaly: f64,
}

/// Mean anomaly at the J2000 epoch.
#[derive(Serialize, Deserialize, Clone)]
pub struct MeanAnomalyAtJ2000 {
    /// **Degrees**.
    pub mean_anomaly: f64,
}

/// Accepts a precession period as a bare float or as the legacy `[seconds, tag]` pair.
/// Both carry seconds, so the legacy form needs unwrapping, not scaling.
fn legacy_time_length<'de, D>(deserializer: D) -> Result<TimeDelta, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Seconds(f64),
        /// Legacy `(seconds, Includes)`; the tag is discarded.
        Tagged(f64, serde::de::IgnoredAny),
    }
    Ok(match Either::deserialize(deserializer)? {
        Either::Seconds(s) => TimeDelta::from_seconds(s),
        Either::Tagged(s, _) => TimeDelta::from_seconds(s),
    })
}

#[cfg(test)]
mod save_compat {
    //! Legacy `[seconds, tag]` and current bare-float precession periods must load to the
    //! same value.
    use super::*;

    const LEGACY: &str = r#"
inclination = 5.1450041826
longitude_of_ascending_node = 125.0636775505
argument_of_periapsis = 318.5214307952
apsidal_precession_period = [189259918.81, "Beginning"]
nodal_precession_period = [-586955486.75, "Both"]
"#;

    const CURRENT: &str = r#"
inclination = 5.1450041826
longitude_of_ascending_node = 125.0636775505
argument_of_periapsis = 318.5214307952
apsidal_precession_period = 189259918.81
nodal_precession_period = -586955486.75
"#;

    #[test]
    fn both_representations_load_identically() {
        let old: KeplerPrecessingEulerAngles = toml::from_str(LEGACY).expect("legacy save must load");
        let new: KeplerPrecessingEulerAngles = toml::from_str(CURRENT).expect("current save must load");

        assert_eq!(old.apsidal_precession_period, new.apsidal_precession_period);
        assert_eq!(old.nodal_precession_period, new.nodal_precession_period);
        assert_eq!(old.apsidal_precession_period.to_seconds(), 189259918.81);
        assert_eq!(old.nodal_precession_period.to_seconds(), -586955486.75);
    }

    #[test]
    fn the_discarded_includes_tag_does_not_change_the_value() {
        for tag in ["Beginning", "End", "Both"] {
            let src = LEGACY.replace("\"Beginning\"", &format!("\"{tag}\""));
            let parsed: KeplerPrecessingEulerAngles = toml::from_str(&src).unwrap();
            assert_eq!(parsed.apsidal_precession_period.to_seconds(), 189259918.81);
        }
    }

    #[test]
    fn round_trips_through_the_current_form() {
        let original: KeplerPrecessingEulerAngles = toml::from_str(LEGACY).unwrap();
        let written = toml::to_string(&original).unwrap();
        assert!(!written.contains('['), "should now write a bare float, got: {written}");
        let back: KeplerPrecessingEulerAngles = toml::from_str(&written).unwrap();
        assert_eq!(original.apsidal_precession_period, back.apsidal_precession_period);
        assert_eq!(original.nodal_precession_period, back.nodal_precession_period);
    }
}
