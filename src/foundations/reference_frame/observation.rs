use std::collections::HashMap;
use bevy::math::{DVec3, DQuat};
use crate::foundations::reference_frame::{ReferenceFrame};
use crate::foundations::time::Instant;

/// An Observation represents standing at a location and seeing another location
/// As such, it is an angle from a reference frame.
pub struct Observation {
    frame: ReferenceFrame,
    time: Instant,
    direction: DQuat,
}

impl Observation {
    pub fn from_azimuth_zenith(azimuth_rad: f64, zenith_rad: f64, frame: &ReferenceFrame, time: Instant) -> Self {
        Self {
            frame: frame.clone(),
            time,
            direction: quat_from_azimuth_zenith(azimuth_rad, zenith_rad),
        }
    }

    pub fn from_azimuth_elevation(azimuth_rad: f64, elevation_rad: f64, frame: &ReferenceFrame, time: Instant) -> Self {
        Self {
            frame: frame.clone(),
            time,
            direction: quat_from_azimuth_elevation(azimuth_rad, elevation_rad),
        }
    }

    pub fn universal_origin(&self) -> DVec3 {
        self.frame.universal_origin()
    }

    pub fn forward(&self) -> DVec3 {
        self.direction * DVec3::X
    }

    pub fn direction(&self) -> &DQuat {
        &self.direction
    }
}

/// Zenith: 0 = up (+Z), π/2 = horizontal, π = down (-Z)
/// Azimuth: 0 = +X direction, π/2 = +Y direction
fn quat_from_azimuth_zenith(azimuth_rad: f64, zenith_rad: f64) -> DQuat {
    // Rotate around Z-axis by azimuth, then Y-axis by zenith
    DQuat::from_rotation_z(azimuth_rad) * DQuat::from_rotation_y(zenith_rad)
}

/// Elevation: -π/2 = down, 0 = horizontal, π/2 = up
/// Azimuth: 0 = +X direction, π/2 = +Y direction
fn quat_from_azimuth_elevation(azimuth_rad: f64, inclination_rad: f64) -> DQuat {
    use std::f64::consts::PI;

    // Convert to zenith or rotate directly
    DQuat::from_rotation_z(azimuth_rad) * DQuat::from_rotation_y(PI / 2.0 - inclination_rad)
}
