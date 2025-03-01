use bevy::prelude::{in_state, App, IntoSystemConfigs, Plugin, ResMut, Resource, Update};
use bevy_egui::{egui, EguiContexts};
use crate::gui::app::AppState;
use crate::gui::settings::Settings;

#[derive(Default, Resource)]
pub struct UiState {

}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build (&self, app: &mut App) {
        app
            .init_resource::<UiState>()
            .add_systems(Update, main_menu.run_if(in_state(AppState::MainMenu)))
        ;
    }
}

pub fn ui_example_system(mut contexts: EguiContexts) {
    egui::Window::new("Hello").show(contexts.ctx_mut(), |ui| {
        ui.label("world");
    });
}

pub fn main_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,

) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Exotic Matters");
    });
}
