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
use crate::sim::world::{self, BodyEntities, SimSystem, Trajectories};
use crate::body::universe;
use crate::foundations::time::Instant;
pub(crate) use crate::camera::{PlanetariumCamera, PlanetariumCameraPlugin};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::presentation;

mod windows;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumSimulationSet;

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
            .init_resource::<BodyInfoState>()
            .init_resource::<SimSystem>()
            .init_resource::<BodyEntities>()
            .init_resource::<Trajectories>()
            .add_message::<CalculateTrajectory>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumSimulationSet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_plugins(PlanetariumCameraPlugin)
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::controls::control_window,
                    windows::body_edit::body_edit_window,
                    windows::body_info::body_info_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,
                    windows::camera::camera_window,

                    presentation::label_bodies,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            .add_systems(Update, (
                (
                    // Simulation, then views onto it. Everything after `advance_simulation`
                    // only reads the arena.
                    world::advance_simulation.after(universe::advance_time),
                    world::sync_body_entities.after(world::advance_simulation),
                    world::calculate_trajectories.after(world::advance_simulation),
                    world::sync_transforms
                        .after(world::sync_body_entities),
                    world::sync_rotations.after(world::sync_transforms),
                    presentation::adjust_lights,
                    presentation::render_axes.after(world::sync_rotations),
                    presentation::render_trajectories.after(world::sync_transforms),
                ).in_set(PlanetariumUISet),
                (
                    universe::advance_time,
                ).in_set(PlanetariumSimulationSet),
                (load_assets).in_set(PlanetariumLoadingSet),
            ))
            .add_systems(OnExit(AppState::PlanetariumLoading), initial_trajectories)
            .add_systems(OnExit(AppState::Planetarium), (unload_simulation_objects, world::forget_body_entities))
        ;


    }
}

fn initial_trajectories(mut calcs: MessageWriter<CalculateTrajectory>) {
    calcs.write(CalculateTrajectory { selection: BodySelection::All });
}

fn load_assets(
    mut ui_state: ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut universe: ResMut<Universe>,
    mut physics: ResMut<UniversePhysics>,
    mut sim_time: ResMut<SimTime>,
    mut system: ResMut<SimSystem>,
    mut entities: ResMut<BodyEntities>,
    mut trajectories: ResMut<Trajectories>,
) {
    if ui_state.current_save.is_none() {
        next_app_state.set(AppState::Planetarium);
        return;
    }

    let save = (ui_state.current_save.clone()).unwrap();
    let path = save.path;

    let Some(universe_file) = UniverseFile::load_from_path(&path) else {
        next_app_state.set(AppState::Planetarium);
        return;
    };

    let (new_universe, _) = Universe::from_file(&universe_file);
    universe.path = new_universe.path.clone();
    universe.clear_all();

    sim_time.time = Instant::from_julian_day(universe_file.contents.time.time_julian_days);
    sim_time.playing = false;
    physics.gravitational_constant = universe_file.contents.physics.gravitational_constant;
    view_settings.tags = HashMap::<String, TagState>::new();

    for body in &universe_file.contents.bodies {
        let id = body.id();
        for tag in body.tags() {
            view_settings.tags.entry(tag.clone()).or_default().members.push(id.clone());
        }
        universe.insert(body.name(), id);
    }

    // Build the arena. Entities are spawned from it by `world::sync_body_entities`, which
    // notices the generation change; nothing is spawned here.
    match em_sim::system::System::from_contents(&universe_file.contents) {
        Ok(built) => {
            system.0 = built;
            trajectories.0.clear();
            // Full reset, not just the map: the generation must also be forgotten, or a
            // new system that happens to start at the same generation would be skipped.
            *entities = BodyEntities::default();
        }
        Err(err) => error!("could not load {}: {err}", path.display()),
    }

    next_app_state.set(AppState::Planetarium);
}
