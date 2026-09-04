//! Presentation layer: Bevy systems that map simulation state to visual transforms and gizmos.
//!
//! This module handles the visual representation of simulation entities,
//! separate from the GUI (egui panels) and the simulation logic itself.

pub mod render_space;

mod lights;
mod labels;
mod rotation;
mod trajectory;

pub use lights::adjust_lights;
pub use labels::label_bodies;
pub use rotation::render_axes;
pub use trajectory::render_trajectories;
