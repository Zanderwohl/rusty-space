//! The Lightcone client.
//!
//! Single process: no server, no network. The parts that decide *what* to draw are plain
//! functions with no engine in them, and the Bevy wiring sits on top — phase 5 found that a
//! renderer made of ECS systems cannot be reused, so this one keeps the decisions separable
//! from the drawing.

pub mod curve;
pub mod session;
pub mod tonemap;
pub mod view;
