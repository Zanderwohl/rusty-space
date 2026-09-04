//! Simulation events and messages.

/// Request trajectory calculation for bodies.
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Message))]
pub struct CalculateTrajectory {
    pub selection: BodySelection,
}

/// Which bodies an operation applies to.
pub enum BodySelection {
    All,
    Tag(String),
    IDs(Vec<String>),
}
