//! The main menu, in Bevy UI. Shown while the app is at `AppState::MainMenu`
//! with `MenuState::Home`.

use bevy::prelude::*;

use crate::gui::app::AppState;
use em_ui::{MenuTheme, MenuUi};

use super::{MenuState, UiState};

/// Active exactly when the main menu's home screen should be on screen. Derived
/// so that arriving from either direction - a change of `AppState` or of
/// `MenuState` - spawns and despawns the screen exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HomeMenu;

impl ComputedStates for HomeMenu {
    type SourceStates = (AppState, MenuState);

    fn compute(sources: (AppState, MenuState)) -> Option<Self> {
        match sources {
            (AppState::MainMenu, MenuState::Home) => Some(HomeMenu),
            _ => None,
        }
    }
}

#[derive(Component)]
struct HomeMenuScreen;

#[derive(Component)]
enum HomeAction {
    Planetarium,
    Settings,
    Quit,
}

pub struct HomeMenuPlugin;

impl Plugin for HomeMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_computed_state::<HomeMenu>()
            .add_systems(OnEnter(HomeMenu), setup_home_menu)
            .add_systems(OnExit(HomeMenu), cleanup_home_menu)
            .add_systems(Update, handle_home_buttons.run_if(in_state(HomeMenu)));
    }
}

fn setup_home_menu(mut commands: Commands) {
    let mut ui = MenuUi::new(&mut commands, MenuTheme::VFD);
    let root = ui.screen((HomeMenuScreen, GlobalZIndex(100)));
    let panel = ui.panel(root);
    ui.title(panel, "Exotic Matters");
    ui.button(panel, "Planetarium", HomeAction::Planetarium);
    ui.button(panel, "Settings", HomeAction::Settings);
    ui.button(panel, "Quit", HomeAction::Quit);
}

fn cleanup_home_menu(mut commands: Commands, query: Query<Entity, With<HomeMenuScreen>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

fn handle_home_buttons(
    interaction_query: Query<(&Interaction, &HomeAction), Changed<Interaction>>,
    mut next_menu: ResMut<NextState<MenuState>>,
    mut ui_state: ResMut<UiState>,
) {
    for (interaction, action) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            HomeAction::Planetarium => next_menu.set(MenuState::Planetarium),
            HomeAction::Settings => next_menu.set(MenuState::Settings),
            HomeAction::Quit => ui_state.quit_requested = true,
        }
    }
}
