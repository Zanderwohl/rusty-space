use bevy::prelude::{Res, ResMut};
use bevy_egui::{egui, EguiContexts};
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::gui::settings::{Settings, UiTheme};

pub fn settings_window(

    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut contexts: EguiContexts,
) {
    let ctx = contexts.ctx_mut();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    
    // Start collapsed: https://github.com/emilk/egui/pull/5661
    egui::Window::new("Settings")
        .vscroll(true)
        .show(ctx, |ui| {
            crate::gui::menu::settings::settings_panel(&mut settings, ui);
        });
}