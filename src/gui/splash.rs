use bevy::app::App;
use bevy::prelude::*;
use bevy::time::TimerMode;
use bevy::ui::{AlignContent, AlignSelf, JustifyContent, JustifySelf};
use crate::gui::app::AppState;
use crate::gui::common::despawn_entities_with;

#[derive(Component)]
struct SplashScreen;

pub struct SplashPlugin;

impl Plugin for SplashPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_systems(OnEnter(AppState::Splash), splash_setup)
            .add_systems(Update, countdown.run_if(in_state(AppState::Splash)))
            .add_systems(OnExit(AppState::Splash), despawn_entities_with::<SplashScreen>)
        ;
    }
}

#[derive(Resource, Deref, DerefMut)]
struct SplashTimer(Timer);

fn splash_setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let icon = asset_server.load("logo.png");

    commands
        .spawn((Node {
            align_content: AlignContent::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(50.0),
            height: Val::Percent(50.0),
            align_self: AlignSelf::Center,
            justify_self: JustifySelf::Center,
            ..Default::default()
        },
        SplashScreen))
        .with_children(|parent| {
            parent.spawn(ImageNode {
                image: icon,
                ..Default::default()
            });
        })
    ;

    commands.insert_resource(SplashTimer(Timer::from_seconds(0.5, TimerMode::Once)))
}

// Tick the timer, and change state when finished
fn countdown(
    mut game_state: ResMut<NextState<AppState>>,
    time: Res<Time>,
    mut timer: ResMut<SplashTimer>,
) {
    if timer.tick(time.delta()).finished() {
        game_state.set(AppState::MainMenu);
    }
}
