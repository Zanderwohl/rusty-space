//! The Bevy layer: states, resources, and the systems that carry actions.

use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use lc_world::sky::{AuthoredStars, StarProvider};

use crate::action::{Action, Effect, apply};
use crate::input::{Requested, read_keys};
use crate::panels;
use crate::session::Session;
use crate::ui::{Screen, UiState};

/// Where the application is. Not what is on top of it: panels are a separate set, because
/// nothing is modal.
#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Boot,
    MainMenu,
    Loading,
    InGame,
}

#[derive(Resource, Deref, DerefMut)]
pub struct Ui(pub UiState);

#[derive(Resource, Deref, DerefMut)]
pub struct Game(pub Session);

/// Where the catalogue is, if one is to be loaded.
#[derive(Resource, Default)]
pub struct Catalogue(pub Option<String>);

/// How many stars the session keeps.
pub const SKY_LIMIT: usize = 6000;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_state::<AppState>()
            .add_message::<Requested>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .init_resource::<Catalogue>()
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::Loading), load_world)
            .add_systems(
                Update,
                (
                    boot.run_if(in_state(AppState::Boot)),
                    read_keys.run_if(in_state(AppState::InGame)),
                    dispatch,
                    // The clock is deliberately not gated on any panel or overlay. See
                    // lightcone/docs/13-client-shell.md: the game does not pause.
                    advance_clock.run_if(in_state(AppState::InGame)),
                    observe.run_if(in_state(AppState::InGame)),
                )
                    .chain(),
            )
            .add_systems(Update, draw_sky.run_if(in_state(AppState::InGame)))
            .add_systems(
                EguiPrimaryContextPass,
                (
                    panels::main_menu.run_if(in_state(AppState::MainMenu)),
                    panels::loading.run_if(in_state(AppState::Loading)),
                    (panels::hud, panels::open_panels).run_if(in_state(AppState::InGame)),
                ),
            );
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 0.0, 0.0)));
}

/// One frame of boot, so the window is up before anything slow happens.
fn boot(mut next: ResMut<NextState<AppState>>, mut ui: ResMut<Ui>) {
    ui.screen = Screen::MainMenu;
    next.set(AppState::MainMenu);
}

fn load_world(
    catalogue: Res<Catalogue>,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    mut next: ResMut<NextState<AppState>>,
) {
    let provider: Box<dyn StarProvider> = match &catalogue.0 {
        #[cfg(feature = "hyg")]
        Some(path) => match lc_world::sky::hyg::HygProvider::load(path) {
            Ok(p) => Box::new(p),
            Err(e) => {
                ui.notify(format!("catalogue: {e}"), 0.0);
                Box::new(AuthoredStars::sample())
            }
        },
        _ => Box::new(AuthoredStars::sample()),
    };
    let count = provider.len();
    game.0 = Session::new(provider.as_ref(), SKY_LIMIT);
    ui.notify(format!("{count} stars loaded"), 0.0);
    ui.screen = Screen::InGame;
    next.set(AppState::InGame);
}

/// Carry every request through the one dispatcher.
fn dispatch(
    mut requests: MessageReader<Requested>,
    mut ui: ResMut<Ui>,
    mut game: ResMut<Game>,
    mut next: ResMut<NextState<AppState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let pending: Vec<Action> = requests.read().map(|r| r.0.clone()).collect();
    for action in pending {
        for effect in apply(action, &mut ui.0, &mut game.0) {
            match effect {
                Effect::Quit => {
                    exit.write(AppExit::Success);
                }
                Effect::StartGame => {
                    ui.screen = Screen::Loading;
                    next.set(AppState::Loading);
                }
                Effect::WriteSnapshot => {
                    ui.notify("snapshot is not wired to a path yet", game.coordinate_time_s());
                }
                Effect::Notify(text) => {
                    let at = game.coordinate_time_s();
                    ui.notify(text, at);
                }
            }
        }
    }
}

/// Advance coordinate time. Runs whatever is on screen.
fn advance_clock(time: Res<Time>, ui: Res<Ui>, mut game: ResMut<Game>) {
    game.advance(time.delta_secs_f64() * ui.time_rate.max(0.0));
}

fn observe(ui: Res<Ui>, mut game: ResMut<Game>) {
    if ui.selected.is_some() {
        let integration = ui.integration_s;
        game.observe(integration);
    }
}

/// Stars as gizmo points on a unit sphere around the camera.
///
/// Immediate mode for a first pass: no entities to manage, and the interesting part is
/// whether the shading and the retarded-time evaluation reach the screen at all.
fn draw_sky(mut gizmos: Gizmos, game: Res<Game>, camera: Query<&Transform, With<Camera3d>>) {
    let Ok(view) = camera.single() else { return };
    const RADIUS: f32 = 100.0;
    for star in game.sky() {
        let colour = crate::session::point_colour(&star.shaded);
        if colour.length_squared() <= 0.0 {
            continue;
        }
        let dir = star.position_ly.normalize_or_zero().as_vec3();
        if dir.length_squared() <= 0.0 {
            continue;
        }
        let at = view.translation + dir * RADIUS;
        let size = crate::session::point_size(&star.shaded, 0.35);
        gizmos.circle(
            Isometry3d::new(at, Quat::from_rotation_arc(Vec3::Z, -dir)),
            size,
            Color::srgb(colour.x, colour.y, colour.z),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Panel;

    /// A headless app, enough to run the schedule.
    fn harness() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<AppState>()
            .add_message::<Requested>()
            .add_message::<AppExit>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .add_systems(Update, (dispatch, advance_clock).chain());
        app.insert_state(AppState::InGame);
        app
    }

    /// The invariant of 13-client-shell.md, asserted rather than assumed.
    ///
    /// The idiomatic Bevy spelling of a pause is one `run_if` on this system, and a
    /// single-player instinct says a menu should stop the clock. Once a server owns time,
    /// anything that learned to pause is broken, so it is checked here.
    #[test]
    fn the_clock_does_not_stop_for_any_panel() {
        let mut app = harness();
        for panel in Panel::ALL {
            app.world_mut().resource_mut::<Ui>().open(panel);
        }
        let before = app.world().resource::<Game>().coordinate_time_s();
        for _ in 0..8 {
            app.update();
        }
        let after = app.world().resource::<Game>().coordinate_time_s();
        assert!(after > before, "time stopped while panels were open: {before} -> {after}");
    }

    #[test]
    fn a_request_reaches_the_session_through_the_dispatcher() {
        let mut app = harness();
        let id = app.world().resource::<Game>().stars[0].id;
        app.world_mut().write_message(Requested(Action::SelectTarget(Some(id))));
        app.update();
        assert_eq!(app.world().resource::<Ui>().selected, Some(id));
        assert_eq!(app.world().resource::<Game>().pointing, Some(id));
    }

    #[test]
    fn quitting_asks_the_engine_rather_than_doing_it() {
        let mut app = harness();
        app.world_mut().write_message(Requested(Action::Quit));
        app.update();
        let exits = app.world().resource::<Messages<AppExit>>().len();
        assert_eq!(exits, 1);
    }

    #[test]
    fn a_zero_time_rate_is_a_debug_control_not_a_pause() {
        // Setting the rate to zero does stop the clock -- that is what the control is for.
        // The distinction is that no *panel* does it.
        let mut app = harness();
        app.world_mut().write_message(Requested(Action::SetTimeRate(0.0)));
        app.update();
        let before = app.world().resource::<Game>().coordinate_time_s();
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(app.world().resource::<Game>().coordinate_time_s(), before);
    }
}
