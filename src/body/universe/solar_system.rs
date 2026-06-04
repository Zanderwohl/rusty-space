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
                        id: "sol".to_string(),
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
                        id: "mercury".to_string(),
                        mass: 3.3011e23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2439.7 * 1000.0, // meters
                        color: AppearanceColor {
                            r: 145,
                            g: 145,
                            b: 145,
                        },
                    }),
                    rotation: Some(iau_rotation(281.0103, 61.4155, 329.5988, 1407.6)),
                }), // Mercury
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "venus".to_string(),
                        mass: 4.8675e24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6051.8 * 1000.0,
                        color: AppearanceColor {
                            r: 224,
                            g: 224,
                            b: 224,
                        },
                    }),
                    rotation: Some(iau_rotation(272.76, 67.16, 160.20, -5832.6)), // retrograde
                }), // Venus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "earth".to_string(),
                        mass: 5.972168e24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall{
                        radius: 6371.0 * 1000.0,
                        color: AppearanceColor {
                            r: 59,
                            g: 179,
                            b: 75
                        },
                    }),
                    rotation: Some(iau_rotation(0.0, 90.0, 190.147, 23.9344696)),
                }), // Earth
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mars".into()),
                        id: "mars".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3389.5 * 1000.0,
                        color: AppearanceColor {
                            r: 242,
                            g: 66,
                            b: 17,
                        },
                    }),
                    rotation: Some(iau_rotation(317.269, 54.432, 176.049, 24.6229)),
                }), // Mars
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ceres".into()),
                        id: "1-ceres".to_string(),
                        mass: 9.3839e20,
                        major: true,
                        designation: Some("1 Ceres".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 966.2 * 1000.0,
                        color: AppearanceColor {
                            r: 145,
                            g: 107,
                            b: 54,
                        },
                    }),
                    rotation: Some(iau_rotation(291.418, 66.764, 170.65, 9.074170)),
                }), // Ceres
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Vesta".into()),
                        id: "4-vesta".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("4 Vesta".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737.4 * 1000.0,
                        color: AppearanceColor {
                            r: 145,
                            g: 107,
                            b: 54,
                        },
                    }),
                    rotation: Some(iau_rotation(305.8, 41.4, 292.0, 5.342128)),
                }), // Vesta
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "luna".to_string(),
                        mass: 7.346e22,
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737.4 * 1000.0,
                        color: AppearanceColor {
                            r: 87,
                            g: 87,
                            b: 87,
                        },
                    }),
                    rotation: Some(tidally_locked_rotation("earth", 269.9949, 66.5392)), // tidally locked to Earth
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
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 69911.1 * 1000.0,
                        color: AppearanceColor {
                            r: 0xb0,
                            g: 0x7f,
                            b: 0x35,
                        },
                    }),
                    rotation: Some(iau_rotation(268.057, 64.495, 284.95, 9.9250)),
                }), // Jupiter
                // TODO: Jovian Moons
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
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 25362.0 * 1000.0,
                        color: AppearanceColor {
                            r: 60,
                            g: 186,
                            b: 180,
                        },
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
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 24622.0 * 1000.0,
                        color: AppearanceColor {
                            r: 60,
                            g: 186,
                            b: 180,
                        },
                    }),
                    rotation: Some(iau_rotation(299.36, 43.46, 253.18, 16.11)),
                }), // Neptune
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eris".into()),
                        id: "eris".to_string(),
                        mass: 1.6466e22,
                        major: true,
                        designation: Some("136199 Eris".into()),
                        tags: vec!["Planet".into(), "Minor Planet".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2326.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                    }),
                    rotation: Some(iau_rotation(79.6, 83.4, 0.0, 25.9)), // approx values
                }), // Eris
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dysnomia".into()),
                        id: "dysnomia".to_string(),
                        mass: 8.2e19,
                        major: true,
                        designation: Some("136199 Eris I".into()),
                        tags: vec!["Moon".into()],
                    },
                    params: KeplerMotive {
                        primary_id: "eris".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 615.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
                    }),
                    rotation: Some(tidally_locked_rotation("eris", 79.6, 83.4)), // tidally locked, uses Eris's pole
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
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 906.0 * 1000.0,
                        color: AppearanceColor {
                            r: 200,
                            g: 200,
                            b: 200,
                        },
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