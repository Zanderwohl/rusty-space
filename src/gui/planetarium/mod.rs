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
use crate::sim::world::{self, SimSystem, BodyEntities, Trajectories, SimMetrics};
pub(crate) use crate::camera::{PlanetariumCamera, PlanetariumCameraPlugin, CameraAction};
use crate::camera::Freecam;
pub use crate::gui::planetarium::focused_body::FocusedBodyState;
pub use crate::gui::planetarium::focused_body::{HoverState, HoveredTrajectoryMarkerKind, TrajectoryHitData};
pub use crate::gui::planetarium::windows::mission_clock::{MissionClockMode, MissionClockSettings, format_sim_time_for_mode};
use crate::presentation::{self, TrajectoryMaterialPlugin, BodyWireframeMaterialPlugin, OccluderMaterialPlugin, BodyPointMaterialPlugin, StarfieldMaterialPlugin, LocalStarfieldMaterialPlugin, SoiPointsMaterialPlugin, SoiRingMaterialPlugin, SoiMeshes, EncounterMarkerMaterialPlugin, EncounterMarkerMesh, EncounterMarker, FlightPlans, ChainLeg, TrajectoryMesh, BodyPointMesh, SoiPointsMesh, SoiRingMesh, FocusedTrajectoryMarker, StarLightingFrameCache};
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
            .init_resource::<SimSystem>()
            .init_resource::<BodyEntities>()
            .init_resource::<Trajectories>()
            .init_resource::<SimMetrics>()
            .init_resource::<StarLightingFrameCache>()
            .init_resource::<SoiMeshes>()
            .init_resource::<EncounterMarkerMesh>()
            .init_resource::<FlightPlans>()
            .init_resource::<windows::right_panels::RightPanels>()
            .init_resource::<windows::body_panel::BodyPickerState>()
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
            .add_plugins(LocalStarfieldMaterialPlugin)
            .add_plugins(SoiPointsMaterialPlugin)
            .add_plugins(SoiRingMaterialPlugin)
            .add_plugins(EncounterMarkerMaterialPlugin)
            .add_plugins(EscapeMenuPlugin)
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::mission_clock::mission_clock_widget,
                    windows::controls::control_window,
                    windows::body_panel::viewer_window,
                    windows::body_panel::editor_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,
                    windows::show_hide_panel::show_hide_panel_widget,
                    windows::right_panels::right_panels_widget,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            // Core simulation and position systems
            .add_systems(Update, (
                presentation::adjust_lights,
                input::handle_go_to_shortcut,
                input::handle_revolve_frame_shortcut,
                world::advance_simulation,
                world::sync_body_entities.after(world::advance_simulation),
                world::calculate_trajectories,
                world::refresh_trajectories_on_arc_change
                    .after(world::advance_simulation)
                    .before(world::calculate_trajectories),
                world::sync_transforms
                    .after(world::advance_simulation)
                    .after(world::sync_body_entities),
                world::sync_rotations.after(world::sync_transforms),
            ).in_set(PlanetariumUISet))
            // Trajectory mesh systems
            .add_systems(Update, (
                presentation::spawn_trajectory_meshes_for_bodies,
                presentation::refresh_precessing_trajectories
                    .before(world::calculate_trajectories),
                presentation::rebuild_trajectory_caches
                    .after(world::calculate_trajectories),
                presentation::update_focused_trajectory_markers
                    .after(world::sync_transforms),
                presentation::update_mouse_hit_marker
                    .after(world::sync_transforms),
                presentation::build_trajectory_meshes
                    .after(world::sync_transforms)
                    .after(presentation::rebuild_trajectory_caches),
                presentation::update_trajectory_material_brightness,
                presentation::cleanup_orphaned_trajectory_meshes,
            ).in_set(PlanetariumUISet))
            // Body wireframe, occluder, and terminator mesh systems
            .add_systems(Update, (
                presentation::spawn_body_wireframe_meshes,
                presentation::spawn_body_occluders,
                presentation::spawn_terminator_meshes,
                presentation::build_star_lighting_cache
                    .after(world::sync_transforms),
                presentation::update_terminator_meshes
                    .after(world::sync_rotations),
                presentation::update_wireframe_lighting
                    .after(world::sync_rotations)
                    .after(presentation::build_star_lighting_cache),
                presentation::update_occluder_lighting
                    .after(world::sync_rotations)
                    .after(presentation::build_star_lighting_cache),
                presentation::update_wireframe_thickness
                    .after(world::sync_transforms),
                presentation::update_occluder_scale
                    .after(presentation::update_wireframe_thickness),
                presentation::cleanup_orphaned_body_wireframes,
            ).in_set(PlanetariumUISet))
            // Body point mesh systems (distant body LOD)
            .add_systems(Update, (
                presentation::spawn_body_point_meshes,
                presentation::update_body_points
                    .after(world::sync_transforms)
                    .after(presentation::build_star_lighting_cache),
                presentation::cleanup_orphaned_body_points,
                presentation::label_bodies
                    .after(world::sync_transforms),
                presentation::draw_trajectory_marker_labels
                    .after(world::sync_transforms)
                    .after(presentation::update_focused_trajectory_markers)
                    .after(presentation::update_mouse_hit_marker),
            ).in_set(PlanetariumUISet))
            // Sphere-of-influence shells for the focused body and its children
            .add_systems(Update, (
                presentation::spawn_soi_meshes,
                presentation::update_soi_shells
                    .after(world::sync_transforms),
                presentation::cleanup_orphaned_soi_meshes,
            ).in_set(PlanetariumUISet))
            // Sphere-of-influence crossing markers for the focused body
            .add_systems(Update, (
                presentation::spawn_encounter_markers,
                presentation::advance_flight_plan
                    .before(world::advance_simulation),
                presentation::update_encounter_markers
                    .after(world::sync_transforms),
                presentation::spawn_chain_legs,
                presentation::update_chain_legs
                    .after(world::sync_transforms),
                presentation::update_chain_leg_thickness
                    .after(presentation::update_chain_legs),
            ).in_set(PlanetariumUISet))
            // Celestial reference markers (Point of Aries, etc.)
            .add_systems(Update, (
                presentation::sync_celestial_markers,
                presentation::update_celestial_markers
                    .after(world::sync_transforms),
                presentation::render_axes
                    .after(world::sync_rotations),
            ).in_set(PlanetariumUISet))
            // Starfield brightness updates (via buffer, not material mutation)
            .add_systems(Update, (
                presentation::update_starfield_brightness,
                presentation::update_local_starfield
                    .after(world::sync_transforms),
            ).in_set(PlanetariumUISet))
            // Asset loading
            .add_systems(Update, (load_assets).in_set(PlanetariumLoadingSet))
            .add_systems(Update, (
                windows::right_panels::update_right_drawer,
                windows::right_panels::sync_panel_zones,
            ).in_set(PlanetariumUISet))
            .add_systems(OnExit(AppState::PlanetariumLoading), (
                initial_trajectories,
                windows::right_panels::spawn_right_drawer,
            ))
            .add_systems(OnExit(AppState::Planetarium), (
                unload_simulation_objects,
                presentation::cleanup_celestial_markers,
                hide_settings_window_on_planetarium_exit,
                cleanup_planetarium,
                presentation::clear_local_starfield,
                windows::right_panels::despawn_right_drawer,
            ))
        ;


    }
}

fn initial_trajectories(mut calcs: MessageWriter<CalculateTrajectory>) {
    calcs.write(CalculateTrajectory { selection: BodySelection::All });
}

fn load_assets(
    ui_state: ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut universe: ResMut<Universe>,
    mut physics: ResMut<UniversePhysics>,
    mut sim_time: ResMut<SimTime>,
    mut system: ResMut<SimSystem>,
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

    let universe_file = match UniverseFile::load_from_path(&path) {
        Ok(file) => file,
        Err(e) => {
            error!("{e}");
            next_app_state.set(AppState::MainMenu);
            return;
        }
    };

    // Build the arena before touching anything else. Everything below overwrites live
    // state, so a failure part-way used to leave the clock, physics, tags and name map
    // describing the new file while the simulation still held the old bodies.
    let loaded = match em_sim::system::System::from_contents(&universe_file.contents) {
        Ok(loaded) => loaded,
        Err(e) => {
            error!("could not build the simulation from {path:?}: {e}");
            next_app_state.set(AppState::MainMenu);
            return;
        }
    };

    let (new_universe, loaded_time) = Universe::from_file(&universe_file);
    universe.path = new_universe.path.clone();
    universe.clear_all();

    sim_time.time = loaded_time.time;
    sim_time.step = loaded_time.step;
    sim_time.gui_speed = loaded_time.gui_speed;
    sim_time.max_frame_time = loaded_time.max_frame_time;
    sim_time.playing = false;

    physics.gravitational_constant = universe_file.contents.physics.gravitational_constant;
    view_settings.tags = HashMap::<String, TagState>::new();

    for body in &universe_file.contents.bodies {
        let id = body.id();
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
        universe.insert(body.name(), id);
    }

    // `sync_body_entities` spawns the views on the next generation bump.
    system.0 = loaded;

    next_app_state.set(AppState::Planetarium);
}

fn cleanup_planetarium(
    mut commands: Commands,
    // Merged because Bevy caps a system at 16 parameters.
    orphans: Query<Entity, Or<(
        With<TrajectoryMesh>,
        With<BodyPointMesh>,
        With<FocusedTrajectoryMarker>,
        With<SoiPointsMesh>,
        With<SoiRingMesh>,
        With<EncounterMarker>,
        With<ChainLeg>,
    )>>,
    mut system: ResMut<SimSystem>,
    mut body_entities: ResMut<BodyEntities>,
    mut trajectories: ResMut<Trajectories>,
    mut sim_time: ResMut<SimTime>,
    mut view_settings: ResMut<ViewSettings>,
    mut focused_body_state: ResMut<FocusedBodyState>,
    mut hover_state: ResMut<HoverState>,
    mut metrics: ResMut<SimMetrics>,
    mut universe: ResMut<Universe>,
    mut asset_cache: ResMut<AssetCache>,
    mut camera: Query<(&mut PlanetariumCamera, &mut Freecam)>,
    mut next_esc_state: ResMut<NextState<EscMenuState>>,
    mut unsaved: ResMut<UnsavedChanges>,
) {
    next_esc_state.set(EscMenuState::Closed);
    unsaved.0 = false;
    // Despawn orphaned presentation entities that aren't children of SimulationObject
    for entity in &orphans {
        commands.entity(entity).despawn();
    }

    // `unload_simulation_objects` despawns the views; this forgets the mapping.
    *system = SimSystem::default();
    *body_entities = BodyEntities::default();
    *trajectories = Trajectories::default();

    // Reset simulation clock
    *sim_time = SimTime::default();

    // Reset view and UI state
    *view_settings = ViewSettings::default();
    *focused_body_state = FocusedBodyState::default();
    *hover_state = HoverState::default();
    *metrics = SimMetrics::default();

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
