use bevy_egui::egui;
use bevy_egui::egui::{RichText, Ui};
use crate::gui::settings::{DisplayGlow, DisplayQuality, Settings, UiTheme};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SettingsTab {
    #[default]
    Display,
    Sound,
    Gui,
    Simulation,
}

/// Live camera controls, only available inside the planetarium where a camera
/// entity exists. `fov_degrees` is edited in degrees; the caller converts to/from
/// radians and writes the values back after the panel returns.
pub struct CameraControls<'a> {
    pub exposure: &'a mut f32,
    pub fov_degrees: &'a mut f32,
}

pub fn settings_panel(settings: &mut Settings, camera: Option<CameraControls>, ui: &mut Ui) {
    let tab_id = ui.make_persistent_id("settings_tab");
    let mut tab = ui.data_mut(|d| d.get_temp::<SettingsTab>(tab_id).unwrap_or_default());

    ui.horizontal(|ui| {
        ui.selectable_value(&mut tab, SettingsTab::Display, "Display");
        ui.selectable_value(&mut tab, SettingsTab::Sound, "Sound");
        ui.selectable_value(&mut tab, SettingsTab::Gui, "GUI");
        ui.selectable_value(&mut tab, SettingsTab::Simulation, "Simulation");
    });
    ui.separator();

    match tab {
        SettingsTab::Display => display_tab(settings, camera, ui),
        SettingsTab::Sound => sound_tab(settings, ui),
        SettingsTab::Gui => gui_tab(settings, ui),
        SettingsTab::Simulation => simulation_tab(settings, ui),
    }

    ui.data_mut(|d| d.insert_temp(tab_id, tab));
}

fn display_tab(settings: &mut Settings, camera: Option<CameraControls>, ui: &mut Ui) {
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

    ui.add_space(8.0);
    ui.label(RichText::new("Trajectory").strong());
    ui.add(egui::Slider::new(&mut settings.display.trajectory_brightness_front, 0.0..=100.0)
        .suffix("%")
        .text("Brightness Front"));
    ui.add(egui::Slider::new(&mut settings.display.trajectory_brightness_back, 0.0..=100.0)
        .suffix("%")
        .text("Brightness Back"));

    ui.add_space(8.0);
    ui.label(RichText::new("Camera").strong());
    match camera {
        Some(cam) => {
            ui.add(egui::Slider::new(cam.exposure, -20.0..=10.0).text("Exposure"));
            ui.add(egui::Slider::new(cam.fov_degrees, 0.5..=190.0).text("FOV"));
        }
        None => {
            // No live camera outside the planetarium; show disabled placeholders.
            let mut exposure = 0.0_f32;
            let mut fov = 60.0_f32;
            ui.add_enabled(false, egui::Slider::new(&mut exposure, -20.0..=10.0).text("Exposure"))
                .on_disabled_hover_text("Available in the planetarium");
            ui.add_enabled(false, egui::Slider::new(&mut fov, 0.5..=190.0).text("FOV"))
                .on_disabled_hover_text("Available in the planetarium");
        }
    }

    ui.add_space(8.0);
    ui.label(RichText::new("Model Fade").strong());
    // Keep start >= end while dragging either slider.
    let fade_end_px = settings.display.model_fade_end_px;
    let fade_start_resp = ui.add(egui::Slider::new(&mut settings.display.model_fade_start_px, 0.0..=5.0)
        .suffix("px")
        .text("Fade Start"))
        .on_hover_text("At or above this screen radius, the distant-body dot is hidden.");
    if fade_start_resp.changed() {
        settings.display.model_fade_start_px = settings.display.model_fade_start_px.max(fade_end_px);
    }
    let fade_start_px = settings.display.model_fade_start_px;
    let fade_end_resp = ui.add(egui::Slider::new(&mut settings.display.model_fade_end_px, 0.0..=5.0)
        .suffix("px")
        .text("Fade End"))
        .on_hover_text("At or below this screen radius, the distant-body dot is fully visible.");
    if fade_end_resp.changed() {
        settings.display.model_fade_end_px = settings.display.model_fade_end_px.min(fade_start_px);
    }

    ui.add_space(8.0);
    ui.label(RichText::new("Distant Objects").strong());
    let local_star_max = settings.display.local_star_brightness_max;
    let local_star_min_resp = ui.add(egui::Slider::new(&mut settings.display.local_star_brightness_min, 0.1..=500.0)
        .logarithmic(true)
        .text("Local Star Brightness Min"));
    if local_star_min_resp.changed() {
        settings.display.local_star_brightness_min = settings.display.local_star_brightness_min.min(local_star_max);
    }
    let local_star_min = settings.display.local_star_brightness_min;
    let local_star_max_resp = ui.add(egui::Slider::new(&mut settings.display.local_star_brightness_max, 0.1..=500.0)
        .logarithmic(true)
        .text("Local Star Brightness Max"));
    if local_star_max_resp.changed() {
        settings.display.local_star_brightness_max = settings.display.local_star_brightness_max.max(local_star_min);
    }
    ui.add(egui::Slider::new(&mut settings.display.star_brightness, 0.1..=500.0)
        .logarithmic(true)
        .text("Star Brightness"));

    // Star radius range (arcminutes). Keep min <= max while dragging either slider:
    // clamp the one that just changed against the other.
    let radius_max = settings.display.star_radius_max;
    let min_resp = ui.add(egui::Slider::new(&mut settings.display.star_radius_min, 0.1..=10.0)
        .suffix("′")
        .text("Star Radius Min"));
    if min_resp.changed() {
        settings.display.star_radius_min = settings.display.star_radius_min.min(radius_max);
    }
    let radius_min = settings.display.star_radius_min;
    let max_resp = ui.add(egui::Slider::new(&mut settings.display.star_radius_max, 0.5..=10.0)
        .suffix("′")
        .text("Star Radius Max"));
    if max_resp.changed() {
        settings.display.star_radius_max = settings.display.star_radius_max.max(radius_min);
    }

    ui.add(egui::Slider::new(&mut settings.display.body_brightness_floor, 0.0..=0.1)
        .text("Body Brightness Floor"))
        .on_hover_text("Minimum brightness a sun-lit distant body can fade to.\n0 lets bodies in full shadow disappear.");
    // Body dot radius range (screen px). Keep min <= max while dragging either slider.
    let body_radius_max = settings.display.body_radius_max;
    let body_min_resp = ui.add(egui::Slider::new(&mut settings.display.body_radius_min, 0.0..=5.0)
        .suffix("px")
        .text("Body Radius Min"))
        .on_hover_text("On-screen dot size for a 1 m object.\nLarger bodies scale up by area; smaller ones shrink below this and can vanish.");
    if body_min_resp.changed() {
        settings.display.body_radius_min = settings.display.body_radius_min.min(body_radius_max);
    } 
    let body_radius_min = settings.display.body_radius_min;
    let body_max_resp = ui.add(egui::Slider::new(&mut settings.display.body_radius_max, 0.0..=20.0)
        .suffix("px")
        .text("Body Radius Max"))
        .on_hover_text("On-screen dot size for a Jupiter-sized object.\nSmaller bodies scale down by area toward the min.");
    if body_max_resp.changed() {
        settings.display.body_radius_max = settings.display.body_radius_max.max(body_radius_min);
    }

    ui.add_space(8.0);
    ui.label(RichText::new("Celestial Markers").strong());
    ui.checkbox(&mut settings.display.show_point_of_aries, "Point of Aries");
}

fn sound_tab(settings: &mut Settings, ui: &mut Ui) {
    ui.checkbox(&mut settings.sound.mute, "Mute");
    ui.add(egui::Slider::new(&mut settings.sound.volume, 0..=100).text("Volume"));
}

fn gui_tab(settings: &mut Settings, ui: &mut Ui) {
    egui::ComboBox::from_label("Theme")
        .selected_text(format!("{:?}", settings.ui.theme))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut settings.ui.theme, UiTheme::Light, "Light");
            ui.selectable_value(&mut settings.ui.theme, UiTheme::Dark, "Dark");
        });

    ui.add_space(8.0);
    ui.label(RichText::new("Windows").strong());
    ui.checkbox(&mut settings.windows.controls, "Controls");
    ui.checkbox(&mut settings.windows.spin, "Spin Gravity Calculator");
    ui.checkbox(&mut settings.windows.body_edit, "Body Edit");
    ui.checkbox(&mut settings.windows.body_info, "Body Info");
}

fn simulation_tab(settings: &mut Settings, ui: &mut Ui) {
    ui.checkbox(&mut settings.simulation.newtonian, "Newtonian Physics")
        .on_hover_text("Enable gravity integration for free-flying bodies.\nDisabling skips sub-steps and positions bodies once per frame.");
}
