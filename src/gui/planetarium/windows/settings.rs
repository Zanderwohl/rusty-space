use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use bevy_egui::{egui, EguiContexts};
use crate::camera::PlanetariumCamera;
use crate::gui::menu::escape::EscMenuContext;
use crate::gui::menu::settings::CameraControls;
use crate::gui::settings::{Settings, UiTheme};

pub fn settings_window(
    mut settings: ResMut<Settings>,
    mut esc_menu: ResMut<EscMenuContext>,
    mut contexts: EguiContexts,
    mut color_grading: Single<&mut ColorGrading>,
    mut camera: Single<&mut Projection, With<PlanetariumCamera>>,
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

    // Pull live camera values out into locals the panel can edit, then write
    // them back once the panel has run.
    let mut exposure = color_grading.global.exposure;
    let mut fov_deg = match camera.as_mut() {
        Projection::Perspective(p) => p.fov.to_degrees(),
        _ => 60.0,
    };

    // Start collapsed: https://github.com/emilk/egui/pull/5661
    let mut open = esc_menu.settings_window_visible;
    egui::Window::new("Settings")
        .open(&mut open)
        .vscroll(true)
        .show(ctx, |ui| {
            crate::gui::menu::settings::settings_panel(
                settings.as_mut(),
                Some(CameraControls {
                    exposure: &mut exposure,
                    fov_degrees: &mut fov_deg,
                }),
                ui,
            );
        });
    esc_menu.settings_window_visible = open;

    color_grading.global.exposure = exposure;
    if let Projection::Perspective(p) = camera.as_mut() {
        p.fov = fov_deg.to_radians();
    }
}
