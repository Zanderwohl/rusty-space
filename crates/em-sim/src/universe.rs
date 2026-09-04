//! The on-disk universe model: settings, physics, and the body entries a save holds.
//!
//! Pure data. Reading and writing files, and turning entries into ECS entities, is the
//! app's job — see `body/universe/save.rs` there.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use em_foundations::mappings;

use crate::appearance::Appearance;
use crate::body::{BodyInfo, BodyRotation};
use crate::motive::kepler::KeplerMotive;
use crate::motive::Motive;
use glam::DVec3;

/// State for a tag (group of bodies).
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Resource))]
#[derive(Serialize, Deserialize, Debug)]
pub struct TagState {
    pub shown: bool,
    pub trajectory: bool,
    pub members: Vec<String>,
}

impl Default for TagState {
    fn default() -> Self {
        Self {
            shown: false,
            trajectory: false,
            members: Vec::new(),
        }
    }
}

/// Supported save file formats
#[derive(Serialize, Deserialize)]
pub struct UniverseFileContents {
    pub version: String,
    pub time: UniverseFileTime,
    pub view: ViewSettings,
    pub physics: UniversePhysics,
    pub bodies: Vec<SomeBody>,
}

#[derive(Serialize, Deserialize)]
pub struct UniverseFileTime {
    pub time_julian_days: f64, // In Julian Days
    /// Physics time step in simulation seconds (default: 0.1)
    #[serde(default = "default_step")]
    pub step: f64,
    /// GUI speed multiplier - sim seconds per real second (default: 1.0)
    #[serde(default = "default_gui_speed")]
    pub gui_speed: f64,
    /// Maximum real-world time to spend on physics per frame (default: 0.016)
    #[serde(default = "default_max_frame_time")]
    pub max_frame_time: f64,
}

fn default_step() -> f64 { 0.1 }
fn default_gui_speed() -> f64 { 1.0 }
fn default_max_frame_time() -> f64 { 0.016 }
fn default_show_axes() -> bool { true }

#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Resource))]
#[derive(Serialize, Deserialize)]
pub struct UniversePhysics {
    pub gravitational_constant: f64,
}

impl Default for UniversePhysics {
    fn default() -> Self {
        Self {
            gravitational_constant: 6.6743015e-11, // Standard G in m³ kg⁻¹ s⁻²
        }
    }
}

#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Resource))]
#[derive(Serialize, Deserialize, Debug)]
pub struct ViewSettings {
    pub distance_scale: f64,
    pub logarithmic_distance_scale: bool,
    pub logarithmic_distance_base: f64,
    pub body_scale: f64,
    pub logarithmic_body_scale: bool,
    pub logarithmic_body_base: f64,
    pub show_labels: bool,
    pub show_trajectories: bool,
    #[serde(default = "default_show_axes")]
    pub show_axes: bool,
    pub tags: HashMap<String, TagState>,
    pub trajectory_resolution: usize,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            distance_scale: 1e-9,
            logarithmic_body_scale: false,
            logarithmic_body_base: 10.0,
            body_scale: 1e-9,
            logarithmic_distance_scale: false,
            logarithmic_distance_base: 10.0,
            show_labels: true,
            show_trajectories: true,
            show_axes: true,
            tags: HashMap::new(),
            trajectory_resolution: 120,
        }
    }
}

impl ViewSettings {
    pub fn body_in_any_visible_tag<T:AsRef<str> + ToString>(&self, body_id: T) -> bool {
        for tag in self.tags.values() {
            if tag.shown && tag.members.contains(&body_id.to_string()) {
                return true;
            }
        }
        false
    }

    pub fn body_in_any_trajectory_tag<T:AsRef<str> + ToString>(&self, body_id: T) -> bool {
        for tag in self.tags.values() {
            if tag.trajectory && tag.members.contains(&body_id.to_string()) {
                return true;
            }
        }
        false
    }
    
    pub fn distance_factor(&self) -> f64 {
        if self.logarithmic_distance_scale {
            mappings::log_scale(self.distance_scale, self.logarithmic_distance_base)
        } else {
            self.distance_scale
        }
    }
    
    pub fn body_scale_factor(&self, radius: f64) -> f32 {
        let n = if self.logarithmic_body_scale {
            mappings::log_scale(radius, self.logarithmic_body_base) * self.body_scale
        } else {
            radius * self.body_scale
        } as f32;
        n
    }
}

#[derive(Serialize, Deserialize)]
pub enum SomeBody {
    /// Legacy fixed motive - loaded as Motive with single Fixed entry
    FixedEntry(FixedEntry),
    /// Legacy newton motive - loaded as Motive with single Newtonian entry
    NewtonEntry(NewtonEntry),
    /// Legacy kepler motive - loaded as Motive with single Keplerian entry
    KeplerEntry(KeplerEntry),
    /// Legacy patched conics - deprecated, kept for backward compatibility
    CompoundEntry(PatchedConicsEntry),
    /// New compound motive format - supports multiple motive types with transitions
    CompoundMotiveEntry(CompoundMotiveEntry),
}

impl SomeBody {
    pub fn id(&self) -> String {
        match self {
            SomeBody::FixedEntry(entry) => (&entry.info.id).clone(),
            SomeBody::NewtonEntry(entry) => (&entry.info.id).clone(),
            SomeBody::KeplerEntry(entry) => (&entry.info.id).clone(),
            SomeBody::CompoundEntry(entry) => (&entry.info.id).clone(),
            SomeBody::CompoundMotiveEntry(entry) => (&entry.info.id).clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            SomeBody::FixedEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::NewtonEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::KeplerEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::CompoundEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
            SomeBody::CompoundMotiveEntry(entry) => (&entry.info.name).clone().unwrap_or(format!("body_{}", self.id())).clone(),
        }
    }

    pub fn tags(&self) -> &Vec<String> {
        match self {
            SomeBody::FixedEntry(entry) => &entry.info.tags,
            SomeBody::NewtonEntry(entry) => &entry.info.tags,
            SomeBody::KeplerEntry(entry) => &entry.info.tags,
            SomeBody::CompoundEntry(entry) => &entry.info.tags,
            SomeBody::CompoundMotiveEntry(entry) => &entry.info.tags,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct FixedEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BodyRotation>,
}

#[derive(Serialize, Deserialize)]
pub struct NewtonEntry {
    pub info: BodyInfo,
    pub position: DVec3,
    pub velocity: DVec3,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BodyRotation>,
}

#[derive(Serialize, Deserialize)]
pub struct KeplerEntry {
    pub info: BodyInfo,
    pub params: KeplerMotive,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BodyRotation>,
}

/// Legacy format - use CompoundMotiveEntry for new saves
#[derive(Serialize, Deserialize)]
pub struct PatchedConicsEntry {
    pub info: BodyInfo,
    pub route: HashMap<u64, KeplerMotive>,
    pub appearance: Appearance,
}

/// The new compound motive format that supports motive transitions over time
#[derive(Serialize, Deserialize)]
pub struct CompoundMotiveEntry {
    pub info: BodyInfo,
    pub motive: Motive,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<BodyRotation>,
}

impl FixedEntry {
    /// A legacy fixed entry has no primary field; it is positioned absolutely.
    pub fn info_primary(&self) -> Option<String> { None }
}
