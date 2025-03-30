use bevy::prelude::{Res, ResMut};
use bevy_egui::{egui, EguiContexts};
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::gui::settings::{Settings, UiTheme};

pub fn body_edit_window(
    mut settings: ResMut<Settings>, 
    mut ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
) {
    let ctx = contexts.ctx_mut();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    
    if settings.windows.body_edit {
        egui::Window::new("Body Edit")
            .vscroll(true)
            .show(ctx, |ui| {});
    }
}
