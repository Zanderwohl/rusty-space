use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use crate::gui::menu::escape::EscMenuContext;
use crate::gui::settings::{Settings, UiTheme};

pub fn settings_window(

    mut settings: ResMut<Settings>,
    mut esc_menu: ResMut<EscMenuContext>,
    mut contexts: EguiContexts,
) {
    if !esc_menu.settings_window_visible {
        return;
    }

    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    
    // Start collapsed: https://github.com/emilk/egui/pull/5661
    let mut open = esc_menu.settings_window_visible;
    egui::Window::new("Settings")
        .open(&mut open)
        .vscroll(true)
        .show(ctx, |ui| {
            crate::gui::menu::settings::settings_panel(&mut settings, ui);
        });
    esc_menu.settings_window_visible = open;
}