//! The Lightcone client.
//!
//! Single process: no server, no network. The parts that decide *what* to draw are plain
//! functions with no engine in them, and the Bevy wiring sits on top — phase 5 found that a
//! renderer made of ECS systems cannot be reused, so this one keeps the decisions separable
//! from the drawing.

// Moved into `lc-world`, because the server is authoritative over ship motion and has to run
// the same code the client predicts with. Re-exported so the client's own paths still read the
// way they did.
pub use lc_world::{coast, flight, navigation, system};

pub mod action;
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod broker;
pub mod app;
pub mod curve;
pub mod entry;
pub mod envelope;
pub mod hud;
pub mod input;
pub mod menu;
pub mod panels;
pub mod pick;
pub mod plot;
pub mod resolved;
pub mod session;
#[cfg(not(target_arch = "wasm32"))]
pub mod signin_ui;
pub mod sky_asset;
pub mod starfield;
pub mod tonemap;
pub mod ui;
#[cfg(not(target_arch = "wasm32"))]
pub mod vault;
pub mod view;
