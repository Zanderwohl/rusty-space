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
    #[serde(default)]
    pub simulation: SimulationSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplaySettings::default(),
            sound: SoundSettings::default(),
            ui: UiSettings::default(),
            windows: WindowSelections::default(),
            simulation: SimulationSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct SimulationSettings {
    /// Enable Newtonian physics integration (gravity affecting free-flying bodies).
    /// When disabled, skips Newtonian sub-steps and only positions bodies once per frame.
    #[serde(default = "default_false")]
    pub newtonian: bool,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            newtonian: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct DisplaySettings {
    #[serde(default)]
    pub quality: DisplayQuality,
    #[serde(default)]
    pub glow: DisplayGlow,
    /// Brightness (percent, 0-100) at the front/leading end of a trajectory.
    /// The trajectory lerps from this to `trajectory_brightness_back` along its length.
    #[serde(default = "default_trajectory_brightness_front")]
    pub trajectory_brightness_front: f32,
    /// Brightness (percent, 0-100) at the back/trailing end of a trajectory.
    #[serde(default = "default_trajectory_brightness_back")]
    pub trajectory_brightness_back: f32,
    /// Brightness multiplier for background stars (0.1 to 10.0).
    #[serde(default = "default_star_brightness")]
    pub star_brightness: f32,
    /// Angular radius (arcminutes) of the faintest catalog stars.
    #[serde(default = "default_star_radius_min")]
    pub star_radius_min: f32,
    /// Angular radius (arcminutes) of the brightest catalog stars.
    /// A star's drawn radius lerps between min and max by its magnitude.
    #[serde(default = "default_star_radius_max")]
    pub star_radius_max: f32,
    /// Minimum brightness a sun-lit distant body can fade to (0.0 to 1.0).
    /// 0.0 lets bodies in full shadow disappear entirely.
    #[serde(default = "default_body_brightness_floor")]
    pub body_brightness_floor: f32,
    /// Minimum on-screen radius (pixels) for distant-body dots. The dot is drawn at
    /// the larger of this and the body's natural angular size, so far bodies stay
    /// visible while near ones grow to their real size.
    #[serde(default = "default_body_radius_min")]
    pub body_radius_min: f32,
    /// Maximum on-screen radius (pixels) for distant-body dots, capping how large a
    /// near body's dot can grow before it hands off to the wireframe.
    #[serde(default = "default_body_radius_max")]
    pub body_radius_max: f32,
    /// Screen-space radius (pixels) where the model->dot transition begins.
    /// At or above this size, the distant-body dot is fully hidden.
    #[serde(default = "default_model_fade_start_px")]
    pub model_fade_start_px: f32,
    /// Screen-space radius (pixels) where the model->dot transition ends.
    /// At or below this size, the distant-body dot is fully visible.
    #[serde(default = "default_model_fade_end_px")]
    pub model_fade_end_px: f32,
    /// Show the Point of Aries (♈) celestial reference marker.
    #[serde(default = "default_true")]
    pub show_point_of_aries: bool,
}

fn default_star_brightness() -> f32 {
    110.0
}

/// Old fixed star size: STAR_THRESHOLD 0.999995 ≈ acos ≈ 0.00316 rad ≈ 10.9 arcmin.
/// Defaulting both ends to this reproduces the previous uniform look (no size
/// variation); widen the wdwmax to make brighter stars larger.
fn default_star_radius_min() -> f32 {
    1.2
}

fn default_star_radius_max() -> f32 {
    7.5
}

fn default_body_brightness_floor() -> f32 {
    0.000
}

fn default_body_radius_min() -> f32 {
    // Dot radius (px) for a 1 m reference object; slider range is 0..1.
    1.0
}

fn default_body_radius_max() -> f32 {
    // High enough to not cap within the dot's visible range by default.
    3.5
}

fn default_model_fade_start_px() -> f32 {
    20.0
}

fn default_model_fade_end_px() -> f32 {
    10.0
}

fn default_trajectory_brightness_front() -> f32 {
    // Matches the previous hardcoded "None" glow look (0.1 -> 10%).
    10.0
}

fn default_trajectory_brightness_back() -> f32 {
    // Matches the previous hardcoded "None" glow look (1.0 -> 100%).
    100.0
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            quality: DisplayQuality::default(),
            glow: DisplayGlow::default(),
            trajectory_brightness_front: default_trajectory_brightness_front(),
            trajectory_brightness_back: default_trajectory_brightness_back(),
            star_brightness: default_star_brightness(),
            star_radius_min: default_star_radius_min(),
            star_radius_max: default_star_radius_max(),
            body_brightness_floor: default_body_brightness_floor(),
            body_radius_min: default_body_radius_min(),
            body_radius_max: default_body_radius_max(),
            model_fade_start_px: default_model_fade_start_px(),
            model_fade_end_px: default_model_fade_end_px(),
            show_point_of_aries: default_true(),
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

impl DisplayGlow {
    /// Whole-trajectory brightness multiplier applied on top of the front/back
    /// range. `None` is the 1.0 baseline (the front/back percentages render as-is);
    /// brighter presets push the trajectory overbright into HDR/bloom territory.
    pub fn brightness_multiplier(self) -> f32 {
        match self {
            DisplayGlow::None => 1.0,
            DisplayGlow::Subtle => 2.0,
            DisplayGlow::VFD => 6.0,
            DisplayGlow::Defcon => 16.0,
        }
    }
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
    #[serde(default = "default_true")]
    pub body_info: bool,
    #[serde(default = "default_false")]
    pub grid: bool,
    #[serde(default = "default_false")]
    pub controls: bool,
}

impl Default for WindowSelections {
    fn default() -> Self {
        Self {
            spin: default_false(),
            spin_data: SpinData::default(),
            body_edit: default_false(),
            body_info: default_false(),
            grid: default_false(),
            controls: default_false(),
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

fn default_true() -> bool {
    true
}
