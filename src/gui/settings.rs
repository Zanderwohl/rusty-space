use std::path::PathBuf;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use crate::gui::util::ensure_toml;

#[derive(Serialize, Deserialize, Debug, Resource)]
pub struct Settings {
    #[serde(default)]
    pub display: DisplaySettings,
    #[serde(default)]
    pub sound: SoundSettings,
    #[serde(default)]
    pub ui: UiSettings,
    #[serde(default)]
    pub windows: WindowSelections,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplaySettings::default(),
            sound: SoundSettings::default(),
            ui: UiSettings::default(),
            windows: WindowSelections::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct DisplaySettings {
    #[serde(default)]
    pub quality: DisplayQuality,
    #[serde(default)]
    pub glow: DisplayGlow,
    /// Minimum distance (in rendered units) for trajectory fade.
    /// Everything closer than this is fully transparent.
    #[serde(default = "default_trajectory_fade_min")]
    pub trajectory_fade_min: f32,
    /// Maximum distance (in rendered units) for trajectory fade.
    /// Everything further than this is fully opaque.
    /// Set both to 0.0 to disable fading.
    #[serde(default = "default_trajectory_fade_max")]
    pub trajectory_fade_max: f32,
}

fn default_trajectory_fade_min() -> f32 {
    0.0
}

fn default_trajectory_fade_max() -> f32 {
    1.5
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            quality: DisplayQuality::default(),
            glow: DisplayGlow::default(),
            trajectory_fade_min: default_trajectory_fade_min(),
            trajectory_fade_max: default_trajectory_fade_max(),
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
    #[serde(default = "default_mute")]
    pub mute: bool,
    #[serde(default = "default_volume")]
    pub volume: i32,
}

fn default_mute() -> bool {
    false
}

fn default_volume() -> i32 {
    50
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            mute: default_mute(),
            volume: default_volume(),
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

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct UiSettings {
    #[serde(default = "default_theme")]
    pub theme: UiTheme,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Default, PartialEq)]
pub enum UiTheme {
    #[default]
    Light,
    Dark,
}

fn default_theme() -> UiTheme {
    UiTheme::Dark
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct WindowSelections {
    #[serde(default = "default_false")]
    pub spin: bool,
    #[serde(skip)]
    pub spin_data: SpinData,
    #[serde(default = "default_false")]
    pub body_edit: bool,
    #[serde(default = "default_false")]
    pub body_info: bool,
    #[serde(default = "default_false")]
    pub grid: bool,
    #[serde(default = "default_false")]
    pub camera: bool,
}

impl Default for WindowSelections {
    fn default() -> Self {
        Self {
            spin: default_false(),
            spin_data: SpinData::default(),
            body_edit: default_false(),
            body_info: default_false(),
            grid: default_false(),
            camera: default_false(),
        }
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub struct SpinData {
    pub radius: f64,
    pub rpm: f64,
    pub vertical_velocity: f64,
}

fn default_false() -> bool {
    false
}
