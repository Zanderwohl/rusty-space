use std::path::PathBuf;
use bevy::math::DVec3;
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape, MeanAnomalyAtJ2000};
use crate::body::universe::save::{FixedEntry, KeplerEntry, SomeBody, UniverseFile, UniverseFileContents, UniverseFileTime};
use crate::gui::util::ensure_folders;
// Mass: Kg
// Distance: Km
// Longitude: From Vernal Equinox
// Angles: Degrees
// Inclination: degrees from ecliptic

pub fn solar_system() -> UniverseFile {
    let solar_system = UniverseFile {
        file: None,
        contents: UniverseFileContents {
            version: "0.0".into(),
            time: UniverseFileTime {
                time: 2451544.500000 // Midnight 2000 January 1 00:00
            },
            bodies: vec![
                SomeBody::FixedEntry(FixedEntry {
                    info: BodyInfo {
                        name: Some("Sol".into()),
                        id: "sol".to_string(),
                        mass: 1.988416e30,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    position: DVec3::ZERO,
                }), // Sun
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mercury".into()),
                        id: "mercury".to_string(),
                        mass: 3.3011e23,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.205630,
                            semi_major_axis: 5.791e7,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 7.005,
                            longitude_of_ascending_node: 48.331,
                            argument_of_periapsis: 29.124,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 174.796,
                        })
                    },
                }), // Mercury
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "venus".to_string(),
                        mass: 4.8675e24,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006772,
                            semi_major_axis: 1.0821e8, // 108,210,000
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
                }), // Venus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "earth".to_string(),
                        mass: 5.972168e24,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0167086,
                            semi_major_axis: 1.49598023e8, // 149,598,023
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
                }), // Earth
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mars".into()),
                        id: "mars".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0934,
                            semi_major_axis: 2.27939366e8, // 227,939,366
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
                }), // Mars
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ceres".into()),
                        id: "1-ceres".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("1 Ceres".into()),
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0785,
                            semi_major_axis: 4.14e8, // 414,000,000
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
                }), // Ceres
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "luna".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("Earth I".into()),
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "earth".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0549,
                            semi_major_axis: 3.84399e5,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles { // TODO: Precession https://en.wikipedia.org/wiki/Orbit_of_the_Moon#Precession
                            inclination: 5.145,
                            longitude_of_ascending_node: 0.0,
                            argument_of_periapsis: 0.0,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 0.0,
                        }),
                    },
                }), // Luna
            ] },
    };
    solar_system
}

pub fn write_temp_system_file() {
    let mut solar_system = solar_system();
    let path = PathBuf::from("data/templates");
    ensure_folders(&[&path]).expect("Folders couldn't be made");
    solar_system.file = Some(PathBuf::from("data/templates/solar_system.toml"));
    solar_system.save().expect("Failed to save system");
}

pub fn tiny_system() -> UniverseFile {
    let solar_system = UniverseFile {
        file: None,
        contents: UniverseFileContents {
            version: "0.0".into(),
            time: UniverseFileTime {
                time: 2451544.500000 // Midnight 2000 January 1 00:00
            },
            bodies: vec![
                SomeBody::FixedEntry(FixedEntry {
                    info: BodyInfo {
                        name: Some("Sol".into()),
                        id: "sol".to_string(),
                        mass: 1.988416e30,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    position: DVec3::ZERO,
                }), // Sun
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "venus".to_string(),
                        mass: 4.8675e24,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.006772,
                            semi_major_axis: 1.0821e8, // 108,210,000
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
                }), // Venus
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "earth".to_string(),
                        mass: 5.972168e24,
                        major: true,
                        designation: None,
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0167086,
                            semi_major_axis: 1.49598023e8, // 149,598,023
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
                }), // Earth
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "luna".to_string(),
                        mass: 6.4171,
                        major: true,
                        designation: Some("Earth I".into()),
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "earth".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0549,
                            semi_major_axis: 3.84399e5,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles { // TODO: Precession https://en.wikipedia.org/wiki/Orbit_of_the_Moon#Precession
                            inclination: 5.145,
                            longitude_of_ascending_node: 0.0,
                            argument_of_periapsis: 0.0,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 0.0,
                        }),
                    },
                }), // Luna
            ] },
    };
    solar_system
}

pub fn write_tiny_system_file() {
    let mut solar_system = tiny_system();
    let path = PathBuf::from("data/templates");
    ensure_folders(&[&path]).expect("Folders couldn't be made");
    solar_system.file = Some(PathBuf::from("data/templates/tiny_system.toml"));
    solar_system.save().expect("Failed to save system");
}