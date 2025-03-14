use std::fs::File;
use std::io::{BufReader, Read};
use bevy::app::{App, Update};
use bevy::prelude::{in_state, Assets, Commands, Image, IntoSystemSetConfigs, Mesh, NextState, OnExit, Plugin, Res, ResMut, StandardMaterial, SystemSet, Time};
use bevy::prelude::IntoSystemConfigs;
use bevy_egui::{egui, EguiContexts};
use lazy_static::lazy_static;
use num_traits::{FloatConst, Pow};
use regex::Regex;
use crate::body::appearance::AssetCache;
use crate::body::universe::save::UniverseFile;
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::{Settings, UiTheme};
use crate::body::unload_simulation_objects;

pub mod time;
mod display;
mod spin;
mod controls;
mod body_edit;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumLoadingSet;

pub struct Planetarium;

impl Plugin for Planetarium {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SimTime>()
            .init_resource::<AssetCache>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_systems(Update, (
                (planetarium_ui, advance_time).in_set(PlanetariumUISet),
                (load_assets).in_set(PlanetariumLoadingSet),
            ))
            .add_systems(OnExit(AppState::Planetarium), unload_simulation_objects)
        ;
    }
}

lazy_static! {
    static ref SCI_RE: Regex = Regex::new(r"\d?\.\d+\s?x\s?10\s?\^\s?\d+").unwrap();
}

fn planetarium_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut time: ResMut<SimTime>,
) {
    let ctx = contexts.ctx_mut();
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::Window::new("Controls")
        .vscroll(true)
        .show(ctx, |ui| {
            controls::planetarium_controls(next_app_state, next_menu_state, &mut time, ui, ui_state);
    });

    // Start collapsed: https://github.com/emilk/egui/pull/5661
    egui::Window::new("Settings")
        .vscroll(true)
        .show(ctx, |ui| {
            crate::gui::menu::settings::settings_panel(&mut settings, ui);
        });

    if settings.windows.spin {
        spin::spin_gravity_calculator(&mut settings, ctx);
    }

    if settings.windows.body_edit {
        body_edit::body_edit_window(&mut settings, ctx);
    }
}

fn advance_time(mut sim_time: ResMut<SimTime>, time: Res<Time>) {
    if sim_time.playing {
        sim_time.previous_time = sim_time.time;
        sim_time.time += sim_time.gui_speed * time.delta_secs_f64();
    }
}

fn load_assets(
    mut commands: Commands,
    mut ui_state: ResMut<UiState>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut cache: ResMut<AssetCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut universe: ResMut<Universe>,
) {
    if ui_state.current_save.is_none() {
        next_app_state.set(AppState::Planetarium);
        return;
    }

    let save = (ui_state.current_save.clone()).unwrap();
    let path = save.path;

    let universe_file: Option<UniverseFile> = UniverseFile::load_from_path(&path);
    if let Some(universe_file) = universe_file {
        let (new_universe, sim_time) = Universe::from_file(&universe_file);
        universe.path = new_universe.path.clone();
        let version = universe_file.contents.version; // TODO: Support multiple file format versions?
        let time = universe_file.contents.time; // TODO: What.
        let bodies = universe_file.contents.bodies;
        for body in bodies {
            body.spawn(&mut commands, &mut cache, &mut meshes, &mut materials, &mut images);
        }
    }

    next_app_state.set(AppState::Planetarium);
}
