use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::gui::style::vfd;

const BUTTON_SIZE: f32 = 24.0;
const BUTTON_SPACING: f32 = 4.0;
const PANEL_MARGIN: f32 = 12.0;
const HOVER_LABEL_GAP: f32 = 8.0;

const ABBR_FONT_SIZE_1: f32 = 14.0;
const ABBR_FONT_SIZE_2: f32 = 11.0;
const ABBR_FONT_SIZE_3: f32 = 9.0;

#[derive(Component)]
pub struct RightDrawer;

#[derive(Component)]
pub struct RightDrawerScrollContent;

#[derive(Component)]
pub struct RightPanelZone {
    pub panel_index: usize,
}

pub enum RightPanelState {
    Open,
    Closed,
    Opening(f64),
    Closing(f64),
}

pub struct RightPanel {
    pub open: bool,
    pub name: String,
    pub abbr: String,
}

#[derive(Resource)]
pub struct RightPanels {
    pub panels: Vec<RightPanel>,
    pub order: Vec<usize>,
    pub state: RightPanelState,
    pub toggle_duration: f32,
    pub panel_width: f32,
}

impl Default for RightPanels {
    fn default() -> Self {
        Self {
            panels: vec![
                RightPanel { open: false, name: "Body Info".to_string(), abbr: "I".to_string() },
                RightPanel { open: false, name: "Body Edit".to_string(), abbr: "E".to_string() },
                RightPanel { open: false, name: "One Letter".to_string(), abbr: "A".to_string() },
                RightPanel { open: false, name: "Two Letters".to_string(), abbr: "AB".to_string() },
                RightPanel { open: false, name: "Three Letters".to_string(), abbr: "ABC".to_string() },
            ],
            order: vec![0, 1, 2, 3, 4],
            state: RightPanelState::Closed,
            toggle_duration: 0.2,
            panel_width: 400.0,
        }
    }
}

pub fn right_panels_widget(
    mut contexts: EguiContexts,
    mut right_panels: ResMut<RightPanels>,
    time: Res<Time>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() {
        return;
    }
    let ctx = ctx.unwrap();

    let panel_bg = to_egui_color(vfd::PANEL_BG);
    let border = to_egui_color(vfd::BUTTON_BORDER);
    let text_color = to_egui_color(vfd::TEXT);
    let button_bg = to_egui_color(vfd::BUTTON_BG);
    let button_hover = to_egui_color(vfd::BUTTON_HOVER);
    let active_fill = to_egui_color(vfd::TEXT);

    let now = time.elapsed_secs_f64();
    let duration = right_panels.toggle_duration as f64;
    let panel_width = right_panels.panel_width;

    let open_fraction = match &right_panels.state {
        RightPanelState::Closed => 0.0,
        RightPanelState::Open => 1.0,
        RightPanelState::Opening(start) => ((now - start) / duration).clamp(0.0, 1.0),
        RightPanelState::Closing(start) => 1.0 - ((now - start) / duration).clamp(0.0, 1.0),
    };

    match &right_panels.state {
        RightPanelState::Opening(start) if (now - start) / duration >= 1.0 => {
            right_panels.state = RightPanelState::Open;
        }
        RightPanelState::Closing(start) if (now - start) / duration >= 1.0 => {
            right_panels.state = RightPanelState::Closed;
        }
        _ => {}
    }

    let x_offset = -PANEL_MARGIN - (panel_width * open_fraction as f32);

    let button_count = right_panels.order.len() + 1; // +1 for toggle button
    let panel_height = button_count as f32 * (BUTTON_SIZE + BUTTON_SPACING) - BUTTON_SPACING;

    let is_transitioning = matches!(right_panels.state, RightPanelState::Opening(_) | RightPanelState::Closing(_));
    let is_open = matches!(right_panels.state, RightPanelState::Open | RightPanelState::Opening(_));
    
    let toggle_abbr = if is_transitioning {
        "-"
    } else if is_open {
        ">"
    } else {
        "<"
    };

    let mut hovered_label: Option<(egui::Pos2, String)> = None;
    let mut clicked_indices: Vec<usize> = Vec::new();
    let mut toggle_clicked = false;

    egui::Area::new("right_panels".into())
        .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(x_offset, 0.0))
        .show(ctx, |ui| {
            ui.set_min_height(panel_height);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = BUTTON_SPACING;

                // Toggle button at the top
                let toggle_response = draw_right_panel_button(
                    ui,
                    false,
                    toggle_abbr,
                    text_color,
                    button_bg,
                    active_fill,
                    button_bg,
                    button_hover,
                    border,
                );

                if toggle_response.clicked() && !is_transitioning {
                    toggle_clicked = true;
                }

                if toggle_response.hovered() {
                    let pos = egui::pos2(
                        toggle_response.rect.left() - HOVER_LABEL_GAP,
                        toggle_response.rect.center().y,
                    );
                    let label = if is_open { "Close Panel" } else { "Open Panel" };
                    hovered_label = Some((pos, label.to_string()));
                }

                // Panel buttons
                for &panel_idx in &right_panels.order.clone() {
                    if let Some(panel) = right_panels.panels.get(panel_idx) {
                        let response = draw_right_panel_button(
                            ui,
                            panel.open,
                            &panel.abbr,
                            text_color,
                            button_bg,
                            active_fill,
                            button_bg,
                            button_hover,
                            border,
                        );

                        if response.clicked() {
                            clicked_indices.push(panel_idx);
                        }

                        if response.hovered() {
                            let pos = egui::pos2(
                                response.rect.left() - HOVER_LABEL_GAP,
                                response.rect.center().y,
                            );
                            hovered_label = Some((pos, panel.name.clone()));
                        }
                    }
                }
            });
        });

    // Track if any panel was turned on or off
    let mut panel_turned_on = false;
    let mut panel_turned_off = false;
    let mut reopen_drawer_from_open_panel_click = false;
    let drawer_is_closed = matches!(right_panels.state, RightPanelState::Closed);

    for idx in clicked_indices {
        if let Some(panel) = right_panels.panels.get_mut(idx) {
            if drawer_is_closed && panel.open {
                // If the drawer is closed, clicking an already-open panel should reopen the drawer
                // rather than toggling that panel off.
                reopen_drawer_from_open_panel_click = true;
                continue;
            }

            let was_open = panel.open;
            panel.open = !panel.open;
            if panel.open && !was_open {
                panel_turned_on = true;
            } else if !panel.open && was_open {
                panel_turned_off = true;
            }
        }
    }

    let any_open = right_panels.panels.iter().any(|p| p.open);

    // Handle explicit toggle button click
    if toggle_clicked {
        right_panels.state = match &right_panels.state {
            RightPanelState::Closed => RightPanelState::Opening(now),
            RightPanelState::Open => RightPanelState::Closing(now),
            other => match other {
                RightPanelState::Opening(t) => RightPanelState::Opening(*t),
                RightPanelState::Closing(t) => RightPanelState::Closing(*t),
                _ => unreachable!(),
            },
        };
    } else {
        // Auto-transition only on actual panel state changes
        right_panels.state = match &right_panels.state {
            // Re-open drawer when user clicks a panel that is already open.
            RightPanelState::Closed if reopen_drawer_from_open_panel_click => RightPanelState::Opening(now),
            // Auto-open when a panel is turned ON while drawer is closed
            RightPanelState::Closed if panel_turned_on => RightPanelState::Opening(now),
            // Auto-close when last panel is turned OFF while drawer is open
            RightPanelState::Open if panel_turned_off && !any_open => RightPanelState::Closing(now),
            // Keep current state otherwise
            state => match state {
                RightPanelState::Open => RightPanelState::Open,
                RightPanelState::Closed => RightPanelState::Closed,
                RightPanelState::Opening(t) => RightPanelState::Opening(*t),
                RightPanelState::Closing(t) => RightPanelState::Closing(*t),
            },
        };
    }

    if let Some((pos, label)) = hovered_label {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("right_panels_hover_label"),
        ));
        let font = egui::FontId::proportional(12.0);
        let galley = painter.layout_no_wrap(label, font.clone(), text_color);
        let text_size = galley.size();
        let padding = egui::vec2(6.0, 2.0);
        let bg_rect = egui::Rect::from_min_size(
            egui::pos2(
                pos.x - text_size.x - padding.x * 2.0,
                pos.y - text_size.y / 2.0 - padding.y,
            ),
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

fn draw_right_panel_button(
    ui: &mut egui::Ui,
    is_open: bool,
    abbr: &str,
    text_color: egui::Color32,
    text_color_active: egui::Color32,
    active_fill: egui::Color32,
    button_bg: egui::Color32,
    button_hover: egui::Color32,
    border_color: egui::Color32,
) -> egui::Response {
    let desired_size = egui::vec2(BUTTON_SIZE, BUTTON_SIZE);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        let fill = if is_open {
            active_fill
        } else if response.hovered() {
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

        let abbr_text: String = if abbr.chars().count() > 3 {
            abbr.chars().take(3).collect()
        } else {
            abbr.to_string()
        };

        let font_size = match abbr_text.chars().count() {
            1 => ABBR_FONT_SIZE_1,
            2 => ABBR_FONT_SIZE_2,
            _ => ABBR_FONT_SIZE_3,
        };

        let font = egui::FontId::proportional(font_size);
        let abbr_color = if is_open { text_color_active } else { text_color };
        let galley = painter.layout_no_wrap(abbr_text, font, abbr_color);
        let text_size = galley.size();
        let text_pos = egui::pos2(
            rect.center().x - text_size.x / 2.0,
            rect.center().y - text_size.y / 2.0,
        );
        painter.galley(text_pos, galley, abbr_color);
    }

    response
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

pub fn spawn_right_drawer(mut commands: Commands, right_panels: Res<RightPanels>) {
    let drawer_width = right_panels.panel_width;
    let initial_right = -drawer_width; // Start off-screen
    
    commands.spawn((
        RightDrawer,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(initial_right),
            top: Val::Px(0.0),
            width: Val::Px(drawer_width),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            border: UiRect::left(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(vfd::PANEL_BG.into()),
        BorderColor::all(vfd::BUTTON_BORDER),
    )).with_children(|parent| {
        // Scroll container that fills the drawer
        parent.spawn((
            RightDrawerScrollContent,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll_y(),
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            ScrollPosition::default(),
        ));
    });
}

pub fn update_right_drawer(
    right_panels: Res<RightPanels>,
    time: Res<Time>,
    mut drawer_query: Query<&mut Node, With<RightDrawer>>,
) {
    let now = time.elapsed_secs_f64();
    let duration = right_panels.toggle_duration as f64;
    let panel_width = right_panels.panel_width;

    let open_fraction = match &right_panels.state {
        RightPanelState::Closed => 0.0,
        RightPanelState::Open => 1.0,
        RightPanelState::Opening(start) => ((now - start) / duration).clamp(0.0, 1.0),
        RightPanelState::Closing(start) => 1.0 - ((now - start) / duration).clamp(0.0, 1.0),
    };

    // When closed (fraction=0): drawer is off-screen to the right
    // When open (fraction=1): drawer's right edge is at screen's right edge
    // Drawer is panel_width wide, so buttons (at panel_width + PANEL_MARGIN) have PANEL_MARGIN gap
    let right_when_open = 0.0;
    let right_when_closed = -panel_width;
    let right_offset = right_when_closed + (right_when_open - right_when_closed) * open_fraction as f32;

    for mut node in drawer_query.iter_mut() {
        node.right = Val::Px(right_offset);
    }
}

pub fn sync_panel_zones(
    mut commands: Commands,
    right_panels: Res<RightPanels>,
    scroll_content_query: Query<Entity, With<RightDrawerScrollContent>>,
    zone_query: Query<(Entity, &RightPanelZone)>,
) {
    let Ok(scroll_content) = scroll_content_query.single() else {
        return;
    };

    // Collect currently open panel indices in order
    let open_panels: Vec<usize> = right_panels.order.iter()
        .filter(|&&idx| right_panels.panels.get(idx).is_some_and(|p| p.open))
        .copied()
        .collect();

    // Collect existing zone panel indices
    let existing_zone_indices: Vec<usize> = zone_query.iter()
        .map(|(_, z)| z.panel_index)
        .collect();

    // Only rebuild if the set of open panels changed
    if existing_zone_indices == open_panels {
        return;
    }

    // Despawn all existing zones to rebuild in correct order
    for (entity, _) in zone_query.iter() {
        commands.entity(entity).despawn();
    }

    // Spawn zones in order for open panels
    for &panel_index in &open_panels {
        if let Some(panel) = right_panels.panels.get(panel_index) {
            let zone = commands.spawn((
                RightPanelZone { panel_index },
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(400.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(vfd::BUTTON_BG.into()),
                BorderColor::all(vfd::BUTTON_BORDER),
            )).with_children(|parent| {
                parent.spawn((
                    Text::new(panel.name.clone()),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(vfd::TEXT.into()),
                ));
            }).id();

            commands.entity(scroll_content).add_child(zone);
        }
    }
}

pub fn despawn_right_drawer(
    mut commands: Commands,
    drawer_query: Query<Entity, With<RightDrawer>>,
) {
    for entity in drawer_query.iter() {
        commands.entity(entity).despawn();
    }
}
