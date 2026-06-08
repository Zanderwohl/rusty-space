use bevy::math::{DVec3, DQuat};
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use crate::foundations::time::Instant;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Component, Clone)]
pub struct BodyInfo {
    pub name: Option<String>,
    pub id: String,
    pub mass: f64,
    pub major: bool,
    pub designation: Option<String>,
    #[serde(default = "Vec::new")]
    pub tags: Vec<String>,
}

#[derive(Component)]
pub struct BodyState {
    pub current_position: DVec3,
    pub last_step_position: DVec3,
    /// Current velocity for Newtonian bodies (None for Fixed/Keplerian)
    pub current_velocity: Option<DVec3>,
    pub current_local_position: Option<DVec3>,
    pub current_primary_position: Option<DVec3>,
    pub trajectory: Option<TimeMap<DVec3>>,
    /// Time at which the current Newtonian state was last initialized/updated
    /// Used to detect motive transitions that require reinitialization
    pub newtonian_init_time: Option<Instant>,
}

impl Default for BodyState {
    fn default() -> Self {
        Self {
            current_position: DVec3::ZERO,
            last_step_position: DVec3::ZERO,
            current_velocity: None,
            current_local_position: None,
            current_primary_position: None,
            trajectory: None,
            newtonian_init_time: None,
        }
    }
}

impl BodyInfo {
    /// Returns the display name without allocation: name if set, else designation, else id.
    pub fn display_name(&self) -> &str {
        self.name.as_deref()
            .or(self.designation.as_deref())
            .unwrap_or(&self.id)
    }
}

impl Default for BodyInfo {
    fn default() -> Self {
        Self {
            name: None,
            id: "[DO NOT USE DEFAULT ID]".into(),
            mass: 0.0,
            major: false,
            designation: None,
            tags: vec![],
        }
    }
}

/// Epoch reference for body rotation data.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum RotationEpoch {
    /// J2000 epoch (2000-01-01 12:00 TT)
    J2000,
    /// Custom epoch specified as Julian Day
    JulianDay(f64),
}

impl RotationEpoch {
    /// Convert this epoch to an Instant.
    pub fn to_instant(&self) -> Instant {
        match self {
            RotationEpoch::J2000 => Instant::from_seconds_since_j2000(0.0),
            RotationEpoch::JulianDay(jd) => Instant::from_julian_day(*jd),
        }
    }
}

/// Rotation mode for a body.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum RotationMode {
    /// Body spins around a fixed pole axis at constant angular velocity.
    Spinning {
        /// Orientation quaternion at the reference epoch (Z-up sim frame).
        /// The local Z-axis of this quaternion is the rotation pole.
        orientation_at_epoch: DQuat,
        /// Angular velocity in radians per second (positive = prograde).
        angular_velocity: f64,
        /// Reference epoch for the orientation.
        epoch: RotationEpoch,
    },
    /// Body is tidally locked to its primary - one face always points toward it.
    TidallyLocked {
        /// ID of the primary body this is locked to.
        primary_id: String,
        /// Pole axis direction in simulation coordinates (Z-up ecliptic frame).
        /// This defines the body's axial tilt - the body rotates so its +X faces
        /// the primary while keeping this pole orientation.
        pole: DVec3,
    },
}

/// Body rotation state component.
///
/// Defines how a body's orientation changes over time, either through
/// constant spin around a pole axis or tidal locking to a primary body.
#[derive(Serialize, Deserialize, Component, Clone, Debug)]
pub struct BodyRotation {
    pub mode: RotationMode,
}

impl BodyRotation {
    /// Create a spinning rotation from IAU-style parameters.
    pub fn spinning(orientation_at_epoch: DQuat, angular_velocity: f64, epoch: RotationEpoch) -> Self {
        Self {
            mode: RotationMode::Spinning {
                orientation_at_epoch,
                angular_velocity,
                epoch,
            },
        }
    }

    /// Create a tidally locked rotation.
    pub fn tidally_locked(primary_id: impl Into<String>, pole: DVec3) -> Self {
        Self {
            mode: RotationMode::TidallyLocked {
                primary_id: primary_id.into(),
                pole: pole.normalize(),
            },
        }
    }

    /// Compute the current orientation at the given simulation time.
    /// For tidally locked bodies, returns None - use `orientation_tidally_locked` instead.
    pub fn orientation_at(&self, time: Instant) -> Option<DQuat> {
        match &self.mode {
            RotationMode::Spinning { orientation_at_epoch, angular_velocity, epoch } => {
                let epoch_instant = epoch.to_instant();
                let dt = time.to_j2000_seconds() - epoch_instant.to_j2000_seconds();
                let pole_axis = *orientation_at_epoch * DVec3::Z;
                let spin = DQuat::from_axis_angle(pole_axis, *angular_velocity * dt);
                Some(spin * *orientation_at_epoch)
            }
            RotationMode::TidallyLocked { .. } => None,
        }
    }

    /// Compute orientation for a tidally locked body given its position and primary's position.
    /// The body's local +X axis will point toward the primary.
    pub fn orientation_tidally_locked(&self, body_pos: DVec3, primary_pos: DVec3) -> Option<DQuat> {
        match &self.mode {
            RotationMode::TidallyLocked { pole, .. } => {
                let to_primary = (primary_pos - body_pos).normalize();
                
                // Build orthonormal basis (right-handed):
                // +X points toward primary
                // +Z is the pole axis (orthogonalized to be perpendicular to +X)
                // +Y completes the right-handed system: +Y = +Z × +X
                let forward = to_primary;
                let up = (*pole - forward * forward.dot(*pole)).normalize(); // orthogonalize pole
                let right = up.cross(forward); // right-handed: Y = Z × X
                
                // Construct rotation matrix: columns are where each basis axis points
                let mat = bevy::math::DMat3::from_cols(forward, right, up);
                Some(DQuat::from_mat3(&mat))
            }
            RotationMode::Spinning { .. } => None,
        }
    }

    /// Get the rotation pole axis (north) in simulation coordinates.
    pub fn pole_axis(&self) -> DVec3 {
        match &self.mode {
            RotationMode::Spinning { orientation_at_epoch, .. } => {
                *orientation_at_epoch * DVec3::Z
            }
            RotationMode::TidallyLocked { pole, .. } => *pole,
        }
    }

    /// Get the primary ID if this is a tidally locked body.
    pub fn tidally_locked_primary(&self) -> Option<&str> {
        match &self.mode {
            RotationMode::TidallyLocked { primary_id, .. } => Some(primary_id),
            RotationMode::Spinning { .. } => None,
        }
    }
}
