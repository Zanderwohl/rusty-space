use bevy::app::{App, Plugin, Update};
use iyes_perf_ui::PerfUiPlugin;
use bevy::prelude::*;
use iyes_perf_ui::entries::{PerfUiFixedTimeEntries, PerfUiFramerateEntries, PerfUiWindowEntries};
use bevy::input::ButtonInput;
use crate::gui::common;

pub struct DebugPlugin;

#[derive(Component)]
struct DebugUI;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_state(DebugState::Off)
            .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin { max_history_length: 100, smoothing_factor: 2.0 / 101.0})
            .add_plugins(PerfUiPlugin)
            .add_systems(OnEnter(DebugState::Off), common::despawn_recursive_entities_with::<DebugUI>)
            .add_systems(OnEnter(DebugState::AllPerf), add_all_perf)
            .add_systems(Update, toggle_perf)
            .init_state::<DebugState>()
        ;
    }
}

fn add_all_perf(mut commands: Commands) {
    commands.spawn((
        DebugUI,
        PerfUiFramerateEntries::default(),
        PerfUiWindowEntries::default(),
        PerfUiFixedTimeEntries::default(),
    ));
}

fn toggle_perf(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<DebugState>>,
    mut next_state: ResMut<NextState<DebugState>>,
) {
    if keyboard.just_pressed(KeyCode::F3) {
        match state.get() {
            DebugState::Off => {
                next_state.set(DebugState::AllPerf);
            },
            DebugState::AllPerf => {
                next_state.set(DebugState::Off);
            },
        }
    }
}

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
enum DebugState {
    #[default]
    Off,
    AllPerf,
}
