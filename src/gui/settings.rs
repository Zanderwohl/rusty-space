use std::path::PathBuf;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use crate::gui::util::ensure_toml;

#[derive(Serialize, Deserialize, Debug, Resource)]
pub struct Settings {
    foo: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            foo: true,
        }
    }
}

pub fn load() -> Settings {
    ensure_toml::<Settings>(&PathBuf::from("data/settings.toml"))
        .unwrap_or_else(|message| {
            println!("Startup error: {}", message);
            std::process::exit(1);
        })
}
