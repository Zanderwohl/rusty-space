use bevy::math::DVec3;
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use uuid::Uuid;
use crate::util::time_map::TimeMap;

#[derive(Serialize, Deserialize, Component, Clone)]
pub struct BodyInfo {
    pub name: Option<String>,
    pub id: String,
    pub mass: f64,
    pub major: bool,
    pub designation: Option<String>,
    #[serde(skip, default = "Uuid::new_v4")]
    pub uuid: Uuid,
    #[serde(default = "Vec::new")]
    pub tags: Vec<String>,
}

#[derive(Component)]
pub struct BodyState {
    pub current_position: DVec3,
    pub last_step_position: DVec3,
    pub current_local_position: Option<DVec3>,
    pub current_primary_position: Option<DVec3>,
    pub trajectory: Option<TimeMap<DVec3>>,
}

impl Default for BodyState {
    fn default() -> Self {
        Self {
            current_position: DVec3::ZERO,
            last_step_position: DVec3::ZERO,
            current_local_position: None,
            current_primary_position: None,
            trajectory: None,
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
            uuid: Uuid::from_u128(0u128),
            tags: vec![],
        }
    }
}
