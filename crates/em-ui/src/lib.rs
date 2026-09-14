//! Bevy-native menu chrome, shared by both products.
//!
//! Nothing here may know a game or TTRPG rule: it is a panel, some labels and some buttons in
//! a palette the caller chooses.

#![forbid(unsafe_code)]

pub mod theme;
pub mod widgets;

pub use theme::{MenuTheme, vfd};
pub use widgets::{MenuButton, MenuUi};

use bevy::prelude::*;

/// Registers the hover response. Every screen built with [`MenuUi`] needs it once.
pub struct MenuUiPlugin;

impl Plugin for MenuUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, widgets::button_hover_system);
    }
}
