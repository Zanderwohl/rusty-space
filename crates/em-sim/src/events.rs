//! Simulation events and messages.
//!
//! `CalculateTrajectory` becomes a Bevy message under the `bevy` feature; without it
//! these are plain types a headless caller can use directly.

/// Message to request trajectory calculation for bodies.
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Message))]
pub struct CalculateTrajectory {
    pub selection: BodySelection,
}

/// Specifies which bodies to select for an operation.
pub enum BodySelection {
    /// All bodies in the simulation
    All,
    /// Bodies with a specific tag
    Tag(String),
    /// Bodies with specific IDs
    IDs(Vec<String>),
}
