use bevy::math::DVec3;
use serde::{Deserialize, Serialize};
use bevy::prelude::Component;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Component)]
pub struct BodyInfo {
    pub name: Option<String>,
    pub id: String,
    pub mass: f64,
    pub major: bool,
    pub designation: Option<String>,
    #[serde(skip, default = "Uuid::new_v4")]
    pub uuid: Uuid,
    #[serde(skip, default = "DVec3::default")]
    pub current_position: DVec3,
    #[serde(skip, default = "DVec3::default")]
    pub last_step_position: DVec3,
}

impl BodyInfo {
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone()
        }
        if let Some(name) = &self.designation {
            return name.clone()
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
            current_position: DVec3::default(),
            last_step_position: DVec3::default(),
        }
    }
}
