use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::body::universe::save::ViewSettings;
use crate::gui::style::vfd;

const BUTTON_SIZE: f32 = 24.0;
const BUTTON_SPACING: f32 = 4.0;
const DOT_RADIUS: f32 = 4.0;
const DOT_OFFSET: f32 = 6.0;
const PANEL_MARGIN: f32 = 12.0;
const HOVER_LABEL_GAP: f32 = 8.0;

const TAG_ORDER: &[&str] = &[
    "Star",
    "Planet",
    "Major Planet",
    "Minor Planet",
    "Moon",
    "Major Moon",
    "Minor Moon",
    "Barycenter",
];

pub fn show_hide_panel_widget(
    mut contexts: EguiContexts,
    mut view_settings: ResMut<ViewSettings>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() {
        return;
    }
    let ctx = ctx.unwrap();

    let panel_bg = to_egui_color(vfd::PANEL_BG);
    let border = to_egui_color(vfd::BUTTON_BORDER);
    let text_color = to_egui_color(vfd::TEXT);
    let text_dim = to_egui_color(vfd::TEXT_DIM);
    let button_bg = to_egui_color(vfd::BUTTON_BG);
    let button_hover = to_egui_color(vfd::BUTTON_HOVER);

    let ordered_tags = build_ordered_tags(&view_settings);
    let button_count = 2 + ordered_tags.len();
    let panel_height = button_count as f32 * (BUTTON_SIZE + BUTTON_SPACING) - BUTTON_SPACING;

    let mut hovered_label: Option<(egui::Pos2, String)> = None;
    let mut selected_clicked = false;
    let mut all_clicked = false;
    let mut tag_clicks: Vec<String> = Vec::new();

    egui::Area::new("show_hide_panel".into())
        .anchor(egui::Align2::LEFT_CENTER, egui::vec2(PANEL_MARGIN, 0.0))
        .show(ctx, |ui| {
            ui.set_min_height(panel_height);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = BUTTON_SPACING;

                let selected_response = draw_show_hide_button(
                    ui,
                    view_settings.show_selected_labels,
                    view_settings.show_selected_trajectories,
                    text_color,
                    text_dim,
                    button_bg,
                    button_hover,
                    border,
                );

                if selected_response.clicked() {
                    selected_clicked = true;
                }

                if selected_response.hovered() {
                    let pos = egui::pos2(
                        selected_response.rect.right() + HOVER_LABEL_GAP,
                        selected_response.rect.center().y,
                    );
                    hovered_label = Some((pos, "Selected".to_string()));
                }

                let all_response = draw_show_hide_button(
                    ui,
                    view_settings.show_labels,
                    view_settings.show_trajectories,
                    text_color,
                    text_dim,
                    button_bg,
                    button_hover,
                    border,
                );

                if all_response.clicked() {
                    all_clicked = true;
                }

                if all_response.hovered() {
                    let pos = egui::pos2(
                        all_response.rect.right() + HOVER_LABEL_GAP,
                        all_response.rect.center().y,
                    );
                    hovered_label = Some((pos, "All".to_string()));
                }

                for tag_name in &ordered_tags {
                    if let Some(tag_state) = view_settings.tags.get(&**tag_name) {
                        let response = draw_show_hide_button(
                            ui,
                            tag_state.shown,
                            tag_state.trajectory,
                            text_color,
                            text_dim,
                            button_bg,
                            button_hover,
                            border,
                        );

                        if response.clicked() {
                            tag_clicks.push(tag_name.clone());
                        }

                        if response.hovered() {
                            let pos = egui::pos2(
                                response.rect.right() + HOVER_LABEL_GAP,
                                response.rect.center().y,
                            );
                            hovered_label = Some((pos, tag_name.clone()));
                        }
                    }
                }
            });
        });

    if selected_clicked {
        let (new_labels, new_traj) = cycle_state(
            view_settings.show_selected_labels,
            view_settings.show_selected_trajectories,
        );
        view_settings.show_selected_labels = new_labels;
        view_settings.show_selected_trajectories = new_traj;
    }

    if all_clicked {
        let (new_labels, new_traj) = cycle_state(
            view_settings.show_labels,
            view_settings.show_trajectories,
        );
        view_settings.show_labels = new_labels;
        view_settings.show_trajectories = new_traj;
    }

    for tag_name in tag_clicks {
        if let Some(tag_state) = view_settings.tags.get_mut(&tag_name) {
            let (new_shown, new_traj) = cycle_state(tag_state.shown, tag_state.trajectory);
            tag_state.shown = new_shown;
            tag_state.trajectory = new_traj;
        }
    }

    if let Some((pos, label)) = hovered_label {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("show_hide_hover_label"),
        ));
        let font = egui::FontId::proportional(12.0);
        let galley = painter.layout_no_wrap(label, font.clone(), text_color);
        let text_size = galley.size();
        let padding = egui::vec2(6.0, 2.0);
        let bg_rect = egui::Rect::from_min_size(
            egui::pos2(pos.x, pos.y - text_size.y / 2.0 - padding.y),
            text_size + padding * 2.0,
        );
        painter.rect_filled(bg_rect, 0.0, panel_bg);
        painter.galley(
            egui::pos2(bg_rect.left() + padding.x, bg_rect.top() + padding.y),
            galley,
            text_color,
        );
    }
}

fn draw_show_hide_button(
    ui: &mut egui::Ui,
    labels_on: bool,
    trajectory_on: bool,
    active_color: egui::Color32,
    inactive_color: egui::Color32,
    button_bg: egui::Color32,
    button_hover: egui::Color32,
    border_color: egui::Color32,
) -> egui::Response {
    let desired_size = egui::vec2(BUTTON_SIZE, BUTTON_SIZE);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        let fill = if response.hovered() {
            button_hover
        } else {
            button_bg
        };
        painter.rect_filled(rect, 2.0, fill);
        painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, border_color),
            egui::StrokeKind::Middle,
        );

        let top_left_dot = egui::pos2(rect.left() + DOT_OFFSET, rect.top() + DOT_OFFSET);
        let bottom_right_dot =
            egui::pos2(rect.right() - DOT_OFFSET, rect.bottom() - DOT_OFFSET);

        let label_color = if labels_on { active_color } else { inactive_color };
        painter.circle_filled(top_left_dot, DOT_RADIUS, label_color);
        painter.circle_stroke(top_left_dot, DOT_RADIUS, egui::Stroke::new(1.0, active_color));

        let traj_color = if trajectory_on {
            active_color
        } else {
            inactive_color
        };
        painter.circle_filled(bottom_right_dot, DOT_RADIUS, traj_color);
        painter.circle_stroke(
            bottom_right_dot,
            DOT_RADIUS,
            egui::Stroke::new(1.0, active_color),
        );
    }

    response
}

fn cycle_state(labels: bool, trajectory: bool) -> (bool, bool) {
    match (labels, trajectory) {
        (false, false) => (true, false),
        (true, false) => (false, true),
        (false, true) => (true, true),
        (true, true) => (false, false),
    }
}

fn build_ordered_tags(view_settings: &ViewSettings) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    for preset_tag in TAG_ORDER {
        if view_settings.tags.contains_key(*preset_tag) {
            result.push((*preset_tag).to_string());
        }
    }

    let mut extra_tags: Vec<String> = view_settings
        .tags
        .keys()
        .filter(|k| !TAG_ORDER.contains(&k.as_str()))
        .cloned()
        .collect();
    extra_tags.sort();
    result.extend(extra_tags);

    result
}

fn to_egui_color(color: Color) -> egui::Color32 {
    let srgb = color.to_srgba();
    egui::Color32::from_rgba_premultiplied(
        (srgb.red * 255.0) as u8,
        (srgb.green * 255.0) as u8,
        (srgb.blue * 255.0) as u8,
        (srgb.alpha * 255.0) as u8,
    )
}
