//! Camera module: freecam controller and planetarium camera behavior.
//!
//! This module handles 3D camera control, separate from the GUI (egui panels).

mod freecam;
mod planetarium;

pub use freecam::{Freecam, FreeCamPlugin, MovementSettings, KeyBindings};
pub use planetarium::{PlanetariumCamera, PlanetariumCameraPlugin, CameraAction, GoTo, RevolveAround};
