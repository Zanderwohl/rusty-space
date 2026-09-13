//! Reusable Bevy rendering for orbital scenes.
//!
//! Shared by both products, so nothing here may know a game or TTRPG rule: it draws bodies,
//! trajectories and reference frames from `em-sim` types and nothing else.

#![forbid(unsafe_code)]

pub mod render_space;

pub use render_space::{ToRender, render_to_sim, sim_to_render};
