//! The Lightcone client.
//!
//! Single process: no server, no network. The parts that decide *what* to draw are plain
//! functions with no engine in them, and the Bevy wiring sits on top — phase 5 found that a
//! renderer made of ECS systems cannot be reused, so this one keeps the decisions separable
//! from the drawing.

pub mod action;
pub mod app;
pub mod coast;
pub mod curve;
pub mod entry;
pub mod envelope;
pub mod flight;
pub mod hud;
pub mod input;
pub mod menu;
pub mod navigation;
pub mod panels;
pub mod plot;
pub mod resolved;
pub mod session;
pub mod sky_asset;
pub mod starfield;
pub mod system;
pub mod tonemap;
pub mod ui;
pub mod view;
