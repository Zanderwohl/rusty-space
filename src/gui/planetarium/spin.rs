use bevy::prelude::ResMut;
use bevy_egui::egui;
use bevy_egui::egui::Context;
use num_traits::FloatConst;
use crate::gui::settings::Settings;

pub fn spin_gravity_calculator(mut settings: &mut ResMut<Settings>, ctx: &mut Context) {
    egui::Window::new("Spin Gravity Calculator")
        .vscroll(true)
        .show(ctx, |ui| {
            ui.add(egui::Slider::new(&mut settings.windows.spin_data.radius, 0.1..=250.0)
                .text("Radius")
                .step_by(0.1)
            );
            ui.add(egui::Slider::new(&mut settings.windows.spin_data.rpm, 0.0..=10.0)
                .text("RPM")
                .step_by(0.1)
            );
            let v = 2.0 * f64::PI() * settings.windows.spin_data.radius * settings.windows.spin_data.rpm / 60.0;
            let accel = v * v / settings.windows.spin_data.radius;
            ui.label(format!("Gravity: {:.2} m/s^2 ({:.2} g)", accel, accel / 9.81));
            ui.label(format!("Tangential Velocity: {:.2} m/s", v));

            ui.separator();
            ui.label("Coriolis Effect");
            ui.add(egui::Slider::new(&mut settings.windows.spin_data.vertical_velocity, -100.0..=100.0)
                .text("Vertical Velocity (positive is inward)")
                .step_by(0.1)
            );
            let omega = v / settings.windows.spin_data.radius;
            let coriolis = 2.0 * omega * settings.windows.spin_data.vertical_velocity;
            ui.label(format!("Coriolis Effect (positive is spinward): {:.2} m/s^2", coriolis));
        });
}