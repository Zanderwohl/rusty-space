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
                // Mercury: fitted to 1501 JPL states over 60.0 yr; residual 1999 km RMS (3.4e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mercury".into()),
                        id: "mercury".to_string(),
                        mass: 3.300999895e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2439400.0,
                        color: AppearanceColor { r: 145, g: 145, b: 145 },
                    }),
                    rotation: None,
                }),
                // Venus: fitted to 1501 JPL states over 60.0 yr; residual 6351 km RMS (5.9e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Venus".into()),
                        id: "venus".to_string(),
                        mass: 4.867304721e+24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6051840.0,
                        color: AppearanceColor { r: 224, g: 224, b: 224 },
                    }),
                    rotation: None,
                }),
                // Earth: fitted to 1501 JPL states over 60.0 yr; residual 10101 km RMS (6.8e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Earth".into()),
                        id: "earth".to_string(),
                        mass: 5.972167057e+24,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6371010.0,
                        color: AppearanceColor { r: 59, g: 179, b: 75 },
                    }),
                    rotation: None,
                }),
                // Mars: fitted to 1501 JPL states over 60.0 yr; residual 36942 km RMS (1.6e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mars".into()),
                        id: "mars".to_string(),
                        mass: 6.416907546e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3389920.0,
                        color: AppearanceColor { r: 242, g: 66, b: 17 },
                    }),
                    rotation: None,
                }),
                // Jupiter: fitted to 1501 JPL states over 60.0 yr; residual 648573 km RMS (8.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Jupiter".into()),
                        id: "Jupiter".to_string(),
                        mass: 1.898124199e+27,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 69911000.0,
                        color: AppearanceColor { r: 201, g: 144, b: 57 },
                    }),
                    rotation: None,
                }),
                // Saturn: fitted to 1501 JPL states over 60.0 yr; residual 2194887 km RMS (1.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Saturn".into()),
                        id: "saturn".to_string(),
                        mass: 5.683172424e+26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58232000.0,
                        color: AppearanceColor { r: 206, g: 184, b: 124 },
                    }),
                    rotation: None,
                }),
                // Uranus: fitted to 1501 JPL states over 60.0 yr; residual 814615 km RMS (2.8e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Uranus".into()),
                        id: "Uranus".to_string(),
                        mass: 8.680984235e+25,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 25362000.0,
                        color: AppearanceColor { r: 60, g: 186, b: 180 },
                    }),
                    rotation: None,
                }),
                // Neptune: fitted to 1501 JPL states over 60.0 yr; residual 834285 km RMS (1.9e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Neptune".into()),
                        id: "Neptune".to_string(),
                        mass: 1.02409218e+26,
                        major: true,
                        designation: None,
                        tags: vec!["Planet".into(), "Major Planet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 24624000.0,
                        color: AppearanceColor { r: 60, g: 186, b: 180 },
                    }),
                    rotation: None,
                }),
                // Pluto: fitted to 1501 JPL states over 60.0 yr; residual 828139 km RMS (1.5e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pluto".into()),
                        id: "pluto".to_string(),
                        mass: 1.302497347e+22,
                        major: true,
                        designation: None,
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.2490476479,
                            semi_major_axis: 5.907594583e+12,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 17.14041658,
                            longitude_of_ascending_node: 110.3026483,
                            argument_of_periapsis: 113.7709211,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 14.84948051,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(90578.63897)),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1188300.0,
                        color: AppearanceColor { r: 200, g: 195, b: 190 },
                    }),
                    rotation: None,
                }),
                // Luna: fitted to 1500 JPL states over 18.5 yr; residual 7562 km RMS (2.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Luna".into()),
                        id: "luna".to_string(),
                        mass: 7.345787519e+22,
                        major: true,
                        designation: Some("Earth I".into()),
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "earth".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1737530.0,
                        color: AppearanceColor { r: 87, g: 87, b: 87 },
                    }),
                    rotation: None,
                }),
                // Phobos: fitted to 1493 JPL states over 0.2 yr; residual 21 km RMS (2.2e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phobos".into()),
                        id: "phobos".to_string(),
                        mass: 8.496833172e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "mars".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 11058.40287,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Deimos: fitted to 1501 JPL states over 0.9 yr; residual 495 km RMS (2.1e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Deimos".into()),
                        id: "deimos".to_string(),
                        mass: 1.499670669e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "mars".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6203.050874,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Io: fitted to 1501 JPL states over 1.2 yr; residual 122 km RMS (2.9e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Io".into()),
                        id: "io".to_string(),
                        mass: 8.929646795e+22,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1821490.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Europa: fitted to 1500 JPL states over 2.4 yr; residual 653 km RMS (9.7e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Europa".into()),
                        id: "europa".to_string(),
                        mass: 4.798572705e+22,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1560800.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Ganymede: fitted to 1501 JPL states over 4.9 yr; residual 1244 km RMS (1.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ganymede".into()),
                        id: "ganymede".to_string(),
                        mass: 1.481478294e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2631200.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Callisto: fitted to 1500 JPL states over 11.4 yr; residual 1356 km RMS (7.2e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Callisto".into()),
                        id: "callisto".to_string(),
                        mass: 1.075660637e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2410300.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Amalthea: fitted to 1505 JPL states over 0.3 yr; residual 1371 km RMS (7.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Amalthea".into()),
                        id: "amalthea".to_string(),
                        mass: 2.466175674e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 83500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Himalia: fitted to 1501 JPL states over 60.0 yr; residual 576030 km RMS (5.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Himalia".into()),
                        id: "himalia".to_string(),
                        mass: 2.270679561e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 85000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Elara: fitted to 1501 JPL states over 60.0 yr; residual 761467 km RMS (6.3e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Elara".into()),
                        id: "elara".to_string(),
                        mass: 4.021238597e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 40000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Pasiphae: fitted to 1501 JPL states over 60.0 yr; residual 4679054 km RMS (1.8e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pasiphae".into()),
                        id: "pasiphae".to_string(),
                        mass: 3.664353671e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Sinope: fitted to 1501 JPL states over 60.0 yr; residual 2423291 km RMS (9.8e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sinope".into()),
                        id: "sinope".to_string(),
                        mass: 1.724106048e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Lysithea: fitted to 1501 JPL states over 60.0 yr; residual 436643 km RMS (3.7e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Lysithea".into()),
                        id: "lysithea".to_string(),
                        mass: 1.085734421e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 12000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Carme: fitted to 1501 JPL states over 60.0 yr; residual 1971477 km RMS (8.2e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Carme".into()),
                        id: "carme".to_string(),
                        mass: 2.120575041e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Ananke: fitted to 1501 JPL states over 60.0 yr; residual 2675785 km RMS (1.2e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ananke".into()),
                        id: "ananke".to_string(),
                        mass: 6.283185307e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 10000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Leda: fitted to 1501 JPL states over 60.0 yr; residual 493206 km RMS (4.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Leda".into()),
                        id: "leda".to_string(),
                        mass: 7.853981634e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Thebe: fitted to 1497 JPL states over 0.5 yr; residual 2485 km RMS (1.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Thebe".into()),
                        id: "thebe".to_string(),
                        mass: 4.509835224e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 49300.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Adrastea: fitted to 1492 JPL states over 0.2 yr; residual 11 km RMS (8.5e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Adrastea".into()),
                        id: "adrastea".to_string(),
                        mass: 1.498284128e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 8200.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Metis: fitted to 1495 JPL states over 0.2 yr; residual 851 km RMS (6.7e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Metis".into()),
                        id: "metis".to_string(),
                        mass: 3.745710319e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 21500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Mimas: fitted to 1503 JPL states over 0.6 yr; residual 1920 km RMS (1.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mimas".into()),
                        id: "mimas".to_string(),
                        mass: 3.750937832e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 198800.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Enceladus: fitted to 1500 JPL states over 0.9 yr; residual 26 km RMS (1.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Enceladus".into()),
                        id: "enceladus".to_string(),
                        mass: 1.080317843e+20,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 252300.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Tethys: fitted to 1501 JPL states over 1.3 yr; residual 525 km RMS (1.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tethys".into()),
                        id: "tethys".to_string(),
                        mass: 6.17442889e+20,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 536300.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Dione: fitted to 1500 JPL states over 1.9 yr; residual 83 km RMS (2.2e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dione".into()),
                        id: "dione".to_string(),
                        mass: 1.095485423e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 562500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Rhea: fitted to 1500 JPL states over 3.1 yr; residual 476 km RMS (9.0e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Rhea".into()),
                        id: "rhea".to_string(),
                        mass: 2.306458586e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 764500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Titan: fitted to 1501 JPL states over 10.9 yr; residual 278 km RMS (2.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Titan".into()),
                        id: "titan".to_string(),
                        mass: 1.345180466e+23,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2575500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Hyperion: fitted to 1500 JPL states over 14.6 yr; residual 174107 km RMS (1.2e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hyperion".into()),
                        id: "hyperion".to_string(),
                        mass: 5.551142693e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 133000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Iapetus: fitted to 1500 JPL states over 54.3 yr; residual 20702 km RMS (5.8e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Iapetus".into()),
                        id: "iapetus".to_string(),
                        mass: 1.805732031e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 734500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Phoebe: fitted to 1501 JPL states over 60.0 yr; residual 273765 km RMS (2.1e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phoebe".into()),
                        id: "phoebe".to_string(),
                        mass: 8.31248034e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 106600.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Janus: fitted to 1500 JPL states over 0.5 yr; residual 302 km RMS (2.0e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Janus".into()),
                        id: "janus".to_string(),
                        mass: 4.534279715e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 89696.63414,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Epimetheus: fitted to 1501 JPL states over 0.5 yr; residual 571 km RMS (3.8e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Epimetheus".into()),
                        id: "epimetheus".to_string(),
                        mass: 1.238383214e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 58195.81163,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Helene: fitted to 1501 JPL states over 1.9 yr; residual 39612 km RMS (1.0e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Helene".into()),
                        id: "helene".to_string(),
                        mass: 7.127936908e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 16000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Atlas: fitted to 1498 JPL states over 0.4 yr; residual 15 km RMS (1.1e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Atlas".into()),
                        id: "atlas".to_string(),
                        mass: 2.155170259e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 15081.13077,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Prometheus: fitted to 1505 JPL states over 0.4 yr; residual 29 km RMS (2.1e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Prometheus".into()),
                        id: "prometheus".to_string(),
                        mass: 5.026974497e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 43089.91174,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Pandora: fitted to 1502 JPL states over 0.4 yr; residual 190 km RMS (1.3e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pandora".into()),
                        id: "pandora".to_string(),
                        mass: 4.215228173e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 40633.14207,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Pan: fitted to 1505 JPL states over 0.4 yr; residual 1 km RMS (1.0e-05 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pan".into()),
                        id: "pan".to_string(),
                        mass: 1.730861729e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "saturn".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 14018.26188,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Ariel: fitted to 1500 JPL states over 1.7 yr; residual 62 km RMS (3.3e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ariel".into()),
                        id: "ariel".to_string(),
                        mass: 1.250018448e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 578897.9067,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Umbriel: fitted to 1500 JPL states over 2.8 yr; residual 227 km RMS (8.5e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Umbriel".into()),
                        id: "umbriel".to_string(),
                        mass: 1.279534645e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 584700.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Titania: fitted to 1500 JPL states over 6.0 yr; residual 607 km RMS (1.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Titania".into()),
                        id: "titania".to_string(),
                        mass: 3.338177036e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 788900.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Oberon: fitted to 1501 JPL states over 9.2 yr; residual 820 km RMS (1.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Oberon".into()),
                        id: "oberon".to_string(),
                        mass: 3.076576628e+21,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 761400.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Miranda: fitted to 1502 JPL states over 1.0 yr; residual 2054 km RMS (1.6e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Miranda".into()),
                        id: "miranda".to_string(),
                        mass: 6.442621749e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 235679.8973,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Puck: fitted to 1501 JPL states over 0.5 yr; residual 542 km RMS (6.3e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Puck".into()),
                        id: "puck".to_string(),
                        mass: 2.868481438e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 77000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Triton: fitted to 1501 JPL states over 4.0 yr; residual 2368 km RMS (6.7e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Triton".into()),
                        id: "triton".to_string(),
                        mass: 2.140291385e+22,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1352600.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Nereid: fitted to 1501 JPL states over 60.0 yr; residual 53804 km RMS (7.6e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Nereid".into()),
                        id: "nereid".to_string(),
                        mass: 3.086928941e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 170000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Despina: fitted to 1491 JPL states over 0.2 yr; residual 3 km RMS (6.4e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Despina".into()),
                        id: "despina".to_string(),
                        mass: 1.748947062e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 74000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Larissa: fitted to 1504 JPL states over 0.4 yr; residual 119 km RMS (1.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Larissa".into()),
                        id: "larissa".to_string(),
                        mass: 3.818227271e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 96000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Proteus: fitted to 1503 JPL states over 0.8 yr; residual 13 km RMS (1.1e-04 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Proteus".into()),
                        id: "proteus".to_string(),
                        mass: 3.865573049e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 208000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Charon: fitted to 1500 JPL states over 4.4 yr; residual 1 km RMS (6.6e-05 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Charon".into()),
                        id: "charon".to_string(),
                        mass: 1.589679459e+21,
                        major: true,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "pluto".to_string(),
                        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                            eccentricity: 0.0001608992363,
                            semi_major_axis: 19595764.48,
                        }),
                        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                            inclination: 112.8948049,
                            longitude_of_ascending_node: 227.3881936,
                            argument_of_periapsis: 172.4742902,
                        }),
                        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                            mean_anomaly: 148.7749765,
                        }),
                        anomalistic_period: Some(TimeDelta::from_days(6.387222133)),
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 606000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Nix: fitted to 1500 JPL states over 17.4 yr; residual 88 km RMS (1.8e-03 of orbit radius).
                // Orbits the Pluto-Charon barycentre; modelled about Pluto, so it carries
                // a further ~2100 km offset the fit cannot remove.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Nix".into()),
                        id: "nix".to_string(),
                        mass: 2.241433055e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "pluto".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Hydra: fitted to 1500 JPL states over 26.6 yr; residual 216 km RMS (3.3e-03 of orbit radius).
                // Orbits the Pluto-Charon barycentre; modelled about Pluto, so it carries
                // a further ~2100 km offset the fit cannot remove.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hydra".into()),
                        id: "hydra".to_string(),
                        mass: 3.011551096e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "pluto".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 18500.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Kerberos: fitted to 1501 JPL states over 22.4 yr; residual 417 km RMS (7.2e-03 of orbit radius).
                // Orbits the Pluto-Charon barycentre; modelled about Pluto, so it carries
                // a further ~2100 km offset the fit cannot remove.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kerberos".into()),
                        id: "kerberos".to_string(),
                        mass: 9.046639562e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "pluto".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6000.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Styx: fitted to 1500 JPL states over 14.5 yr; residual 278 km RMS (6.6e-03 of orbit radius).
                // Orbits the Pluto-Charon barycentre; modelled about Pluto, so it carries
                // a further ~2100 km offset the fit cannot remove.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Styx".into()),
                        id: "styx".to_string(),
                        mass: 6.068050717e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "pluto".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5200.0,
                        color: AppearanceColor { r: 140, g: 140, b: 140 },
                    }),
                    rotation: None,
                }),
                // Dysnomia: fitted to 1500 JPL states over 10.8 yr; residual 0 km RMS (4.9e-06 of orbit radius).
                // radius: repo value, halved (it was a diameter)
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Dysnomia".into()),
                        id: "dysnomia".to_string(),
                        mass: 1.432359626e+20,
                        major: false,
                        designation: Some("Eris I".into()),
                        tags: vec!["Moon".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "eris".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 307500.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                    }),
                    rotation: None,
                }),
                // Ceres: fitted to 1501 JPL states over 60.0 yr; residual 1454693 km RMS (3.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ceres".into()),
                        id: "1-ceres".to_string(),
                        mass: 9.383513765e+20,
                        major: true,
                        designation: Some("1 Ceres".into()),
                        tags: vec!["Dwarf Planet".into(), "Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 469700.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Pallas: fitted to 1501 JPL states over 60.0 yr; residual 3611206 km RMS (8.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pallas".into()),
                        id: "2-pallas".to_string(),
                        mass: 2.042161266e+20,
                        major: true,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 256500.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Juno: fitted to 1501 JPL states over 60.0 yr; residual 2006646 km RMS (4.9e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Juno".into()),
                        id: "3-juno".to_string(),
                        mass: 2.119924868e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 123298.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Vesta: fitted to 1501 JPL states over 60.0 yr; residual 788653 km RMS (2.2e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Vesta".into()),
                        id: "4-vesta".to_string(),
                        mass: 2.590275552e+20,
                        major: true,
                        designation: Some("4 Vesta".into()),
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 261385.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Hygiea: fitted to 1501 JPL states over 60.0 yr; residual 14410998 km RMS (3.0e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hygiea".into()),
                        id: "10-hygiea".to_string(),
                        mass: 1.048798889e+20,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 203560.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Eunomia: fitted to 1501 JPL states over 60.0 yr; residual 1243161 km RMS (3.1e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eunomia".into()),
                        id: "15-eunomia".to_string(),
                        mass: 1.758241926e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 115844.5,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Psyche: fitted to 1501 JPL states over 60.0 yr; residual 3154063 km RMS (7.1e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Psyche".into()),
                        id: "16-psyche".to_string(),
                        mass: 2.398752888e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 111000.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Lutetia: fitted to 1501 JPL states over 60.0 yr; residual 935376 km RMS (2.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Lutetia".into()),
                        id: "21-lutetia".to_string(),
                        mass: 1.699054201e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 49000.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // 52 Europa: fitted to 1501 JPL states over 60.0 yr; residual 6322467 km RMS (1.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.4 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("52 Europa".into()),
                        id: "52-europa".to_string(),
                        mass: 2.057765709e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 151959.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Sylvia: fitted to 1501 JPL states over 60.0 yr; residual 8258708 km RMS (1.6e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 4.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sylvia".into()),
                        id: "87-sylvia".to_string(),
                        mass: 3.393772977e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 126525.5,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Kleopatra: fitted to 1501 JPL states over 60.0 yr; residual 3065496 km RMS (7.1e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 4.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Kleopatra".into()),
                        id: "216-kleopatra".to_string(),
                        mass: 3.803103158e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 61000.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Ida: fitted to 1501 JPL states over 60.0 yr; residual 1491981 km RMS (3.5e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ida".into()),
                        id: "243-ida".to_string(),
                        mass: 4.120281351e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 16000.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Mathilde: fitted to 1501 JPL states over 60.0 yr; residual 2193638 km RMS (5.4e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Mathilde".into()),
                        id: "253-mathilde".to_string(),
                        mass: 1.032317764e+17,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 26400.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Eros: fitted to 1501 JPL states over 60.0 yr; residual 575502 km RMS (2.6e-03 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eros".into()),
                        id: "433-eros".to_string(),
                        mass: 6.686842061e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 8420.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Davida: fitted to 1501 JPL states over 60.0 yr; residual 6532397 km RMS (1.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.4 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Davida".into()),
                        id: "511-davida".to_string(),
                        mass: 1.448087926e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 135163.5,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Interamnia: fitted to 1501 JPL states over 60.0 yr; residual 6699899 km RMS (1.4e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Interamnia".into()),
                        id: "704-interamnia".to_string(),
                        mass: 7.491420638e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 153156.5,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Gaspra: fitted to 1501 JPL states over 60.0 yr; residual 275772 km RMS (8.2e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gaspra".into()),
                        id: "951-gaspra".to_string(),
                        mass: 2.567094632e+15,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 6100.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Icarus: fitted to 1501 JPL states over 60.0 yr; residual 779562 km RMS (3.6e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Icarus".into()),
                        id: "1566-icarus".to_string(),
                        mass: 1.308996939e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 500.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Geographos: fitted to 1501 JPL states over 60.0 yr; residual 1712619 km RMS (8.7e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Geographos".into()),
                        id: "1620-geographos".to_string(),
                        mass: 2.371823034e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1280.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Steins: fitted to 1501 JPL states over 60.0 yr; residual 902203 km RMS (2.5e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 2.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Steins".into()),
                        id: "2867-steins".to_string(),
                        mass: 1.798405971e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2580.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Phaethon: fitted to 1501 JPL states over 60.0 yr; residual 708620 km RMS (2.7e-03 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.4 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Phaethon".into()),
                        id: "3200-phaethon".to_string(),
                        mass: 1.789644253e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3125.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Toutatis: fitted to 1501 JPL states over 60.0 yr; residual 28806280 km RMS (6.4e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Toutatis".into()),
                        id: "4179-toutatis".to_string(),
                        mass: 2.226094855e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2700.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Itokawa: fitted to 1501 JPL states over 60.0 yr; residual 6648790 km RMS (3.2e-02 of orbit radius).
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Itokawa".into()),
                        id: "25143-itokawa".to_string(),
                        mass: 3.146396668e+10,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 165.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Apophis: fitted to 1501 JPL states over 60.0 yr; residual 135944315 km RMS (8.8e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 2.7 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Apophis".into()),
                        id: "99942-apophis".to_string(),
                        mass: 5.556472095e+10,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 170.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Bennu: fitted to 1501 JPL states over 60.0 yr; residual 4909888 km RMS (2.9e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.4 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Bennu".into()),
                        id: "101955-bennu".to_string(),
                        mass: 8.20857504e+10,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 241.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Ryugu: fitted to 1501 JPL states over 60.0 yr; residual 14230447 km RMS (7.8e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ryugu".into()),
                        id: "162173-ryugu".to_string(),
                        mass: 4.494852383e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Asteroid".into(), "NEO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 448.0,
                        color: AppearanceColor { r: 145, g: 107, b: 54 },
                    }),
                    rotation: None,
                }),
                // Eris: fitted to 1501 JPL states over 60.0 yr; residual 830549 km RMS (5.9e-05 of orbit radius).
                // radius: repo value, halved (it was a diameter)
                // mass: repo value; JPL publishes no GM for this body
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Eris".into()),
                        id: "eris".to_string(),
                        mass: 1.6466e+22,
                        major: true,
                        designation: Some("136199 Eris".into()),
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1163000.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                    }),
                    rotation: None,
                }),
                // Makemake: fitted to 1501 JPL states over 60.0 yr; residual 831049 km RMS (1.1e-04 of orbit radius).
                // radius: ESTIMATE from H=-0.25 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Makemake".into()),
                        id: "136472-makemake".to_string(),
                        mass: 9.644973973e+22,
                        major: false,
                        designation: None,
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2485270.876,
                        color: AppearanceColor { r: 200, g: 195, b: 190 },
                    }),
                    rotation: None,
                }),
                // Haumea: fitted to 1501 JPL states over 60.0 yr; residual 831330 km RMS (1.1e-04 of orbit radius).
                // radius: ESTIMATE from H=0.14 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Haumea".into()),
                        id: "136108-haumea".to_string(),
                        mass: 5.627312845e+22,
                        major: false,
                        designation: None,
                        tags: vec!["Dwarf Planet".into(), "TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2076699.845,
                        color: AppearanceColor { r: 200, g: 195, b: 190 },
                    }),
                    rotation: None,
                }),
                // Sedna: fitted to 1501 JPL states over 60.0 yr; residual 831277 km RMS (6.7e-05 of orbit radius).
                // radius: repo value, halved (it was a diameter)
                // mass: repo value; JPL publishes no GM for this body
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Sedna".into()),
                        id: "Sedna".to_string(),
                        mass: 2e+21,
                        major: false,
                        designation: Some("90377 Sedna".into()),
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 453000.0,
                        color: AppearanceColor { r: 200, g: 200, b: 200 },
                    }),
                    rotation: None,
                }),
                // Quaoar: fitted to 1501 JPL states over 60.0 yr; residual 830156 km RMS (1.3e-04 of orbit radius).
                // radius: ESTIMATE from H=2.41 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Quaoar".into()),
                        id: "50000-quaoar".to_string(),
                        mass: 2.445124966e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 730085.5125,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Orcus: fitted to 1501 JPL states over 60.0 yr; residual 830569 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=2.13 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Orcus".into()),
                        id: "90482-orcus".to_string(),
                        mass: 3.599988057e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 830565.2,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Gonggong: fitted to 1501 JPL states over 60.0 yr; residual 831281 km RMS (6.2e-05 of orbit radius).
                // radius: ESTIMATE from H=1.82 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Gonggong".into()),
                        id: "225088-gonggong".to_string(),
                        mass: 5.524602811e+21,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 958018.1357,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Ixion: fitted to 1501 JPL states over 60.0 yr; residual 829598 km RMS (1.5e-04 of orbit radius).
                // radius: ESTIMATE from H=3.47 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Ixion".into()),
                        id: "28978-ixion".to_string(),
                        mass: 5.653287341e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 448098.7481,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Varuna: fitted to 1501 JPL states over 60.0 yr; residual 830429 km RMS (1.3e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Varuna".into()),
                        id: "20000-varuna".to_string(),
                        mass: 5.725552611e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 450000.0,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Arrokoth: fitted to 1501 JPL states over 60.0 yr; residual 829975 km RMS (1.3e-04 of orbit radius).
                // radius: ESTIMATE from H=11.06 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Arrokoth".into()),
                        id: "486958-arrokoth".to_string(),
                        mass: 1.578705275e+16,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 13594.82841,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Huya: fitted to 1501 JPL states over 60.0 yr; residual 832287 km RMS (1.8e-04 of orbit radius).
                // radius: ESTIMATE from H=4.79 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Huya".into()),
                        id: "38628-huya".to_string(),
                        mass: 9.126432794e+19,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 243990.9571,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Varda: fitted to 1501 JPL states over 60.0 yr; residual 830882 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=3.46 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Varda".into()),
                        id: "174567-varda".to_string(),
                        mass: 5.731932402e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 450167.0779,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Salacia: fitted to 1501 JPL states over 60.0 yr; residual 830969 km RMS (1.2e-04 of orbit radius).
                // radius: ESTIMATE from H=4.12 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Salacia".into()),
                        id: "120347-salacia".to_string(),
                        mass: 2.303037768e+20,
                        major: false,
                        designation: None,
                        tags: vec!["TNO".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 332180.1911,
                        color: AppearanceColor { r: 190, g: 185, b: 180 },
                    }),
                    rotation: None,
                }),
                // Chiron: fitted to 1501 JPL states over 60.0 yr; residual 998151 km RMS (4.6e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.4 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Chiron".into()),
                        id: "2060-chiron".to_string(),
                        mass: 3.353134099e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 83000.0,
                        color: AppearanceColor { r: 160, g: 140, b: 130 },
                    }),
                    rotation: None,
                }),
                // Pholus: fitted to 1501 JPL states over 60.0 yr; residual 826311 km RMS (2.0e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Pholus".into()),
                        id: "5145-pholus".to_string(),
                        mass: 3.591364002e+18,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 95000.0,
                        color: AppearanceColor { r: 160, g: 140, b: 130 },
                    }),
                    rotation: None,
                }),
                // Chariklo: fitted to 1501 JPL states over 60.0 yr; residual 946664 km RMS (3.9e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 1.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Chariklo".into()),
                        id: "10199-chariklo".to_string(),
                        mass: 1.442179942e+19,
                        major: false,
                        designation: None,
                        tags: vec!["Centaur".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 151000.0,
                        color: AppearanceColor { r: 160, g: 140, b: 130 },
                    }),
                    rotation: None,
                }),
                // Halley: fitted to 1501 JPL states over 60.0 yr; residual 832298 km RMS (1.9e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Halley".into()),
                        id: "1p-halley".to_string(),
                        mass: 3.484549852e+14,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 5500.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Encke: fitted to 1501 JPL states over 60.0 yr; residual 8201130 km RMS (1.8e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Encke".into()),
                        id: "2p-encke".to_string(),
                        mass: 2.89529179e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2400.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Tempel 1: fitted to 1501 JPL states over 60.0 yr; residual 246893753 km RMS (4.5e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Tempel 1".into()),
                        id: "9p-tempel1".to_string(),
                        mass: 5.654866776e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 3000.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Borrelly: fitted to 1501 JPL states over 60.0 yr; residual 32021464 km RMS (5.0e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Borrelly".into()),
                        id: "19p-borrelly".to_string(),
                        mass: 2.89529179e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2400.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Churyumov-Gerasimenko: fitted to 1501 JPL states over 60.0 yr; residual 74557893 km RMS (1.2e-01 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Churyumov-Gerasimenko".into()),
                        id: "67p-cg".to_string(),
                        mass: 9.921637493e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1700.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Wild 2: fitted to 1501 JPL states over 60.0 yr; residual 32952857 km RMS (5.5e-02 of orbit radius).
                // A two-body model is a poor fit here; treat its position as indicative.
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Wild 2".into()),
                        id: "81p-wild2".to_string(),
                        mass: 1.675516082e+13,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 2000.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Hartley 2: fitted to 1501 JPL states over 60.0 yr; residual 22428135 km RMS (3.4e-02 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hartley 2".into()),
                        id: "103p-hartley2".to_string(),
                        mass: 1.072330292e+12,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 800.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // Hale-Bopp: fitted to 1501 JPL states over 60.0 yr; residual 829524 km RMS (1.1e-04 of orbit radius).
                // mass: ESTIMATE from radius at assumed density 0.5 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Hale-Bopp".into()),
                        id: "c1995o1-halebopp".to_string(),
                        mass: 5.654866776e+16,
                        major: false,
                        designation: None,
                        tags: vec!["Comet".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 30000.0,
                        color: AppearanceColor { r: 170, g: 200, b: 210 },
                    }),
                    rotation: None,
                }),
                // 'Oumuamua: fitted to 1501 JPL states over 10.0 yr; residual 604215 km RMS (2.4e-04 of orbit radius).
                // radius: ESTIMATE from H=22.08 at assumed albedo 0.09
                // mass: ESTIMATE from radius at assumed density 1.0 g/cm3
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("'Oumuamua".into()),
                        id: "1i-oumuamua".to_string(),
                        mass: 2571637801.0,
                        major: false,
                        designation: None,
                        tags: vec!["Interstellar".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 84.99115488,
                        color: AppearanceColor { r: 200, g: 170, b: 210 },
                    }),
                    rotation: None,
                }),
                // Borisov: fitted to 1501 JPL states over 10.0 yr; residual 509480 km RMS (1.8e-04 of orbit radius).
                // mass: unknown; no GM and no size published
                SomeBody::KeplerEntry(KeplerEntry {
                    info: BodyInfo {
                        name: Some("Borisov".into()),
                        id: "2i-borisov".to_string(),
                        mass: 0.0,
                        major: false,
                        designation: None,
                        tags: vec!["Interstellar".into()],
                        ..Default::default()
                    },
                    params: KeplerMotive {
                        primary_id: "sol".to_string(),
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
                    },
                    appearance: Appearance::DebugBall(DebugBall {
                        radius: 1000.0,
                        color: AppearanceColor { r: 200, g: 170, b: 210 },
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
    };
    earth_moon
}
