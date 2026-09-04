//! Per-body identity, dynamic state, and rotation.

use glam::{DVec3, DQuat};
use serde::{Deserialize, Serialize};
use em_foundations::time::Instant;
use crate::time_map::TimeMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct BodyInfo {
    pub name: Option<String>,
    pub id: String,
    pub mass: f64,
    pub major: bool,
    pub designation: Option<String>,
    #[serde(default = "Vec::new")]
    pub tags: Vec<String>,
}

pub struct BodyState {
    pub current_position: DVec3,
    pub last_step_position: DVec3,
    /// Newtonian: integrated global velocity. Keplerian: relative to the primary.
    /// `None` for Fixed bodies.
    pub current_velocity: Option<DVec3>,
    pub current_local_position: Option<DVec3>,
    pub current_primary_position: Option<DVec3>,
    pub trajectory: Option<TimeMap<DVec3>>,
    /// When the Newtonian state was last (re)initialised; detects motive transitions.
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
    /// Name, else designation, else id.
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
    /// 2000-01-01 12:00 TT.
    J2000,
    /// Julian Day.
    JulianDay(f64),
}

impl RotationEpoch {
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
        /// Orientation at the reference epoch (Z-up sim frame); its local Z is the pole.
        orientation_at_epoch: DQuat,
        /// Radians per second; positive is prograde.
        angular_velocity: f64,
        epoch: RotationEpoch,
    },
    /// Tidally locked: one face always points at the primary.
    TidallyLocked {
        primary_id: String,
        /// Pole direction in sim coordinates (Z-up ecliptic); sets the axial tilt.
        pole: DVec3,
    },
}

/// How a body's orientation changes over time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BodyRotation {
    pub mode: RotationMode,
}

impl BodyRotation {
    pub fn spinning(orientation_at_epoch: DQuat, angular_velocity: f64, epoch: RotationEpoch) -> Self {
        Self {
            mode: RotationMode::Spinning {
                orientation_at_epoch,
                angular_velocity,
                epoch,
            },
        }
    }

    pub fn tidally_locked(primary_id: impl Into<String>, pole: DVec3) -> Self {
        Self {
            mode: RotationMode::TidallyLocked {
                primary_id: primary_id.into(),
                pole: pole.normalize(),
            },
        }
    }

    /// Orientation at `time`. `None` when tidally locked — use
    /// `orientation_tidally_locked`.
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

    /// Orientation of a tidally locked body; its local +X points at the primary.
    pub fn orientation_tidally_locked(&self, body_pos: DVec3, primary_pos: DVec3) -> Option<DQuat> {
        match &self.mode {
            RotationMode::TidallyLocked { pole, .. } => {
                let to_primary = (primary_pos - body_pos).normalize();
                
                // Right-handed basis: X at the primary, Z the orthogonalised pole,
                // Y = Z × X.
                let forward = to_primary;
                let up = (*pole - forward * forward.dot(*pole)).normalize();
                let right = up.cross(forward);
                
                let mat = glam::DMat3::from_cols(forward, right, up);
                Some(DQuat::from_mat3(&mat))
            }
            RotationMode::Spinning { .. } => None,
        }
    }

    /// North pole axis in simulation coordinates.
    pub fn pole_axis(&self) -> DVec3 {
        match &self.mode {
            RotationMode::Spinning { orientation_at_epoch, .. } => {
                *orientation_at_epoch * DVec3::Z
            }
            RotationMode::TidallyLocked { pole, .. } => *pole,
        }
    }

    /// Primary id, if tidally locked.
    pub fn tidally_locked_primary(&self) -> Option<&str> {
        match &self.mode {
            RotationMode::TidallyLocked { primary_id, .. } => Some(primary_id),
            RotationMode::Spinning { .. } => None,
        }
    }
}
