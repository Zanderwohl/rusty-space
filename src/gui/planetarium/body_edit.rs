use bevy::prelude::ResMut;
use bevy_egui::egui;
use bevy_egui::egui::Context;
use crate::gui::settings::Settings;

pub fn body_edit_window(mut settings: &mut ResMut<Settings>, ctx: &mut Context) {
    egui::Window::new("Body Edit")
        .vscroll(true)
        .show(ctx, |ui| {
    });
}
