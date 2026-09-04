//! Escape menu for the planetarium - handles saving, quitting, and related dialogs.

use std::path::PathBuf;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_ui_text_input::{TextInputNode, TextInputPlugin, TextInputBuffer};

use crate::sim::world::SimSystem;
use crate::body::universe::save::{
    SaveFormat, UniverseFile, UniverseFileTime, UniversePhysics, ViewSettings,
};
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::sim::SimTime;

use super::{MenuState, UiState, SaveFileMeta};
use super::widgets::{spawn_button, spawn_message, spawn_panel, spawn_title};
use crate::gui::style::vfd;

// ============================================================================
// State Machine
// ============================================================================

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum EscMenuState {
    #[default]
    Closed,
    Main,
    SaveNag,
    Naming,
    ConfirmOverwrite,
}

// ============================================================================
// Resources
// ============================================================================

#[derive(Resource, Default)]
pub struct EscMenuContext {
    pub intended_name: String,
    pub quit_after_save: bool,
    pub came_from_save_nag: bool,
    pub settings_window_visible: bool,
    pub was_playing_before_open: bool,
    pub restore_playing_on_close: bool,
}

#[derive(Resource, Default)]
pub struct UnsavedChanges(pub bool);

// ============================================================================
// Marker Components for Menu Screens
// ============================================================================

#[derive(Component)]
pub struct EscMenuOverlay;

#[derive(Component)]
pub struct MainMenuScreen;

#[derive(Component)]
pub struct SaveNagScreen;

#[derive(Component)]
pub struct NamingScreen;

#[derive(Component)]
pub struct ConfirmOverwriteScreen;

#[derive(Component)]
pub struct FileNameInput;

// ============================================================================
// Helper Functions
// ============================================================================

pub fn has_save_path(universe: &Universe) -> bool {
    universe
        .path
        .as_ref()
        .map_or(false, |p| p.starts_with("data/saves"))
}

fn file_exists(name: &str) -> bool {
    let path = PathBuf::from("data/saves").join(format!("{}.em", name));
    path.exists()
}

// ============================================================================
// Save Functionality
// ============================================================================

pub fn reconstruct_universe_file(
    sim_time: &SimTime,
    physics: &UniversePhysics,
    view_settings: &ViewSettings,
    system: &em_sim::system::System,
    path: Option<PathBuf>,
) -> UniverseFile {
    let time = UniverseFileTime {
        time_julian_days: sim_time.time.to_julian_day(),
        step: sim_time.step,
        gui_speed: sim_time.gui_speed,
        max_frame_time: sim_time.max_frame_time,
    };
    UniverseFile {
        file: path,
        contents: system.to_contents(time, physics.clone(), view_settings.clone()),
    }
}

pub fn perform_save(
    sim_time: &SimTime,
    physics: &UniversePhysics,
    view_settings: &ViewSettings,
    universe: &mut Universe,
    system: &em_sim::system::System,
    path: PathBuf,
) -> Result<(), String> {
    eprintln!("Saving {} bodies to {:?}", system.len(), path);
    eprintln!("  Time: {} Julian days (J2000 seconds: {})", 
        sim_time.time.to_julian_day(), 
        sim_time.time.to_j2000_seconds());

    let mut universe_file = reconstruct_universe_file(
        sim_time,
        physics,
        view_settings,
        system,
        Some(path.clone()),
    );

    match universe_file.save_as(path.clone(), SaveFormat::Sqlite) {
        Ok(()) => {
            eprintln!("Save successful!");
            universe.path = Some(path);
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("Failed to save: {:?}", e);
            eprintln!("{}", err_msg);
            Err(err_msg)
        }
    }
}

// ============================================================================
// Esc Key Handler
// ============================================================================

pub fn handle_escape_key(
    keys: Res<ButtonInput<KeyCode>>,
    current_state: Res<State<EscMenuState>>,
    mut next_state: ResMut<NextState<EscMenuState>>,
    mut context: ResMut<EscMenuContext>,
    mut sim_time: ResMut<SimTime>,
    mut contexts: EguiContexts,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }

    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    match current_state.get() {
        EscMenuState::Closed => {
            context.was_playing_before_open = sim_time.playing;
            context.restore_playing_on_close = true;
            sim_time.playing = false;
            next_state.set(EscMenuState::Main);
        }
        _ => next_state.set(EscMenuState::Closed),
    }
}

// ============================================================================
// Dirty Tracking
// ============================================================================

pub fn track_unsaved_changes(
    sim_time: Res<SimTime>,
    mut unsaved: ResMut<UnsavedChanges>,
) {
    if sim_time.is_changed() && sim_time.playing {
        unsaved.0 = true;
    }
}

// ============================================================================
// UI Building Helpers
// ============================================================================

fn spawn_overlay(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(vfd::OVERLAY_BACKDROP.into()),
            EscMenuOverlay,
            GlobalZIndex(100),
        ))
        .id()
}

#[derive(Component, Clone)]
pub enum MenuAction {
    Resume,
    Settings,
    Save,
    Quit,
    SaveAndQuit,
    QuitWithoutSaving,
    Back,
    ConfirmSave,
    CancelNaming,
    Overwrite,
}

// ============================================================================
// Main Menu Screen
// ============================================================================

pub fn setup_main_menu(mut commands: Commands) {
    let overlay = spawn_overlay(&mut commands);
    commands.entity(overlay).insert(MainMenuScreen);

    let panel = spawn_panel(&mut commands, overlay);
    spawn_title(&mut commands, panel, "Paused");
    spawn_button(&mut commands, panel, "Resume", MenuAction::Resume);
    spawn_button(&mut commands, panel, "Settings", MenuAction::Settings);
    spawn_button(&mut commands, panel, "Save", MenuAction::Save);
    spawn_button(&mut commands, panel, "Quit", MenuAction::Quit);
}

pub fn cleanup_main_menu(
    mut commands: Commands,
    query: Query<Entity, With<MainMenuScreen>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

pub fn handle_main_menu_buttons(
    interaction_query: Query<(&Interaction, &MenuAction), (Changed<Interaction>, With<Button>)>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut context: ResMut<EscMenuContext>,
    mut unsaved: ResMut<UnsavedChanges>,
    mut ui_state: ResMut<UiState>,
    mut universe: ResMut<Universe>,
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    system: Res<SimSystem>,
) {
    for (interaction, action) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            MenuAction::Resume => {
                next_esc_state.set(EscMenuState::Closed);
            }
            MenuAction::Settings => {
                context.settings_window_visible = true;
                context.restore_playing_on_close = false;
                next_esc_state.set(EscMenuState::Closed);
            }
            MenuAction::Save => {
                context.quit_after_save = false;
                context.came_from_save_nag = false;
                
                if has_save_path(&universe) {
                    if let Some(path) = universe.path.clone() {
                        if perform_save(
                            &sim_time,
                            &physics,
                            &view_settings,
                            &mut universe,
                            &system.0,
                            path,
                        ).is_ok() {
                            unsaved.0 = false;
                        }
                    }
                    next_esc_state.set(EscMenuState::Closed);
                } else {
                    next_esc_state.set(EscMenuState::Naming);
                }
            }
            MenuAction::Quit => {
                if unsaved.0 {
                    next_esc_state.set(EscMenuState::SaveNag);
                } else {
                    context.restore_playing_on_close = false;
                    ui_state.current_save = None;
                    next_esc_state.set(EscMenuState::Closed);
                    next_app_state.set(AppState::MainMenu);
                    next_menu_state.set(MenuState::Planetarium);
                }
            }
            _ => {}
        }
    }
}

// ============================================================================
// Save Nag Screen
// ============================================================================

pub fn setup_save_nag(mut commands: Commands) {
    let overlay = spawn_overlay(&mut commands);
    commands.entity(overlay).insert(SaveNagScreen);

    let panel = spawn_panel(&mut commands, overlay);
    spawn_title(&mut commands, panel, "Unsaved Changes");
    spawn_message(&mut commands, panel, "You have unsaved changes.");
    spawn_button(&mut commands, panel, "Save and Quit", MenuAction::SaveAndQuit);
    spawn_button(&mut commands, panel, "Quit without Saving", MenuAction::QuitWithoutSaving);
    spawn_button(&mut commands, panel, "Back", MenuAction::Back);
}

pub fn cleanup_save_nag(
    mut commands: Commands,
    query: Query<Entity, With<SaveNagScreen>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

pub fn handle_save_nag_buttons(
    interaction_query: Query<(&Interaction, &MenuAction), (Changed<Interaction>, With<Button>)>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut context: ResMut<EscMenuContext>,
    mut unsaved: ResMut<UnsavedChanges>,
    mut ui_state: ResMut<UiState>,
    mut universe: ResMut<Universe>,
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    system: Res<SimSystem>,
) {
    for (interaction, action) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            MenuAction::SaveAndQuit => {
                context.quit_after_save = true;
                context.came_from_save_nag = true;
                context.restore_playing_on_close = false;
                
                if has_save_path(&universe) {
                    if let Some(path) = universe.path.clone() {
                        if perform_save(
                            &sim_time,
                            &physics,
                            &view_settings,
                            &mut universe,
                            &system.0,
                            path,
                        ).is_ok() {
                            unsaved.0 = false;
                        }
                    }
                    ui_state.current_save = None;
                    next_esc_state.set(EscMenuState::Closed);
                    next_app_state.set(AppState::MainMenu);
                    next_menu_state.set(MenuState::Planetarium);
                } else {
                    next_esc_state.set(EscMenuState::Naming);
                }
            }
            MenuAction::QuitWithoutSaving => {
                context.restore_playing_on_close = false;
                ui_state.current_save = None;
                next_esc_state.set(EscMenuState::Closed);
                next_app_state.set(AppState::MainMenu);
                next_menu_state.set(MenuState::Planetarium);
            }
            MenuAction::Back => {
                next_esc_state.set(EscMenuState::Main);
            }
            _ => {}
        }
    }
}

// ============================================================================
// Naming Screen
// ============================================================================

pub fn setup_naming(mut commands: Commands, mut context: ResMut<EscMenuContext>) {
    context.intended_name.clear();

    let overlay = spawn_overlay(&mut commands);
    commands.entity(overlay).insert(NamingScreen);

    let panel = spawn_panel(&mut commands, overlay);
    spawn_title(&mut commands, panel, "Save As");
    spawn_message(&mut commands, panel, "Enter a name for your save file:");

    let input = commands
        .spawn((
            TextInputNode::default(),
            Node {
                width: Val::Px(300.0),
                height: Val::Px(40.0),
                border: UiRect::all(Val::Px(2.0)),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                margin: UiRect::bottom(Val::Px(10.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexStart,
                ..default()
            },
            BackgroundColor(Color::srgb(0.05, 0.12, 0.08).into()),
            BorderColor::all(vfd::BUTTON_BORDER),
            FileNameInput,
        ))
        .id();
    commands.entity(panel).add_child(input);

    spawn_button(&mut commands, panel, "Save", MenuAction::ConfirmSave);
    spawn_button(&mut commands, panel, "Cancel", MenuAction::CancelNaming);
}

pub fn cleanup_naming(
    mut commands: Commands,
    query: Query<Entity, With<NamingScreen>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

pub fn handle_naming_buttons(
    interaction_query: Query<(&Interaction, &MenuAction), (Changed<Interaction>, With<Button>)>,
    text_input_query: Query<&TextInputBuffer, With<FileNameInput>>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut context: ResMut<EscMenuContext>,
    mut unsaved: ResMut<UnsavedChanges>,
    mut ui_state: ResMut<UiState>,
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    system: Res<SimSystem>,
    mut universe: ResMut<Universe>,
) {
    for (interaction, action) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            MenuAction::ConfirmSave => {
                let name = text_input_query
                    .iter()
                    .next()
                    .map(|buf| buf.get_text().trim().to_string())
                    .unwrap_or_default();

                if name.is_empty() {
                    continue;
                }

                context.intended_name = name.clone();

                if file_exists(&name) {
                    next_esc_state.set(EscMenuState::ConfirmOverwrite);
                } else {
                    let path = PathBuf::from("data/saves").join(format!("{}.em", name));
                    match perform_save(
                        &sim_time,
                        &physics,
                        &view_settings,
                        &mut universe,
                        &system.0,
                        path.clone(),
                    ) {
                        Ok(()) => {
                            unsaved.0 = false;
                            ui_state.current_save = Some(SaveFileMeta {
                                path: path.clone(),
                                file_name: format!("{}.em", name),
                            });

                            if context.quit_after_save {
                                ui_state.current_save = None;
                                next_esc_state.set(EscMenuState::Closed);
                                next_app_state.set(AppState::MainMenu);
                                next_menu_state.set(MenuState::Planetarium);
                            } else {
                                next_esc_state.set(EscMenuState::Closed);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to save file: {}", e);
                        }
                    }
                }
            }
            MenuAction::CancelNaming => {
                if context.came_from_save_nag {
                    next_esc_state.set(EscMenuState::SaveNag);
                } else {
                    next_esc_state.set(EscMenuState::Main);
                }
            }
            _ => {}
        }
    }
}

// ============================================================================
// Confirm Overwrite Screen
// ============================================================================

pub fn setup_confirm_overwrite(mut commands: Commands, context: Res<EscMenuContext>) {
    let overlay = spawn_overlay(&mut commands);
    commands.entity(overlay).insert(ConfirmOverwriteScreen);

    let panel = spawn_panel(&mut commands, overlay);
    spawn_title(&mut commands, panel, "Confirm Overwrite");
    spawn_message(
        &mut commands,
        panel,
        &format!("\"{}\" already exists. Overwrite?", context.intended_name),
    );
    spawn_button(&mut commands, panel, "Overwrite", MenuAction::Overwrite);
    spawn_button(&mut commands, panel, "Back", MenuAction::Back);
}

pub fn cleanup_confirm_overwrite(
    mut commands: Commands,
    query: Query<Entity, With<ConfirmOverwriteScreen>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

pub fn handle_confirm_overwrite_buttons(
    interaction_query: Query<(&Interaction, &MenuAction), (Changed<Interaction>, With<Button>)>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    context: Res<EscMenuContext>,
    mut unsaved: ResMut<UnsavedChanges>,
    mut ui_state: ResMut<UiState>,
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    system: Res<SimSystem>,
    mut universe: ResMut<Universe>,
) {
    for (interaction, action) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            MenuAction::Overwrite => {
                let path = PathBuf::from("data/saves").join(format!("{}.em", context.intended_name));
                match perform_save(
                    &sim_time,
                    &physics,
                    &view_settings,
                    &mut universe,
                    &system.0,
                    path.clone(),
                ) {
                    Ok(()) => {
                        unsaved.0 = false;
                        ui_state.current_save = Some(SaveFileMeta {
                            path: path.clone(),
                            file_name: format!("{}.em", context.intended_name),
                        });

                        if context.quit_after_save {
                            ui_state.current_save = None;
                            next_esc_state.set(EscMenuState::Closed);
                            next_app_state.set(AppState::MainMenu);
                            next_menu_state.set(MenuState::Planetarium);
                        } else {
                            next_esc_state.set(EscMenuState::Closed);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to save file: {}", e);
                    }
                }
            }
            MenuAction::Back => {
                next_esc_state.set(EscMenuState::Naming);
            }
            _ => {}
        }
    }
}

// ============================================================================
// Context Reset on Menu Close
// ============================================================================

pub fn reset_context_on_close(mut context: ResMut<EscMenuContext>, mut sim_time: ResMut<SimTime>) {
    if context.restore_playing_on_close {
        sim_time.playing = context.was_playing_before_open;
    }

    context.intended_name.clear();
    context.quit_after_save = false;
    context.came_from_save_nag = false;
    context.was_playing_before_open = false;
    context.restore_playing_on_close = false;
}

// ============================================================================
// Plugin Registration Helper
// ============================================================================

pub struct EscapeMenuPlugin;

impl Plugin for EscapeMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TextInputPlugin)
            .insert_state(EscMenuState::Closed)
            .init_resource::<EscMenuContext>()
            .init_resource::<UnsavedChanges>()
            .add_systems(
                Update,
                (
                    handle_escape_key,
                    track_unsaved_changes,
                )
                    .run_if(in_state(AppState::Planetarium)),
            )
            .add_systems(OnEnter(EscMenuState::Main), setup_main_menu)
            .add_systems(OnExit(EscMenuState::Main), cleanup_main_menu)
            .add_systems(
                Update,
                handle_main_menu_buttons.run_if(in_state(EscMenuState::Main)),
            )
            .add_systems(OnEnter(EscMenuState::SaveNag), setup_save_nag)
            .add_systems(OnExit(EscMenuState::SaveNag), cleanup_save_nag)
            .add_systems(
                Update,
                handle_save_nag_buttons.run_if(in_state(EscMenuState::SaveNag)),
            )
            .add_systems(OnEnter(EscMenuState::Naming), setup_naming)
            .add_systems(OnExit(EscMenuState::Naming), cleanup_naming)
            .add_systems(
                Update,
                handle_naming_buttons.run_if(in_state(EscMenuState::Naming)),
            )
            .add_systems(OnEnter(EscMenuState::ConfirmOverwrite), setup_confirm_overwrite)
            .add_systems(OnExit(EscMenuState::ConfirmOverwrite), cleanup_confirm_overwrite)
            .add_systems(
                Update,
                handle_confirm_overwrite_buttons.run_if(in_state(EscMenuState::ConfirmOverwrite)),
            )
            .add_systems(OnEnter(EscMenuState::Closed), reset_context_on_close);
    }
}
