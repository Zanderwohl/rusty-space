use std::collections::HashMap;
use glam::{DVec3, DQuat};
use crate::reference_frame::{ReferenceFrame};
use crate::time::Instant;

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
    // Zenith is measured down from +Z; elevation is measured up from the XY plane.
    quat_from_azimuth_elevation(azimuth_rad, std::f64::consts::FRAC_PI_2 - zenith_rad)
}

/// Elevation: -π/2 = down, 0 = horizontal, π/2 = up
/// Azimuth: 0 = +X direction, π/2 = +Y direction
fn quat_from_azimuth_elevation(azimuth_rad: f64, elevation_rad: f64) -> DQuat {
    // A rotation of `t` about +Y maps +X to (cos t, 0, -sin t), so raising the forward
    // vector by `elevation` requires a rotation of `-elevation`.
    DQuat::from_rotation_z(azimuth_rad) * DQuat::from_rotation_y(-elevation_rad)
}
