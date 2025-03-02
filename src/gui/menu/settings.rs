use bevy::prelude::ResMut;
use bevy_egui::egui;
use bevy_egui::egui::Ui;
use crate::gui::settings::{DisplayGlow, DisplayQuality, Settings};

pub fn settings_panel(mut settings: &mut ResMut<Settings>, ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.heading("Display");
        egui::ComboBox::from_label("Quality")
            .selected_text(format!("{:?}", settings.display.quality))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.display.quality, DisplayQuality::Low, "Low");
                ui.selectable_value(&mut settings.display.quality, DisplayQuality::Medium, "Medium");
                ui.selectable_value(&mut settings.display.quality, DisplayQuality::High, "High");
            });
        egui::ComboBox::from_label("Glow")
            .selected_text(format!("{:?}", settings.display.glow))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.display.glow, DisplayGlow::None, "None");
                ui.selectable_value(&mut settings.display.glow, DisplayGlow::Subtle, "Subtle");
                ui.selectable_value(&mut settings.display.glow, DisplayGlow::VFD, "VFD");
                ui.selectable_value(&mut settings.display.glow, DisplayGlow::Defcon, "DEFCON");
            });
    });

    ui.separator();
    ui.vertical(|ui| {
        ui.heading("Sound");

        ui.checkbox(&mut settings.sound.mute, "Mute");
        ui.add(egui::Slider::new(&mut settings.sound.volume, 0..=100).text("Volume"));
    });
}