use glam::{DVec3, DQuat};
use crate::reference_frame::{ReferenceFrame};
use crate::time::Instant;

/// A direction seen from a reference frame at a given time.
pub struct Observation {
    frame: ReferenceFrame,
    time: Instant,
    direction: DQuat,
}

impl Observation {
    /// When the observation was made; bearings are frame- and time-dependent.
    pub fn time(&self) -> Instant { self.time }

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

/// Zenith: 0 = +Z, π/2 = horizontal, π = -Z. Azimuth: 0 = +X, π/2 = +Y.
fn quat_from_azimuth_zenith(azimuth_rad: f64, zenith_rad: f64) -> DQuat {
    quat_from_azimuth_elevation(azimuth_rad, std::f64::consts::FRAC_PI_2 - zenith_rad)
}

/// Elevation: -π/2 = down, 0 = horizontal, π/2 = up. Azimuth: 0 = +X, π/2 = +Y.
fn quat_from_azimuth_elevation(azimuth_rad: f64, elevation_rad: f64) -> DQuat {
    // A rotation of `t` about +Y maps +X to (cos t, 0, -sin t), so raising by
    // `elevation` needs a rotation of `-elevation`.
    DQuat::from_rotation_z(azimuth_rad) * DQuat::from_rotation_y(-elevation_rad)
}
