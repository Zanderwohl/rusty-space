use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Context;
use crate::gui::planetarium::PlanetariumCamera;
use crate::gui::settings::{Settings, UiTheme};

pub fn camera_window(
    mut settings: ResMut<Settings>,
    mut contexts: EguiContexts,
    mut tonemapping: Single<&mut Tonemapping>,
    mut color_grading: Single<&mut ColorGrading>,
    mut camera: Single<&mut Projection, With<PlanetariumCamera>>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    if settings.windows.camera {
        camera_settings_window(ctx, camera, tonemapping, color_grading);
    }
}

fn camera_settings_window(ctx: &mut Context, mut camera: Single<&mut Projection, With<PlanetariumCamera>>, tonemapping: Single<&mut Tonemapping>, mut color_grading: Single<&mut ColorGrading>) {
    egui::Window::new("Camera Settings")
        .vscroll(true)
        .show(ctx, |ui| {
            ui.heading("Exposure");
            ui.add(egui::Slider::new(&mut color_grading.global.exposure, -20.0..=10.0).text("Exposure"));

            if let Projection::Perspective(perspective) = camera.as_mut() {
                ui.heading("Field of View");
                let mut fov_deg = perspective.fov.to_degrees();
                ui.add(egui::Slider::new(&mut fov_deg, 0.5..=190.0).text("FOV"));
                perspective.fov = fov_deg.to_radians();
            }
        });
}