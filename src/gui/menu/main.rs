use std::path::PathBuf;
use bevy::prelude::*;

use bevy::app::AppExit;
use crate::gui::{common, menu};
use crate::gui::common::{BackGlow, text};
use crate::gui::common::color::NORMAL_BUTTON;
use crate::gui::menu::{save_select, settings, settings_display, settings_sound, settings_ui_test};
use crate::gui::menu::save_select::{OnSaveSelectScreen, SaveEntry};
use crate::gui::menu::settings::OnSettingsMenuScreen;
use crate::gui::menu::settings_display::{GlowSetting, OnDisplaySettingsMenuScreen, QualitySetting};
use crate::gui::menu::settings_sound::{OnSoundSettingsMenuScreen, VolumeSetting};
use crate::gui::menu::settings_ui_test::OnUITestScreen;
use super::super::common::{AppState, despawn_screen, DisplayQuality, Volume};

// This plugin manages the menu, with 5 different screens:
// - a main menu with "New Game", "Settings", "Quit"
// - a settings menu with two submenus and a back button
// - two settings screen with a setting that can be set and a back button
pub fn menu_plugin(app: &mut App) {
    app
        // At start, the menu is not enabled. This will be changed in `menu_setup` when
        // entering the `GameState::Menu` state.
        // Current screen in the menu is handled by an independent state from `GameState`
        .init_state::<MenuState>()
        .add_systems(OnEnter(AppState::Menu), menu_setup)
        // Systems to handle the main menu screen
        .add_systems(OnEnter(MenuState::Main), main_menu_setup)
        .add_systems(OnExit(MenuState::Main), despawn_screen::<OnMainMenuScreen>)
        // Systems to handle the settings menu screen
        .add_systems(OnEnter(MenuState::Settings), settings::settings_menu_setup)
        .add_systems(
            OnExit(MenuState::Settings),
            despawn_screen::<OnSettingsMenuScreen>,
        )
        // Systems to handle the display settings screen
        .add_systems(
            OnEnter(MenuState::SettingsDisplay),
            settings_display::display_settings_menu_setup,
        )
        .add_systems(
            Update,
            (setting_button::<DisplayQuality, QualitySetting>.run_if(in_state(MenuState::SettingsDisplay)),),
        )
        .add_systems(
            Update,
            (setting_button::<BackGlow, GlowSetting>.run_if(in_state(MenuState::SettingsDisplay)),),
        )
        .add_systems(
            OnExit(MenuState::SettingsDisplay),
            despawn_screen::<OnDisplaySettingsMenuScreen>,
        )
        // Systems to handle the sound settings screen
        .add_systems(OnEnter(MenuState::SettingsSound), settings_sound::sound_settings_menu_setup)
        .add_systems(OnEnter(MenuState::SaveSelect), save_select::save_select_setup)
        .add_systems(
            Update,
            setting_button::<Volume, VolumeSetting>.run_if(in_state(MenuState::SettingsSound)),
        )
        .add_systems(
            OnExit(MenuState::SettingsSound),
            despawn_screen::<OnSoundSettingsMenuScreen>,
        )
        .add_systems(
            OnEnter(MenuState::SettingsUITest),
            settings_ui_test::ui_test_menu_setup,
        )
        .add_systems(
            OnExit(MenuState::SettingsUITest),
            despawn_screen::<OnUITestScreen>,
        )
        .add_systems(
            OnExit(MenuState::SaveSelect),
            despawn_screen::<OnSaveSelectScreen>,
        )
        // Common systems to all screens that handles buttons behavior
        .add_systems(
            Update,
            (menu_action, menu::common::button_system).run_if(in_state(AppState::Menu)),
        );
}

// State used for the current menu screen
#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub(crate) enum MenuState {
    Main,
    SaveSelect,
    Settings,
    SettingsDisplay,
    SettingsSound,
    SettingsUITest,
    #[default]
    Disabled,
}

// Tag component used to tag entities added on the main menu screen
#[derive(Component)]
struct OnMainMenuScreen;

// Tag component used to mark which setting is currently selected
#[derive(Component)]
pub(crate) struct SelectedOption;

// All actions that can be triggered from a button click
#[derive(Component)]
pub(crate) enum MenuButtonAction {
    Play,
    SaveSelect,
    LoadSave(SaveEntry),
    Settings,
    SettingsDisplay,
    SettingsSound,
    SettingsUITest,
    BackToMainMenu,
    BackToSettings,
    Quit,
}

// This system updates the settings when a new value for a setting is selected, and marks
// the button as the one currently selected
fn setting_button<T: Resource + Component + PartialEq + Copy, U: Component>(
    interaction_query: Query<(&Interaction, &T, &U, Entity), (Changed<Interaction>, With<Button>)>,
    mut selected_query: Query<(Entity, &mut BackgroundColor, &U), With<SelectedOption>>,
    mut commands: Commands,
    mut setting: ResMut<T>,
) {
    for (interaction, button_setting, setting_type, entity) in &interaction_query {
        if *interaction == Interaction::Pressed && *setting != *button_setting {
            let (previous_button, mut previous_color, setting_type_) = selected_query.single_mut();
            *previous_color = NORMAL_BUTTON.into();
            commands.entity(previous_button).remove::<SelectedOption>();
            commands.entity(entity).insert(SelectedOption);
            *setting = *button_setting;
        }
    }
}

fn menu_setup(
    menu_state: Res<State<MenuState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>
) {
    if *menu_state == MenuState::Disabled {
        next_menu_state.set(MenuState::Main);
    }
}

fn main_menu_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // Common style for all buttons on the screen
    let button_style = Style {
        width: Val::Px(250.0),
        height: Val::Px(65.0),
        margin: UiRect::all(Val::Px(20.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };
    let button_icon_style = Style {
        width: Val::Px(30.0),
        // This takes the icons out of the flexbox flow, to be positioned exactly
        position_type: PositionType::Absolute,
        // The icon will be close to the left border of the button
        left: Val::Px(10.0),
        ..default()
    };
    let button_text_style = text::primary(asset_server.clone());

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnMainMenuScreen)
        .with_children(|parent| {
            parent
                .spawn(NodeBundle {
                    style: common::panel::vertical(),
                    background_color: common::color::FOREGROUND.into(),
                    ..default()
                })
                .with_children(|parent| {
                    // Display the game name
                    parent.spawn(
                        TextBundle::from_section(
                            "EXOTIC MATTERS",
                            button_text_style.clone(),
                        )
                            .with_style(Style {
                                margin: UiRect::all(Val::Px(50.0)),
                                ..default()
                            }),
                    );

                    // Display three buttons for each action available from the main menu:
                    // - new game
                    // - settings
                    // - quit
                    parent
                        .spawn((
                            ButtonBundle {
                                style: button_style.clone(),
                                background_color: NORMAL_BUTTON.into(),
                                ..default()
                            },
                            MenuButtonAction::SaveSelect,
                        ))
                        .with_children(|parent| {
                            let icon = asset_server.load("icons/play.png");
                            parent.spawn(ImageBundle {
                                style: button_icon_style.clone(),
                                image: UiImage::new(icon),
                                ..default()
                            });
                            parent.spawn(TextBundle::from_section(
                                "START",
                                button_text_style.clone(),
                            ));
                        });
                    parent
                        .spawn((
                            ButtonBundle {
                                style: button_style.clone(),
                                background_color: NORMAL_BUTTON.into(),
                                ..default()
                            },
                            MenuButtonAction::Settings,
                        ))
                        .with_children(|parent| {
                            let icon = asset_server.load("icons/settings.png");
                            parent.spawn(ImageBundle {
                                style: button_icon_style.clone(),
                                image: UiImage::new(icon),
                                ..default()
                            });
                            parent.spawn(TextBundle::from_section(
                                "SETTINGS",
                                button_text_style.clone(),
                            ));
                        });
                    parent
                        .spawn((
                            ButtonBundle {
                                style: button_style,
                                background_color: NORMAL_BUTTON.into(),
                                ..default()
                            },
                            MenuButtonAction::Quit,
                        ))
                        .with_children(|parent| {
                            let icon = asset_server.load("icons/power.png");
                            parent.spawn(ImageBundle {
                                style: button_icon_style,
                                image: UiImage::new(icon),
                                ..default()
                            });
                            parent.spawn(TextBundle::from_section("QUIT", button_text_style));
                        });
                });
        });
}

pub(crate) fn menu_action(
    interaction_query: Query<
        (&Interaction, &MenuButtonAction),
        (Changed<Interaction>, With<Button>),
    >,
    mut app_exit_events: EventWriter<AppExit>,
    mut menu_state: ResMut<NextState<MenuState>>,
    mut game_state: ResMut<NextState<AppState>>,
    mut save: ResMut<SaveEntry>,
) {
    for (interaction, menu_button_action) in &interaction_query {
        if *interaction == Interaction::Pressed {
            match menu_button_action {
                MenuButtonAction::Quit => {
                    app_exit_events.send(AppExit);
                }
                MenuButtonAction::Play => {
                    game_state.set(AppState::Planetarium);
                    menu_state.set(MenuState::Disabled);
                }
                MenuButtonAction::LoadSave(new_save) => {
                    info!("{:?}", new_save);
                    save.name = new_save.name.clone();
                    save.path = new_save.path.clone();
                    game_state.set(AppState::Planetarium);
                    menu_state.set(MenuState::Disabled);
                }
                MenuButtonAction::SaveSelect => {
                    menu_state.set(MenuState::SaveSelect);
                }
                MenuButtonAction::Settings => menu_state.set(MenuState::Settings),
                MenuButtonAction::SettingsDisplay => {
                    menu_state.set(MenuState::SettingsDisplay);
                }
                MenuButtonAction::SettingsSound => {
                    menu_state.set(MenuState::SettingsSound);
                }
                MenuButtonAction::SettingsUITest => {
                    menu_state.set(MenuState::SettingsUITest)
                }
                MenuButtonAction::BackToMainMenu => menu_state.set(MenuState::Main),
                MenuButtonAction::BackToSettings => {
                    menu_state.set(MenuState::Settings);
                }
            }
        }
    }
}
