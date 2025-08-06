use std::default::Default;
use std::path::PathBuf;
use bevy::math::DVec3;
use crate::body::appearance::{Appearance, AppearanceColor, DebugBall, StarBall};
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerPrecessingEulerAngles, KeplerRotation, KeplerShape, MeanAnomalyAtEpoch, MeanAnomalyAtJ2000};
use crate::body::universe::save::{FixedEntry, KeplerEntry, NewtonEntry, SomeBody, UniverseFile, UniverseFileContents, UniverseFileTime, UniversePhysics, ViewSettings};
use crate::gui::util::ensure_folders;
// Mass: Kg
// Distance: Km
// Longitude: From Vernal Equinox
// Angles: Degrees
// Inclination: degrees from ecliptic

pub fn solar_system() -> UniverseFile {
    let solar_system = UniverseFile {
        file: Some(PathBuf::from("data/templates/solar_system.toml")),
        contents: UniverseFileContents {
            version: "0.0".into(),
            time: UniverseFileTime {
                time_julian_days: 2451544.500000 // Midnight 2000 January 1 00:00
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
                        intensity: 10000.0,
                    }),
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
                            epoch_julian_day: 2453300.5,
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
                }), // Luna
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jupiter".into()),
                        id: "Jupiter".to_string(),
                        mass: 1.8982e27,
                        major: true,
                        designation: None,
                        uuid: Default::default(),
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
                }), // Jupiter
                // TODO: Jovian Moons
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Uranus".into()),
                        id: "Uranus".to_string(),
                        mass: 8.681e25,
                        major: true,
                        designation: None,
                        uuid: Default::default(),
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
                }), // Uranus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Neptune".into()),
                        id: "Neptune".to_string(),
                        mass: 1.02409e26,
                        major: true,
                        designation: None,
                        uuid: Default::default(),
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
                }), // Neptune
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sedna".into()),
                        id: "Sedna".to_string(),
                        mass: 2.0e21, // Estimate from https://www.rasc.ca/asteroid/90377
                        major: true,
                        designation: Some("90377 Sedna".into()),
                        uuid: Default::default(),
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
                            epoch_julian_day: 2458900.5,
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
                time_julian_days: 2451544.500000 // Midnight 2000 January 1 00:00
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
                        uuid: Default::default(),
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
                }), // Test Newtonian Body A
                SomeBody::NewtonEntry(NewtonEntry {
                    info: BodyInfo {
                        name: Some("Newtonian Test Body B".into()),
                        id: "NTB-B".to_string(),
                        mass: 1000.0,
                        major: false,
                        designation: Some("TB-B".into()),
                        uuid: Default::default(),
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