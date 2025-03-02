use bevy::prelude::Component;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Component, Serialize, Deserialize)]
pub struct BodyInfo {
    pub name: String,
    pub id: String,
    #[serde(skip, default = "Uuid::new_v4")]
    pub uuid: Uuid,
}

/*
/// Basically, this manual implementation allows for a default uuid.
impl<'de> Deserialize<'de> for BodyInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>

    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct Repr {
            pub name: String,
            pub id: String,
        }

        let s = Repr::deserialize(deserializer)?;
        Ok(BodyInfo {
            name: s.name,
            id: s.id,
            uuid: Uuid::new_v4(),
        })
    }
}*/
