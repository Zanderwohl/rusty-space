use bevy::app::AppExit;
use bevy::asset::AssetServer;
use bevy::prelude::{AlignItems, BackgroundColor, BuildChildren, Button, ButtonBundle, Changed, Commands, Component, default, EventWriter, Interaction, JustifyContent, NextState, NodeBundle, PositionType, Query, Res, ResMut, Style, TextBundle, TextStyle, Val, With};
use crate::gui::common;
use crate::gui::common::AppState;
use crate::gui::editor::display::OnEditorScreen;

#[derive(Component)]
struct OnEditorUI;

#[derive(Component)]
pub struct DebugText;

#[derive(Component)]
pub(crate) enum GUIButtonAction {
    BackToMainMenu,
}

pub(super) fn ui_setup(commands: &mut Commands, asset_server: AssetServer) {
    commands
        .spawn((
            NodeBundle { // The main UI area
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    align_items: AlignItems::Start,
                    justify_content: JustifyContent::Start,
                    ..default()
                },
                ..default()
            },
            OnEditorUI,
            OnEditorScreen,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    ButtonBundle {
                        style: common::button_style(),
                        background_color: BackgroundColor::from(common::color::BACKGROUND),
                        ..default()
                    },
                    GUIButtonAction::BackToMainMenu,
                ))
                .with_children(|parent| {
                    parent.spawn(TextBundle::from_section("Back", common::text::primary(asset_server.clone()).clone()));
                });
            parent
                .spawn((
                    TextBundle::from_section(
                        "Time handler not started.",
                        TextStyle {
                            font: asset_server.load("fonts/Jost.ttf"),
                            font_size: 26.0,
                            ..default()
                        },
                    )
                        .with_style(Style {
                            position_type: PositionType::Absolute,
                            bottom: Val::Px(10.0),
                            left: Val::Px(10.0),
                            ..default()
                        }),
                    DebugText
                ));
        });
}

pub(crate) fn menu_action(
    interaction_query: Query<
        (&Interaction, &GUIButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut app_exit_events: EventWriter<AppExit>,
    mut menu_state: ResMut<NextState<crate::gui::menu::main::MenuState>>,
    mut game_state: ResMut<NextState<AppState>>,
) {
    for (interaction, menu_button_action) in &interaction_query {
        if *interaction == Interaction::Pressed {
            match menu_button_action {
                GUIButtonAction::BackToMainMenu => {
                    menu_state.set(crate::gui::menu::main::MenuState::Main);
                    game_state.set(AppState::Menu);
                },
            }
        }
    }
}
