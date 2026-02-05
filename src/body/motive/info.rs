use bevy::math::DVec3;
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use uuid::Uuid;
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
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone()
        }
        if let Some(designation) = &self.designation {
            return designation.clone()
        }
        (&self.id).clone()
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
