use std::path::PathBuf;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use crate::gui::util::ensure_toml;

#[derive(Serialize, Deserialize, Debug, Resource)]
pub struct Settings {
    pub display: DisplaySettings,
    pub sound: SoundSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplaySettings::default(),
            sound: SoundSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct DisplaySettings {
    pub quality: DisplayQuality,
    pub glow: DisplayGlow,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            quality: DisplayQuality::default(),
            glow: DisplayGlow::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Default, PartialEq)]
pub enum DisplayQuality {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Default, PartialEq)]
pub enum DisplayGlow {
    None,
    #[default]
    Subtle,
    VFD,
    Defcon,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct SoundSettings {
    pub mute: bool,
    pub volume: i32,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            mute: false,
            volume: 50,
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
