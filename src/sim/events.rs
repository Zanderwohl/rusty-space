//! Simulation events and messages.
//!
//! These are Bevy messages used to communicate between simulation systems.

use bevy::prelude::*;

/// Message to request trajectory calculation for bodies.
#[derive(Message)]
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
