//! Bundled universe presets.
//!
//! Plain data, so a headless tool or an editor can build a system without an engine.
//! Writing these to disk is the app's job.

use std::default::Default;
use std::f64::consts::PI;
use glam::{DVec3, DQuat};
use crate::appearance::{Appearance, AppearanceColor, DebugBall, StarBall};
use crate::body::{BodyInfo, BodyRotation, RotationEpoch};
use crate::motive::kepler::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerPrecessingEulerAngles, KeplerRotation, KeplerShape, MeanAnomalyAtEpoch, MeanAnomalyAtJ2000};
use crate::universe::{FixedEntry, KeplerEntry, NewtonEntry, SomeBody, UniverseFileContents, UniverseFileTime, UniversePhysics, ViewSettings};
use em_foundations::time::{Instant, TimeDelta};

// =============================================================================
// Solar System Template Data
// =============================================================================
//
// Unit Conventions (all values stored in SI at rest):
//
//   Quantity                                  | Unit    | Notes
//   ------------------------------------------|---------|----------------------------------
//   Mass                                      | kg      |
//   Distance (position, semi-major axis, r)  | m       | Source often in km; multiply by 1000
//   Angles (inclination, Ω, ω)               | degrees | Converted to radians at math boundaries
//   Mean anomaly                              | degrees | Converted to radians at math boundaries
//   Time (epoch)                              | JD      | Julian days
//
// Source data is often in km; we convert to meters with `* 1000.0` at the point of entry.
// Outer planets (Jupiter+) have values entered directly in meters since the source data varies.
// =============================================================================

// Obliquity of the ecliptic at J2000 in radians (~23.4393 degrees)
const OBLIQUITY_J2000_RAD: f64 = 23.4392911_f64 * PI / 180.0;

/// Create a BodyRotation from IAU pole parameters.
///
/// - `ra_deg`: Right ascension of north pole in ICRF (degrees)
/// - `dec_deg`: Declination of north pole in ICRF (degrees)  
/// - `w0_deg`: Prime meridian angle at J2000 (degrees)
/// - `period_hours`: Rotation period in hours (negative = retrograde)
fn iau_rotation(ra_deg: f64, dec_deg: f64, w0_deg: f64, period_hours: f64) -> BodyRotation {
    let pole_ecliptic = equatorial_to_ecliptic_pole(ra_deg, dec_deg);
    let orientation = pole_to_orientation(pole_ecliptic, w0_deg);
    let angular_velocity = 2.0 * PI / (period_hours * 3600.0); // rad/s
    
    BodyRotation::spinning(orientation, angular_velocity, RotationEpoch::J2000)
}

/// Create a tidally locked BodyRotation.
///
/// - `primary_id`: ID of the body this is tidally locked to
/// - `ra_deg`: Right ascension of north pole in ICRF (degrees)
/// - `dec_deg`: Declination of north pole in ICRF (degrees)
fn tidally_locked_rotation(primary_id: &str, ra_deg: f64, dec_deg: f64) -> BodyRotation {
    let pole_ecliptic = equatorial_to_ecliptic_pole(ra_deg, dec_deg);
    BodyRotation::tidally_locked(primary_id, pole_ecliptic)
}

/// Convert equatorial (ICRF) pole direction to ecliptic J2000 unit vector.
///
/// Input: RA and Dec in degrees (J2000 equatorial frame)
/// Output: Unit vector in ecliptic J2000 frame (Z-up = ecliptic north)
fn equatorial_to_ecliptic_pole(ra_deg: f64, dec_deg: f64) -> DVec3 {
    let ra = ra_deg.to_radians();
    let dec = dec_deg.to_radians();
    
    // Unit vector in equatorial frame
    let eq_x = dec.cos() * ra.cos();
    let eq_y = dec.cos() * ra.sin();
    let eq_z = dec.sin();
    
    // Rotate from equatorial to ecliptic: rotation around X by obliquity
    // Equatorial Z (celestial north) tilts toward equatorial +Y by obliquity angle
    // This is equivalent to rotating the coordinate system by -obliquity around X
    let cos_obl = OBLIQUITY_J2000_RAD.cos();
    let sin_obl = OBLIQUITY_J2000_RAD.sin();
    
    let ecl_x = eq_x;
    let ecl_y = eq_y * cos_obl + eq_z * sin_obl;
    let ecl_z = -eq_y * sin_obl + eq_z * cos_obl;
    
    DVec3::new(ecl_x, ecl_y, ecl_z).normalize()
}

/// Create an orientation DQuat that aligns local +Z with the given pole direction,
/// then applies the prime meridian rotation.
///
/// - `pole`: Unit vector pointing to north pole in ecliptic frame
/// - `w0_deg`: Prime meridian angle at epoch in degrees
fn pole_to_orientation(pole: DVec3, w0_deg: f64) -> DQuat {
    // Create a rotation that takes +Z to the pole direction
    // Using rotation_arc: finds shortest rotation from one vector to another
    let base_rotation = if pole.z > 0.9999 {
        // Pole is nearly +Z, use identity
        DQuat::IDENTITY
    } else if pole.z < -0.9999 {
        // Pole is nearly -Z, rotate 180° around X
        DQuat::from_rotation_x(PI)
    } else {
        DQuat::from_rotation_arc(DVec3::Z, pole)
    };
    
    // Apply prime meridian rotation around the pole axis
    let prime_meridian = DQuat::from_axis_angle(pole, w0_deg.to_radians());
    
    prime_meridian * base_rotation
}
/// Canonical on-disk location for this preset, used by the app when writing it out.
pub const SOLAR_SYSTEM_PATH: &str = "data/templates/solar_system.toml";

/// The bundled Solar System: the Sun and 112 orbiting bodies.
///
/// Every orbit is a mean-element set fitted by least squares against a JPL Horizons
/// position series — `docs/horizons-golden-vectors.md` records the procedure. Each entry
/// carries its own fit residual, so a body whose motion a fixed ellipse cannot represent
/// says so rather than looking as trustworthy as the rest.
///
/// Masses come from published GM where JPL has one. Where it does not — most small moons,
/// most asteroids, every comet — the entry states how the value was arrived at. Same for
/// radii.
pub fn solar_system() -> UniverseFileContents {
    let solar_system = UniverseFileContents {
            version: "0.0".into(),
            time: UniverseFileTime {
                time_julian_days: 2451544.500000, // Midnight 2000 January 1 00:00
                step: 0.1,
                gui_speed: 1.0,
                max_frame_time: 0.016,
            },
            physics: UniversePhysics::default(),
            view: ViewSettings::default(),
            bodies: vec![
                SomeBody::FixedEntry(FixedEntry {
                    info: BodyInfo {
                        name: Some("Sol".into()),
                        id: "Sol".to_string(),
                        mass: 1.988416e+30,
                        major: true,
                        designation: None,
                        tags: vec!["Star".into()],
                        ..Default::default()
                    },
                    position: DVec3::ZERO,
                    appearance: Appearance::Star(StarBall {
                        radius: 695700000.0,
                        color: AppearanceColor { r: 219, g: 222, b: 35 },
                        light: AppearanceColor { r: 3570, g: 3570, b: 3570 },
                        absolute_magnitude: 4.829999923706055,
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.05262721038, 0.03506855909, 0.6689295119, 0.7406307318),
                        2.865329085e-06, RotationEpoch::J2000)),
                }), // Sun
                // Mercury: fitted to 1501 JPL states over 60.0 yr; residual 1999 km RMS (3.4e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mercury".into()),
                        id: "Mercury".to_string(),
                        mass: 3.3011e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2056384229,
                            semi_major_axis: 5.790911945e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 7.002933432,
                            longitude_of_ascending_node: 48.33197152,
                            argument_of_periapsis: 29.12406778,
                            apsidal_precession_period: TimeDelta::from_days(45660581.82),
                            nodal_precession_period: TimeDelta::from_days(-101699936.6),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 174.794781,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(87.96934945)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327120552e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2439700.0,
                        color: AppearanceColor { r: 145, g: 145, b: 145 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.02744466869, -0.05489161633, 0.261705058, -0.9631947691),
                        1.239932688e-06, RotationEpoch::J2000)),
                }),
                // Venus: fitted to 1501 JPL states over 60.0 yr; residual 6351 km RMS (5.9e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "Venus".to_string(),
                        mass: 4.8675e+24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006757108616,
                            semi_major_axis: 1.082088531e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 3.394382474,
                            longitude_of_ascending_node: 76.68000699,
                            argument_of_periapsis: 54.89302052,
                            apsidal_precession_period: TimeDelta::from_days(50316069.0),
                            nodal_precession_period: TimeDelta::from_days(-47250108.43),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 50.40721839,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(224.7007325)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327124471e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6051800.0,
                        color: AppearanceColor { r: 224, g: 224, b: 224 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.008272079686, 0.00696246615, 0.9850517432, 0.1719190505),
                        -2.992369187e-07, RotationEpoch::J2000)),
                }),
                // Earth: fitted to 1501 JPL states over 60.0 yr; residual 10101 km RMS (6.8e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "Earth".to_string(),
                        mass: 5.972168e+24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.01669575226,
                            semi_major_axis: 1.495979431e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 0.003937064881,
                            longitude_of_ascending_node: 174.5779073,
                            argument_of_periapsis: 288.4566314,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 357.4294398,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(365.2563638)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327130212e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6371000.0,
                        color: AppearanceColor { r: 59, g: 179, b: 75 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.01796290283, 0.2023272178, 0.9753169712, -0.08659005037),
                        7.292115853e-05, RotationEpoch::J2000)),
                }),
                // Mars: fitted to 1501 JPL states over 60.0 yr; residual 36942 km RMS (1.6e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mars".into()),
                        id: "Mars".to_string(),
                        mass: 6.4171e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.09343132438,
                            semi_major_axis: 2.279404003e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 1.847250694,
                            longitude_of_ascending_node: 49.5598462,
                            argument_of_periapsis: 286.4935656,
                            apsidal_precession_period: TimeDelta::from_days(17267810.8),
                            nodal_precession_period: TimeDelta::from_days(-44098416.64),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 19.39225722,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(686.9963355)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327050135e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3389500.0,
                        color: AppearanceColor { r: 242, g: 66, b: 17 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.2195423474, -0.01220482989, 0.9749468313, 0.0336284986),
                        7.088235959e-05, RotationEpoch::J2000)),
                }),
                // Phobos: fitted to 1493 JPL states over 0.2 yr; residual 20.9 km RMS (2.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phobos".into()),
                        id: "Phobos".to_string(),
                        mass: 1.08e+16,
                        major: true,
                        designation: Some("Mars I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Mars".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.01511642398,
                            semi_major_axis: 9374904.339,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 26.35759814,
                            longitude_of_ascending_node: 84.92921417,
                            argument_of_periapsis: 342.9324579,
                            apsidal_precession_period: TimeDelta::from_days(844.2860508),
                            nodal_precession_period: TimeDelta::from_days(56898.54495),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 189.573376,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.3190321909)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(4.281182818e+13),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 11100.0,
                        color: AppearanceColor { r: 120, g: 105, b: 90 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Mars",
                        DVec3::new(0.446041664, -0.05554849573, 0.8932867393))),
                }),
                // Deimos: fitted to 1501 JPL states over 0.9 yr; residual 495 km RMS (2.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Deimos".into()),
                        id: "Deimos".to_string(),
                        mass: 1.8e+15,
                        major: true,
                        designation: Some("Mars II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Mars".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.000257808332,
                            semi_major_axis: 23457533.64,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.94802755,
                            longitude_of_ascending_node: 85.71575532,
                            argument_of_periapsis: 208.5667817,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 6.623618162,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.262440641)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(4.283091586e+13),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6200.0,
                        color: AppearanceColor { r: 120, g: 105, b: 90 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Mars",
                        DVec3::new(0.4322335342, -0.05447956625, 0.9001145198))),
                }),
                // Ceres: fitted to 1501 JPL states over 60.0 yr; residual 1454693 km RMS (3.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ceres".into()),
                        id: "1-Ceres".to_string(),
                        mass: 9.3839e+20,
                        major: true,
                        designation: Some("1 Ceres".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.07775648924,
                            semi_major_axis: 4.139985492e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 10.58892,
                            longitude_of_ascending_node: 80.12166831,
                            argument_of_periapsis: 73.44471799,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 7.266998603,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1681.599948)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327037938e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 966200.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.07082251626, 0.02015065352, 0.9939674524, 0.08128238407),
                        0.0001923403741, RotationEpoch::J2000)),
                }),
                // Vesta: fitted to 1501 JPL states over 60.0 yr; residual 788653 km RMS (2.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Vesta".into()),
                        id: "4-Vesta".to_string(),
                        mass: 2.59076e+20,
                        major: true,
                        designation: Some("4 Vesta".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.08933164428,
                            semi_major_axis: 3.533024556e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.138014176,
                            longitude_of_ascending_node: 103.6994911,
                            argument_of_periapsis: 150.8687707,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 339.8835996,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1325.655557)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327121932e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 262700.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.0003590324921, -0.2750046731, 0.5376320034, -0.7970722237),
                        0.0003267104892, RotationEpoch::J2000)),
                }),
                // Luna: fitted to 1500 JPL states over 18.5 yr; residual 7562 km RMS (2.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "Luna".to_string(),
                        mass: 7.346e+22,
                        major: true,
                        designation: Some("Earth I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Earth".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05483385719,
                            semi_major_axis: 384374822.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 5.146884942,
                            longitude_of_ascending_node: 125.0815567,
                            argument_of_periapsis: 318.6030345,
                            apsidal_precession_period: TimeDelta::from_days(2191.464675),
                            nodal_precession_period: TimeDelta::from_days(-6791.621712),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 134.6348082,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(27.55434218)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.95564176e+14),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737400.0,
                        color: AppearanceColor { r: 87, g: 87, b: 87 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Earth",
                        DVec3::new(-3.543751263e-05, -0.0003753996676, 0.9999999289))),
                }),
                // Jupiter: fitted to 1501 JPL states over 60.0 yr; residual 648573 km RMS (8.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jupiter".into()),
                        id: "Jupiter".to_string(),
                        mass: 1.8982e+27,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.04836165266,
                            semi_major_axis: 7.783344142e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 1.303628228,
                            longitude_of_ascending_node: 100.4901113,
                            argument_of_periapsis: 273.6262248,
                            apsidal_precession_period: TimeDelta::from_days(6219725.081),
                            nodal_precession_period: TimeDelta::from_days(75519768.04),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 20.28262414,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(4336.186913)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326218137e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 69911100.0,
                        color: AppearanceColor { r: 176, g: 127, b: 53 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.01865334277, -0.005120364307, 0.6089935726, -0.7929392557),
                        0.0001758518138, RotationEpoch::J2000)),
                }),
                // Metis: fitted to 1495 JPL states over 0.2 yr; residual 851 km RMS (6.7e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Metis".into()),
                        id: "Metis".to_string(),
                        mass: 3.75e+16,
                        major: true,
                        designation: Some("Jupiter XVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.033537372e-09,
                            semi_major_axis: 127974699.7,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.102087591,
                            longitude_of_ascending_node: 311.0714246,
                            argument_of_periapsis: 197.7840241,
                            apsidal_precession_period: TimeDelta::from_days(115.7126536),
                            nodal_precession_period: TimeDelta::from_days(588.545229),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 15.36550354,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.2956800893)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.267828318e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 21500.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01464915199, -0.03572966593, 0.9992541185))),
                }),
                // Adrastea: fitted to 1492 JPL states over 0.2 yr; residual 10.9 km RMS (8.5e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Adrastea".into()),
                        id: "Adrastea".to_string(),
                        mass: 1.5e+15,
                        major: true,
                        designation: Some("Jupiter XV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0005897538286,
                            semi_major_axis: 128979875.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.21602158,
                            longitude_of_ascending_node: 337.8764413,
                            argument_of_periapsis: 220.5930098,
                            apsidal_precession_period: TimeDelta::from_days(42.82661625),
                            nodal_precession_period: TimeDelta::from_days(-382250.8689),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 14.20593684,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.3003517923)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.257875479e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 8200.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01464915199, -0.03572966593, 0.9992541185))),
                }),
                // Amalthea: fitted to 1505 JPL states over 0.3 yr; residual 1371 km RMS (7.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Amalthea".into()),
                        id: "Amalthea".to_string(),
                        mass: 2.47e+18,
                        major: true,
                        designation: Some("Jupiter V".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.246865281e-12,
                            semi_major_axis: 181361429.4,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.134806466,
                            longitude_of_ascending_node: 332.3445052,
                            argument_of_periapsis: 204.3224226,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 235.1376335,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.4981791001)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.271147753e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 83500.0,
                        color: AppearanceColor { r: 170, g: 100, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01464915199, -0.03572966593, 0.9992541185))),
                }),
                // Thebe: fitted to 1497 JPL states over 0.5 yr; residual 2485 km RMS (1.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thebe".into()),
                        id: "Thebe".to_string(),
                        mass: 4.51e+17,
                        major: true,
                        designation: Some("Jupiter XIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.01760107128,
                            semi_major_axis: 221873638.1,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.138235393,
                            longitude_of_ascending_node: 320.9854264,
                            argument_of_periapsis: 44.2092901,
                            apsidal_precession_period: TimeDelta::from_days(292.800867),
                            nodal_precession_period: TimeDelta::from_days(38410.1535),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 182.0995256,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6761052133)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.263632526e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 49300.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01464915199, -0.03572966593, 0.9992541185))),
                }),
                // Io: fitted to 1501 JPL states over 1.2 yr; residual 122 km RMS (2.9e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Io".into()),
                        id: "Io".to_string(),
                        mass: 8.932e+22,
                        major: true,
                        designation: Some("Jupiter I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.004132401517,
                            semi_major_axis: 421765445.9,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.192031697,
                            longitude_of_ascending_node: 336.2855649,
                            argument_of_periapsis: 70.70889532,
                            apsidal_precession_period: TimeDelta::from_days(-487.1721672),
                            nodal_precession_period: TimeDelta::from_days(493303.1199),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 331.1837309,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.762742995)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.276930457e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1821490.0,
                        color: AppearanceColor { r: 220, g: 200, b: 60 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01464915199, -0.03572966593, 0.9992541185))),
                }),
                // Europa: fitted to 1500 JPL states over 2.4 yr; residual 653 km RMS (9.7e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Europa".into()),
                        id: "Europa".to_string(),
                        mass: 4.8e+22,
                        major: true,
                        designation: Some("Jupiter II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00927385348,
                            semi_major_axis: 671060377.3,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 1.755842608,
                            longitude_of_ascending_node: 330.3119832,
                            argument_of_periapsis: 257.2808922,
                            apsidal_precession_period: TimeDelta::from_days(-479.5978987),
                            nodal_precession_period: TimeDelta::from_days(34559.7633),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 345.1218385,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(3.525437883)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.285851773e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1560800.0,
                        color: AppearanceColor { r: 220, g: 220, b: 230 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.0144185873, -0.03556230971, 0.999263442))),
                }),
                // Ganymede: fitted to 1501 JPL states over 4.9 yr; residual 1244 km RMS (1.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ganymede".into()),
                        id: "Ganymede".to_string(),
                        mass: 1.482e+23,
                        major: true,
                        designation: Some("Jupiter III".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001601325029,
                            semi_major_axis: 1070431194.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.209992159,
                            longitude_of_ascending_node: 342.9004206,
                            argument_of_periapsis: 302.8174645,
                            apsidal_precession_period: TimeDelta::from_days(84521.84159),
                            nodal_precession_period: TimeDelta::from_days(-297671.0724),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 294.2896708,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(7.154987069)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.267041428e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2631200.0,
                        color: AppearanceColor { r: 180, g: 170, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.01348803311, -0.03454303226, 0.9993121894))),
                }),
                // Callisto: fitted to 1500 JPL states over 11.4 yr; residual 1356 km RMS (7.2e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Callisto".into()),
                        id: "Callisto".to_string(),
                        mass: 1.076e+23,
                        major: true,
                        designation: Some("Jupiter IV".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.007347401388,
                            semi_major_axis: 1882749729.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.022894176,
                            longitude_of_ascending_node: 336.8885459,
                            argument_of_periapsis: 16.16772118,
                            apsidal_precession_period: TimeDelta::from_days(169555.6276),
                            nodal_precession_period: TimeDelta::from_days(-991400.9477),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 86.11481527,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(16.69036746)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.26700473e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2410300.0,
                        color: AppearanceColor { r: 100, g: 90, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Jupiter",
                        DVec3::new(-0.00950062501, -0.03010460531, 0.9995016012))),
                }),
                // Themisto: fitted to 1501 JPL states over 60.0 yr; residual 661398 km RMS (8.6e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Themisto".into()),
                        id: "Themisto".to_string(),
                        mass: 1e+15,
                        major: false,
                        designation: Some("Jupiter XVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2850332014,
                            semi_major_axis: 7387903425.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 44.39966786,
                            longitude_of_ascending_node: 203.3525016,
                            argument_of_periapsis: 238.9216037,
                            apsidal_precession_period: TimeDelta::from_days(549440.3405),
                            nodal_precession_period: TimeDelta::from_days(-203917.3795),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 286.1740821,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(129.9675048)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.262485267e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 4000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Leda: fitted to 1501 JPL states over 60.0 yr; residual 493206 km RMS (4.4e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Leda".into()),
                        id: "Leda".to_string(),
                        mass: 1.4e+15,
                        major: false,
                        designation: Some("Jupiter XIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1597537899,
                            semi_major_axis: 1.115048965e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.24251571,
                            longitude_of_ascending_node: 215.1601794,
                            argument_of_periapsis: 264.6674511,
                            apsidal_precession_period: TimeDelta::from_days(48633.04525),
                            nodal_precession_period: TimeDelta::from_days(-115542.1563),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 239.8319124,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(241.6138602)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.255944304e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Himalia: fitted to 1501 JPL states over 60.0 yr; residual 576030 km RMS (5.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Himalia".into()),
                        id: "Himalia".to_string(),
                        mass: 2.27e+18,
                        major: false,
                        designation: Some("Jupiter VI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1588412951,
                            semi_major_axis: 1.144560168e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.87617031,
                            longitude_of_ascending_node: 66.69427786,
                            argument_of_periapsis: 332.705622,
                            apsidal_precession_period: TimeDelta::from_days(52558.8281),
                            nodal_precession_period: TimeDelta::from_days(-108128.3632),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 63.92401002,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(251.1799914)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.256834383e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 85000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Lysithea: fitted to 1501 JPL states over 60.0 yr; residual 436643 km RMS (3.7e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Lysithea".into()),
                        id: "Lysithea".to_string(),
                        mass: 1.9e+16,
                        major: false,
                        designation: Some("Jupiter X".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1164723591,
                            semi_major_axis: 1.170833784e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 26.75861371,
                            longitude_of_ascending_node: 7.908631196,
                            argument_of_periapsis: 47.39974646,
                            apsidal_precession_period: TimeDelta::from_days(47830.24882),
                            nodal_precession_period: TimeDelta::from_days(-103737.6539),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 330.4975205,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(259.9561321)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.256081603e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 12000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Elara: fitted to 1501 JPL states over 60.0 yr; residual 761467 km RMS (6.3e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Elara".into()),
                        id: "Elara".to_string(),
                        mass: 7e+17,
                        major: false,
                        designation: Some("Jupiter VII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2122827846,
                            semi_major_axis: 1.171382286e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.97773963,
                            longitude_of_ascending_node: 115.3670292,
                            argument_of_periapsis: 142.8748989,
                            apsidal_precession_period: TimeDelta::from_days(47753.85715),
                            nodal_precession_period: TimeDelta::from_days(-100378.3105),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 331.1024342,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(260.3865204)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.253693032e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 40000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Ananke: fitted to 1501 JPL states over 60.0 yr; residual 2675785 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ananke".into()),
                        id: "Ananke".to_string(),
                        mass: 1.1e+16,
                        major: false,
                        designation: Some("Jupiter XII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2188238792,
                            semi_major_axis: 2.103867236e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 148.6497975,
                            longitude_of_ascending_node: 23.90284033,
                            argument_of_periapsis: 83.25912439,
                            apsidal_precession_period: TimeDelta::from_days(45127.56964),
                            nodal_precession_period: TimeDelta::from_days(43363.36334),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 272.8183192,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(629.3253437)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.243474259e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 10000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Taygete: fitted to 1501 JPL states over 60.0 yr; residual 1890058 km RMS (7.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Taygete".into()),
                        id: "Taygete".to_string(),
                        mass: 2e+14,
                        major: false,
                        designation: Some("Jupiter XX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.249950998,
                            semi_major_axis: 2.329196186e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.8304488,
                            longitude_of_ascending_node: 300.3340919,
                            argument_of_periapsis: 227.4385888,
                            apsidal_precession_period: TimeDelta::from_days(28656.13034),
                            nodal_precession_period: TimeDelta::from_days(31085.10233),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 94.9039113,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(733.8400685)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.240930962e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2500.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Sinope: fitted to 1501 JPL states over 60.0 yr; residual 2423291 km RMS (9.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sinope".into()),
                        id: "Sinope".to_string(),
                        mass: 3e+16,
                        major: false,
                        designation: Some("Jupiter IX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2489038255,
                            semi_major_axis: 2.380733831e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 158.3623909,
                            longitude_of_ascending_node: 301.0945192,
                            argument_of_periapsis: 343.0883593,
                            apsidal_precession_period: TimeDelta::from_days(31545.16672),
                            nodal_precession_period: TimeDelta::from_days(31100.39691),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 166.7207099,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(758.5677468)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.240155192e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Callirrhoe: fitted to 1501 JPL states over 60.0 yr; residual 4873357 km RMS (2.0e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Callirrhoe".into()),
                        id: "Callirrhoe".to_string(),
                        mass: 1e+15,
                        major: false,
                        designation: Some("Jupiter XVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2822400962,
                            semi_major_axis: 2.363375527e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 147.0433946,
                            longitude_of_ascending_node: 273.7093287,
                            argument_of_periapsis: 28.13140707,
                            apsidal_precession_period: TimeDelta::from_days(47531.19474),
                            nodal_precession_period: TimeDelta::from_days(30485.24073),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 94.95410961,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(752.1976543)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.233861759e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 4300.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Pasiphae: fitted to 1501 JPL states over 60.0 yr; residual 4679054 km RMS (1.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pasiphae".into()),
                        id: "Pasiphae".to_string(),
                        mass: 6.4e+16,
                        major: false,
                        designation: Some("Jupiter VIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3971748007,
                            semi_major_axis: 2.32907144e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 151.1296824,
                            longitude_of_ascending_node: 310.2007003,
                            argument_of_periapsis: 165.8740083,
                            apsidal_precession_period: TimeDelta::from_days(28819.78476),
                            nodal_precession_period: TimeDelta::from_days(29176.00873),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 279.0523652,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(743.8177479)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.207668137e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Carme: fitted to 1501 JPL states over 60.0 yr; residual 1971477 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Carme".into()),
                        id: "Carme".to_string(),
                        mass: 3.7e+16,
                        major: false,
                        designation: Some("Jupiter XI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2537654799,
                            semi_major_axis: 2.331956327e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.860709,
                            longitude_of_ascending_node: 122.5147256,
                            argument_of_periapsis: 35.69877333,
                            apsidal_precession_period: TimeDelta::from_days(30937.28858),
                            nodal_precession_period: TimeDelta::from_days(33618.68719),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 233.3419833,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(735.570981)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.239493674e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Megaclite: fitted to 1501 JPL states over 60.0 yr; residual 4586203 km RMS (1.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Megaclite".into()),
                        id: "Megaclite".to_string(),
                        mass: 2e+14,
                        major: false,
                        designation: Some("Jupiter XIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4010632441,
                            semi_major_axis: 2.350646994e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 152.4967633,
                            longitude_of_ascending_node: 280.0566604,
                            argument_of_periapsis: 283.060467,
                            apsidal_precession_period: TimeDelta::from_days(27639.17821),
                            nodal_precession_period: TimeDelta::from_days(28856.67504),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 140.4997532,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(753.728322)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.209107251e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2700.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Chaldene: fitted to 1501 JPL states over 60.0 yr; residual 1928551 km RMS (8.1e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Chaldene".into()),
                        id: "Chaldene".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2590778023,
                            semi_major_axis: 2.310264879e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.88412,
                            longitude_of_ascending_node: 140.9899188,
                            argument_of_periapsis: 250.8741987,
                            apsidal_precession_period: TimeDelta::from_days(31429.88176),
                            nodal_precession_period: TimeDelta::from_days(34390.08709),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 265.8897307,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(725.1532707)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.240103575e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Harpalyke: fitted to 1501 JPL states over 60.0 yr; residual 3008854 km RMS (1.4e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Harpalyke".into()),
                        id: "Harpalyke".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2192729676,
                            semi_major_axis: 2.08658667e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 148.9792231,
                            longitude_of_ascending_node: 31.56143695,
                            argument_of_periapsis: 126.5271969,
                            apsidal_precession_period: TimeDelta::from_days(47930.60879),
                            nodal_precession_period: TimeDelta::from_days(42151.57554),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 225.5442473,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(622.166274)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.241162389e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kalyke: fitted to 1501 JPL states over 60.0 yr; residual 1982157 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kalyke".into()),
                        id: "Kalyke".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2583927799,
                            semi_major_axis: 2.348266041e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 194.4719551,
                            longitude_of_ascending_node: 223.4184629,
                            argument_of_periapsis: 39.22842382,
                            apsidal_precession_period: TimeDelta::from_days(28836.40294),
                            nodal_precession_period: TimeDelta::from_days(31246.19659),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 255.447576,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(743.454619)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.238982627e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Iocaste: fitted to 1501 JPL states over 60.0 yr; residual 2933799 km RMS (1.4e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Iocaste".into()),
                        id: "Iocaste".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.215363926,
                            semi_major_axis: 2.106848005e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 149.1768285,
                            longitude_of_ascending_node: 271.0504637,
                            argument_of_periapsis: 62.07387413,
                            apsidal_precession_period: TimeDelta::from_days(54941.88253),
                            nodal_precession_period: TimeDelta::from_days(41206.99901),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 217.8310062,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(629.1723316)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.249374495e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Erinome: fitted to 1501 JPL states over 60.0 yr; residual 2000613 km RMS (8.3e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Erinome".into()),
                        id: "Erinome".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2670942365,
                            semi_major_axis: 2.319380289e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.3833249,
                            longitude_of_ascending_node: 309.1855994,
                            argument_of_periapsis: 1.448570682,
                            apsidal_precession_period: TimeDelta::from_days(28225.10585),
                            nodal_precession_period: TimeDelta::from_days(30952.92654),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 265.6304364,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(730.1308085)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.237789486e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Isonoe: fitted to 1501 JPL states over 60.0 yr; residual 1798661 km RMS (7.5e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Isonoe".into()),
                        id: "Isonoe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2376336988,
                            semi_major_axis: 2.315133309e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.5896214,
                            longitude_of_ascending_node: 139.1635182,
                            argument_of_periapsis: 124.7472156,
                            apsidal_precession_period: TimeDelta::from_days(31554.37816),
                            nodal_precession_period: TimeDelta::from_days(34710.23323),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 125.2416348,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(727.8137982)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.238852759e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Praxidike: fitted to 1501 JPL states over 60.0 yr; residual 2510161 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Praxidike".into()),
                        id: "Praxidike".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2522715528,
                            semi_major_axis: 2.093419077e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 149.2008167,
                            longitude_of_ascending_node: 271.9081446,
                            argument_of_periapsis: 207.2950316,
                            apsidal_precession_period: TimeDelta::from_days(67617.54794),
                            nodal_precession_period: TimeDelta::from_days(37418.78714),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 92.34311192,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(620.7719661)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.259031489e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Autonoe: fitted to 1501 JPL states over 60.0 yr; residual 3815410 km RMS (1.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Autonoe".into()),
                        id: "Autonoe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3310293953,
                            semi_major_axis: 2.378353263e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 153.0228516,
                            longitude_of_ascending_node: 272.259005,
                            argument_of_periapsis: 52.15880013,
                            apsidal_precession_period: TimeDelta::from_days(33997.6629),
                            nodal_precession_period: TimeDelta::from_days(31726.88735),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 145.159141,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(759.664853)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.232869959e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Thyone: fitted to 1501 JPL states over 60.0 yr; residual 2641344 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thyone".into()),
                        id: "Thyone".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2136186703,
                            semi_major_axis: 2.098613477e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 147.1454592,
                            longitude_of_ascending_node: 244.6124348,
                            argument_of_periapsis: 87.76537836,
                            apsidal_precession_period: TimeDelta::from_days(48405.02691),
                            nodal_precession_period: TimeDelta::from_days(46033.32451),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 259.0828717,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(626.7094123)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.244506559e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Hermippe: fitted to 1501 JPL states over 60.0 yr; residual 2438372 km RMS (1.1e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hermippe".into()),
                        id: "Hermippe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2067616547,
                            semi_major_axis: 2.113198868e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 151.181303,
                            longitude_of_ascending_node: 332.3147807,
                            argument_of_periapsis: 290.9042449,
                            apsidal_precession_period: TimeDelta::from_days(44579.11887),
                            nodal_precession_period: TimeDelta::from_days(41694.29124),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 143.1673417,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(633.2190696)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.244644686e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Aitne: fitted to 1501 JPL states over 60.0 yr; residual 1989604 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aitne".into()),
                        id: "Aitne".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2660149168,
                            semi_major_axis: 2.324277758e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 194.565632,
                            longitude_of_ascending_node: 183.6492781,
                            argument_of_periapsis: 273.0347858,
                            apsidal_precession_period: TimeDelta::from_days(28248.16398),
                            nodal_precession_period: TimeDelta::from_days(30921.13963),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 105.7478172,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(731.6728201)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.24040208e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eurydome: fitted to 1501 JPL states over 60.0 yr; residual 3674710 km RMS (1.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eurydome".into()),
                        id: "Eurydome".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2775183092,
                            semi_major_axis: 2.287848051e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 150.652456,
                            longitude_of_ascending_node: 302.4042108,
                            argument_of_periapsis: 221.9230559,
                            apsidal_precession_period: TimeDelta::from_days(39371.83689),
                            nodal_precession_period: TimeDelta::from_days(34131.50341),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 288.9566619,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(715.2478545)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.23794302e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Euanthe: fitted to 1501 JPL states over 60.0 yr; residual 3170180 km RMS (1.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Euanthe".into()),
                        id: "Euanthe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2273627489,
                            semi_major_axis: 2.079535185e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 148.4420747,
                            longitude_of_ascending_node: 255.7094726,
                            argument_of_periapsis: 316.209822,
                            apsidal_precession_period: TimeDelta::from_days(50946.48532),
                            nodal_precession_period: TimeDelta::from_days(42974.3961),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 339.4073967,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(619.0268178)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.241115336e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Euporie: fitted to 1501 JPL states over 60.0 yr; residual 775223 km RMS (4.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Euporie".into()),
                        id: "Euporie".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1147057195,
                            semi_major_axis: 1.931495524e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 146.1698941,
                            longitude_of_ascending_node: 61.83240259,
                            argument_of_periapsis: 93.63197543,
                            apsidal_precession_period: TimeDelta::from_days(-1390760.402),
                            nodal_precession_period: TimeDelta::from_days(51559.46816),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 68.07894485,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(544.6086284)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.284826053e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Orthosie: fitted to 1501 JPL states over 60.0 yr; residual 4031723 km RMS (1.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Orthosie".into()),
                        id: "Orthosie".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3029646782,
                            semi_major_axis: 2.079313734e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 145.4090463,
                            longitude_of_ascending_node: 214.0582855,
                            argument_of_periapsis: 225.7962691,
                            apsidal_precession_period: TimeDelta::from_days(63154.8122),
                            nodal_precession_period: TimeDelta::from_days(36312.39547),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 188.5032474,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(618.1113406)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.244396826e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Sponde: fitted to 1501 JPL states over 60.0 yr; residual 4100968 km RMS (1.6e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sponde".into()),
                        id: "Sponde".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3293450796,
                            semi_major_axis: 2.348640925e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 151.1253553,
                            longitude_of_ascending_node: 127.9138925,
                            argument_of_periapsis: 71.31437357,
                            apsidal_precession_period: TimeDelta::from_days(37412.78002),
                            nodal_precession_period: TimeDelta::from_days(34242.65312),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 175.9270446,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(746.9099174)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.228133767e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kale: fitted to 1501 JPL states over 60.0 yr; residual 1957433 km RMS (8.1e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kale".into()),
                        id: "Kale".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2556302867,
                            semi_major_axis: 2.323002499e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.2537712,
                            longitude_of_ascending_node: 61.27433616,
                            argument_of_periapsis: 46.08713986,
                            apsidal_precession_period: TimeDelta::from_days(29589.44748),
                            nodal_precession_period: TimeDelta::from_days(32264.68301),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 212.2381835,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(731.1143809)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.240253981e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Pasithee: fitted to 1501 JPL states over 60.0 yr; residual 1955119 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pasithee".into()),
                        id: "Pasithee".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.269150885,
                            semi_major_axis: 2.300378638e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.7555133,
                            longitude_of_ascending_node: 322.6340144,
                            argument_of_periapsis: 226.9594992,
                            apsidal_precession_period: TimeDelta::from_days(28245.24466),
                            nodal_precession_period: TimeDelta::from_days(30999.30222),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 215.7084291,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(721.0419839)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.238252244e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Hegemone: fitted to 1501 JPL states over 60.0 yr; residual 3579898 km RMS (1.4e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hegemone".into()),
                        id: "Hegemone".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XXXIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3557300524,
                            semi_major_axis: 2.333499319e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 154.8063551,
                            longitude_of_ascending_node: 317.0282811,
                            argument_of_periapsis: 198.3484353,
                            apsidal_precession_period: TimeDelta::from_days(30309.29911),
                            nodal_precession_period: TimeDelta::from_days(30386.64983),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 234.2744872,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(739.7844245)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.227848886e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Mneme: fitted to 1501 JPL states over 60.0 yr; residual 2901781 km RMS (1.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mneme".into()),
                        id: "Mneme".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: Some("Jupiter XL".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2462569451,
                            semi_major_axis: 2.081907575e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 149.588184,
                            longitude_of_ascending_node: 5.436770693,
                            argument_of_periapsis: 45.76417484,
                            apsidal_precession_period: TimeDelta::from_days(61642.32972),
                            nodal_precession_period: TimeDelta::from_days(37481.66898),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 243.217569,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(616.0523689)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.25742278e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Aoede: fitted to 1501 JPL states over 60.0 yr; residual 3894093 km RMS (1.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aoede".into()),
                        id: "Aoede".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4339108929,
                            semi_major_axis: 2.375161862e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 158.0665644,
                            longitude_of_ascending_node: 176.6267725,
                            argument_of_periapsis: 63.80023995,
                            apsidal_precession_period: TimeDelta::from_days(25705.56576),
                            nodal_precession_period: TimeDelta::from_days(28419.56796),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 198.59234,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(763.5868944)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.215332057e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Thelxinoe: fitted to 1501 JPL states over 60.0 yr; residual 2519366 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thelxinoe".into()),
                        id: "Thelxinoe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2183981342,
                            semi_major_axis: 2.100790673e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 150.5636984,
                            longitude_of_ascending_node: 174.3941848,
                            argument_of_periapsis: 314.8989551,
                            apsidal_precession_period: TimeDelta::from_days(48756.316),
                            nodal_precession_period: TimeDelta::from_days(42661.1198),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 271.263769,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(626.9018672)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.247617526e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Arche: fitted to 1501 JPL states over 60.0 yr; residual 1937898 km RMS (8.0e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Arche".into()),
                        id: "Arche".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.255816495,
                            semi_major_axis: 2.326388808e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 194.5325631,
                            longitude_of_ascending_node: 153.5106213,
                            argument_of_periapsis: 346.1343084,
                            apsidal_precession_period: TimeDelta::from_days(28151.74329),
                            nodal_precession_period: TimeDelta::from_days(30770.03392),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 40.0694212,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(733.4644574)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.237715998e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kallichore: fitted to 1501 JPL states over 60.0 yr; residual 1867739 km RMS (7.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kallichore".into()),
                        id: "Kallichore".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2435713721,
                            semi_major_axis: 2.319008926e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.5074012,
                            longitude_of_ascending_node: 26.38218321,
                            argument_of_periapsis: 6.683862842,
                            apsidal_precession_period: TimeDelta::from_days(28834.55086),
                            nodal_precession_period: TimeDelta::from_days(31439.24158),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 54.29562867,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(729.7610179)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.238449184e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Helike: fitted to 1501 JPL states over 60.0 yr; residual 1406293 km RMS (6.6e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Helike".into()),
                        id: "Helike".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1462133042,
                            semi_major_axis: 2.100981761e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 154.8369562,
                            longitude_of_ascending_node: 93.58074302,
                            argument_of_periapsis: 296.7658661,
                            apsidal_precession_period: TimeDelta::from_days(43381.35638),
                            nodal_precession_period: TimeDelta::from_days(42536.18644),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 50.86449452,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(626.1315459)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.251030588e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Carpo: fitted to 1501 JPL states over 60.0 yr; residual 4276643 km RMS (2.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Carpo".into()),
                        id: "Carpo".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4212893646,
                            semi_major_axis: 1.677731546e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 51.13814627,
                            longitude_of_ascending_node: 66.47893241,
                            argument_of_periapsis: 68.4071753,
                            apsidal_precession_period: TimeDelta::from_days(185723.6392),
                            nodal_precession_period: TimeDelta::from_days(-35596.02805),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 352.2325028,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(452.0293154)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.222268056e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eukelade: fitted to 1501 JPL states over 60.0 yr; residual 1968172 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eukelade".into()),
                        id: "Eukelade".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2646847566,
                            semi_major_axis: 2.322395736e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.5034571,
                            longitude_of_ascending_node: 197.628224,
                            argument_of_periapsis: 313.6008596,
                            apsidal_precession_period: TimeDelta::from_days(30641.00098),
                            nodal_precession_period: TimeDelta::from_days(33714.38119),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 203.7072844,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(731.903356)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.236611984e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Cyllene: fitted to 1501 JPL states over 60.0 yr; residual 5104063 km RMS (2.0e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Cyllene".into()),
                        id: "Cyllene".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4251394316,
                            semi_major_axis: 2.34404935e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 150.6307074,
                            longitude_of_ascending_node: 252.4216759,
                            argument_of_periapsis: 193.2147358,
                            apsidal_precession_period: TimeDelta::from_days(29692.50921),
                            nodal_precession_period: TimeDelta::from_days(29318.80853),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 120.7324605,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(751.6648826)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.205546545e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kore: fitted to 1501 JPL states over 60.0 yr; residual 6354156 km RMS (2.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kore".into()),
                        id: "Kore".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3200483999,
                            semi_major_axis: 2.383121511e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 145.0406973,
                            longitude_of_ascending_node: 307.6283988,
                            argument_of_periapsis: 139.1618633,
                            apsidal_precession_period: TimeDelta::from_days(39528.03767),
                            nodal_precession_period: TimeDelta::from_days(29229.45494),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 23.70273876,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(771.3747145)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.202929055e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Herse: fitted to 1501 JPL states over 60.0 yr; residual 1938456 km RMS (8.0e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Herse".into()),
                        id: "Herse".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2489742305,
                            semi_major_axis: 2.33237615e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.1829249,
                            longitude_of_ascending_node: 289.049261,
                            argument_of_periapsis: 322.2786748,
                            apsidal_precession_period: TimeDelta::from_days(28639.10659),
                            nodal_precession_period: TimeDelta::from_days(31367.31095),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 141.8422533,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(736.1565408)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.238191097e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2010 J1: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2010 J1".into()),
                        id: "S2010 J1".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3212028993,
                            semi_major_axis: 2.243089708e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 160.8347004,
                            longitude_of_ascending_node: 284.7281703,
                            argument_of_periapsis: 175.5929107,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 183.4113318,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2010 J2: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2010 J2".into()),
                        id: "S2010 J2".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2056878478,
                            semi_major_axis: 2.112378199e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 146.8412537,
                            longitude_of_ascending_node: 357.2786898,
                            argument_of_periapsis: 28.83546357,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 292.4675021,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Dia: fitted to 1501 JPL states over 60.0 yr; residual 861558 km RMS (6.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dia".into()),
                        id: "Dia".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2336870021,
                            semi_major_axis: 1.22612386e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 27.68032491,
                            longitude_of_ascending_node: 296.8000029,
                            argument_of_periapsis: 179.1290576,
                            apsidal_precession_period: TimeDelta::from_days(45068.08018),
                            nodal_precession_period: TimeDelta::from_days(-82048.9123),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 302.8792361,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(278.9921249)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(7.668109229e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2016 J1: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2016 J1".into()),
                        id: "S2016 J1".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3876314373,
                            semi_major_axis: 2.034428463e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 145.0196897,
                            longitude_of_ascending_node: 247.7413899,
                            argument_of_periapsis: 298.4684741,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 215.3203057,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2003 J18: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2003 J18".into()),
                        id: "S2003 J18".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.06394034837,
                            semi_major_axis: 2.078565475e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 145.8386602,
                            longitude_of_ascending_node: 166.7516005,
                            argument_of_periapsis: 86.1653201,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 252.5603705,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2011 J2: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2011 J2".into()),
                        id: "S2011 J2".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5104080421,
                            semi_major_axis: 2.391254847e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 157.1655043,
                            longitude_of_ascending_node: 24.85633525,
                            argument_of_periapsis: 262.9457027,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 297.5578087,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eirene: fitted to 1501 JPL states over 60.0 yr; residual 1910432 km RMS (7.9e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eirene".into()),
                        id: "Eirene".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2530392786,
                            semi_major_axis: 2.322925718e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.6732998,
                            longitude_of_ascending_node: 184.9131942,
                            argument_of_periapsis: 99.33727672,
                            apsidal_precession_period: TimeDelta::from_days(31021.07384),
                            nodal_precession_period: TimeDelta::from_days(34008.04655),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 329.5647849,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(731.3478203)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(8.325524034e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Philophrosyne: fitted to 1501 JPL states over 60.0 yr; residual 2974230 km RMS (1.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Philophrosyne".into()),
                        id: "Philophrosyne".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2520401791,
                            semi_major_axis: 2.259565279e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 147.2518528,
                            longitude_of_ascending_node: 223.5880349,
                            argument_of_periapsis: 38.09856964,
                            apsidal_precession_period: TimeDelta::from_days(86054.6423),
                            nodal_precession_period: TimeDelta::from_days(34007.41354),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 39.5391935,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(693.8853293)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.267160186e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J1: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J1".into()),
                        id: "S2017 J1".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2935391914,
                            semi_major_axis: 2.264220505e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 145.3933954,
                            longitude_of_ascending_node: 251.8376127,
                            argument_of_periapsis: 74.68605177,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 250.0458015,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eupheme: fitted to 1501 JPL states over 60.0 yr; residual 3099511 km RMS (1.4e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eupheme".into()),
                        id: "Eupheme".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2317083153,
                            semi_major_axis: 2.072521854e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 148.1685278,
                            longitude_of_ascending_node: 232.3619936,
                            argument_of_periapsis: 60.65778181,
                            apsidal_precession_period: TimeDelta::from_days(60856.80606),
                            nodal_precession_period: TimeDelta::from_days(41442.30319),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 358.020574,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(614.8119862)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.62406631e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2003 J19: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2003 J19".into()),
                        id: "S2003 J19".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1591918826,
                            semi_major_axis: 2.36821812e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 165.1147475,
                            longitude_of_ascending_node: 21.88897894,
                            argument_of_periapsis: 186.8140602,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 196.8826603,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Valetudo: fitted to 1501 JPL states over 60.0 yr; residual 2561930 km RMS (1.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Valetudo".into()),
                        id: "Valetudo".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2195471842,
                            semi_major_axis: 1.867727517e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 33.85831562,
                            longitude_of_ascending_node: 291.3290061,
                            argument_of_periapsis: 344.03328,
                            apsidal_precession_period: TimeDelta::from_days(17686.8783),
                            nodal_precession_period: TimeDelta::from_days(-49642.69926),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 57.9195462,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(537.9189012)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(2.42752517e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J2: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J2".into()),
                        id: "S2017 J2".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1960402869,
                            semi_major_axis: 2.205726242e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 164.4410317,
                            longitude_of_ascending_node: 359.4426796,
                            argument_of_periapsis: 153.0441088,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 278.099887,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J3: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J3".into()),
                        id: "S2017 J3".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2127536434,
                            semi_major_axis: 2.152652421e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 149.5741448,
                            longitude_of_ascending_node: 27.92474068,
                            argument_of_periapsis: 105.7233317,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 235.3542414,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Pandia: fitted to 1501 JPL states over 60.0 yr; residual 550926 km RMS (4.7e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pandia".into()),
                        id: "Pandia".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.182100555,
                            semi_major_axis: 1.148499516e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.02933806,
                            longitude_of_ascending_node: 252.0187396,
                            argument_of_periapsis: 198.6089364,
                            apsidal_precession_period: TimeDelta::from_days(55941.34761),
                            nodal_precession_period: TimeDelta::from_days(-98738.92675),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 146.7551076,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(252.4070788)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(8.908078201e+17),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J5: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J5".into()),
                        id: "S2017 J5".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3390546419,
                            semi_major_axis: 2.371772383e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 166.9585497,
                            longitude_of_ascending_node: 42.34182928,
                            argument_of_periapsis: 292.4843144,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 69.26007544,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J6: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J6".into()),
                        id: "S2017 J6".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.347023045,
                            semi_major_axis: 2.252199959e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 142.4457416,
                            longitude_of_ascending_node: 312.7518814,
                            argument_of_periapsis: 15.49794864,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 146.0414186,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J7: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J7".into()),
                        id: "S2017 J7".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.336297321,
                            semi_major_axis: 2.090519093e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 148.2631541,
                            longitude_of_ascending_node: 264.9210419,
                            argument_of_periapsis: 288.7132795,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 352.9308825,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J8: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J8".into()),
                        id: "S2017 J8".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2602936167,
                            semi_major_axis: 2.237498979e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 165.1375023,
                            longitude_of_ascending_node: 97.28611772,
                            argument_of_periapsis: 330.5256738,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 349.3890857,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2017 J9: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J9".into()),
                        id: "S2017 J9".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2942443286,
                            semi_major_axis: 2.119492461e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 156.1109508,
                            longitude_of_ascending_node: 245.3169068,
                            argument_of_periapsis: 276.4180128,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 243.2255548,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Ersa: fitted to 1501 JPL states over 60.0 yr; residual 409900 km RMS (3.6e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ersa".into()),
                        id: "Ersa".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1146682656,
                            semi_major_axis: 1.140722802e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 30.37973227,
                            longitude_of_ascending_node: 114.414106,
                            argument_of_periapsis: 302.5910167,
                            apsidal_precession_period: TimeDelta::from_days(49370.35279),
                            nodal_precession_period: TimeDelta::from_days(-118593.4531),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 268.1732076,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(249.9654657)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.129252994e+18),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2011 J1: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2011 J1".into()),
                        id: "S2011 J1".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1788216129,
                            semi_major_axis: 2.302369511e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 164.1666964,
                            longitude_of_ascending_node: 253.5365372,
                            argument_of_periapsis: 50.38252724,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 225.399633,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Saturn: fitted to 1501 JPL states over 60.0 yr; residual 2194887 km RMS (1.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Saturn".into()),
                        id: "Saturn".to_string(),
                        mass: 5.6834e+26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05410903262,
                            semi_major_axis: 1.427145719e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.487231495,
                            longitude_of_ascending_node: 113.5688489,
                            argument_of_periapsis: 338.6637128,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 317.7218956,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(10753.82418)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.581249855e+14),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58232000.0,
                        color: AppearanceColor { r: 200, g: 171, b: 90 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.210056825, 0.1208977584, 0.32305636, 0.9148193541),
                        0.0001637884058, RotationEpoch::J2000)),
                }),
                // Mimas: fitted to 1503 JPL states over 0.6 yr; residual 1920 km RMS (1.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mimas".into()),
                        id: "Mimas".to_string(),
                        mass: 3.75e+19,
                        major: true,
                        designation: Some("Saturn I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.01965094113,
                            semi_major_axis: 185529101.1,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.77029303,
                            longitude_of_ascending_node: 173.6417428,
                            argument_of_periapsis: 101.5199845,
                            apsidal_precession_period: TimeDelta::from_days(347.8111758),
                            nodal_precession_period: TimeDelta::from_days(-11039.36978),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 43.43903795,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.944933748)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.782384307e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 198800.0,
                        color: AppearanceColor { r: 200, g: 200, b: 210 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Enceladus: fitted to 1500 JPL states over 0.9 yr; residual 25.5 km RMS (1.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Enceladus".into()),
                        id: "Enceladus".to_string(),
                        mass: 1.0805e+20,
                        major: true,
                        designation: Some("Saturn II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00473526651,
                            semi_major_axis: 238035388.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.04179656,
                            longitude_of_ascending_node: 169.5010501,
                            argument_of_periapsis: 132.8372078,
                            apsidal_precession_period: TimeDelta::from_days(1067.800457),
                            nodal_precession_period: TimeDelta::from_days(3982615.955),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.604139775,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.371974733)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.789349007e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 252300.0,
                        color: AppearanceColor { r: 230, g: 230, b: 240 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Tethys: fitted to 1501 JPL states over 1.3 yr; residual 525 km RMS (1.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tethys".into()),
                        id: "Tethys".to_string(),
                        mass: 6.176e+20,
                        major: true,
                        designation: Some("Saturn III".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.68674692e-12,
                            semi_major_axis: 294648817.5,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 27.08224853,
                            longitude_of_ascending_node: 168.1990769,
                            argument_of_periapsis: 157.9614711,
                            apsidal_precession_period: TimeDelta::from_days(82424.96571),
                            nodal_precession_period: TimeDelta::from_days(49046.938),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 350.2871654,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.887905758)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.795646314e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 536300.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Dione: fitted to 1500 JPL states over 1.9 yr; residual 83.2 km RMS (2.2e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dione".into()),
                        id: "Dione".to_string(),
                        mass: 1.09572e+21,
                        major: true,
                        designation: Some("Saturn IV".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.002291365707,
                            semi_major_axis: 377415256.9,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.02289161,
                            longitude_of_ascending_node: 169.4879509,
                            argument_of_periapsis: 173.8351264,
                            apsidal_precession_period: TimeDelta::from_days(4278.023703),
                            nodal_precession_period: TimeDelta::from_days(-6385176.999),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 323.1334086,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2.738667236)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.790635946e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 562500.0,
                        color: AppearanceColor { r: 190, g: 190, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Rhea: fitted to 1500 JPL states over 3.1 yr; residual 476 km RMS (9.0e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Rhea".into()),
                        id: "Rhea".to_string(),
                        mass: 2.309e+21,
                        major: true,
                        designation: Some("Saturn V".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001053366856,
                            semi_major_axis: 527067559.2,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.11967149,
                            longitude_of_ascending_node: 168.8341835,
                            argument_of_periapsis: 170.2997023,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 202.5232779,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(4.51750255)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.794322998e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 764500.0,
                        color: AppearanceColor { r: 195, g: 195, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Titan: fitted to 1501 JPL states over 10.9 yr; residual 278 km RMS (2.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Titan".into()),
                        id: "Titan".to_string(),
                        mass: 1.34553e+23,
                        major: true,
                        designation: Some("Saturn VI".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.02871794313,
                            semi_major_axis: 1221864429.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 27.71932752,
                            longitude_of_ascending_node: 169.1958589,
                            argument_of_periapsis: 164.2062124,
                            apsidal_precession_period: TimeDelta::from_days(243505.4728),
                            nodal_precession_period: TimeDelta::from_days(-22176281.04),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 163.6864639,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(15.94648339)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.793774101e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2575500.0,
                        color: AppearanceColor { r: 210, g: 170, b: 60 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Hyperion: fitted to 1500 JPL states over 14.6 yr; residual 174107 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hyperion".into()),
                        id: "Hyperion".to_string(),
                        mass: 1.08e+19,
                        major: true,
                        designation: Some("Saturn VII".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1014528833,
                            semi_major_axis: 1471543304.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 27.09733509,
                            longitude_of_ascending_node: 168.2161323,
                            argument_of_periapsis: 204.9060052,
                            apsidal_precession_period: TimeDelta::from_days(-6675.155664),
                            nodal_precession_period: TimeDelta::from_days(2676669.255),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 45.11578474,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(21.20899856)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.746373451e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 133000.0,
                        color: AppearanceColor { r: 160, g: 140, b: 120 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Iapetus: fitted to 1500 JPL states over 54.3 yr; residual 20702 km RMS (5.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Iapetus".into()),
                        id: "Iapetus".to_string(),
                        mass: 1.8059e+21,
                        major: true,
                        designation: Some("Saturn VIII".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.02843597956,
                            semi_major_axis: 3560824608.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 17.18642197,
                            longitude_of_ascending_node: 137.8947372,
                            argument_of_periapsis: 230.8725813,
                            apsidal_precession_period: TimeDelta::from_days(1247065.081),
                            nodal_precession_period: TimeDelta::from_days(-7416983.044),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 208.5228768,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(79.3352108)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.793604392e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 734500.0,
                        color: AppearanceColor { r: 150, g: 130, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Phoebe: fitted to 1501 JPL states over 60.0 yr; residual 273765 km RMS (2.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phoebe".into()),
                        id: "Phoebe".to_string(),
                        mass: 8.289e+18,
                        major: false,
                        designation: Some("Saturn IX".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1629859865,
                            semi_major_axis: 1.294479789e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 172.9383147,
                            longitude_of_ascending_node: 262.7869019,
                            argument_of_periapsis: 358.9927516,
                            apsidal_precession_period: TimeDelta::from_days(224733.0434),
                            nodal_precession_period: TimeDelta::from_days(414689.086),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 52.63584735,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(550.9071238)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.779734523e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 106600.0,
                        color: AppearanceColor { r: 90, g: 85, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Janus: fitted to 1500 JPL states over 0.5 yr; residual 302 km RMS (2.0e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Janus".into()),
                        id: "Janus".to_string(),
                        mass: 1.98e+18,
                        major: false,
                        designation: Some("Saturn X".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006742698027,
                            semi_major_axis: 151440940.6,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.05221039,
                            longitude_of_ascending_node: 169.6098479,
                            argument_of_periapsis: 128.9577079,
                            apsidal_precession_period: TimeDelta::from_days(175.1752307),
                            nodal_precession_period: TimeDelta::from_days(-398559.0375),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 114.890348,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6972823895)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.777850374e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 90000.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Epimetheus: fitted to 1501 JPL states over 0.5 yr; residual 571 km RMS (3.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Epimetheus".into()),
                        id: "Epimetheus".to_string(),
                        mass: 5.5e+17,
                        major: false,
                        designation: Some("Saturn XI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.009844047872,
                            semi_major_axis: 151487163.5,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.06126912,
                            longitude_of_ascending_node: 170.1435571,
                            argument_of_periapsis: 237.2667943,
                            apsidal_precession_period: TimeDelta::from_days(174.8716665),
                            nodal_precession_period: TimeDelta::from_days(-50869.28563),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 191.2685171,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6976066378)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.777796366e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58000.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Helene: fitted to 1501 JPL states over 1.9 yr; residual 39612 km RMS (1.0e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Helene".into()),
                        id: "Helene".to_string(),
                        mass: 2.55e+16,
                        major: false,
                        designation: Some("Saturn XII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.007728928334,
                            semi_major_axis: 375295902.6,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.02831459,
                            longitude_of_ascending_node: 170.0016773,
                            argument_of_periapsis: 194.7109046,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 349.2980074,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2.735970439)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.734486717e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 16000.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Telesto: fitted to 1501 JPL states over 1.3 yr; residual 2595 km RMS (8.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Telesto".into()),
                        id: "Telesto".to_string(),
                        mass: 7.2e+15,
                        major: false,
                        designation: Some("Saturn XIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.627988286e-08,
                            semi_major_axis: 294601435.4,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 26.99846749,
                            longitude_of_ascending_node: 169.1997028,
                            argument_of_periapsis: 215.7952185,
                            apsidal_precession_period: TimeDelta::from_days(626525.3067),
                            nodal_precession_period: TimeDelta::from_days(45531.65902),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 349.9744141,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.887819212)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.794163348e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 12600.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Calypso: fitted to 1500 JPL states over 1.3 yr; residual 6630 km RMS (2.2e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Calypso".into()),
                        id: "Calypso".to_string(),
                        mass: 3.6e+15,
                        major: false,
                        designation: Some("Saturn XIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.051137523e-12,
                            semi_major_axis: 294398462.8,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 26.9475809,
                            longitude_of_ascending_node: 167.7322228,
                            argument_of_periapsis: 75.72852207,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 17.52806217,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.887960737)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.785758876e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 10300.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Atlas: fitted to 1498 JPL states over 0.4 yr; residual 15.2 km RMS (1.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Atlas".into()),
                        id: "Atlas".to_string(),
                        mass: 8.4e+15,
                        major: false,
                        designation: Some("Saturn XV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001179722253,
                            semi_major_axis: 137665601.1,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.05075404,
                            longitude_of_ascending_node: 169.5300491,
                            argument_of_periapsis: 47.33164053,
                            apsidal_precession_period: TimeDelta::from_days(124.9841809),
                            nodal_precession_period: TimeDelta::from_days(-11093012.06),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 285.6972075,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6046038484)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.774563566e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15900.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Prometheus: fitted to 1505 JPL states over 0.4 yr; residual 28.6 km RMS (2.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Prometheus".into()),
                        id: "Prometheus".to_string(),
                        mass: 1.4e+17,
                        major: false,
                        designation: Some("Saturn XVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00221936396,
                            semi_major_axis: 139377717.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.05076557,
                            longitude_of_ascending_node: 169.532354,
                            argument_of_periapsis: 316.8955433,
                            apsidal_precession_period: TimeDelta::from_days(130.5962187),
                            nodal_precession_period: TimeDelta::from_days(-4390914.964),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 98.82385895,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6158794082)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.775034489e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 46000.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Pandora: fitted to 1502 JPL states over 0.4 yr; residual 190 km RMS (1.3e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pandora".into()),
                        id: "Pandora".to_string(),
                        mass: 1.3e+17,
                        major: false,
                        designation: Some("Saturn XVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.004222290794,
                            semi_major_axis: 141713376.7,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.0519557,
                            longitude_of_ascending_node: 169.540118,
                            argument_of_periapsis: 172.7848523,
                            apsidal_precession_period: TimeDelta::from_days(138.2732032),
                            nodal_precession_period: TimeDelta::from_days(-48025115.47),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 124.7773058,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.6313805791)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.775568488e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 41500.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Pan: fitted to 1505 JPL states over 0.4 yr; residual 1.36 km RMS (1.0e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pan".into()),
                        id: "Pan".to_string(),
                        mass: 4.95e+15,
                        major: false,
                        designation: Some("Saturn XVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.476321203e-06,
                            semi_major_axis: 133584448.7,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.05118269,
                            longitude_of_ascending_node: 169.5270064,
                            argument_of_periapsis: 142.4567868,
                            apsidal_precession_period: TimeDelta::from_days(119.903013),
                            nodal_precession_period: TimeDelta::from_days(-238135903.8),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 324.1622557,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.5778219318)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.775825502e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14300.0,
                        color: AppearanceColor { r: 150, g: 150, b: 150 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Saturn",
                        DVec3::new(0.08547883186, 0.4624416774, 0.8825197246))),
                }),
                // Ymir: fitted to 1501 JPL states over 60.0 yr; residual 1889628 km RMS (7.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ymir".into()),
                        id: "Ymir".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.337852608,
                            semi_major_axis: 2.305278041e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 172.1737466,
                            longitude_of_ascending_node: 207.2826179,
                            argument_of_periapsis: 35.73477504,
                            apsidal_precession_period: TimeDelta::from_days(82624.39218),
                            nodal_precession_period: TimeDelta::from_days(107859.0498),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 231.1707896,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1320.411203)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.716069186e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Paaliaq: fitted to 1501 JPL states over 60.0 yr; residual 1963347 km RMS (1.1e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Paaliaq".into()),
                        id: "Paaliaq".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5457324774,
                            semi_major_axis: 1.49550774e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 41.39981589,
                            longitude_of_ascending_node: 353.029593,
                            argument_of_periapsis: 237.1368109,
                            apsidal_precession_period: TimeDelta::from_days(227231.2376),
                            nodal_precession_period: TimeDelta::from_days(-121217.9932),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 306.4510169,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(685.6088625)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.763096897e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Tarvos: fitted to 1501 JPL states over 60.0 yr; residual 3624136 km RMS (1.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tarvos".into()),
                        id: "Tarvos".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5039949638,
                            semi_major_axis: 1.804865781e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: -39.10601253,
                            longitude_of_ascending_node: 264.4834488,
                            argument_of_periapsis: 101.1617408,
                            apsidal_precession_period: TimeDelta::from_days(84439.12911),
                            nodal_precession_period: TimeDelta::from_days(-151809.3555),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 268.3997442,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(929.8700869)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.596022753e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Ijiraq: fitted to 1501 JPL states over 60.0 yr; residual 902760 km RMS (7.3e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ijiraq".into()),
                        id: "Ijiraq".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4201105213,
                            semi_major_axis: 1.133273619e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 48.39201786,
                            longitude_of_ascending_node: 151.9785814,
                            argument_of_periapsis: 68.13595725,
                            apsidal_precession_period: TimeDelta::from_days(1513065.944),
                            nodal_precession_period: TimeDelta::from_days(-298536.3773),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 29.91383127,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(450.9328846)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.785408681e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Suttungr: fitted to 1501 JPL states over 60.0 yr; residual 506135 km RMS (2.6e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Suttungr".into()),
                        id: "Suttungr".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1143159238,
                            semi_major_axis: 1.946637223e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 174.026362,
                            longitude_of_ascending_node: 252.6884842,
                            argument_of_periapsis: 59.37924596,
                            apsidal_precession_period: TimeDelta::from_days(143124.6901),
                            nodal_precession_period: TimeDelta::from_days(227205.6774),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 321.128239,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1019.332012)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.754533604e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kiviuq: fitted to 1501 JPL states over 60.0 yr; residual 221710 km RMS (1.9e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kiviuq".into()),
                        id: "Kiviuq".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1553020294,
                            semi_major_axis: 1.130597844e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 48.73812447,
                            longitude_of_ascending_node: 351.786575,
                            argument_of_periapsis: 93.59431678,
                            apsidal_precession_period: TimeDelta::from_days(-480322.6395),
                            nodal_precession_period: TimeDelta::from_days(-482270.5733),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 168.6343819,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(448.2907031)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.803095629e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Mundilfari: fitted to 1501 JPL states over 60.0 yr; residual 760260 km RMS (4.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mundilfari".into()),
                        id: "Mundilfari".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2045583246,
                            semi_major_axis: 1.864820498e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 169.2933114,
                            longitude_of_ascending_node: 78.44146731,
                            argument_of_periapsis: 301.3752628,
                            apsidal_precession_period: TimeDelta::from_days(97869.94176),
                            nodal_precession_period: TimeDelta::from_days(136278.92),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 101.3047339,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(955.5661611)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.755967543e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Albiorix: fitted to 1501 JPL states over 60.0 yr; residual 2252190 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Albiorix".into()),
                        id: "Albiorix".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5390500988,
                            semi_major_axis: 1.628583147e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 35.11828433,
                            longitude_of_ascending_node: 111.6075227,
                            argument_of_periapsis: 55.35547672,
                            apsidal_precession_period: TimeDelta::from_days(94735.62665),
                            nodal_precession_period: TimeDelta::from_days(-98049.74636),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 26.84839282,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(784.2471959)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.714121541e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Skathi: fitted to 1501 JPL states over 60.0 yr; residual 908088 km RMS (5.6e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skathi".into()),
                        id: "Skathi".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2745580651,
                            semi_major_axis: 1.55808288e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 149.7399617,
                            longitude_of_ascending_node: 283.199237,
                            argument_of_periapsis: 212.5826415,
                            apsidal_precession_period: TimeDelta::from_days(223787.6883),
                            nodal_precession_period: TimeDelta::from_days(229376.5729),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 104.1597289,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(728.1238885)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.773056939e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Erriapus: fitted to 1501 JPL states over 60.0 yr; residual 3140629 km RMS (1.6e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Erriapus".into()),
                        id: "Erriapus".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4415769229,
                            semi_major_axis: 1.738101509e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 40.23760464,
                            longitude_of_ascending_node: 132.4736844,
                            argument_of_periapsis: 288.9680166,
                            apsidal_precession_period: TimeDelta::from_days(90430.90983),
                            nodal_precession_period: TimeDelta::from_days(-185673.8215),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 304.098254,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(874.8863685)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.6278921e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Siarnaq: fitted to 1501 JPL states over 60.0 yr; residual 1994084 km RMS (1.0e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Siarnaq".into()),
                        id: "Siarnaq".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4594389004,
                            semi_major_axis: 1.784422363e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 44.08433532,
                            longitude_of_ascending_node: 64.96366846,
                            argument_of_periapsis: 67.59932263,
                            apsidal_precession_period: TimeDelta::from_days(210040.0893),
                            nodal_precession_period: TimeDelta::from_days(-105039.1767),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 189.5334448,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(893.0796075)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.767427279e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Thrymr: fitted to 1501 JPL states over 60.0 yr; residual 2039922 km RMS (9.0e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thrymr".into()),
                        id: "Thrymr".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.468814725,
                            semi_major_axis: 2.033931112e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 173.9542034,
                            longitude_of_ascending_node: 254.0113373,
                            argument_of_periapsis: 93.07640508,
                            apsidal_precession_period: TimeDelta::from_days(108925.8696),
                            nodal_precession_period: TimeDelta::from_days(169018.0877),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 357.5120161,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1095.560449)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.707390048e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Narvi: fitted to 1501 JPL states over 60.0 yr; residual 2198146 km RMS (1.1e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Narvi".into()),
                        id: "Narvi".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3418718465,
                            semi_major_axis: 1.921892057e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 139.1502579,
                            longitude_of_ascending_node: 179.985797,
                            argument_of_periapsis: 174.6957255,
                            apsidal_precession_period: TimeDelta::from_days(129181.7438),
                            nodal_precession_period: TimeDelta::from_days(203151.4311),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 115.5846083,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1006.483744)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.706002412e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Methone: fitted to 1503 JPL states over 0.7 yr; residual 1379 km RMS (7.1e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Methone".into()),
                        id: "Methone".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0004971195557,
                            semi_major_axis: 194241334.2,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.03982159,
                            longitude_of_ascending_node: 169.5306321,
                            argument_of_periapsis: 193.3564454,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 317.561108,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.00969729)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.801673609e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Pallene: fitted to 1501 JPL states over 0.8 yr; residual 166 km RMS (7.8e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pallene".into()),
                        id: "Pallene".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.003993994727,
                            semi_major_axis: 212282834.6,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 27.94340435,
                            longitude_of_ascending_node: 169.1408101,
                            argument_of_periapsis: 99.66625651,
                            apsidal_precession_period: TimeDelta::from_days(580.2236102),
                            nodal_precession_period: TimeDelta::from_days(126853.1635),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.819519899,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.156053704)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.785479224e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Polydeuces: fitted to 1501 JPL states over 1.9 yr; residual 103280 km RMS (2.7e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Polydeuces".into()),
                        id: "Polydeuces".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.01794643348,
                            semi_major_axis: 363029646.8,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.18567654,
                            longitude_of_ascending_node: 168.9943659,
                            argument_of_periapsis: 13.33413569,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 72.88566868,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2.737733226)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.375796648e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Daphnis: fitted to 1500 JPL states over 0.4 yr; residual 3.27 km RMS (2.4e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Daphnis".into()),
                        id: "Daphnis".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.085039189e-05,
                            semi_major_axis: 136505542.3,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.05194766,
                            longitude_of_ascending_node: 169.5261288,
                            argument_of_periapsis: 46.0712823,
                            apsidal_precession_period: TimeDelta::from_days(121.2503807),
                            nodal_precession_period: TimeDelta::from_days(17927421.33),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 67.54051186,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.5970049534)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.774220217e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Aegir: fitted to 1501 JPL states over 60.0 yr; residual 1198390 km RMS (5.6e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aegir".into()),
                        id: "Aegir".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2623679221,
                            semi_major_axis: 2.07397031e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 166.6954068,
                            longitude_of_ascending_node: 187.49166,
                            argument_of_periapsis: 252.3913075,
                            apsidal_precession_period: TimeDelta::from_days(101985.3073),
                            nodal_precession_period: TimeDelta::from_days(122435.3774),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 23.94824668,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1121.280613)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.752418169e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Bebhionn: fitted to 1501 JPL states over 60.0 yr; residual 3068992 km RMS (1.6e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bebhionn".into()),
                        id: "Bebhionn".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4667986546,
                            semi_major_axis: 1.693827466e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 37.92821816,
                            longitude_of_ascending_node: 198.8431161,
                            argument_of_periapsis: 9.963114708,
                            apsidal_precession_period: TimeDelta::from_days(105404.7447),
                            nodal_precession_period: TimeDelta::from_days(-154028.1529),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 157.6633224,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(836.6298992)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.67174914e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Bergelmir: fitted to 1501 JPL states over 60.0 yr; residual 599916 km RMS (3.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bergelmir".into()),
                        id: "Bergelmir".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1286798775,
                            semi_major_axis: 1.93210355e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 157.4274155,
                            longitude_of_ascending_node: 210.5171596,
                            argument_of_periapsis: 134.4565289,
                            apsidal_precession_period: TimeDelta::from_days(122766.5376),
                            nodal_precession_period: TimeDelta::from_days(171338.4263),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 313.3191088,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1007.783483)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.755683401e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Bestla: fitted to 1501 JPL states over 60.0 yr; residual 4352700 km RMS (1.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bestla".into()),
                        id: "Bestla".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5697612336,
                            semi_major_axis: 2.010593295e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 142.8789677,
                            longitude_of_ascending_node: 299.7695976,
                            argument_of_periapsis: 92.99766525,
                            apsidal_precession_period: TimeDelta::from_days(140731.8681),
                            nodal_precession_period: TimeDelta::from_days(110937.6726),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 251.6408016,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1084.834944)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.652393907e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Farbauti: fitted to 1501 JPL states over 60.0 yr; residual 1088830 km RMS (5.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Farbauti".into()),
                        id: "Farbauti".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2184612614,
                            semi_major_axis: 2.034209581e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 157.0966407,
                            longitude_of_ascending_node: 139.2772282,
                            argument_of_periapsis: 342.6960035,
                            apsidal_precession_period: TimeDelta::from_days(101329.1652),
                            nodal_precession_period: TimeDelta::from_days(148405.4846),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 285.6576454,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1091.011703)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.739904564e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Fenrir: fitted to 1501 JPL states over 60.0 yr; residual 782039 km RMS (3.4e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Fenrir".into()),
                        id: "Fenrir".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1290146347,
                            semi_major_axis: 2.244643856e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 162.7888845,
                            longitude_of_ascending_node: 235.9415134,
                            argument_of_periapsis: 125.4561504,
                            apsidal_precession_period: TimeDelta::from_days(106336.8787),
                            nodal_precession_period: TimeDelta::from_days(142811.9644),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 137.317998,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1264.14971)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.742635777e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Fornjot: fitted to 1501 JPL states over 60.0 yr; residual 1447928 km RMS (5.7e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Fornjot".into()),
                        id: "Fornjot".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.208034802,
                            semi_major_axis: 2.509863447e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 167.8266882,
                            longitude_of_ascending_node: 267.8899852,
                            argument_of_periapsis: 330.7714383,
                            apsidal_precession_period: TimeDelta::from_days(100036.7857),
                            nodal_precession_period: TimeDelta::from_days(132909.7597),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 215.6763985,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1499.331493)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.719522637e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Hati: fitted to 1501 JPL states over 60.0 yr; residual 1633819 km RMS (7.7e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hati".into()),
                        id: "Hati".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3794265667,
                            semi_major_axis: 1.973845875e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.943006,
                            longitude_of_ascending_node: 316.931268,
                            argument_of_periapsis: 15.47512452,
                            apsidal_precession_period: TimeDelta::from_days(106488.8046),
                            nodal_precession_period: TimeDelta::from_days(137450.9593),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 176.0295365,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1042.61954)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.7412803e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Hyrrokkin: fitted to 1501 JPL states over 60.0 yr; residual 1598146 km RMS (8.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hyrrokkin".into()),
                        id: "Hyrrokkin".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.36629218,
                            semi_major_axis: 1.833543757e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 153.2084036,
                            longitude_of_ascending_node: 44.63207774,
                            argument_of_periapsis: 268.1790755,
                            apsidal_precession_period: TimeDelta::from_days(136923.841),
                            nodal_precession_period: TimeDelta::from_days(127652.3015),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 294.2676008,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(931.3257937)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.758398495e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kari: fitted to 1501 JPL states over 60.0 yr; residual 3068149 km RMS (1.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kari".into()),
                        id: "Kari".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4578290209,
                            semi_major_axis: 2.20045998e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 150.547135,
                            longitude_of_ascending_node: 285.6831857,
                            argument_of_periapsis: 167.3087892,
                            apsidal_precession_period: TimeDelta::from_days(109388.3012),
                            nodal_precession_period: TimeDelta::from_days(159501.3919),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 286.6650824,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1234.922138)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.694822016e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Loge: fitted to 1501 JPL states over 60.0 yr; residual 1171191 km RMS (5.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Loge".into()),
                        id: "Loge".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1948486723,
                            semi_major_axis: 2.304449113e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 167.7215671,
                            longitude_of_ascending_node: 333.528197,
                            argument_of_periapsis: 23.81516307,
                            apsidal_precession_period: TimeDelta::from_days(95721.10744),
                            nodal_precession_period: TimeDelta::from_days(114280.8916),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 335.3078224,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1314.468481)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.745702367e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Skoll: fitted to 1501 JPL states over 60.0 yr; residual 1747114 km RMS (8.9e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skoll".into()),
                        id: "Skoll".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4688748621,
                            semi_major_axis: 1.762042021e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 158.4098763,
                            longitude_of_ascending_node: 292.9601078,
                            argument_of_periapsis: 193.2134525,
                            apsidal_precession_period: TimeDelta::from_days(132680.3992),
                            nodal_precession_period: TimeDelta::from_days(173674.553),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 38.65872495,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(879.6881462)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.738725289e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Surtur: fitted to 1501 JPL states over 60.0 yr; residual 2466391 km RMS (9.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Surtur".into()),
                        id: "Surtur".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4414400738,
                            semi_major_axis: 2.280505618e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 165.7881386,
                            longitude_of_ascending_node: 257.5580043,
                            argument_of_periapsis: 322.920014,
                            apsidal_precession_period: TimeDelta::from_days(107450.3049),
                            nodal_precession_period: TimeDelta::from_days(166536.5402),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 149.5692334,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1301.602385)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.702277943e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Anthe: fitted to 1500 JPL states over 0.7 yr; residual 1572 km RMS (8.0e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Anthe".into()),
                        id: "Anthe".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001095701791,
                            semi_major_axis: 197632532.3,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.04205004,
                            longitude_of_ascending_node: 169.4839546,
                            argument_of_periapsis: 55.01676105,
                            apsidal_precession_period: TimeDelta::from_days(408.216436),
                            nodal_precession_period: TimeDelta::from_days(812939.1839),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 179.7543987,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.038912553)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.782244241e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Jarnsaxa: fitted to 1501 JPL states over 60.0 yr; residual 894184 km RMS (4.5e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jarnsaxa".into()),
                        id: "Jarnsaxa".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2311169781,
                            semi_major_axis: 1.933146878e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 164.6871826,
                            longitude_of_ascending_node: 11.10866355,
                            argument_of_periapsis: 227.8651443,
                            apsidal_precession_period: TimeDelta::from_days(108758.1417),
                            nodal_precession_period: TimeDelta::from_days(125640.7225),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 196.3605004,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1007.724836)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.762208711e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Greip: fitted to 1501 JPL states over 60.0 yr; residual 1118617 km RMS (5.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Greip".into()),
                        id: "Greip".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.314116974,
                            semi_major_axis: 1.842420619e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 173.4306828,
                            longitude_of_ascending_node: 336.9511433,
                            argument_of_periapsis: 139.6220749,
                            apsidal_precession_period: TimeDelta::from_days(133008.3636),
                            nodal_precession_period: TimeDelta::from_days(211087.0644),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 314.5686287,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(939.4780414)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.74735934e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Tarqeq: fitted to 1501 JPL states over 60.0 yr; residual 1119011 km RMS (6.2e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tarqeq".into()),
                        id: "Tarqeq".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1454805932,
                            semi_major_axis: 1.773481524e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 49.56049669,
                            longitude_of_ascending_node: 94.57749495,
                            argument_of_periapsis: 65.45675944,
                            apsidal_precession_period: TimeDelta::from_days(1298553.311),
                            nodal_precession_period: TimeDelta::from_days(-247152.195),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 124.6825021,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(882.4468185)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.788219834e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Aegaeon: fitted to 1502 JPL states over 0.6 yr; residual 32.7 km RMS (2.0e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aegaeon".into()),
                        id: "Aegaeon".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 8.132834628e-05,
                            semi_major_axis: 167490958.7,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.0513713,
                            longitude_of_ascending_node: 169.5274058,
                            argument_of_periapsis: 21.95471722,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 42.886032,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.808092412)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.805259681e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Gridr: fitted to 1501 JPL states over 60.0 yr; residual 776140 km RMS (4.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gridr".into()),
                        id: "Gridr".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1824845551,
                            semi_major_axis: 1.931403639e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 163.1525819,
                            longitude_of_ascending_node: 333.8947063,
                            argument_of_periapsis: 281.7997193,
                            apsidal_precession_period: TimeDelta::from_days(125595.8804),
                            nodal_precession_period: TimeDelta::from_days(161572.1729),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 128.4037407,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1006.421098)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.761767264e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Angrboda: fitted to 1501 JPL states over 60.0 yr; residual 987701 km RMS (4.7e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Angrboda".into()),
                        id: "Angrboda".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2144338257,
                            semi_major_axis: 2.067847968e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 177.2356413,
                            longitude_of_ascending_node: 277.9731552,
                            argument_of_periapsis: 90.4770041,
                            apsidal_precession_period: TimeDelta::from_days(263274.35),
                            nodal_precession_period: TimeDelta::from_days(950074.971),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 112.6176653,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1117.336207)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.745590749e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Skrymir: fitted to 1501 JPL states over 60.0 yr; residual 2157839 km RMS (9.1e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skrymir".into()),
                        id: "Skrymir".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4351634256,
                            semi_major_axis: 2.150163021e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 176.6433756,
                            longitude_of_ascending_node: 213.6121232,
                            argument_of_periapsis: 113.2560173,
                            apsidal_precession_period: TimeDelta::from_days(120211.4664),
                            nodal_precession_period: TimeDelta::from_days(183726.137),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 53.10447245,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1188.861913)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.71949166e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Gerd: fitted to 1501 JPL states over 60.0 yr; residual 2430074 km RMS (1.0e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gerd".into()),
                        id: "Gerd".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5203978778,
                            semi_major_axis: 2.095849044e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 172.5351281,
                            longitude_of_ascending_node: 267.3824917,
                            argument_of_periapsis: 314.282973,
                            apsidal_precession_period: TimeDelta::from_days(127491.2578),
                            nodal_precession_period: TimeDelta::from_days(231171.3687),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 207.2374509,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1147.758276)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.69582486e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2004 S26: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S26".into()),
                        id: "S2004 S26".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.08893405434,
                            semi_major_axis: 2.710737685e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 170.7149964,
                            longitude_of_ascending_node: 328.8113586,
                            argument_of_periapsis: 148.3657215,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 131.2398057,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eggther: fitted to 1501 JPL states over 60.0 yr; residual 664745 km RMS (3.3e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eggther".into()),
                        id: "Eggther".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1492072625,
                            semi_major_axis: 1.991607327e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 167.2744418,
                            longitude_of_ascending_node: 83.12053282,
                            argument_of_periapsis: 135.6415891,
                            apsidal_precession_period: TimeDelta::from_days(93157.27382),
                            nodal_precession_period: TimeDelta::from_days(129028.3562),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 272.3524268,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1055.445076)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(9.456348211e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2004 S29: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S29".into()),
                        id: "S2004 S29".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4778123509,
                            semi_major_axis: 1.685697436e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 38.70210508,
                            longitude_of_ascending_node: 157.9106166,
                            argument_of_periapsis: 309.4641332,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 199.9696747,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Beli: fitted to 1501 JPL states over 60.0 yr; residual 549790 km RMS (2.6e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Beli".into()),
                        id: "Beli".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.09470048209,
                            semi_major_axis: 2.07789574e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 156.856918,
                            longitude_of_ascending_node: 257.597841,
                            argument_of_periapsis: 262.4432006,
                            apsidal_precession_period: TimeDelta::from_days(165880.4324),
                            nodal_precession_period: TimeDelta::from_days(157907.3377),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 349.0180026,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1121.416588)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(2.014360417e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Gunnlod: fitted to 1501 JPL states over 60.0 yr; residual 1302338 km RMS (5.9e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gunnlod".into()),
                        id: "Gunnlod".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2657428553,
                            semi_major_axis: 2.120669864e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 159.2034818,
                            longitude_of_ascending_node: 292.8420401,
                            argument_of_periapsis: 40.24233646,
                            apsidal_precession_period: TimeDelta::from_days(131011.6728),
                            nodal_precession_period: TimeDelta::from_days(129664.7967),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 157.4040402,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1157.986288)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.761357496e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Thiazzi: fitted to 1501 JPL states over 60.0 yr; residual 3109101 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thiazzi".into()),
                        id: "Thiazzi".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.4956210607,
                            semi_major_axis: 2.357742053e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 159.7722125,
                            longitude_of_ascending_node: 78.75715257,
                            argument_of_periapsis: 309.8867436,
                            apsidal_precession_period: TimeDelta::from_days(85448.46037),
                            nodal_precession_period: TimeDelta::from_days(130356.0923),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 119.1337226,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1372.820598)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.677846059e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // S2004 S34: JPL publishes no ephemeris under this designation, so these
                // are the elements the save already carried, unmodified.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S34".into()),
                        id: "S2004 S34".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3001902862,
                            semi_major_axis: 2.434729891e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 164.6782006,
                            longitude_of_ascending_node: 292.5798181,
                            argument_of_periapsis: 354.490949,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 25.14445375,
                        }),
                        anomalistic_period: None,
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Alvaldi: fitted to 1501 JPL states over 60.0 yr; residual 1257432 km RMS (5.5e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Alvaldi".into()),
                        id: "Alvaldi".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2370179979,
                            semi_major_axis: 2.209880263e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 177.2343444,
                            longitude_of_ascending_node: 324.8418053,
                            argument_of_periapsis: 195.5263238,
                            apsidal_precession_period: TimeDelta::from_days(175158.268),
                            nodal_precession_period: TimeDelta::from_days(344171.4926),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 317.5537413,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1236.074057)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(4.995691732e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Geirrod: fitted to 1501 JPL states over 60.0 yr; residual 3401494 km RMS (1.3e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Geirrod".into()),
                        id: "Geirrod".to_string(),
                        mass: 8.7e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5338986388,
                            semi_major_axis: 2.220147299e+10,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 157.3543869,
                            longitude_of_ascending_node: 123.2198775,
                            argument_of_periapsis: 346.2129897,
                            apsidal_precession_period: TimeDelta::from_days(86579.99922),
                            nodal_precession_period: TimeDelta::from_days(121459.0954),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 100.649553,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1256.388074)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(3.666316971e+16),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 80, g: 80, b: 80 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Uranus: fitted to 1501 JPL states over 60.0 yr; residual 814615 km RMS (2.8e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Uranus".into()),
                        id: "Uranus".to_string(),
                        mass: 8.681e+25,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0473687244,
                            semi_major_axis: 2.870548814e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 0.7718745765,
                            longitude_of_ascending_node: 74.00768962,
                            argument_of_periapsis: 97.12359419,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 142.0814888,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(30679.95175)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328977771e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 25362000.0,
                        color: AppearanceColor { r: 60, g: 186, b: 180 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.2702946672, -0.5997992937, 0.7369158477, -0.1553596912),
                        -0.0001012371956, RotationEpoch::J2000)),
                }),
                // Neptune: fitted to 1501 JPL states over 60.0 yr; residual 834285 km RMS (1.9e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Neptune".into()),
                        id: "Neptune".to_string(),
                        mass: 1.02409e+26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.008709753444,
                            semi_major_axis: 4.498992087e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.770268368,
                            longitude_of_ascending_node: 131.785231,
                            argument_of_periapsis: 272.6356548,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 260.4720594,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(60200.94837)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328835659e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 24622000.0,
                        color: AppearanceColor { r: 60, g: 186, b: 180 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(0.05300821663, -0.2362720466, 0.7790265757, -0.5783452631),
                        0.0001083382528, RotationEpoch::J2000)),
                }),
                // Pluto Barycenter: fitted to 1501 JPL states over 60.0 yr; residual 828134 km RMS (1.5e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pluto Barycenter".into()),
                        id: "Pluto Barycenter".to_string(),
                        mass: 1.4656e+22,
                        major: false,
                        designation: None,
                        tags: vec!["Barycenter".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2490476448,
                            semi_major_axis: 5.90759453e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 17.14041643,
                            longitude_of_ascending_node: 110.302648,
                            argument_of_periapsis: 113.7709197,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 14.84948179,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(90578.63787)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.32896248e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1000.0,
                        color: AppearanceColor { r: 120, g: 120, b: 120 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Pluto: fitted to 1500 JPL states over 4.4 yr; residual 0.154 km RMS (7.2e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pluto".into()),
                        id: "Pluto".to_string(),
                        mass: 1.307e+22,
                        major: true,
                        designation: Some("134340 Pluto".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0001605983187,
                            semi_major_axis: 2131510.142,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 112.8908007,
                            longitude_of_ascending_node: 227.3916778,
                            argument_of_periapsis: 352.5817879,
                            apsidal_precession_period: TimeDelta::from_days(-114250291.2),
                            nodal_precession_period: TimeDelta::from_days(-1241625589.0),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 148.6688214,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(6.387221787)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1255366308.0),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1188300.0,
                        color: AppearanceColor { r: 200, g: 180, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Charon",
                        DVec3::new(-0.678037266, 0.6236690791, -0.388976022))),
                }),
                // Charon: fitted to 1500 JPL states over 4.4 yr; residual 0.176 km RMS (1.0e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Charon".into()),
                        id: "Charon".to_string(),
                        mass: 1.586e+21,
                        major: true,
                        designation: Some("Pluto I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0001609378765,
                            semi_major_axis: 17464254.34,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 112.8907943,
                            longitude_of_ascending_node: 227.3916835,
                            argument_of_periapsis: 172.4619461,
                            apsidal_precession_period: TimeDelta::from_days(3509039655.0),
                            nodal_precession_period: TimeDelta::from_days(-1239152734.0),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 148.7886797,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(6.387222158)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.904915222e+11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 606000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Pluto",
                        DVec3::new(-0.6780373547, 0.6236688671, -0.3889762073))),
                }),
                // Styx: fitted to 1500 JPL states over 14.5 yr; residual 278 km RMS (6.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Styx".into()),
                        id: "Styx".to_string(),
                        mass: 6.07e+15,
                        major: false,
                        designation: Some("Pluto V".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.82884551e-13,
                            semi_major_axis: 42408675.07,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 112.8884548,
                            longitude_of_ascending_node: 227.3988454,
                            argument_of_periapsis: 188.2553481,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 19.60669711,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(20.1619168)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(9.922765515e+11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5200.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Nix: fitted to 1500 JPL states over 17.4 yr; residual 88.4 km RMS (1.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Nix".into()),
                        id: "Nix".to_string(),
                        mass: 2.24e+16,
                        major: false,
                        designation: Some("Pluto II".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.002048278839,
                            semi_major_axis: 48689519.44,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 112.8908917,
                            longitude_of_ascending_node: 227.3954103,
                            argument_of_periapsis: 164.5050808,
                            apsidal_precession_period: TimeDelta::from_days(1900.746372),
                            nodal_precession_period: TimeDelta::from_days(-275795284.1),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 304.719563,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(25.18403464)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(9.624719913e+11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18000.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Kerberos: fitted to 1501 JPL states over 22.4 yr; residual 417 km RMS (7.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kerberos".into()),
                        id: "Kerberos".to_string(),
                        mass: 9.05e+15,
                        major: false,
                        designation: Some("Pluto IV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0004038607066,
                            semi_major_axis: 57746766.53,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 112.8594248,
                            longitude_of_ascending_node: 227.3661927,
                            argument_of_periapsis: 203.915999,
                            apsidal_precession_period: TimeDelta::from_days(-102585.1693),
                            nodal_precession_period: TimeDelta::from_days(-54078891.04),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 80.98207497,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(32.15785308)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(9.847844276e+11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6000.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Hydra: fitted to 1500 JPL states over 26.6 yr; residual 216 km RMS (3.3e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hydra".into()),
                        id: "Hydra".to_string(),
                        mass: 3.01e+16,
                        major: false,
                        designation: Some("Pluto III".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.005568428978,
                            semi_major_axis: 64718806.66,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 112.8958578,
                            longitude_of_ascending_node: 227.5004397,
                            argument_of_periapsis: 275.3037396,
                            apsidal_precession_period: TimeDelta::from_days(5122.156462),
                            nodal_precession_period: TimeDelta::from_days(-15231230.06),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 334.826681,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(38.48911776)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(9.677158211e+11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18500.0,
                        color: AppearanceColor { r: 100, g: 100, b: 100 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Eris: fitted to 1501 JPL states over 60.0 yr; residual 830549 km RMS (5.9e-05 of orbit radius).
                // radius: halved: the save stored a diameter
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eris".into()),
                        id: "Eris".to_string(),
                        mass: 1.6466e+22,
                        major: true,
                        designation: Some("136199 Eris".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.438243992,
                            semi_major_axis: 1.014951548e+13,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 43.99256682,
                            longitude_of_ascending_node: 35.9763874,
                            argument_of_periapsis: 151.2119814,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 193.8592435,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(204012.3238)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328481091e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1163000.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.2582014006, 0.01073896545, 0.0, 0.9660314236),
                        6.738722981e-05, RotationEpoch::J2000)),
                }),
                // Dysnomia: fitted to 1500 JPL states over 10.8 yr; residual 0.182 km RMS (4.9e-06 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dysnomia".into()),
                        id: "Dysnomia".to_string(),
                        mass: 8.2e+19,
                        major: true,
                        designation: Some("136199 Eris I".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Eris".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006111032471,
                            semi_major_axis: 37272973.46,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 61.60986879,
                            longitude_of_ascending_node: 139.0977677,
                            argument_of_periapsis: 155.5590929,
                            apsidal_precession_period: TimeDelta::from_days(2.577612974e+12),
                            nodal_precession_period: TimeDelta::from_days(-4433514221.0),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 114.8918443,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(15.78590265)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.098943922e+12),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 615000.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::tidally_locked("Eris",
                        DVec3::new(0.02074835616, 0.4988613332, 0.8664334227))),
                }),
                // Sedna: fitted to 1501 JPL states over 60.0 yr; residual 831277 km RMS (6.7e-05 of orbit radius).
                // radius: halved: the save stored a diameter
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sedna".into()),
                        id: "Sedna".to_string(),
                        mass: 2e+21,
                        major: true,
                        designation: Some("90377 Sedna".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.8499442797,
                            semi_major_axis: 7.596094482e+13,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 11.92844176,
                            longitude_of_ascending_node: 144.4008044,
                            argument_of_periapsis: 311.2763435,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 357.6040748,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(4176942.981)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328575477e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 453000.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(BodyRotation::spinning(
                        DQuat::from_xyzw(-0.2031230389, 3.12680072e-17, 0.0, 0.9791532214),
                        0.0001698947972, RotationEpoch::J2000)),
                }),
                // Ariel: fitted to 1500 JPL states over 1.7 yr; residual 62.2 km RMS (3.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ariel".into()),
                        id: "Ariel".to_string(),
                        mass: 1.250018448e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001408308797,
                            semi_major_axis: 190929537.9,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 97.70468123,
                            longitude_of_ascending_node: 167.6402235,
                            argument_of_periapsis: 48.16887304,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 149.9743418,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2.520381792)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.794539786e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 578897.9067,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Umbriel: fitted to 1500 JPL states over 2.8 yr; residual 227 km RMS (8.5e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Umbriel".into()),
                        id: "Umbriel".to_string(),
                        mass: 1.279534645e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.003804971871,
                            semi_major_axis: 265981148.4,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 97.66579949,
                            longitude_of_ascending_node: 167.6409154,
                            argument_of_periapsis: 345.075788,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 261.0936031,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(4.144174822)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.794402859e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 584700.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Titania: fitted to 1500 JPL states over 6.0 yr; residual 607 km RMS (1.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Titania".into()),
                        id: "Titania".to_string(),
                        mass: 3.338177036e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.001968467408,
                            semi_major_axis: 436281321.8,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 97.82030026,
                            longitude_of_ascending_node: 167.5991061,
                            argument_of_periapsis: 224.6685435,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 51.84219618,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(8.705869628)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.794390952e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 788900.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Oberon: fitted to 1501 JPL states over 9.2 yr; residual 820 km RMS (1.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Oberon".into()),
                        id: "Oberon".to_string(),
                        mass: 3.076576628e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00108178755,
                            semi_major_axis: 583448454.4,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 97.91911095,
                            longitude_of_ascending_node: 167.7481203,
                            argument_of_periapsis: 174.4269801,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 173.1064848,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(13.46324051)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.794827661e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 761400.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Miranda: fitted to 1502 JPL states over 1.0 yr; residual 2054 km RMS (1.6e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Miranda".into()),
                        id: "Miranda".to_string(),
                        mass: 6.442621749e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00135132496,
                            semi_major_axis: 129847392.9,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 98.51567346,
                            longitude_of_ascending_node: 170.9807066,
                            argument_of_periapsis: 265.4399451,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 57.46968915,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.413467665)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.795090181e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 235679.8973,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Puck: fitted to 1501 JPL states over 0.5 yr; residual 542 km RMS (6.3e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Puck".into()),
                        id: "Puck".to_string(),
                        mass: 2.868481438e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Uranus".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.007214551326,
                            semi_major_axis: 86006330.96,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 97.485135,
                            longitude_of_ascending_node: 166.5525499,
                            argument_of_periapsis: 306.2515716,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 318.6649387,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.7618334568)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(5.79700176e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 77000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Triton: fitted to 1501 JPL states over 4.0 yr; residual 2368 km RMS (6.7e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Triton".into()),
                        id: "Triton".to_string(),
                        mass: 2.140291385e+22,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Neptune".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0001265056032,
                            semi_major_axis: 354759067.3,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 130.4080249,
                            longitude_of_ascending_node: 216.5656565,
                            argument_of_periapsis: 93.59899278,
                            apsidal_precession_period: TimeDelta::from_days(218504.282),
                            nodal_precession_period: TimeDelta::from_days(613193.6206),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 341.4904289,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(5.876965087)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.836385487e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1352600.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Nereid: fitted to 1501 JPL states over 60.0 yr; residual 53804 km RMS (7.6e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Nereid".into()),
                        id: "Nereid".to_string(),
                        mass: 3.086928941e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Neptune".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.7493978513,
                            semi_major_axis: 5513808446.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 5.062948021,
                            longitude_of_ascending_node: 318.7294979,
                            argument_of_periapsis: 297.4915478,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 216.2365678,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(360.1152529)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.836034432e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 170000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Despina: fitted to 1491 JPL states over 0.2 yr; residual 3.35 km RMS (6.4e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Despina".into()),
                        id: "Despina".to_string(),
                        mass: 1.748947062e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Neptune".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0003340284734,
                            semi_major_axis: 52525944.83,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 28.52694702,
                            longitude_of_ascending_node: 48.91519572,
                            argument_of_periapsis: 315.7765611,
                            apsidal_precession_period: TimeDelta::from_days(281.8951209),
                            nodal_precession_period: TimeDelta::from_days(-216748.1258),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 142.4200104,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.3350528167)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.826959473e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 74000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Larissa: fitted to 1504 JPL states over 0.4 yr; residual 119 km RMS (1.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Larissa".into()),
                        id: "Larissa".to_string(),
                        mass: 3.818227271e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Neptune".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.00108816835,
                            semi_major_axis: 73548309.05,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.41969842,
                            longitude_of_ascending_node: 48.54963826,
                            argument_of_periapsis: 196.6655314,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 141.4454647,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(0.554653301)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.83922867e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 96000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Proteus: fitted to 1503 JPL states over 0.8 yr; residual 12.9 km RMS (1.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Proteus".into()),
                        id: "Proteus".to_string(),
                        mass: 3.865573049e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Neptune".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.000516646796,
                            semi_major_axis: 117646995.3,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.98936008,
                            longitude_of_ascending_node: 48.28029488,
                            argument_of_periapsis: 5.34759278,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 245.0793719,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1.122314751)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(6.836682777e+15),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 208000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 2-Pallas: fitted to 1501 JPL states over 60.0 yr; residual 3611206 km RMS (8.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("2-Pallas".into()),
                        id: "2-Pallas".to_string(),
                        mass: 2.042161266e+20,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2306760506,
                            semi_major_axis: 4.144812603e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 34.89544705,
                            longitude_of_ascending_node: 172.8983531,
                            argument_of_periapsis: 310.5066978,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 351.8837589,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1684.466994)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327155893e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 256500.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 3-Juno: fitted to 1501 JPL states over 60.0 yr; residual 2006646 km RMS (4.9e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("3-Juno".into()),
                        id: "3-Juno".to_string(),
                        mass: 1.570314717e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2572503564,
                            semi_major_axis: 3.993304333e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 12.97376988,
                            longitude_of_ascending_node: 169.6504436,
                            argument_of_periapsis: 248.5015961,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 240.5738016,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1593.201486)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326746878e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 123298.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 10-Hygiea: fitted to 1501 JPL states over 60.0 yr; residual 14410998 km RMS (3.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("10-Hygiea".into()),
                        id: "10-Hygiea".to_string(),
                        mass: 1.048798889e+20,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1086827434,
                            semi_major_axis: 4.703768321e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.839548276,
                            longitude_of_ascending_node: 282.6373393,
                            argument_of_periapsis: 317.2805511,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 340.8320583,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2039.001402)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.323838392e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 203560.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 15-Eunomia: fitted to 1501 JPL states over 60.0 yr; residual 1243161 km RMS (3.1e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("15-Eunomia".into()),
                        id: "15-Eunomia".to_string(),
                        mass: 1.302401427e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1866448127,
                            semi_major_axis: 3.955116663e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 11.74870164,
                            longitude_of_ascending_node: 292.8968566,
                            argument_of_periapsis: 98.26445076,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 106.244035,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1570.239313)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327023012e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 115844.5,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 16-Psyche: fitted to 1501 JPL states over 60.0 yr; residual 3154063 km RMS (7.1e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("16-Psyche".into()),
                        id: "16-Psyche".to_string(),
                        mass: 2.398752888e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1369001312,
                            semi_major_axis: 4.372059636e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.101629844,
                            longitude_of_ascending_node: 149.9897497,
                            argument_of_periapsis: 228.8297805,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 336.7416044,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1825.041378)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326920955e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 111000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 21-Lutetia: fitted to 1501 JPL states over 60.0 yr; residual 935376 km RMS (2.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("21-Lutetia".into()),
                        id: "21-Lutetia".to_string(),
                        mass: 1.699054201e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1634066758,
                            semi_major_axis: 3.642821112e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.064716001,
                            longitude_of_ascending_node: 80.78252327,
                            argument_of_periapsis: 250.0245823,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 314.3034706,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1387.89863)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327181417e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 49000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 52-Europa: fitted to 1501 JPL states over 60.0 yr; residual 6322467 km RMS (1.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("52-Europa".into()),
                        id: "52-Europa".to_string(),
                        mass: 2.939665298e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1078292586,
                            semi_major_axis: 4.632344567e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.477273681,
                            longitude_of_ascending_node: 128.5172868,
                            argument_of_periapsis: 341.7634001,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 42.64088921,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1989.051255)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328748756e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 151959.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 87-Sylvia: fitted to 1501 JPL states over 60.0 yr; residual 8258708 km RMS (1.6e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("87-Sylvia".into()),
                        id: "87-Sylvia".to_string(),
                        mass: 1.696886489e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.09010434611,
                            semi_major_axis: 5.213579312e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 10.84942437,
                            longitude_of_ascending_node: 73.61577002,
                            argument_of_periapsis: 261.6816981,
                            apsidal_precession_period: TimeDelta::from_days(456818.725),
                            nodal_precession_period: TimeDelta::from_days(-3331438.624),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 105.8006229,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2387.107215)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.315213793e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 126525.5,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 216-Kleopatra: fitted to 1501 JPL states over 60.0 yr; residual 3065496 km RMS (7.1e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("216-Kleopatra".into()),
                        id: "216-Kleopatra".to_string(),
                        mass: 1.901551579e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2497256065,
                            semi_major_axis: 4.182572091e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 13.1110963,
                            longitude_of_ascending_node: 215.2394139,
                            argument_of_periapsis: 180.4658423,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 22.9698842,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1707.524169)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327176974e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 61000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 243-Ida: fitted to 1501 JPL states over 60.0 yr; residual 1491981 km RMS (3.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("243-Ida".into()),
                        id: "243-Ida".to_string(),
                        mass: 4.120281351e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.04388613765,
                            semi_major_axis: 4.281195733e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.127182634,
                            longitude_of_ascending_node: 323.4465728,
                            argument_of_periapsis: 111.5867307,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 245.9842542,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1768.512753)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326817257e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 16000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 253-Mathilde: fitted to 1501 JPL states over 60.0 yr; residual 2193638 km RMS (5.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("253-Mathilde".into()),
                        id: "253-Mathilde".to_string(),
                        mass: 1.032317764e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2652704585,
                            semi_major_axis: 3.960288665e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 6.743518986,
                            longitude_of_ascending_node: 179.3683614,
                            argument_of_periapsis: 157.6863985,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 224.7623409,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1573.687699)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326403563e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 26400.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 433-Eros: fitted to 1501 JPL states over 60.0 yr; residual 575502 km RMS (2.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("433-Eros".into()),
                        id: "433-Eros".to_string(),
                        mass: 6.686842061e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2227826488,
                            semi_major_axis: 2.181358519e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 10.82800084,
                            longitude_of_ascending_node: 304.2547196,
                            argument_of_periapsis: 178.9331556,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 57.62384667,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(643.1464133)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327069594e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 8420.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 511-Davida: fitted to 1501 JPL states over 60.0 yr; residual 6532397 km RMS (1.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("511-Davida".into()),
                        id: "511-Davida".to_string(),
                        mass: 2.068697037e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1854960908,
                            semi_major_axis: 4.73755906e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 15.9388348,
                            longitude_of_ascending_node: 107.7785746,
                            argument_of_periapsis: 338.6307533,
                            apsidal_precession_period: TimeDelta::from_days(-1984531.441),
                            nodal_precession_period: TimeDelta::from_days(-14218875.1),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 177.9208708,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2054.183835)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.332654433e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 135163.5,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 704-Interamnia: fitted to 1501 JPL states over 60.0 yr; residual 6699899 km RMS (1.4e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("704-Interamnia".into()),
                        id: "704-Interamnia".to_string(),
                        mass: 7.491420638e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1506893496,
                            semi_major_axis: 4.579661772e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 17.30171935,
                            longitude_of_ascending_node: 280.0871594,
                            argument_of_periapsis: 94.5287867,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 242.8071647,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1955.641105)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328171484e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 153156.5,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 951-Gaspra: fitted to 1501 JPL states over 60.0 yr; residual 275772 km RMS (8.2e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("951-Gaspra".into()),
                        id: "951-Gaspra".to_string(),
                        mass: 1.901551579e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1735329605,
                            semi_major_axis: 3.305845162e+11,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 4.106283221,
                            longitude_of_ascending_node: 253.3114755,
                            argument_of_periapsis: 129.3972118,
                            apsidal_precession_period: TimeDelta::from_days(6453134.172),
                            nodal_precession_period: TimeDelta::from_days(-10377370.48),
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 96.38033364,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1199.986586)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326865782e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6100.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 1566-Icarus: fitted to 1501 JPL states over 60.0 yr; residual 779562 km RMS (3.6e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("1566-Icarus".into()),
                        id: "1566-Icarus".to_string(),
                        mass: 1.047197551e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.8270002664,
                            semi_major_axis: 1.612564777e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 22.79143688,
                            longitude_of_ascending_node: 87.92323117,
                            argument_of_periapsis: 31.46862467,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 106.6312294,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(408.8125097)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326891482e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 500.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 1620-Geographos: fitted to 1501 JPL states over 60.0 yr; residual 1712619 km RMS (8.7e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("1620-Geographos".into()),
                        id: "1620-Geographos".to_string(),
                        mass: 1.756905951e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3354212012,
                            semi_major_axis: 1.863015387e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 13.33790766,
                            longitude_of_ascending_node: 337.1165329,
                            argument_of_periapsis: 276.9804042,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 348.3452608,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(507.7044578)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326659966e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1280.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 2867-Steins: fitted to 1501 JPL states over 60.0 yr; residual 902203 km RMS (2.5e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("2867-Steins".into()),
                        id: "2867-Steins".to_string(),
                        mass: 1.438724777e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1461956228,
                            semi_major_axis: 3.535893687e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 9.932785384,
                            longitude_of_ascending_node: 55.29137965,
                            argument_of_periapsis: 250.9743753,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 177.3455811,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1327.352324)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326958738e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2580.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 3200-Phaethon: fitted to 1501 JPL states over 60.0 yr; residual 708620 km RMS (2.7e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("3200-Phaethon".into()),
                        id: "3200-Phaethon".to_string(),
                        mass: 2.556634646e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.8897513302,
                            semi_major_axis: 1.901923097e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 22.32883157,
                            longitude_of_ascending_node: 265.0533136,
                            argument_of_periapsis: 322.3452006,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 142.7851897,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(523.6158784)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.327044522e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3125.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 4179-Toutatis: fitted to 1501 JPL states over 60.0 yr; residual 28806280 km RMS (6.4e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("4179-Toutatis".into()),
                        id: "4179-Toutatis".to_string(),
                        mass: 1.648959152e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.624751596,
                            semi_major_axis: 3.794462589e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 0.4502017603,
                            longitude_of_ascending_node: 125.6873582,
                            argument_of_periapsis: 277.5472241,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 296.2722921,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1482.388932)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.314797139e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2700.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 25143-Itokawa: fitted to 1501 JPL states over 60.0 yr; residual 6648790 km RMS (3.2e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("25143-Itokawa".into()),
                        id: "25143-Itokawa".to_string(),
                        mass: 3.146396668e+10,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2802387096,
                            semi_major_axis: 1.978525346e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.626811082,
                            longitude_of_ascending_node: 69.11284686,
                            argument_of_periapsis: 162.7717828,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 41.59360994,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(556.0529229)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.32472231e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 165.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 99942-Apophis: fitted to 1501 JPL states over 60.0 yr; residual 135944315 km RMS (8.8e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("99942-Apophis".into()),
                        id: "99942-Apophis".to_string(),
                        mass: 4.115905255e+10,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3324235173,
                            semi_major_axis: 7.16771308e+10,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.090160925,
                            longitude_of_ascending_node: 203.5980063,
                            argument_of_periapsis: 97.16979087,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 260.1165575,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(323.5057588)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.860844387e+19),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 170.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 101955-Bennu: fitted to 1501 JPL states over 60.0 yr; residual 4909888 km RMS (2.9e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("101955-Bennu".into()),
                        id: "101955-Bennu".to_string(),
                        mass: 1.172653577e+11,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2040249902,
                            semi_major_axis: 1.684938668e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 6.034396621,
                            longitude_of_ascending_node: 1.87716262,
                            argument_of_periapsis: 66.42771229,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 31.44258462,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(436.6662342)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326738468e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 241.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 162173-Ryugu: fitted to 1501 JPL states over 60.0 yr; residual 14230447 km RMS (7.8e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("162173-Ryugu".into()),
                        id: "162173-Ryugu".to_string(),
                        mass: 4.494852383e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1920841641,
                            semi_major_axis: 1.776448232e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 5.862381336,
                            longitude_of_ascending_node: 251.231363,
                            argument_of_periapsis: 211.7684165,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 298.1032833,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(474.9170948)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.314481007e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 448.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Makemake: fitted to 1501 JPL states over 60.0 yr; residual 831049 km RMS (1.1e-04 of orbit radius).
                // radius: ESTIMATE from H=-.250 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Makemake".into()),
                        id: "Makemake".to_string(),
                        mass: 1.28599653e+23,
                        major: false,
                        designation: None,
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1604614159,
                            semi_major_axis: 6.805949619e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 29.00245758,
                            longitude_of_ascending_node: 79.44203576,
                            argument_of_periapsis: 296.0567166,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 140.1153403,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(112005.8345)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328976034e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2485270.876,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Haumea: fitted to 1501 JPL states over 60.0 yr; residual 831330 km RMS (1.1e-04 of orbit radius).
                // radius: ESTIMATE from H=.140 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Haumea".into()),
                        id: "Haumea".to_string(),
                        mass: 7.503083794e+22,
                        major: false,
                        designation: None,
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1950765465,
                            semi_major_axis: 6.447607564e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.2052803,
                            longitude_of_ascending_node: 121.9472362,
                            argument_of_periapsis: 239.9560792,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 190.4143822,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(103277.4694)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328975162e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2076699.845,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Quaoar: fitted to 1501 JPL states over 60.0 yr; residual 830156 km RMS (1.3e-04 of orbit radius).
                // radius: ESTIMATE from H=2.41 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Quaoar".into()),
                        id: "Quaoar".to_string(),
                        mass: 3.260166621e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.03692034142,
                            semi_major_axis: 6.482879038e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.990779926,
                            longitude_of_ascending_node: 188.9123095,
                            argument_of_periapsis: 157.5310551,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 265.1674673,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(104128.0412)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328925421e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 730085.5125,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Orcus: fitted to 1501 JPL states over 60.0 yr; residual 830569 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=2.13 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Orcus".into()),
                        id: "Orcus".to_string(),
                        mass: 4.799984076e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2239060829,
                            semi_major_axis: 5.875804173e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 20.56797231,
                            longitude_of_ascending_node: 268.5837489,
                            argument_of_periapsis: 73.02056636,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 151.061093,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(89849.3148)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328937794e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 830565.2,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Gonggong: fitted to 1501 JPL states over 60.0 yr; residual 831281 km RMS (6.2e-05 of orbit radius).
                // radius: ESTIMATE from H=1.82 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gonggong".into()),
                        id: "Gonggong".to_string(),
                        mass: 7.366137081e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5033914541,
                            semi_major_axis: 1.00333646e+13,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 30.80170146,
                            longitude_of_ascending_node: 336.8399536,
                            argument_of_periapsis: 206.9936437,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 93.64887728,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(200488.611)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328900976e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 958018.1357,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Ixion: fitted to 1501 JPL states over 60.0 yr; residual 829598 km RMS (1.5e-04 of orbit radius).
                // radius: ESTIMATE from H=3.47 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ixion".into()),
                        id: "Ixion".to_string(),
                        mass: 7.537716455e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2440389091,
                            semi_major_axis: 5.908795292e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 19.62966732,
                            longitude_of_ascending_node: 71.03141787,
                            argument_of_periapsis: 299.8551857,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 257.4147294,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(90609.03297)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328881005e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 448098.7481,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Varuna: fitted to 1501 JPL states over 60.0 yr; residual 830429 km RMS (1.3e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Varuna".into()),
                        id: "Varuna".to_string(),
                        mass: 7.634070148e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05390949767,
                            semi_major_axis: 6.424679769e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 17.17497486,
                            longitude_of_ascending_node: 97.29723684,
                            argument_of_periapsis: 267.2627543,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 87.9715287,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(102730.1831)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328894746e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 450000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Arrokoth: fitted to 1501 JPL states over 60.0 yr; residual 829975 km RMS (1.3e-04 of orbit radius).
                // radius: ESTIMATE from H=11.06 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Arrokoth".into()),
                        id: "Arrokoth".to_string(),
                        mass: 2.104940366e+16,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.03778864744,
                            semi_major_axis: 6.6170905e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.45000337,
                            longitude_of_ascending_node: 159.0454654,
                            argument_of_periapsis: 183.7919631,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 283.6065911,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(107378.5828)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328917864e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 13594.82841,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Huya: fitted to 1501 JPL states over 60.0 yr; residual 832287 km RMS (1.8e-04 of orbit radius).
                // radius: ESTIMATE from H=4.79 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Huya".into()),
                        id: "Huya".to_string(),
                        mass: 1.216857706e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.276044519,
                            semi_major_axis: 5.895715808e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 15.4736229,
                            longitude_of_ascending_node: 169.3229004,
                            argument_of_periapsis: 67.8728357,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 337.9946579,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(90306.66281)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328930557e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 243990.9571,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Varda: fitted to 1501 JPL states over 60.0 yr; residual 830882 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=3.46 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Varda".into()),
                        id: "Varda".to_string(),
                        mass: 7.642576536e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1425966317,
                            semi_major_axis: 6.843448887e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 21.51097756,
                            longitude_of_ascending_node: 184.0965058,
                            argument_of_periapsis: 183.210816,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 248.1791355,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(112935.3806)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328915283e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 450167.0779,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // Salacia: fitted to 1501 JPL states over 60.0 yr; residual 830969 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=4.12 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Salacia".into()),
                        id: "Salacia".to_string(),
                        mass: 3.070717023e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.106719153,
                            semi_major_axis: 6.286968802e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 23.92965979,
                            longitude_of_ascending_node: 280.0856138,
                            argument_of_periapsis: 309.702094,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 98.87882141,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(99440.53265)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.329013167e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 332180.1911,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 2060-Chiron: fitted to 1501 JPL states over 60.0 yr; residual 998151 km RMS (4.6e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("2060-Chiron".into()),
                        id: "2060-Chiron".to_string(),
                        mass: 4.79019157e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.3805667688,
                            semi_major_axis: 2.04403264e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 6.933909372,
                            longitude_of_ascending_node: 209.2796789,
                            argument_of_periapsis: 339.5543315,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 27.72272624,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(18435.40296)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328894371e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 83000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 5145-Pholus: fitted to 1501 JPL states over 60.0 yr; residual 826311 km RMS (2.0e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("5145-Pholus".into()),
                        id: "5145-Pholus".to_string(),
                        mass: 7.182728004e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5726344847,
                            semi_major_axis: 3.039873109e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 24.68986666,
                            longitude_of_ascending_node: 119.3411227,
                            argument_of_periapsis: 354.7977767,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 32.52287076,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(33434.08707)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328983779e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 95000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 10199-Chariklo: fitted to 1501 JPL states over 60.0 yr; residual 946664 km RMS (3.9e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("10199-Chariklo".into()),
                        id: "10199-Chariklo".to_string(),
                        mass: 2.884359885e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.1705355777,
                            semi_major_axis: 2.356825339e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 23.38990132,
                            longitude_of_ascending_node: 300.419395,
                            argument_of_periapsis: 241.7286579,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 337.1964628,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(22822.29636)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.329216632e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 151000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 1P-Halley: fitted to 1501 JPL states over 60.0 yr; residual 832298 km RMS (1.9e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("1P-Halley".into()),
                        id: "1P-Halley".to_string(),
                        mass: 1.393819941e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.9672962482,
                            semi_major_axis: 2.672542154e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 162.2444994,
                            longitude_of_ascending_node: 58.84453875,
                            argument_of_periapsis: 111.7449351,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 66.28500065,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(27562.20257)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328856556e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5500.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 2P-Encke: fitted to 1501 JPL states over 60.0 yr; residual 8201130 km RMS (1.8e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("2P-Encke".into()),
                        id: "2P-Encke".to_string(),
                        mass: 1.158116716e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.8478330361,
                            semi_major_axis: 3.314757685e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 11.47386634,
                            longitude_of_ascending_node: 334.1665877,
                            argument_of_periapsis: 187.1230117,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 284.7413154,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(1206.057692)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.324193478e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2400.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 9P-Tempel-1: fitted to 1501 JPL states over 60.0 yr; residual 246893753 km RMS (4.5e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("9P-Tempel-1".into()),
                        id: "9P-Tempel-1".to_string(),
                        mass: 2.261946711e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5056082335,
                            semi_major_axis: 4.316029981e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 10.384707,
                            longitude_of_ascending_node: 67.11366058,
                            argument_of_periapsis: 183.7680851,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 67.44211686,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2177.953218)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(8.963724836e+19),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 19P-Borrelly: fitted to 1501 JPL states over 60.0 yr; residual 32021464 km RMS (5.0e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("19P-Borrelly".into()),
                        id: "19P-Borrelly".to_string(),
                        mass: 1.158116716e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.6313270554,
                            semi_major_axis: 5.395893523e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 29.63315523,
                            longitude_of_ascending_node: 74.54890882,
                            argument_of_periapsis: 352.5597344,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 274.1320075,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2502.789849)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.326397197e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2400.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 67P-Churyumov-Gerasimenko: fitted to 1501 JPL states over 60.0 yr; residual 74557893 km RMS (1.2e-01 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("67P-Churyumov-Gerasimenko".into()),
                        id: "67P-Churyumov-Gerasimenko".to_string(),
                        mass: 9.921637493e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.6422292652,
                            semi_major_axis: 5.194277469e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.760656459,
                            longitude_of_ascending_node: 42.99181293,
                            argument_of_periapsis: 17.03949149,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 222.5269179,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2376.816385)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.311947396e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1700.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 81P-Wild-2: fitted to 1501 JPL states over 60.0 yr; residual 32952857 km RMS (5.5e-02 of orbit radius).
                // A fixed ellipse fits this poorly; treat the position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("81P-Wild-2".into()),
                        id: "81P-Wild-2".to_string(),
                        mass: 6.702064328e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.5403884129,
                            semi_major_axis: 5.125088479e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.990538808,
                            longitude_of_ascending_node: 134.3682448,
                            argument_of_periapsis: 43.73487826,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 146.0009203,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2330.854076)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.310406937e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 103P-Hartley-2: fitted to 1501 JPL states over 60.0 yr; residual 22428135 km RMS (3.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("103P-Hartley-2".into()),
                        id: "103P-Hartley-2".to_string(),
                        mass: 4.28932117e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.6991811772,
                            semi_major_axis: 5.169173028e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 13.56348756,
                            longitude_of_ascending_node: 219.6273355,
                            argument_of_periapsis: 181.603749,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 114.9890215,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(2359.335715)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.312248172e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 800.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // C-1995-O1-Hale-Bopp: fitted to 1501 JPL states over 60.0 yr; residual 829524 km RMS (1.1e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("C-1995-O1-Hale-Bopp".into()),
                        id: "C-1995-O1-Hale-Bopp".to_string(),
                        mass: 2.261946711e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.9948758209,
                            semi_major_axis: 2.682328738e+13,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 89.36714034,
                            longitude_of_ascending_node: 282.5451694,
                            argument_of_periapsis: 130.6612443,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 0.4129556577,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(876377.1647)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(1.328881189e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 30000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 1I-Oumuamua: fitted to 1501 JPL states over 10.0 yr; residual 604215 km RMS (2.4e-04 of orbit radius).
                // radius: ESTIMATE from H=22.08 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("1I-Oumuamua".into()),
                        id: "1I-Oumuamua".to_string(),
                        mass: 5143275603.0,
                        major: false,
                        designation: None,
                        tags: vec!["Interstellar".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.201497225,
                            semi_major_axis: -1.902632137e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 122.7479684,
                            longitude_of_ascending_node: 24.61388286,
                            argument_of_periapsis: 241.834986,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 241.6135848,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(524.0334581)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(-1.326412797e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 84.99115488,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
                // 2I-Borisov: fitted to 1501 JPL states over 10.0 yr; residual 509480 km RMS (1.8e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("2I-Borisov".into()),
                        id: "2I-Borisov".to_string(),
                        mass: 8.37758041e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Interstellar".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.358215293,
                            semi_major_axis: -1.272576978e+11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 44.03858084,
                            longitude_of_ascending_node: 308.1631974,
                            argument_of_periapsis: 209.1134777,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 216.9368322,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(286.6915309)),
                        // mu implied by the fitted period and semi-major axis.
                        gravitational_parameter: Some(-1.326034157e+20),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1000.0,
                        color: AppearanceColor { r: 170, g: 165, b: 160 },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }),
            ],
    };
    solar_system
}

/// Canonical on-disk location for this preset, used by the app when writing it out.
pub const EARTH_MOON_PATH: &str = "data/templates/earth_moon.toml";

pub fn earth_moon() -> UniverseFileContents {
    let earth_moon = UniverseFileContents {
            version: "0.0".into(),
            time: UniverseFileTime {
                time_julian_days: 2451544.500000, // Midnight 2000 January 1 00:00
                step: 0.1,
                gui_speed: 1.0,
                max_frame_time: 0.016,
            },
            physics: UniversePhysics::default(),
            view: ViewSettings::default(),
            bodies: vec![
                /*SomeBody::FixedEntry(FixedEntry {
                    info: BodyInfo {
                        name: Some("Sol".into()),
                        id: "sol".to_string(),
                        mass: 1.988416e30,
                        major: true,
                        designation: None,
                        tags: vec!["Star".into()],
                        ..Default::default()
                    },
                    position: DVec3::new(0.0, 1.49598023e8 * 1000.0, 0.0),
                    appearance: Appearance::Star(StarBall {
                        radius: 6.957e8,
                        color: AppearanceColor {
                            r: 219,
                            g: 222,
                            b: 35,
                        },
                        light: AppearanceColor {
                            r: 255 * 14,
                            g: 255 * 14,
                            b: 255 * 14,
                        },
                        intensity: 10000.0,
                    }),
                }), // Sun*/
                SomeBody::FixedEntry(FixedEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "earth".to_string(),
                        mass: 5.972168e24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    position: DVec3::ZERO,
                    appearance: Appearance::DebugBall(DebugBall{
                        radius: 6371.0 * 1000.0,
                        color: AppearanceColor {
                            r: 59,
                            g: 179,
                            b: 75
                        },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: Some(iau_rotation(0.0, 90.0, 190.147, 23.9344696)),
                }), // Earth
                /*SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "luna".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("Earth I".into()),
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "earth".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05490,
                            semi_major_axis: 384400.0 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles { // TODO: Precession https://en.wikipedia.org/wiki/Orbit_of_the_Moon#Precession
                            inclination: 5.240010829674768e0,
                            longitude_of_ascending_node: 1.239837028145578e2,
                            argument_of_periapsis: 3.081359034620368e2,
                            apsidal_precession_period: 3231.50,
                            nodal_precession_period: 6798.38,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.407402571142365e02,
                        }),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737.4 * 1000.0,
                        color: AppearanceColor {
                            r: 87,
                            g: 87,
                            b: 87,
                        },
                        highlight_latitudes: Vec::new(),
                    }),
                }),*/ // Luna
                SomeBody::NewtonEntry(NewtonEntry {
                    info: BodyInfo {
                        name: Some("Newtonian Test Body A".into()),
                        id: "NTB-A".to_string(),
                        mass: 1000.0,
                        major: false,
                        designation: Some("TB-A".into()),
                        tags: vec!["Test Body".into()],
                    },
                    position: DVec3::new(384400.0 * 1000.0, 0.0, 0.0),
                    velocity: DVec3::new(1.5e3, 0.0, 0.0),
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 100.0,
                        color: AppearanceColor {
                            r: 255,
                            g: 0,
                            b: 0,
                        },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }), // Test Newtonian Body A
                SomeBody::NewtonEntry(NewtonEntry {
                    info: BodyInfo {
                        name: Some("Newtonian Test Body B".into()),
                        id: "NTB-B".to_string(),
                        mass: 1000.0,
                        major: false,
                        designation: Some("TB-B".into()),
                        tags: vec!["Test Body".into()],
                    },
                    position: DVec3::new(384400.0 * 1000.0, 0.0, 0.0),
                    velocity: DVec3::new(0.0, 0.0, 0.0),
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 100.0,
                        color: AppearanceColor {
                            r: 255,
                            g: 0,
                            b: 0,
                        },
                        highlight_latitudes: Vec::new(),
                    }),
                    rotation: None,
                }), // Test Newtonian Body B
            ]
    };
    earth_moon
}
