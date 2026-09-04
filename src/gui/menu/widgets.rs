//! Bevy-native menu widgets in the VFD style, shared by the main menu and the
//! in-planetarium escape menu.

use bevy::prelude::*;

use crate::gui::style::vfd;

/// Marks a button that should react to hover with the VFD palette.
#[derive(Component)]
pub struct MenuButton;

/// The panel every menu screen is built inside: a bordered column of widgets.
pub fn spawn_panel(commands: &mut Commands, parent: Entity) -> Entity {
    let panel = commands
        .spawn((
            Node {
                width: Val::Px(400.0),
                min_height: Val::Px(200.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(30.0)),
                row_gap: Val::Px(15.0),
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(vfd::PANEL_BG.into()),
            BorderColor::all(vfd::BUTTON_BORDER),
        ))
        .id();
    commands.entity(parent).add_child(panel);
    panel
}

pub fn spawn_title(commands: &mut Commands, panel: Entity, text: &str) {
    spawn_label(commands, panel, text, 28.0, vfd::TEXT);
}

pub fn spawn_message(commands: &mut Commands, panel: Entity, text: &str) {
    spawn_label(commands, panel, text, 18.0, vfd::TEXT_DIM);
}

fn spawn_label(commands: &mut Commands, panel: Entity, text: &str, font_size: f32, color: Color) {
    let label = commands
        .spawn((
            Text::new(text),
            TextFont {
                font_size,
                ..default()
            },
            TextColor(color.into()),
            Node {
                margin: UiRect::bottom(Val::Px(10.0)),
                ..default()
            },
        ))
        .id();
    commands.entity(panel).add_child(label);
}

/// A menu button carrying `action`, which the screen's handler system reads
/// back out of the `Interaction` query.
pub fn spawn_button<A: Component>(commands: &mut Commands, panel: Entity, text: &str, action: A) {
    let btn = commands
        .spawn((
            Button,
            Node {
                width: Val::Px(250.0),
                height: Val::Px(45.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(vfd::BUTTON_BG.into()),
            BorderColor::all(vfd::BUTTON_BORDER),
            MenuButton,
            action,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(text),
                TextFont {
                    font_size: 18.0,
                    ..default()
                },
                TextColor(vfd::TEXT.into()),
            ));
        })
        .id();
    commands.entity(panel).add_child(btn);
}

pub fn button_hover_system(
    mut interaction_query: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<MenuButton>),
    >,
) {
    for (interaction, mut color) in &mut interaction_query {
        match *interaction {
            Interaction::Hovered | Interaction::Pressed => *color = vfd::BUTTON_HOVER.into(),
            Interaction::None => *color = vfd::BUTTON_BG.into(),
        }
    }
}
