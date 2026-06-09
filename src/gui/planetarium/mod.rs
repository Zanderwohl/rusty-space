use std::collections::HashMap;
use bevy::app::{App, Update};
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use crate::body::appearance::AssetCache;
use crate::body::universe::save::{UniverseFile, UniversePhysics, ViewSettings};
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::gui::menu::UiState;
use crate::body::universe::save::TagState;
use crate::sim::{SimTime, unload_simulation_objects, CalculateTrajectory, BodySelection};
use crate::body::motive::calculate_body_positions::{self, PhysicsGraph, PositionCache, SimulationPerformanceMetrics};
use crate::body::motive::kepler_motive;
pub(crate) use crate::camera::{PlanetariumCamera, PlanetariumCameraPlugin, CameraAction};
use crate::camera::Freecam;
pub use crate::gui::planetarium::focused_body::FocusedBodyState;
pub use crate::gui::planetarium::focused_body::{HoverState, HoveredTrajectoryMarkerKind, TrajectoryHitData};
pub use crate::gui::planetarium::windows::mission_clock::{MissionClockMode, MissionClockSettings, format_sim_time_for_mode};
use crate::presentation::{self, BodyWireframeMaterial, TrajectoryMaterialPlugin, BodyWireframeMaterialPlugin, OccluderMaterialPlugin, BodyPointMaterialPlugin, StarfieldMaterialPlugin, TrajectoryMesh, BodyPointMesh, FocusedTrajectoryMarker, StarLightingFrameCache};
use crate::gui::menu::escape::{EscapeMenuPlugin, EscMenuContext, EscMenuState, UnsavedChanges};

mod windows;
mod input;
mod focused_body;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumLoadingSet;

pub struct PlanetariumUI;

impl Plugin for PlanetariumUI {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SimTime>()
            .init_resource::<UniversePhysics>()
            .init_resource::<ViewSettings>()
            .init_resource::<AssetCache>()
            .init_resource::<FocusedBodyState>()
            .init_resource::<HoverState>()
            .init_resource::<MissionClockSettings>()
            .init_resource::<PhysicsGraph>()
            .init_resource::<PositionCache>()
            .init_resource::<SimulationPerformanceMetrics>()
            .init_resource::<StarLightingFrameCache>()
            .add_message::<CalculateTrajectory>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_plugins(PlanetariumCameraPlugin)
            .add_plugins(TrajectoryMaterialPlugin)
            .add_plugins(BodyWireframeMaterialPlugin)
            .add_plugins(OccluderMaterialPlugin)
            .add_plugins(BodyPointMaterialPlugin)
            .add_plugins(StarfieldMaterialPlugin)
            .add_plugins(EscapeMenuPlugin)
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::mission_clock::mission_clock_widget,
                    windows::controls::control_window,
                    windows::body_edit::body_edit_window,
                    windows::body_info::body_info_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,
                    windows::camera::camera_window,
                    windows::show_hide_panel::show_hide_panel_widget,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            // Core simulation and position systems
            .add_systems(Update, (
                presentation::adjust_lights,
                input::handle_go_to_shortcut,
                input::handle_revolve_frame_shortcut,
                calculate_body_positions::calculate_body_positions,
                kepler_motive::calculate_trajectory,
                presentation::position_bodies.after(calculate_body_positions::calculate_body_positions),
                presentation::orient_bodies.after(presentation::position_bodies),
            ).in_set(PlanetariumUISet))
            // Trajectory mesh systems
            .add_systems(Update, (
                presentation::spawn_trajectory_meshes_for_bodies,
                presentation::refresh_precessing_trajectories
                    .before(kepler_motive::calculate_trajectory),
                presentation::rebuild_trajectory_caches
                    .after(kepler_motive::calculate_trajectory),
                presentation::update_focused_trajectory_markers
                    .after(presentation::position_bodies),
                presentation::update_mouse_hit_marker
                    .after(presentation::position_bodies),
                presentation::build_trajectory_meshes
                    .after(presentation::position_bodies)
                    .after(presentation::rebuild_trajectory_caches),
                presentation::cleanup_orphaned_trajectory_meshes,
            ).in_set(PlanetariumUISet))
            // Body wireframe, occluder, and terminator mesh systems
            .add_systems(Update, (
                presentation::spawn_body_wireframe_meshes,
                presentation::spawn_body_occluders,
                presentation::spawn_terminator_meshes,
                presentation::build_star_lighting_cache
                    .after(presentation::position_bodies),
                presentation::update_terminator_meshes
                    .after(presentation::orient_bodies),
                presentation::update_wireframe_lighting
                    .after(presentation::orient_bodies)
                    .after(presentation::build_star_lighting_cache),
                presentation::update_occluder_lighting
                    .after(presentation::orient_bodies)
                    .after(presentation::build_star_lighting_cache),
                presentation::update_wireframe_thickness
                    .after(presentation::position_bodies),
                presentation::update_occluder_scale
                    .after(presentation::update_wireframe_thickness),
                presentation::cleanup_orphaned_body_wireframes,
            ).in_set(PlanetariumUISet))
            // Body point mesh systems (distant body LOD)
            .add_systems(Update, (
                presentation::spawn_body_point_meshes,
                presentation::update_body_points
                    .after(presentation::position_bodies)
                    .after(presentation::build_star_lighting_cache),
                presentation::cleanup_orphaned_body_points,
                presentation::label_bodies
                    .after(presentation::position_bodies),
                presentation::draw_trajectory_marker_labels
                    .after(presentation::position_bodies)
                    .after(presentation::update_focused_trajectory_markers)
                    .after(presentation::update_mouse_hit_marker),
            ).in_set(PlanetariumUISet))
            // Celestial reference markers (Point of Aries, etc.)
            .add_systems(Update, (
                presentation::spawn_celestial_markers,
                presentation::update_celestial_markers
                    .after(presentation::position_bodies),
            ).in_set(PlanetariumUISet))
            // Starfield brightness updates (via buffer, not material mutation)
            .add_systems(Update, (
                presentation::update_starfield_brightness,
            ).in_set(PlanetariumUISet))
            // Asset loading
            .add_systems(Update, (load_assets).in_set(PlanetariumLoadingSet))
            .add_systems(OnExit(AppState::PlanetariumLoading), initial_trajectories)
            .add_systems(OnExit(AppState::Planetarium), (
                unload_simulation_objects,
                presentation::cleanup_celestial_markers,
                hide_settings_window_on_planetarium_exit,
                cleanup_planetarium,
            ))
        ;


    }
}

fn initial_trajectories(mut calcs: MessageWriter<CalculateTrajectory>) {
    calcs.write(CalculateTrajectory { selection: BodySelection::All });
}

fn load_assets(
    mut commands: Commands,
    ui_state: ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut cache: ResMut<AssetCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut star_materials: ResMut<Assets<BodyWireframeMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut universe: ResMut<Universe>,
    mut physics: ResMut<UniversePhysics>,
    mut sim_time: ResMut<SimTime>,
    mut esc_menu_context: ResMut<EscMenuContext>,
    mut unsaved: ResMut<UnsavedChanges>,
) {
    unsaved.0 = false;
    // Entering/loading planetarium should always start with this window hidden.
    esc_menu_context.settings_window_visible = false;
    
    if ui_state.current_save.is_none() {
        next_app_state.set(AppState::Planetarium);
        return;
    }

    let save = (ui_state.current_save.clone()).unwrap();
    let path = save.path;

    let universe_file: Option<UniverseFile> = UniverseFile::load_from_path(&path);
    if let Some(universe_file) = universe_file {
        let (new_universe, loaded_time) = Universe::from_file(&universe_file);
        universe.path = new_universe.path.clone();
        universe.clear_all();
        let _version = universe_file.contents.version; // TODO: Support multiple file format versions?

        // Apply loaded time to the actual resource
        sim_time.time = loaded_time.time;
        sim_time.step = loaded_time.step;
        sim_time.gui_speed = loaded_time.gui_speed;
        sim_time.max_frame_time = loaded_time.max_frame_time;
        sim_time.playing = false;

        physics.gravitational_constant = universe_file.contents.physics.gravitational_constant;
        view_settings.tags = HashMap::<String, TagState>::new();

        let bodies = universe_file.contents.bodies;
        for body in bodies {
            let id = body.id();
            let name = body.name();
            for tag in body.tags() {
                let default_state = if tag == "Major Moon" || tag == "Major Planet" || tag == "Minor Planet" {
                    TagState { shown: true, trajectory: true, ..Default::default() }
                } else if tag == "Barycenter" {
                    TagState { shown: false, trajectory: true, ..Default::default() }
                } else {
                    TagState::default()
                };
                view_settings.tags.entry(tag.clone()).or_insert(default_state).members.insert(id.clone());
            }
            // info!("{:?}", view_settings);
            universe.insert(name, id);
            body.spawn(&mut commands, &mut cache, &mut meshes, &mut materials, &mut star_materials, &mut images);
        }
    }

    next_app_state.set(AppState::Planetarium);
}

fn cleanup_planetarium(
    mut commands: Commands,
    trajectory_meshes: Query<Entity, With<TrajectoryMesh>>,
    body_point_meshes: Query<Entity, With<BodyPointMesh>>,
    trajectory_markers: Query<Entity, With<FocusedTrajectoryMarker>>,
    mut graph: ResMut<PhysicsGraph>,
    mut cache: ResMut<PositionCache>,
    mut sim_time: ResMut<SimTime>,
    mut view_settings: ResMut<ViewSettings>,
    mut focused_body_state: ResMut<FocusedBodyState>,
    mut hover_state: ResMut<HoverState>,
    mut metrics: ResMut<SimulationPerformanceMetrics>,
    mut universe: ResMut<Universe>,
    mut asset_cache: ResMut<AssetCache>,
    mut camera: Query<(&mut PlanetariumCamera, &mut Freecam)>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut unsaved: ResMut<UnsavedChanges>,
) {
    next_esc_state.set(EscMenuState::Closed);
    unsaved.0 = false;
    // Despawn orphaned presentation entities that aren't children of SimulationObject
    for entity in &trajectory_meshes {
        commands.entity(entity).despawn();
    }
    for entity in &body_point_meshes {
        commands.entity(entity).despawn();
    }
    for entity in &trajectory_markers {
        commands.entity(entity).despawn();
    }

    // Reset physics state
    graph.clear();
    graph.needs_rebuild = true;
    *cache = PositionCache::default();

    // Reset simulation clock
    *sim_time = SimTime::default();

    // Reset view and UI state
    *view_settings = ViewSettings::default();
    *focused_body_state = FocusedBodyState::default();
    *hover_state = HoverState::default();
    *metrics = SimulationPerformanceMetrics::default();

    // Clear universe maps so stale IDs don't linger
    universe.clear_all();
    universe.path = None;

    // Clear cached mesh/material handles
    asset_cache.meshes.clear();
    asset_cache.materials.clear();

    // Reset camera to free mode at origin
    if let Ok((mut pcam, mut fcam)) = camera.single_mut() {
        pcam.action = CameraAction::Free;
        fcam.bevy_pos = bevy::math::DVec3::new(20., 2., 0.);
    }
}

fn hide_settings_window_on_planetarium_exit(mut esc_menu_context: ResMut<EscMenuContext>) {
    esc_menu_context.settings_window_visible = false;
    esc_menu_context.restore_playing_on_close = false;
    esc_menu_context.was_playing_before_open = false;
}
