use std::default::Default;
use std::f64::consts::PI;
use std::path::PathBuf;
use bevy::math::{DVec3, DQuat};
use crate::body::appearance::{Appearance, AppearanceColor, DebugBall, StarBall};
use crate::body::motive::info::{BodyInfo, BodyRotation, RotationEpoch};
use crate::body::motive::kepler_motive::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerPrecessingEulerAngles, KeplerRotation, KeplerShape, MeanAnomalyAtEpoch, MeanAnomalyAtJ2000};
use crate::body::universe::save::{FixedEntry, KeplerEntry, NewtonEntry, SomeBody, UniverseFile, UniverseFileContents, UniverseFileTime, UniversePhysics, ViewSettings};
use crate::foundations::time::{Instant, TimeLength};
use crate::gui::util::ensure_folders;

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

pub fn solar_system() -> UniverseFile {
    let solar_system = UniverseFile {
        file: Some(PathBuf::from("data/templates/solar_system.toml")),
        contents: UniverseFileContents {
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
                        mass: 1.988416e30,
                        major: true,
                        designation: None,
                        tags: vec!["Star".into()],
                        ..Default::default()
                    },
                    position: DVec3::ZERO,
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
                        absolute_magnitude: 4.83,
                    }),
                    rotation: Some(iau_rotation(286.13, 63.87, 84.176, 609.12)),
                }), // Sun
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mercury".into()),
                        id: "Mercury".to_string(),
                        mass: 3.3011e23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.205630,
                            semi_major_axis: 5.791e7 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.005,
                            longitude_of_ascending_node: 48.331,
                            argument_of_periapsis: 29.124,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 174.796,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2439.7 * 1000.0, // meters
                        color: AppearanceColor {
                            r: 145,
                            g: 145,
                            b: 145,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(281.0103, 61.4155, 329.5988, 1407.6)),
                }), // Mercury
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "Venus".to_string(),
                        mass: 4.8675e24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006772,
                            semi_major_axis: 1.0821e8 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.39458,
                            longitude_of_ascending_node: 76.680,
                            argument_of_periapsis: 54.884,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 50.115,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6051.8 * 1000.0,
                        color: AppearanceColor {
                            r: 224,
                            g: 224,
                            b: 224,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(272.76, 67.16, 160.20, -5832.6)), // retrograde
                }), // Venus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "Earth".to_string(),
                        mass: 5.972168e24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0167086,
                            semi_major_axis: 1.49598023e8 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 0.00005, // haha, the J2000 ecliptic is nonzero
                            longitude_of_ascending_node: -11.26064,
                            argument_of_periapsis: 114.20783,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 358.617,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall{
                        radius: 6371.0 * 1000.0,
                        color: AppearanceColor {
                            r: 59,
                            g: 179,
                            b: 75
                        },
                        highlight_latitudes: vec![23.44, 66.56],
                    }),
                    rotation: Some(iau_rotation(0.0, 90.0, 190.147, 23.9344696)),
                }), // Earth
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mars".into()),
                        id: "Mars".to_string(),
                        mass: 6.4171e23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0934,
                            semi_major_axis: 2.27939366e8 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.850,
                            longitude_of_ascending_node: 49.57854,
                            argument_of_periapsis: 286.5,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 19.412,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3389.5 * 1000.0,
                        color: AppearanceColor {
                            r: 242,
                            g: 66,
                            b: 17,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(317.269, 54.432, 176.049, 24.6229)),
                }), // Mars
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phobos".into()),
                        id: "Phobos".to_string(),
                        mass: 1.08e16,
                        major: true,
                        designation: Some("Mars I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Mars".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.469841851969655e-02,
                            semi_major_axis: 9378.7 * 1000.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.605670195883539e01,
                            longitude_of_ascending_node: 8.481514244056581e01,
                            argument_of_periapsis: 3.427848642391356e02,
                            apsidal_precession_period: TimeLength::period_from_julian_day(413.33), // prograde, ~1.13 yr
                            nodal_precession_period: TimeLength::period_from_julian_day(-826.11), // retrograde, ~2.26 yr
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.898224342278868e02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 11.1 * 1000.0,
                        color: AppearanceColor {
                            r: 120,
                            g: 105,
                            b: 90,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Mars", 317.67, 52.89)),
                }), // Phobos
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Deimos".into()),
                        id: "Deimos".to_string(),
                        mass: 1.80e15,
                        major: true,
                        designation: Some("Mars II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Mars".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.299624878237082e-04,
                            semi_major_axis: 23458.2 * 1000.0,
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 2.756936980386812e01,
                            longitude_of_ascending_node: 8.366926662588133e01,
                            argument_of_periapsis: 2.118947925261645e02,
                            apsidal_precession_period: TimeLength::period_from_julian_day(9996.05), // prograde, ~27.4 yr
                            nodal_precession_period: TimeLength::period_from_julian_day(-19920.78), // retrograde, ~54.5 yr
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 5.093950813525924e00,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6.2 * 1000.0,
                        color: AppearanceColor {
                            r: 120,
                            g: 105,
                            b: 90,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Mars", 316.65, 53.53)),
                }), // Deimos
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ceres".into()),
                        id: "1-Ceres".to_string(),
                        mass: 9.3839e20,
                        major: true,
                        designation: Some("1 Ceres".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0785,
                            semi_major_axis: 4.14e8 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 10.6,
                            longitude_of_ascending_node: 80.3,
                            argument_of_periapsis: 73.6,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 291.4,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 966.2 * 1000.0,
                        color: AppearanceColor {
                            r: 145,
                            g: 107,
                            b: 54,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(291.418, 66.764, 170.65, 9.074170)),
                }), // Ceres
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Vesta".into()),
                        id: "4-Vesta".to_string(),
                        mass: 2.59076e20,
                        major: true,
                        designation: Some("4 Vesta".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0894,
                            semi_major_axis: 3.84e8 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.1422,
                            longitude_of_ascending_node: 103.71,
                            argument_of_periapsis: 151.66,
                        }),
                        epoch: KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
                            epoch: Instant::from_julian_day(2453300.5),
                            mean_anomaly: 169.4,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 262.7 * 1000.0,
                        color: AppearanceColor {
                            r: 145,
                            g: 107,
                            b: 54,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(305.8, 41.4, 292.0, 5.342128)),
                }), // Vesta
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "Luna".to_string(),
                        mass: 7.346e22,
                        major: true,
                        designation: Some("Earth I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Earth".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05490,
                            semi_major_axis: 384400.0 * 1000.0, // Convert km to m
                        }),
                        rotation: KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                            inclination: 5.240010829674768e0,
                            longitude_of_ascending_node: 1.239837028145578e2,
                            argument_of_periapsis: 3.081359034620368e2,
                            apsidal_precession_period: TimeLength::period_from_julian_day(3231.50), // prograde, ~8.85 yr
                            nodal_precession_period: TimeLength::period_from_julian_day(-6798.38), // retrograde, ~18.61 yr
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.407402571142365e02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737.4 * 1000.0,
                        color: AppearanceColor {
                            r: 87,
                            g: 87,
                            b: 87,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Earth", 269.9949, 66.5392)), // tidally locked to Earth
                }), // Luna
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jupiter".into()),
                        id: "Jupiter".to_string(),
                        mass: 1.8982e27,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0489,
                            semi_major_axis: 7.78479e11,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.303,
                            longitude_of_ascending_node:100.464,
                            argument_of_periapsis: 273.867,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 20.020,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 69911.1 * 1000.0,
                        color: AppearanceColor {
                            r: 0xb0,
                            g: 0x7f,
                            b: 0x35,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(268.057, 64.495, 284.95, 9.9250)),
                }), // Jupiter
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Metis".into()),
                        id: "Metis".to_string(),
                        mass: 3.75e16,
                        major: true,
                        designation: Some("Jupiter XVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.499e-03,
                            semi_major_axis: 128743.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.217,
                            longitude_of_ascending_node: 337.655,
                            argument_of_periapsis: 185.876,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 0.685,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 21.5 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.05, 64.50)),
                }), // Metis
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Adrastea".into()),
                        id: "Adrastea".to_string(),
                        mass: 1.50e15,
                        major: true,
                        designation: Some("Jupiter XV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 7.208e-03,
                            semi_major_axis: 129831.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.210,
                            longitude_of_ascending_node: 337.742,
                            argument_of_periapsis: 229.896,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 5.010,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 8.2 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.05, 64.50)),
                }), // Adrastea
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Amalthea".into()),
                        id: "Amalthea".to_string(),
                        mass: 2.47e18,
                        major: true,
                        designation: Some("Jupiter V".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.036e-03,
                            semi_major_axis: 181879.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.442,
                            longitude_of_ascending_node: 330.411,
                            argument_of_periapsis: 109.139,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 332.264,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 83.5 * 1000.0,
                        color: AppearanceColor {
                            r: 170,
                            g: 100,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.05, 64.50)),
                }), // Amalthea
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thebe".into()),
                        id: "Thebe".to_string(),
                        mass: 4.51e17,
                        major: true,
                        designation: Some("Jupiter XIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.560e-02,
                            semi_major_axis: 222234.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.284,
                            longitude_of_ascending_node: 338.097,
                            argument_of_periapsis: 27.653,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 181.553,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 49.3 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.05, 64.50)),
                }), // Thebe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Io".into()),
                        id: "Io".to_string(),
                        mass: 8.932e22,
                        major: true,
                        designation: Some("Jupiter I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.820e-03,
                            semi_major_axis: 422029.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.213,
                            longitude_of_ascending_node: 336.852,
                            argument_of_periapsis: 67.427,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 333.899,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1821.49 * 1000.0,
                        color: AppearanceColor {
                            r: 220,
                            g: 200,
                            b: 60,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.05, 64.50)),
                }), // Io
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Europa".into()),
                        id: "Europa".to_string(),
                        mass: 4.800e22,
                        major: true,
                        designation: Some("Jupiter II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 9.662e-03,
                            semi_major_axis: 671224.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.791,
                            longitude_of_ascending_node: 332.628,
                            argument_of_periapsis: 254.315,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 345.739,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1560.8 * 1000.0,
                        color: AppearanceColor {
                            r: 220,
                            g: 220,
                            b: 230,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.08, 64.51)),
                }), // Europa
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ganymede".into()),
                        id: "Ganymede".to_string(),
                        mass: 1.482e23,
                        major: true,
                        designation: Some("Jupiter III".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.416e-03,
                            semi_major_axis: 1070587.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.214,
                            longitude_of_ascending_node: 343.173,
                            argument_of_periapsis: 315.906,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 280.947,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2631.2 * 1000.0,
                        color: AppearanceColor {
                            r: 180,
                            g: 170,
                            b: 160,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.20, 64.57)),
                }), // Ganymede
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Callisto".into()),
                        id: "Callisto".to_string(),
                        mass: 1.076e23,
                        major: true,
                        designation: Some("Jupiter IV".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 7.424e-03,
                            semi_major_axis: 1882709.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.017,
                            longitude_of_ascending_node: 337.943,
                            argument_of_periapsis: 16.191,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 85.055,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2410.3 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 90,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Jupiter", 268.72, 64.83)),
                }), // Callisto
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Themisto".into()),
                        id: "Themisto".to_string(),
                        mass: 1.0e15,
                        major: false,
                        designation: Some("Jupiter XVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.967e-01,
                            semi_major_axis: 7396000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 45.983,
                            longitude_of_ascending_node: 202.668,
                            argument_of_periapsis: 236.334,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 289.109,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 4.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Themisto
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Leda".into()),
                        id: "Leda".to_string(),
                        mass: 1.4e15,
                        major: false,
                        designation: Some("Jupiter XIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.727e-01,
                            semi_major_axis: 11139000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 27.578,
                            longitude_of_ascending_node: 216.877,
                            argument_of_periapsis: 271.310,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 232.578,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Leda
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Himalia".into()),
                        id: "Himalia".to_string(),
                        mass: 2.27e18,
                        major: false,
                        designation: Some("Jupiter VI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.662e-01,
                            semi_major_axis: 11370000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 30.247,
                            longitude_of_ascending_node: 64.191,
                            argument_of_periapsis: 321.063,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 78.318,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 85.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Himalia
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Lysithea".into()),
                        id: "Lysithea".to_string(),
                        mass: 1.9e16,
                        major: false,
                        designation: Some("Jupiter X".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.366e-01,
                            semi_major_axis: 11737000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 27.861,
                            longitude_of_ascending_node: 9.239,
                            argument_of_periapsis: 48.292,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 328.423,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 12.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Lysithea
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Elara".into()),
                        id: "Elara".to_string(),
                        mass: 7.0e17,
                        major: false,
                        designation: Some("Jupiter VII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.216e-01,
                            semi_major_axis: 11737000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 28.912,
                            longitude_of_ascending_node: 112.798,
                            argument_of_periapsis: 129.938,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 346.883,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 40.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Elara
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ananke".into()),
                        id: "Ananke".to_string(),
                        mass: 1.1e16,
                        major: false,
                        designation: Some("Jupiter XII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.804e-01,
                            semi_major_axis: 21681000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 151.654,
                            longitude_of_ascending_node: 13.695,
                            argument_of_periapsis: 78.753,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 271.783,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 10.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Ananke
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Taygete".into()),
                        id: "Taygete".to_string(),
                        mass: 2.0e14,
                        major: false,
                        designation: Some("Jupiter XX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.923e-01,
                            semi_major_axis: 22214000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 162.032,
                            longitude_of_ascending_node: 300.783,
                            argument_of_periapsis: 207.689,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 116.510,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.5 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Taygete
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sinope".into()),
                        id: "Sinope".to_string(),
                        mass: 3.0e16,
                        major: false,
                        designation: Some("Jupiter IX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.165e-01,
                            semi_major_axis: 22969000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 152.138,
                            longitude_of_ascending_node: 308.021,
                            argument_of_periapsis: 354.277,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 157.449,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Sinope
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Callirrhoe".into()),
                        id: "Callirrhoe".to_string(),
                        mass: 1.0e15,
                        major: false,
                        designation: Some("Jupiter XVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.978e-01,
                            semi_major_axis: 23032000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 139.163,
                            longitude_of_ascending_node: 278.186,
                            argument_of_periapsis: 13.486,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 117.929,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 4.3 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Callirrhoe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pasiphae".into()),
                        id: "Pasiphae".to_string(),
                        mass: 6.4e16,
                        major: false,
                        designation: Some("Jupiter VIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.796e-01,
                            semi_major_axis: 23425000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 140.087,
                            longitude_of_ascending_node: 315.748,
                            argument_of_periapsis: 172.848,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 279.206,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Pasiphae
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Carme".into()),
                        id: "Carme".to_string(),
                        mass: 3.7e16,
                        major: false,
                        designation: Some("Jupiter XI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.425e-01,
                            semi_major_axis: 24203000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 164.723,
                            longitude_of_ascending_node: 115.499,
                            argument_of_periapsis: 6.507,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 259.453,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Carme
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Megaclite".into()),
                        id: "Megaclite".to_string(),
                        mass: 2.0e14,
                        major: false,
                        designation: Some("Jupiter XIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.466e-01,
                            semi_major_axis: 24609000.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 151.963,
                            longitude_of_ascending_node: 292.160,
                            argument_of_periapsis: 309.940,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 116.095,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.7 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Megaclite
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Chaldene".into()),
                        id: "Chaldene".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.024e-01,
                            semi_major_axis: 23420513.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 165.726,
                            longitude_of_ascending_node: 134.455,
                            argument_of_periapsis: 231.554,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 275.428,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Chaldene
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Harpalyke".into()),
                        id: "Harpalyke".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.546e-01,
                            semi_major_axis: 21567318.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 149.961,
                            longitude_of_ascending_node: 35.557,
                            argument_of_periapsis: 104.316,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 255.427,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Harpalyke
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kalyke".into()),
                        id: "Kalyke".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.040e-01,
                            semi_major_axis: 22247635.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 166.444,
                            longitude_of_ascending_node: 37.705,
                            argument_of_periapsis: 240.462,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 224.973,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kalyke
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Iocaste".into()),
                        id: "Iocaste".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.426e-01,
                            semi_major_axis: 21114303.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 148.986,
                            longitude_of_ascending_node: 268.896,
                            argument_of_periapsis: 91.387,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 182.027,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Iocaste
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Erinome".into()),
                        id: "Erinome".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.237e-01,
                            semi_major_axis: 23503332.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 161.228,
                            longitude_of_ascending_node: 311.179,
                            argument_of_periapsis: 359.870,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 273.154,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Erinome
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Isonoe".into()),
                        id: "Isonoe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.520e-01,
                            semi_major_axis: 23869526.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 166.040,
                            longitude_of_ascending_node: 134.070,
                            argument_of_periapsis: 130.965,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 116.401,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Isonoe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Praxidike".into()),
                        id: "Praxidike".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.816e-01,
                            semi_major_axis: 20445002.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 143.141,
                            longitude_of_ascending_node: 281.305,
                            argument_of_periapsis: 182.497,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 127.998,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Praxidike
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Autonoe".into()),
                        id: "Autonoe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.157e-01,
                            semi_major_axis: 24859992.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 150.268,
                            longitude_of_ascending_node: 264.776,
                            argument_of_periapsis: 61.198,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 134.071,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Autonoe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thyone".into()),
                        id: "Thyone".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.303e-01,
                            semi_major_axis: 20599613.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 149.900,
                            longitude_of_ascending_node: 240.773,
                            argument_of_periapsis: 105.078,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 242.571,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Thyone
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hermippe".into()),
                        id: "Hermippe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.832e-01,
                            semi_major_axis: 21294255.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 149.920,
                            longitude_of_ascending_node: 330.326,
                            argument_of_periapsis: 277.243,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 158.492,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hermippe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aitne".into()),
                        id: "Aitne".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.780e-01,
                            semi_major_axis: 22196073.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 165.808,
                            longitude_of_ascending_node: 358.413,
                            argument_of_periapsis: 72.172,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 120.567,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Aitne
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eurydome".into()),
                        id: "Eurydome".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.205e-01,
                            semi_major_axis: 22873898.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 146.077,
                            longitude_of_ascending_node: 297.691,
                            argument_of_periapsis: 223.414,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 281.769,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Eurydome
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Euanthe".into()),
                        id: "Euanthe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.337e-01,
                            semi_major_axis: 20527290.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 145.972,
                            longitude_of_ascending_node: 265.166,
                            argument_of_periapsis: 312.367,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 353.573,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Euanthe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Euporie".into()),
                        id: "Euporie".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.207e-01,
                            semi_major_axis: 19541281.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 147.083,
                            longitude_of_ascending_node: 63.398,
                            argument_of_periapsis: 109.549,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 56.332,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Euporie
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Orthosie".into()),
                        id: "Orthosie".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.006e-01,
                            semi_major_axis: 20687579.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 144.294,
                            longitude_of_ascending_node: 212.355,
                            argument_of_periapsis: 244.730,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 161.144,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Orthosie
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sponde".into()),
                        id: "Sponde".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.487e-01,
                            semi_major_axis: 24527011.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 151.979,
                            longitude_of_ascending_node: 109.278,
                            argument_of_periapsis: 56.619,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 177.420,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Sponde
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kale".into()),
                        id: "Kale".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.312e-01,
                            semi_major_axis: 23059839.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 166.719,
                            longitude_of_ascending_node: 56.682,
                            argument_of_periapsis: 69.163,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 175.965,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kale
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pasithee".into()),
                        id: "Pasithee".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.184e-01,
                            semi_major_axis: 23235300.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 163.165,
                            longitude_of_ascending_node: 317.998,
                            argument_of_periapsis: 207.947,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 237.712,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Pasithee
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hegemone".into()),
                        id: "Hegemone".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XXXIX".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.783e-01,
                            semi_major_axis: 23243391.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 146.420,
                            longitude_of_ascending_node: 311.461,
                            argument_of_periapsis: 192.193,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 241.140,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hegemone
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mneme".into()),
                        id: "Mneme".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: Some("Jupiter XL".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.092e-01,
                            semi_major_axis: 20473452.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 147.917,
                            longitude_of_ascending_node: 0.933,
                            argument_of_periapsis: 50.147,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 236.047,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Mneme
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aoede".into()),
                        id: "Aoede".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.974612777083392e-1,
                            semi_major_axis: 2.299130466596223e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.564749089403921e2,
                            longitude_of_ascending_node: 1.564757607761237e2,
                            argument_of_periapsis: 4.305616297289463e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.076504120453782e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Aoede
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thelxinoe".into()),
                        id: "Thelxinoe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.705357869190795e-1,
                            semi_major_axis: 2.067028428645477e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.509878246938660e2,
                            longitude_of_ascending_node: 1.749205809301939e2,
                            argument_of_periapsis: 3.069998250908127e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.750748509441603e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Thelxinoe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Arche".into()),
                        id: "Arche".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.054706497998864e-1,
                            semi_major_axis: 2.246169303562129e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.626573715832007e2,
                            longitude_of_ascending_node: 3.333458942493500e2,
                            argument_of_periapsis: 1.725343251787509e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.395811585203073e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Arche
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kallichore".into()),
                        id: "Kallichore".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.620862484654328e-1,
                            semi_major_axis: 2.212895467094195e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.657356406241262e2,
                            longitude_of_ascending_node: 2.389883113943349e1,
                            argument_of_periapsis: 3.526303403370928e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 6.679184674250222e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kallichore
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Helike".into()),
                        id: "Helike".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.525457145739460e-1,
                            semi_major_axis: 2.081525660606640e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.552715391761386e2,
                            longitude_of_ascending_node: 9.456556747573211e1,
                            argument_of_periapsis: 3.051344742462124e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 4.577536652765550e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Helike
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Carpo".into()),
                        id: "Carpo".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.029594179759565e-1,
                            semi_major_axis: 1.691992234237657e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 5.870137390763132e1,
                            longitude_of_ascending_node: 5.211173548505003e1,
                            argument_of_periapsis: 9.051088341198802e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.366058638220729e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Carpo
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eukelade".into()),
                        id: "Eukelade".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.216466782120759e-1,
                            semi_major_axis: 2.422204724121446e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.657835917342832e2,
                            longitude_of_ascending_node: 2.004367024726523e2,
                            argument_of_periapsis: 2.900951872871678e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.341676415908599e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Eukelade
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Cyllene".into()),
                        id: "Cyllene".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.803963043009347e-1,
                            semi_major_axis: 2.334592269157281e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.414367024159966e2,
                            longitude_of_ascending_node: 2.536366520237451e2,
                            argument_of_periapsis: 1.745710735675481e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.532580480489584e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Cyllene
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kore".into()),
                        id: "Kore".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.476988517396408e-1,
                            semi_major_axis: 2.365257752858388e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.372745367557715e2,
                            longitude_of_ascending_node: 3.241690015646634e2,
                            argument_of_periapsis: 1.352825179066613e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 4.064754734087054e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kore
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Herse".into()),
                        id: "Herse".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.149467966476225e-1,
                            semi_major_axis: 2.324240889970542e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.615792885470867e2,
                            longitude_of_ascending_node: 2.994613691441868e2,
                            argument_of_periapsis: 3.440635315074221e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.231312948193408e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Herse
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2010 J1".into()),
                        id: "S2010 J1".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.212028992581479e-1,
                            semi_major_axis: 2.243089708104447e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.608347004099496e2,
                            longitude_of_ascending_node: 2.847281703224716e2,
                            argument_of_periapsis: 1.755929107491854e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.834113317928559e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2010 J1
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2010 J2".into()),
                        id: "S2010 J2".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.056878477645451e-1,
                            semi_major_axis: 2.112378199280442e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.468412536986783e2,
                            longitude_of_ascending_node: 3.572786898204026e2,
                            argument_of_periapsis: 2.883546356768227e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.924675021096460e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2010 J2
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dia".into()),
                        id: "Dia".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.827965905366508e-1,
                            semi_major_axis: 1.231316953601086e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.756252210438170e1,
                            longitude_of_ascending_node: 2.939159992887429e2,
                            argument_of_periapsis: 1.641360401227196e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.208848358859603e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Dia
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2016 J1".into()),
                        id: "S2016 J1".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.876314373184279e-1,
                            semi_major_axis: 2.034428463360986e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.450196896612210e2,
                            longitude_of_ascending_node: 2.477413899225406e2,
                            argument_of_periapsis: 2.984684741184129e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.153203057148663e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2016 J1
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2003 J18".into()),
                        id: "S2003 J18".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.394034837471894e-2,
                            semi_major_axis: 2.078565474718022e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.458386601816063e2,
                            longitude_of_ascending_node: 1.667516004851825e2,
                            argument_of_periapsis: 8.616532009787061e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.525603705261962e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2003 J18
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2011 J2".into()),
                        id: "S2011 J2".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.104080420617612e-1,
                            semi_major_axis: 2.391254846867616e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.571655043499222e2,
                            longitude_of_ascending_node: 2.485633524621820e1,
                            argument_of_periapsis: 2.629457027117417e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.975578087251839e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2011 J2
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eirene".into()),
                        id: "Eirene".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.387145948797907e-1,
                            semi_major_axis: 2.238869129277225e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.660119155931481e2,
                            longitude_of_ascending_node: 1.800322490519131e2,
                            argument_of_periapsis: 8.638957561357289e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.367113829959906e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Eirene
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Philophrosyne".into()),
                        id: "Philophrosyne".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 9.150055221306930e-2,
                            semi_major_axis: 2.180447309070500e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.455065812442692e2,
                            longitude_of_ascending_node: 2.343300589597866e2,
                            argument_of_periapsis: 9.135227463166030e-1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 8.889854492921442e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Philophrosyne
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J1".into()),
                        id: "S2017 J1".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.935391914309157e-1,
                            semi_major_axis: 2.264220504654459e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.453933953873527e2,
                            longitude_of_ascending_node: 2.518376127196585e2,
                            argument_of_periapsis: 7.468605176734287e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.500458014620400e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J1
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eupheme".into()),
                        id: "Eupheme".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.900756396147893e-1,
                            semi_major_axis: 2.093799971048320e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.496084394261087e2,
                            longitude_of_ascending_node: 2.274789759918696e2,
                            argument_of_periapsis: 7.357088091872168e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.398274046763659e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Eupheme
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2003 J19".into()),
                        id: "S2003 J19".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.591918826300066e-1,
                            semi_major_axis: 2.368218119921291e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.651147475074845e2,
                            longitude_of_ascending_node: 2.188897894151245e1,
                            argument_of_periapsis: 1.868140602289981e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.968826602639435e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2003 J19
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Valetudo".into()),
                        id: "Valetudo".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.481821546913007e-1,
                            semi_major_axis: 1.873548537243888e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.206614296543535e1,
                            longitude_of_ascending_node: 2.912151313490808e2,
                            argument_of_periapsis: 3.163188893737358e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.049723581196345e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Valetudo
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J2".into()),
                        id: "S2017 J2".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.960402869204020e-1,
                            semi_major_axis: 2.205726241862479e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.644410317239353e2,
                            longitude_of_ascending_node: 3.594426795967146e2,
                            argument_of_periapsis: 1.530441087670198e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.780998870167303e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J2
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J3".into()),
                        id: "S2017 J3".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.127536433558462e-1,
                            semi_major_axis: 2.152652421098936e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.495741447826146e2,
                            longitude_of_ascending_node: 2.792474068454250e1,
                            argument_of_periapsis: 1.057233316657347e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.353542414136148e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pandia".into()),
                        id: "Pandia".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.457525427667159e-1,
                            semi_major_axis: 1.154587064926278e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.819666686532248e1,
                            longitude_of_ascending_node: 2.497849878215551e2,
                            argument_of_periapsis: 1.886508067415504e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.585209653706456e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Pandia
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J5".into()),
                        id: "S2017 J5".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.390546419324528e-1,
                            semi_major_axis: 2.371772383227587e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.669585496835971e2,
                            longitude_of_ascending_node: 4.234182927753569e1,
                            argument_of_periapsis: 2.924843144199102e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 6.926007543983482e1,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J5
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J6".into()),
                        id: "S2017 J6".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.470230450084309e-1,
                            semi_major_axis: 2.252199959491500e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.424457415877656e2,
                            longitude_of_ascending_node: 3.127518813634208e2,
                            argument_of_periapsis: 1.549794863574072e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.460414185997246e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J6
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J7".into()),
                        id: "S2017 J7".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.362973209887887e-1,
                            semi_major_axis: 2.090519093320552e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.482631541499851e2,
                            longitude_of_ascending_node: 2.649210419236576e2,
                            argument_of_periapsis: 2.887132794684595e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.529308825221443e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J7
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J8".into()),
                        id: "S2017 J8".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.602936167397107e-1,
                            semi_major_axis: 2.237498979052974e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.651375023317549e2,
                            longitude_of_ascending_node: 9.728611772422347e1,
                            argument_of_periapsis: 3.305256738014888e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.493890856789200e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J8
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2017 J9".into()),
                        id: "S2017 J9".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.942443286114848e-1,
                            semi_major_axis: 2.119492460767397e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.561109507713700e2,
                            longitude_of_ascending_node: 2.453169068385442e2,
                            argument_of_periapsis: 2.764180127848383e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.432255547669899e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2017 J9
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ersa".into()),
                        id: "Ersa".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.373348195763628e-1,
                            semi_major_axis: 1.134045193748272e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.996613049337234e1,
                            longitude_of_ascending_node: 1.137317072135144e2,
                            argument_of_periapsis: 3.007077373309253e2,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.707714962453674e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Ersa
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2011 J1".into()),
                        id: "S2011 J1".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Jupiter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.788216128845463e-1,
                            semi_major_axis: 2.302369510822003e7 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.641666963662268e2,
                            longitude_of_ascending_node: 2.535365371593302e2,
                            argument_of_periapsis: 5.038252723968456e1,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.253996329861696e2,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2011 J1
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Saturn".into()),
                        id: "Saturn".to_string(),
                        mass: 5.6834e26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.05256,
                            semi_major_axis: 1.4261e12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.485,
                            longitude_of_ascending_node: 113.709,
                            argument_of_periapsis: 338.917,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 317.378,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58232.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 171,
                            b: 90,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(40.589, 83.537, 38.90, 10.656)),
                }), // Saturn
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mimas".into()),
                        id: "Mimas".to_string(),
                        mass: 3.75e19,
                        major: true,
                        designation: Some("Saturn I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.304944827227298e-02,
                            semi_major_axis: 1.866450888491645e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.700219363533107e+01,
                            longitude_of_ascending_node: 1.720549588581378e+02,
                            argument_of_periapsis: 1.111209071229047e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.500202849913553e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 198.8 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 210,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Mimas
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Enceladus".into()),
                        id: "Enceladus".to_string(),
                        mass: 1.0805e20,
                        major: true,
                        designation: Some("Saturn II".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 7.576815376182076e-03,
                            semi_major_axis: 2.390102858882088e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805190204772423e+01,
                            longitude_of_ascending_node: 1.695063751089069e+02,
                            argument_of_periapsis: 1.352937811928203e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 7.133519212791637e+00,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 252.3 * 1000.0,
                        color: AppearanceColor {
                            r: 230,
                            g: 230,
                            b: 240,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Enceladus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tethys".into()),
                        id: "Tethys".to_string(),
                        mass: 6.176e20,
                        major: true,
                        designation: Some("Saturn III".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.956778090016517e-03,
                            semi_major_axis: 2.955751683695809e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.722120628492100e+01,
                            longitude_of_ascending_node: 1.679993998500507e+02,
                            argument_of_periapsis: 1.512850829467467e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.571499148028073e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 536.3 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Tethys
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dione".into()),
                        id: "Dione".to_string(),
                        mass: 1.09572e21,
                        major: true,
                        designation: Some("Saturn IV".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.577444049819936e-03,
                            semi_major_axis: 3.782403974715018e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.804130878272679e+01,
                            longitude_of_ascending_node: 1.694701294979720e+02,
                            argument_of_periapsis: 1.566582029881260e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.403258673354471e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 562.5 * 1000.0,
                        color: AppearanceColor {
                            r: 190,
                            g: 190,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Dione
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Rhea".into()),
                        id: "Rhea".to_string(),
                        mass: 2.309e21,
                        major: true,
                        designation: Some("Saturn V".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.596376049598053e-03,
                            semi_major_axis: 5.265364266051896e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.824150776588579e+01,
                            longitude_of_ascending_node: 1.689842430594633e+02,
                            argument_of_periapsis: 1.894780939498878e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.831959834599272e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 764.5 * 1000.0,
                        color: AppearanceColor {
                            r: 195,
                            g: 195,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Rhea
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Titan".into()),
                        id: "Titan".to_string(),
                        mass: 1.34553e23,
                        major: true,
                        designation: Some("Saturn VI".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.860859137154326e-02,
                            semi_major_axis: 1.221632020109312e+06 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.771834075085665e+01,
                            longitude_of_ascending_node: 1.692390692704889e+02,
                            argument_of_periapsis: 1.643969037907502e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.634483043682111e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2575.5 * 1000.0,
                        color: AppearanceColor {
                            r: 210,
                            g: 170,
                            b: 60,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Titan
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hyperion".into()),
                        id: "Hyperion".to_string(),
                        mass: 1.08e19,
                        major: true,
                        designation: Some("Saturn VII".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.266526540710251e-01,
                            semi_major_axis: 1.484009475306915e+06 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.720886656218138e+01,
                            longitude_of_ascending_node: 1.683051051282069e+02,
                            argument_of_periapsis: 1.884202025989788e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 7.086358801576610e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 133.0 * 1000.0,
                        color: AppearanceColor {
                            r: 160,
                            g: 140,
                            b: 120,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hyperion
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Iapetus".into()),
                        id: "Iapetus".to_string(),
                        mass: 1.8059e21,
                        major: true,
                        designation: Some("Saturn VIII".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.834298742907040e-02,
                            semi_major_axis: 3.561185856982774e+06 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.723866718729459e+01,
                            longitude_of_ascending_node: 1.396824722733234e+02,
                            argument_of_periapsis: 2.294754597306644e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.082495044906248e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 734.5 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 130,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Iapetus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phoebe".into()),
                        id: "Phoebe".to_string(),
                        mass: 8.289e18,
                        major: false,
                        designation: Some("Saturn IX".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.649291579206485e-01,
                            semi_major_axis: 1.294708153104546e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.732628668423100e+02,
                            longitude_of_ascending_node: 2.633092786889129e+02,
                            argument_of_periapsis: 3.540384192327454e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 5.852072786786734e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 106.6 * 1000.0,
                        color: AppearanceColor {
                            r: 90,
                            g: 85,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Phoebe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Janus".into()),
                        id: "Janus".to_string(),
                        mass: 1.98e18,
                        major: false,
                        designation: Some("Saturn X".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.237520193890089e-03,
                            semi_major_axis: 1.519570125717878e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.798454463513117e+01,
                            longitude_of_ascending_node: 1.698490401494235e+02,
                            argument_of_periapsis: 1.631999319676445e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 8.055355280195826e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 90.0 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Janus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Epimetheus".into()),
                        id: "Epimetheus".to_string(),
                        mass: 5.5e17,
                        major: false,
                        designation: Some("Saturn XI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.687307769245854e-03,
                            semi_major_axis: 1.521211011017337e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.773793441284173e+01,
                            longitude_of_ascending_node: 1.698739498970633e+02,
                            argument_of_periapsis: 2.485786533489944e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.800707535775454e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58.0 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Epimetheus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Helene".into()),
                        id: "Helene".to_string(),
                        mass: 2.55e16,
                        major: false,
                        designation: Some("Saturn XII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 8.455701784718297e-03,
                            semi_major_axis: 3.783099732498895e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.793439140809558e+01,
                            longitude_of_ascending_node: 1.699070373021618e+02,
                            argument_of_periapsis: 1.734012565347206e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.580560782449584e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 16.0 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Helene
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Telesto".into()),
                        id: "Telesto".to_string(),
                        mass: 7.2e15,
                        major: false,
                        designation: Some("Saturn XIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.784641368824397e-03,
                            semi_major_axis: 2.952901546018719e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.688453165049023e+01,
                            longitude_of_ascending_node: 1.691071504731081e+02,
                            argument_of_periapsis: 2.376445965657280e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.289103603550566e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 12.6 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Telesto
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Calypso".into()),
                        id: "Calypso".to_string(),
                        mass: 3.6e15,
                        major: false,
                        designation: Some("Saturn XIV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.338343060619068e-03,
                            semi_major_axis: 2.951756006843169e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.819819887710563e+01,
                            longitude_of_ascending_node: 1.663573778676561e+02,
                            argument_of_periapsis: 4.416716097122119e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 4.689963542077857e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 10.3 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Calypso
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Atlas".into()),
                        id: "Atlas".to_string(),
                        mass: 8.4e15,
                        major: false,
                        designation: Some("Saturn XV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.696320265526116e-03,
                            semi_major_axis: 1.376644150693280e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.804826531240664e+01,
                            longitude_of_ascending_node: 1.695258338157791e+02,
                            argument_of_periapsis: 3.559178534166915e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.371219592679082e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15.9 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Atlas
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Prometheus".into()),
                        id: "Prometheus".to_string(),
                        mass: 1.4e17,
                        major: false,
                        designation: Some("Saturn XVI".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.723761922800126e-03,
                            semi_major_axis: 1.399378202024290e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805162961284825e+01,
                            longitude_of_ascending_node: 1.695106684012418e+02,
                            argument_of_periapsis: 7.961055486826459e+00,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 4.764167006559136e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 46.0 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Prometheus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pandora".into()),
                        id: "Pandora".to_string(),
                        mass: 1.3e17,
                        major: false,
                        designation: Some("Saturn XVII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.360501351849018e-03,
                            semi_major_axis: 1.417911768734643e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.803633642040110e+01,
                            longitude_of_ascending_node: 1.696294332376278e+02,
                            argument_of_periapsis: 2.060165873196423e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.137558088878005e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 41.5 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Pandora
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pan".into()),
                        id: "Pan".to_string(),
                        mass: 4.95e15,
                        major: false,
                        designation: Some("Saturn XVIII".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.712724292525571e-03,
                            semi_major_axis: 1.346792216515826e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805112331826122e+01,
                            longitude_of_ascending_node: 1.695265420397804e+02,
                            argument_of_periapsis: 9.361188792235703e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.292407197256752e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14.3 * 1000.0,
                        color: AppearanceColor {
                            r: 150,
                            g: 150,
                            b: 150,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Saturn", 40.589, 83.537)),
                }), // Pan
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ymir".into()),
                        id: "Ymir".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.960056369886100e-01,
                            semi_major_axis: 2.236878301368270e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.729334687211404e+02,
                            longitude_of_ascending_node: 2.044714545080754e+02,
                            argument_of_periapsis: 4.527062173744442e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.164781787946409e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Ymir
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Paaliaq".into()),
                        id: "Paaliaq".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.650046456194178e-01,
                            semi_major_axis: 1.490457483663394e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.734192176153435e+01,
                            longitude_of_ascending_node: 3.531565045747327e+02,
                            argument_of_periapsis: 2.375756041834131e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.045535513482095e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Paaliaq
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tarvos".into()),
                        id: "Tarvos".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.452693385138289e-01,
                            semi_major_axis: 1.858413333861460e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.389309757052952e+01,
                            longitude_of_ascending_node: 9.569293161414333e+01,
                            argument_of_periapsis: 2.829113167308168e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.559238013836795e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Tarvos
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ijiraq".into()),
                        id: "Ijiraq".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.684021793686749e-01,
                            semi_major_axis: 1.133532094400400e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.960704774551659e+01,
                            longitude_of_ascending_node: 1.525137699056548e+02,
                            argument_of_periapsis: 6.864434169981251e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.834038611686825e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Ijiraq
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Suttungr".into()),
                        id: "Suttungr".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.207722173433759e-01,
                            semi_major_axis: 1.958327452133809e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.747018683614053e+02,
                            longitude_of_ascending_node: 2.532235779182518e+02,
                            argument_of_periapsis: 6.502560621106278e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.173493776956957e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Suttungr
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kiviuq".into()),
                        id: "Kiviuq".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.576488028945827e-01,
                            semi_major_axis: 1.130871695723827e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.876769918576248e+01,
                            longitude_of_ascending_node: 3.526794273669869e+02,
                            argument_of_periapsis: 9.175668298413184e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.685755849251828e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kiviuq
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mundilfari".into()),
                        id: "Mundilfari".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.479760723303488e-01,
                            semi_major_axis: 1.870815280625378e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.695003627799693e+02,
                            longitude_of_ascending_node: 7.917173634326272e+01,
                            argument_of_periapsis: 3.054172278862669e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.734316645718071e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Mundilfari
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Albiorix".into()),
                        id: "Albiorix".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.909488158719077e-01,
                            semi_major_axis: 1.637819998715169e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.709143159316906e+01,
                            longitude_of_ascending_node: 1.111182776427578e+02,
                            argument_of_periapsis: 6.279530951091620e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.724527420781896e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Albiorix
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skathi".into()),
                        id: "Skathi".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.250452372328385e-01,
                            semi_major_axis: 1.547574752328257e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.478596884694831e+02,
                            longitude_of_ascending_node: 2.840683415452783e+02,
                            argument_of_periapsis: 2.022512528484603e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.175547479950973e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Skathi
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Erriapus".into()),
                        id: "Erriapus".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.174416717413577e-01,
                            semi_major_axis: 1.723459054175537e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.431594941352273e+01,
                            longitude_of_ascending_node: 1.462152288238680e+02,
                            argument_of_periapsis: 2.797977655485130e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.067473470678961e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Erriapus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Siarnaq".into()),
                        id: "Siarnaq".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.894406645825536e-01,
                            semi_major_axis: 1.757777961659166e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.850118539723859e+01,
                            longitude_of_ascending_node: 6.396910738562925e+01,
                            argument_of_periapsis: 6.837948629546355e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.906795228181239e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Siarnaq
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thrymr".into()),
                        id: "Thrymr".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.062102641801430e-01,
                            semi_major_axis: 2.034930710181572e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.749728330653180e+02,
                            longitude_of_ascending_node: 2.473344410807781e+02,
                            argument_of_periapsis: 9.218490489916995e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.508955523664343e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Thrymr
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Narvi".into()),
                        id: "Narvi".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.691547366470835e-01,
                            semi_major_axis: 1.949407052838147e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.388115411777406e+02,
                            longitude_of_ascending_node: 1.789876063189706e+02,
                            argument_of_periapsis: 1.779388529915659e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.094185480150650e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Narvi
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Methone".into()),
                        id: "Methone".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.619639177898456e-03,
                            semi_major_axis: 1.952877244120293e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805424293953650e+01,
                            longitude_of_ascending_node: 1.694894193565344e+02,
                            argument_of_periapsis: 1.394306266102731e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.100550067689297e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Methone
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pallene".into()),
                        id: "Pallene".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.869940289430949e-03,
                            semi_major_axis: 2.130739337831560e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.803387560637843e+01,
                            longitude_of_ascending_node: 1.691396541813085e+02,
                            argument_of_periapsis: 9.206245651408196e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.376807344471779e+00,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Pallene
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Polydeuces".into()),
                        id: "Polydeuces".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.961379818013259e-02,
                            semi_major_axis: 3.772366102680395e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.820512415271671e+01,
                            longitude_of_ascending_node: 1.697006403965680e+02,
                            argument_of_periapsis: 3.396232910904550e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 7.300453932963467e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Polydeuces
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Daphnis".into()),
                        id: "Daphnis".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.603634653453164e-03,
                            semi_major_axis: 1.376346710474970e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805215459279434e+01,
                            longitude_of_ascending_node: 1.695309003389035e+02,
                            argument_of_periapsis: 1.022786159753504e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.125753580017765e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Daphnis
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aegir".into()),
                        id: "Aegir".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.855750503321143e-01,
                            semi_major_axis: 2.079207487654852e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.678697406359073e+02,
                            longitude_of_ascending_node: 1.819398153563598e+02,
                            argument_of_periapsis: 2.447436087061326e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.728523902848012e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Aegir
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bebhionn".into()),
                        id: "Bebhionn".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.769481560548620e-01,
                            semi_major_axis: 1.720635683402431e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 4.336243226073397e+01,
                            longitude_of_ascending_node: 1.976930182608390e+02,
                            argument_of_periapsis: 5.062294973550801e+00,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.638127420866718e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Bebhionn
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bergelmir".into()),
                        id: "Bergelmir".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.202979685545001e-01,
                            semi_major_axis: 1.903786198734940e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.592499162324661e+02,
                            longitude_of_ascending_node: 2.078688530048516e+02,
                            argument_of_periapsis: 1.338841258652128e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.114961436542187e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Bergelmir
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bestla".into()),
                        id: "Bestla".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.187775261150686e-01,
                            semi_major_axis: 2.014584883687460e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.462678895073352e+02,
                            longitude_of_ascending_node: 2.829588525185609e+02,
                            argument_of_periapsis: 8.561736566184713e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.390332372295691e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Bestla
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Farbauti".into()),
                        id: "Farbauti".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.450238023820605e-01,
                            semi_major_axis: 2.043604125862465e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.564492233358501e+02,
                            longitude_of_ascending_node: 1.380345444507934e+02,
                            argument_of_periapsis: 3.435705943037993e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.850406896578938e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Farbauti
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Fenrir".into()),
                        id: "Fenrir".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.930314472492596e-01,
                            semi_major_axis: 2.185807732845963e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.645997284401325e+02,
                            longitude_of_ascending_node: 2.346334958535516e+02,
                            argument_of_periapsis: 1.145486134691146e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.477720526644105e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Fenrir
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Fornjot".into()),
                        id: "Fornjot".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.658840305836739e-01,
                            semi_major_axis: 2.502207545823203e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.679663460974808e+02,
                            longitude_of_ascending_node: 2.692887738377069e+02,
                            argument_of_periapsis: 3.219284816604039e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.320581470250386e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Fornjot
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hati".into()),
                        id: "Hati".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.278105470934305e-01,
                            semi_major_axis: 1.941430327394820e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.604962970721458e+02,
                            longitude_of_ascending_node: 3.163372666673570e+02,
                            argument_of_periapsis: 1.035808125439364e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.832660374982928e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hati
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hyrrokkin".into()),
                        id: "Hyrrokkin".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.627113815794469e-01,
                            semi_major_axis: 1.855881010857667e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.547790771483627e+02,
                            longitude_of_ascending_node: 4.017643450907252e+01,
                            argument_of_periapsis: 2.648326166956842e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.951611062515516e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hyrrokkin
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kari".into()),
                        id: "Kari".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.701617398721125e-01,
                            semi_major_axis: 2.243413591921663e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.464927663593965e+02,
                            longitude_of_ascending_node: 2.882769501419679e+02,
                            argument_of_periapsis: 1.602480020077187e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.995891984388363e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kari
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Loge".into()),
                        id: "Loge".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.923551269378318e-01,
                            semi_major_axis: 2.271938578902677e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.652599698746866e+02,
                            longitude_of_ascending_node: 3.329803814569782e+02,
                            argument_of_periapsis: 2.054940710818259e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.380010402469146e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Loge
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skoll".into()),
                        id: "Skoll".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.569495937976359e-01,
                            semi_major_axis: 1.766158939726701e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.532427254755488e+02,
                            longitude_of_ascending_node: 2.940304716753566e+02,
                            argument_of_periapsis: 1.860990725041137e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 4.918852973515387e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Skoll
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Surtur".into()),
                        id: "Surtur".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.444934116594777e-01,
                            semi_major_axis: 2.219186642335005e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.678885481560803e+02,
                            longitude_of_ascending_node: 2.553300385641240e+02,
                            argument_of_periapsis: 3.143770161549464e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.605918445925140e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Surtur
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Anthe".into()),
                        id: "Anthe".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.809171334437095e-03,
                            semi_major_axis: 1.981106091256190e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.804622779208723e+01,
                            longitude_of_ascending_node: 1.694889550383712e+02,
                            argument_of_periapsis: 2.926855522547690e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.035441097580642e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Anthe
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jarnsaxa".into()),
                        id: "Jarnsaxa".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.842889272714874e-01,
                            semi_major_axis: 1.898256008412305e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.632830652044825e+02,
                            longitude_of_ascending_node: 9.249390866892076e+00,
                            argument_of_periapsis: 2.291522649877330e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.940199040783659e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Jarnsaxa
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Greip".into()),
                        id: "Greip".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.022963448299479e-01,
                            semi_major_axis: 1.832492510239414e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.724878065145815e+02,
                            longitude_of_ascending_node: 3.358056394181943e+02,
                            argument_of_periapsis: 1.462400997364562e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.074233694952010e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Greip
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tarqeq".into()),
                        id: "Tarqeq".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.028912900165764e-01,
                            semi_major_axis: 1.766646408837352e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 5.037060383581524e+01,
                            longitude_of_ascending_node: 9.334830869115289e+01,
                            argument_of_periapsis: 7.215618962195391e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.194997882554026e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Tarqeq
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Aegaeon".into()),
                        id: "Aegaeon".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.642402369613430e-03,
                            semi_major_axis: 1.680301899946263e+05 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 2.805003572224044e+01,
                            longitude_of_ascending_node: 1.695280950830151e+02,
                            argument_of_periapsis: 3.501443775903467e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.974206388855881e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Aegaeon
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gridr".into()),
                        id: "Gridr".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.606466089705330e-01,
                            semi_major_axis: 1.925023159823715e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.622209136985274e+02,
                            longitude_of_ascending_node: 3.308085745867855e+02,
                            argument_of_periapsis: 2.714072834143512e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.385900765542180e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Gridr
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Angrboda".into()),
                        id: "Angrboda".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.907229262937485e-01,
                            semi_major_axis: 2.097297459128225e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.775348450309556e+02,
                            longitude_of_ascending_node: 2.791439496024868e+02,
                            argument_of_periapsis: 1.042352057898789e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.011812800261595e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Angrboda
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Skrymir".into()),
                        id: "Skrymir".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.783043257513537e-01,
                            semi_major_axis: 2.139626889078068e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.780012012109848e+02,
                            longitude_of_ascending_node: 1.777103213351242e+02,
                            argument_of_periapsis: 6.813748531602066e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 6.426546372679269e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Skrymir
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gerd".into()),
                        id: "Gerd".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.185995929278439e-01,
                            semi_major_axis: 2.061700229318310e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.742221137590790e+02,
                            longitude_of_ascending_node: 2.604176119276092e+02,
                            argument_of_periapsis: 3.072198813221850e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.102902903878485e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Gerd
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S26".into()),
                        id: "S2004 S26".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 8.893405433835576e-02,
                            semi_major_axis: 2.710737685109238e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.707149964485510e+02,
                            longitude_of_ascending_node: 3.288113586182426e+02,
                            argument_of_periapsis: 1.483657214524654e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.312398057189274e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2004 S26
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eggther".into()),
                        id: "Eggther".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.718731632108119e-01,
                            semi_major_axis: 2.013623578027407e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.672829754402396e+02,
                            longitude_of_ascending_node: 8.446191466828638e+01,
                            argument_of_periapsis: 1.301088594622210e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.806232489567824e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Eggther
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S29".into()),
                        id: "S2004 S29".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 4.778123509329507e-01,
                            semi_major_axis: 1.685697436427454e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 3.870210507518409e+01,
                            longitude_of_ascending_node: 1.579106166398866e+02,
                            argument_of_periapsis: 3.094641331867484e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.999696746900916e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2004 S29
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Beli".into()),
                        id: "Beli".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 9.422454922878501e-02,
                            semi_major_axis: 2.074483362093299e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.577112457016805e+02,
                            longitude_of_ascending_node: 2.587254270455072e+02,
                            argument_of_periapsis: 2.635412276010011e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.491274544738752e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Beli
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gunnlod".into()),
                        id: "Gunnlod".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.249584855490627e-01,
                            semi_major_axis: 2.121693477859071e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.567424155346715e+02,
                            longitude_of_ascending_node: 2.933396693830800e+02,
                            argument_of_periapsis: 2.472030565503373e+01,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.780578809608396e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Gunnlod
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thiazzi".into()),
                        id: "Thiazzi".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.463174749479023e-01,
                            semi_major_axis: 2.310636686984910e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.630106846658764e+02,
                            longitude_of_ascending_node: 7.306983489954744e+01,
                            argument_of_periapsis: 3.032334416849299e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.208235615864407e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Thiazzi
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("S2004 S34".into()),
                        id: "S2004 S34".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 3.001902861770800e-01,
                            semi_major_axis: 2.434729890702568e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.646782006373132e+02,
                            longitude_of_ascending_node: 2.925798180988236e+02,
                            argument_of_periapsis: 3.544909489579410e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 2.514445374823418e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // S2004 S34
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Alvaldi".into()),
                        id: "Alvaldi".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.592189348233380e-01,
                            semi_major_axis: 2.212645708786864e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.760401749379577e+02,
                            longitude_of_ascending_node: 3.217711189743628e+02,
                            argument_of_periapsis: 1.880997255114425e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.218983717284696e+02,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Alvaldi
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Geirrod".into()),
                        id: "Geirrod".to_string(),
                        mass: 8.7e13,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Saturn".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 5.744602795087238e-01,
                            semi_major_axis: 2.224904182751363e+07 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.504798898183301e+02,
                            longitude_of_ascending_node: 1.279240678270667e+02,
                            argument_of_periapsis: 3.495929144831567e+02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 9.802479199008438e+01,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2.0 * 1000.0,
                        color: AppearanceColor {
                            r: 80,
                            g: 80,
                            b: 80,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Geirrod
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Uranus".into()),
                        id: "Uranus".to_string(),
                        mass: 8.681e25,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.04717,
                            semi_major_axis: 2.870972e12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 0.773,
                            longitude_of_ascending_node: 74.006,
                            argument_of_periapsis: 96.998857,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 142.2386,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 25362.0 * 1000.0,
                        color: AppearanceColor {
                            r: 60,
                            g: 186,
                            b: 180,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(257.311, -15.175, 203.81, -17.24)), // retrograde, tilted ~98°
                }), // Uranus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Neptune".into()),
                        id: "Neptune".to_string(),
                        mass: 1.02409e26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.008678,
                            semi_major_axis: 4.5e12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.77,
                            longitude_of_ascending_node: 131.783,
                            argument_of_periapsis: 273.187,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 259.883,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 24622.0 * 1000.0,
                        color: AppearanceColor {
                            r: 60,
                            g: 186,
                            b: 180,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(299.36, 43.46, 253.18, 16.11)),
                }), // Neptune
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pluto Barycenter".into()),
                        id: "Pluto Barycenter".to_string(),
                        mass: 1.4656e22, // total Pluto system mass
                        major: false,
                        designation: None,
                        tags: vec!["Barycenter".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.489763560923634e-01,
                            semi_major_axis: 5.907233278822115e12, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.714055930776762e01,
                            longitude_of_ascending_node: 1.103012538561415e02,
                            argument_of_periapsis: 1.137774660018993e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.484874908032896e01,
                        }),
                        gravitational_parameter: None, // uses G*M_sun
                    },
                    appearance: Appearance::Empty,
                    rotation: None,
                }), // Pluto Barycenter
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pluto".into()),
                        id: "Pluto".to_string(),
                        mass: 1.307e22,
                        major: true,
                        designation: Some("134340 Pluto".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.535104837022045e-04,
                            semi_major_axis: 2.131253489085252e06, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.128907994954536e02,
                            longitude_of_ascending_node: 2.273916747205139e02,
                            argument_of_periapsis: 3.320213368181675e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.692310582980274e02,
                        }),
                        gravitational_parameter: Some(1.2554846715321102e9), // Horizons Keplerian GM in m^3/s^2
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1188.3 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 180,
                            b: 160,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(132.993, -6.163, 302.695, 56.3625)),
                }), // Pluto
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Charon".into()),
                        id: "Charon".to_string(),
                        mass: 1.586e21,
                        major: true,
                        designation: Some("Pluto I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.659371523463578e-04,
                            semi_major_axis: 1.746421899437215e07, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.128908110174224e02,
                            longitude_of_ascending_node: 2.273916881630542e02,
                            argument_of_periapsis: 1.733606665035887e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.478897561707338e02,
                        }),
                        gravitational_parameter: Some(6.9049510564781247e11), // Horizons Keplerian GM in m^3/s^2
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 606.0 * 1000.0,
                        color: AppearanceColor {
                            r: 140,
                            g: 140,
                            b: 140,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Pluto", 132.993, -6.163)),
                }), // Charon
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Styx".into()),
                        id: "Styx".to_string(),
                        mass: 6.07e15,
                        major: false,
                        designation: Some("Pluto V".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 2.932325930342067e-02,
                            semi_major_axis: 4.349956289918996e07, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.128625753178244e02,
                            longitude_of_ascending_node: 2.273440223231328e02,
                            argument_of_periapsis: 1.956316936757438e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 1.164762075569714e01,
                        }),
                        gravitational_parameter: Some(9.7543074507001313e11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5.2 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Styx
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Nix".into()),
                        id: "Nix".to_string(),
                        mass: 2.24e16,
                        major: false,
                        designation: Some("Pluto II".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.346018266947629e-02,
                            semi_major_axis: 4.920633853117793e07, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.128886700008344e02,
                            longitude_of_ascending_node: 2.274158705502064e02,
                            argument_of_periapsis: 1.348490977263693e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.348254884831692e02,
                        }),
                        gravitational_parameter: Some(9.7542637791035281e11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18.0 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Nix
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kerberos".into()),
                        id: "Kerberos".to_string(),
                        mass: 9.05e15,
                        major: false,
                        designation: Some("Pluto IV".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 6.419234904797077e-03,
                            semi_major_axis: 5.820514501624906e07, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.132595550200874e02,
                            longitude_of_ascending_node: 2.271713663539564e02,
                            argument_of_periapsis: 3.295312369041157e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.158666684234013e02,
                        }),
                        gravitational_parameter: Some(9.7543068527824357e11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6.0 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Kerberos
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hydra".into()),
                        id: "Hydra".to_string(),
                        mass: 3.01e16,
                        major: false,
                        designation: Some("Pluto III".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Pluto Barycenter".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 1.542913778309203e-02,
                            semi_major_axis: 6.535472219278106e07, // km -> m
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 1.127041805913466e02,
                            longitude_of_ascending_node: 2.276191979399739e02,
                            argument_of_periapsis: 2.660579229073483e02,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 3.443275412772040e02,
                        }),
                        gravitational_parameter: Some(9.7542484255673514e11),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18.5 * 1000.0,
                        color: AppearanceColor {
                            r: 100,
                            g: 100,
                            b: 100,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Hydra
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eris".into()),
                        id: "Eris".to_string(),
                        mass: 1.6466e22,
                        major: true,
                        designation: Some("136199 Eris".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.43568,
                            semi_major_axis: 10.180e12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 43.822,
                            longitude_of_ascending_node: 36.046,
                            argument_of_periapsis: 150.714,
                        }),
                        epoch: KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
                            epoch: Instant::from_julian_day(2460800.5),
                            mean_anomaly: 211.032,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2326.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(79.6, 83.4, 0.0, 25.9)), // approx values
                }), // Eris
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dysnomia".into()),
                        id: "Dysnomia".to_string(),
                        mass: 8.2e19,
                        major: true,
                        designation: Some("136199 Eris I".into()),
                        tags: vec!["Moon".into(), "Minor Moon".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Eris".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0062,
                            semi_major_axis: 37273.0 * 1000.0,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 61.59,
                            longitude_of_ascending_node: 126.17,
                            argument_of_periapsis: 180.83,
                        }),
                        epoch: KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
                            epoch: Instant::from_julian_day(2453979.0),
                            mean_anomaly: 0.0 // TODO: Find this?
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 615.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(tidally_locked_rotation("Eris", 79.6, 83.4)), // tidally locked, uses Eris's pole
                }), // Dysnomia
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sedna".into()),
                        id: "Sedna".to_string(),
                        mass: 2.0e21, // Estimate from https://www.rasc.ca/asteroid/90377
                        major: true,
                        designation: Some("90377 Sedna".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "Sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.8496,
                            semi_major_axis: 76e12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 11.9307,
                            longitude_of_ascending_node: 144.248,
                            argument_of_periapsis: 311.352,
                        }),
                        epoch: KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
                            epoch: Instant::from_julian_day(2458900.5),
                            mean_anomaly: 358.117,
                        }),
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 906.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                        highlight_latitudes: vec![],
                    }),
                    rotation: Some(iau_rotation(0.0, 90.0, 0.0, 10.273)), // pole unknown, using approx period
                }), // Sedna
            ] },
    };
    solar_system
}

pub fn write_temp_system_file() {
    let solar_system = solar_system();
    let path = PathBuf::from("data/templates");
    ensure_folders(&[&path]).expect("Folders couldn't be made");
    solar_system.save().expect("Failed to save system");
}

pub fn earth_moon() -> UniverseFile {
    let solar_system = UniverseFile {
        file: Some(PathBuf::from("data/templates/earth_moon.toml")),
        contents: UniverseFileContents {
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
                        id: "Sol".to_string(),
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
                        id: "Earth".to_string(),
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
                        highlight_latitudes: vec![23.44, 66.56],
                    }),
                    rotation: Some(iau_rotation(0.0, 90.0, 190.147, 23.9344696)),
                }), // Earth
                /*SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "Luna".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("Earth I".into()),
                        tags: vec!["Moon".into(), "Major Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "Earth".to_string(),
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
                        gravitational_parameter: None,
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737.4 * 1000.0,
                        color: AppearanceColor {
                            r: 87,
                            g: 87,
                            b: 87,
                        },
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
                        highlight_latitudes: vec![],
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
                        highlight_latitudes: vec![],
                    }),
                    rotation: None,
                }), // Test Newtonian Body B
            ]
        },
    };
    solar_system
}

pub fn write_earth_moon_file() {
    let solar_system = earth_moon();
    let path = PathBuf::from("data/templates");
    ensure_folders(&[&path]).expect("Folders couldn't be made");
    solar_system.save().expect("Failed to save system");
}