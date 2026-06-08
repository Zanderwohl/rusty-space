use bevy::prelude::*;
use bevy::math::DVec3;
use bevy_egui::egui::{self, Ui};

use crate::body::universe::Universe;

#[derive(Resource, Default)]
pub struct FocusedBodyState {
    pub current_body_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoveredTrajectoryMarkerKind {
    Periapsis,
    Apoapsis,
    MouseHit,
}

/// Data for a trajectory line segment hit from mouse hover raycasting.
#[derive(Clone)]
pub struct TrajectoryHitData {
    /// Index of the segment start point in the trajectory cache.
    pub segment_start_idx: usize,
    /// Relative time at segment start (within orbital period).
    pub start_time: f64,
    /// Relative time at segment end (within orbital period).
    pub end_time: f64,
    /// Interpolation parameter along the segment (0.0 = start, 1.0 = end).
    pub t: f64,
    /// Hit point position in simulation space (local to primary).
    pub hit_position_local: DVec3,
    /// Hit point position in bevy space.
    pub hit_position_bevy: Vec3,
}

#[derive(Resource, Default)]
pub struct HoverState {
    pub hovered_body_id: Option<String>,
    pub hovered_marker_kind: Option<HoveredTrajectoryMarkerKind>,
    /// Trajectory line hit data (only present when MouseHit marker should be shown).
    pub hovered_trajectory_hit: Option<TrajectoryHitData>,
}

impl FocusedBodyState {
    pub fn is_focused(&self, body_id: &str) -> bool {
        self.current_body_id.as_deref() == Some(body_id)
    }
}

impl HoverState {
    pub fn is_body_hovered(&self, body_id: &str) -> bool {
        self.hovered_body_id.as_deref() == Some(body_id)
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
