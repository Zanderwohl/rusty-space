use bevy::prelude::*;
use bevy_egui::egui::{self, Ui};

use crate::body::universe::Universe;

#[derive(Resource, Default)]
pub struct FocusedBodyState {
    pub current_body_id: Option<String>,
}

impl FocusedBodyState {
    pub fn is_focused(&self, body_id: &str) -> bool {
        self.current_body_id.as_deref() == Some(body_id)
    }
}

pub fn sorted_body_options(universe: &Universe) -> Vec<(String, String)> {
    let mut body_options: Vec<(String, String)> = universe
        .id_to_name_iter()
        .map(|(id, name)| (name.clone(), id.clone()))
        .collect();
    body_options.sort_by(|a, b| a.0.cmp(&b.0));
    body_options
}

pub fn body_select_dropdown(
    universe: &Universe,
    focused_body_state: &mut FocusedBodyState,
    ui: &mut Ui,
    body_options: &[(String, String)],
) {
    egui::ComboBox::from_label("Body")
        .selected_text(
            focused_body_state
                .current_body_id
                .as_ref()
                .and_then(|id| universe.get_by_id(id))
                .cloned()
                .unwrap_or_else(|| "Choose a body".to_string()),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut focused_body_state.current_body_id,
                None,
                "Choose a body",
            );

            for (name, id) in body_options {
                ui.selectable_value(
                    &mut focused_body_state.current_body_id,
                    Some(id.clone()),
                    name,
                );
            }
        });
}
