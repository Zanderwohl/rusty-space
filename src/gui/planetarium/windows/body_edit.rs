use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};

pub fn body_edit_window(
    mut settings: ResMut<Settings>, 
    mut ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    bodies: Query<(Entity, &mut BodyInfo, &BodyState, Option<&mut FixedMotive>, Option<&mut KeplerMotive>, Option<&mut NewtonMotive>)>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    
    if settings.windows.body_edit {
        egui::Window::new("Body Edit")
            .vscroll(true)
            .show(ctx, |ui| {
                let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                    .map(|(id, name)| (name.clone(), id.clone()))
                    .collect();
                body_options.sort_by(|a, b| a.0.cmp(&b.0));
                crate::gui::planetarium::windows::body_info::body_select_dropdown(universe, &mut body_info_state, ui, body_options);

                let selected_body = bodies.iter().filter(|(e, info, state, fixed_motive, kepler_motive, newton_motive)| {
                    if body_info_state.current_body_id.is_none() { return false; }
                    <std::string::String as AsRef<str>>::as_ref(&info.id) == body_info_state.current_body_id.as_ref().unwrap()
                }).collect::<Vec<_>>();

                let selected_body = selected_body.get(0);
                match selected_body {
                    None => { ui.label("No body Selected"); },
                    Some((entity, info, state, fixed_motive, kepler_motive, newton_motive)) => {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.label(info.display_name());
                        });
                    }
                }
            });
    }
}
