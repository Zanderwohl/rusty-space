//! Simulation state and propagation for orbital mechanics.
//!
//! Built on [`em_foundations`]. Engine-agnostic: the optional `bevy` feature adds
//! `Component`/`Resource`/`Message` derives, but nothing here depends on an ECS
//! running, and the crate builds and propagates headless without it.

#![forbid(unsafe_code)]

pub mod appearance;
pub mod body;
pub mod motive;
pub mod presets;
pub mod propagate;
pub mod system;
pub mod events;
pub mod id;
pub mod time;
pub mod time_map;
pub mod universe;

pub use em_foundations as foundations;
