//! Simulation state and propagation for orbital mechanics.
//!
//! Built on [`em_foundations`]. Engine-agnostic: the `bevy` feature only adds ECS derives;
//! everything here builds and propagates headless.

#![forbid(unsafe_code)]

pub mod appearance;
pub mod body;
pub mod motive;
pub mod patch;
pub mod presets;
pub mod propagate;
pub mod system;
pub mod events;
pub mod id;
pub mod influence;
pub mod time;
pub mod time_map;
pub mod trajectory;
pub mod universe;

pub use em_foundations as foundations;
