use bevy::app::{App, AppExit, Update};
use bevy::asset::AssetServer;
use bevy::input::ButtonInput;
use bevy::prelude::{AlignItems, BackgroundColor, BuildChildren, Button, ButtonBundle, Changed, Commands, Component, default, EventWriter, in_state, Interaction, IntoSystemConfigs, JustifyContent, KeyCode, NextState, NodeBundle, OnEnter, OnExit, PositionType, Query, Reflect, Res, ResMut, State, States, Style, TextBundle, TextStyle, Val, With};
use crate::gui::common;
use crate::gui::common::{AppState, despawn_screen};
use crate::gui::menu::main::MenuState;
use crate::gui::planetarium::planetarium::OnPlanetariumScreen;

#[derive(Component)]
struct OnEditorUI;

#[derive(Component)]
struct OnEscMenuUI;

#[derive(Component)]
pub struct DebugText;

#[derive(Component)]
pub(crate) enum GUIButtonAction {
    BackToMainMenu,
    BackToSaveSelect,
}

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub(crate) enum EscMenuState {
    Pause,
    #[default]
    Disabled,
}

pub fn esc_menu_plugin(app: &mut App) {
    app
        .init_state::<EscMenuState>()
        .add_systems(OnEnter(EscMenuState::Pause), esc_menu_setup)
        .add_systems(OnExit(EscMenuState::Pause), despawn_screen::<OnEscMenuUI>)
        .add_systems(
            Update,
            handle_keys,
        );
}

fn handle_keys(keyboard: Res<ButtonInput<KeyCode>>,
               state: Res<State<EscMenuState>>,
               mut next_state: ResMut<NextState<EscMenuState>>) {
    if keyboard.just_pressed(KeyCode::Escape) {
        match state.get() {
            EscMenuState::Pause => {
                next_state.set(EscMenuState::Disabled)
            }
            EscMenuState::Disabled => {
                next_state.set(EscMenuState::Pause)
            }
        }
    }
}

fn esc_menu_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
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
            OnEscMenuUI,
            OnEditorUI,
            OnPlanetariumScreen,
        )).with_children(|parent| {
        parent
            .spawn((
                ButtonBundle {
                    style: common::button_style(),
                    background_color: BackgroundColor::from(common::color::BACKGROUND),
                    ..default()
                },
                GUIButtonAction::BackToSaveSelect,
            ))
            .with_children(|parent| {
                parent.spawn(TextBundle::from_section("BACK", common::text::primary(asset_server.clone()).clone()));
            });
    });

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
            OnPlanetariumScreen,
        ))
        .with_children(|parent| {
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
                GUIButtonAction::BackToSaveSelect => {
                    menu_state.set(crate::gui::menu::main::MenuState::SaveSelect);
                    game_state.set(AppState::Menu);
                }
            }
        }
    }
}
