//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

mod bodies;
mod lights;
mod labels;
mod trajectory;

pub use bodies::position_bodies;
pub use lights::adjust_lights;
pub use labels::label_bodies;
pub use trajectory::render_trajectories;
