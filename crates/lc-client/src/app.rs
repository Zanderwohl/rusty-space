//! The Bevy layer: states, resources, and the systems that carry actions.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::DVec3;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use lc_world::sky::{AuthoredStars, StarProvider};

use em_render::relativistic_starfield_material::RelativisticStarfieldMaterialPlugin;
use em_render::render_space::sim_to_render;

use crate::action::{Action, Effect, apply, refresh_exposure};
use crate::input::{Looking, Requested, grab_cursor, look_around, read_keys};
use crate::panels;
use crate::session::Session;
use crate::starfield::{spawn_sky, update_sky};
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

/// Development entry: skip the menu, and optionally photograph the sky and quit.
///
/// The renderer's output is the one thing that cannot be asserted from a test, and a window
/// nobody is looking at proves nothing. This makes the running client produce the same kind of
/// artifact the headless snapshot does -- a PNG -- except through the actual pipeline.
#[derive(Resource, Default)]
pub struct DevEntry {
    pub observe_immediately: bool,
    /// Point at the nearest star carrying a swarm, for showing the thing off.
    pub target_swarm: bool,
    pub screenshot: Option<String>,
    /// Frames to let the sky settle before the shutter. Pipelines compile lazily.
    pub after_frames: u32,
    /// Run once on reaching the sky. Actions rather than flags, so a development entry can
    /// reach anything the interface can and needs no plumbing of its own.
    pub actions: Vec<Action>,
}

/// How many stars the session keeps.
pub const SKY_LIMIT: usize = 6000;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EguiPlugin::default(), RelativisticStarfieldMaterialPlugin))
            .init_state::<AppState>()
            .add_message::<Requested>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .init_resource::<Catalogue>()
            .init_resource::<DevEntry>()
            .init_resource::<Looking>()
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::Loading), load_world)
            .add_systems(OnEnter(AppState::InGame), (spawn_sky, run_dev_actions))
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(
                Update,
                (
                    boot.run_if(in_state(AppState::Boot)),
                    photograph.run_if(in_state(AppState::InGame)),
                    (read_keys, grab_cursor, look_around).chain().run_if(in_state(AppState::InGame)),
                    dispatch,
                    // The clock is deliberately not gated on any panel or overlay. See
                    // lightcone/docs/13-client-shell.md: the game does not pause.
                    advance_clock.run_if(in_state(AppState::InGame)),
                    observe.run_if(in_state(AppState::InGame)),
                    hold_exposure.run_if(in_state(AppState::InGame)),
                )
                    .chain(),
            )
            .add_systems(Update, (aim_camera, update_sky).chain().run_if(in_state(AppState::InGame)))
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

/// The camera the sky is drawn for.
///
/// HDR with bloom is not decoration here: the tone map deliberately pushes anything above the
/// displayed window past the knee, so overflow has to become a halo somewhere. `min_radius`
/// keeps a faint star to a couple of pixels rather than letting bloom eat the field.
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}

/// Point the camera where the interface says it is looking.
///
/// The camera never translates. Distance to a star is tens of trillions of kilometres and no
/// float holds that next to a render unit, so the ship stays at the render origin and the sky
/// moves around it; what changes when the ship flies is the direction to each star.
fn aim_camera(ui: Res<Ui>, mut camera: Query<&mut Transform, With<Camera3d>>) {
    let Ok(mut transform) = camera.single_mut() else { return };
    let forward = sim_to_render(ui.look.forward()).as_vec3();
    let up = sim_to_render(DVec3::Z).as_vec3();
    transform.look_to(forward, up);
}

/// Keep the exposure window on the scene while the scene's brightness is moving.
///
/// Doppler beaming is the reason: at 0.996c the forward sky is some eighteen stops brighter
/// than it was at rest, which is a white screen with a fixed window. Re-placed on progress
/// rather than every frame, because placing it costs a pass over every star.
fn hold_exposure(ui: Res<Ui>, mut game: ResMut<Game>, mut last: Local<f64>) {
    let Some(cruise) = &game.cruise else {
        *last = -1.0;
        return;
    };
    let progress = cruise.progress(game.coordinate_time_s());
    if (progress - *last).abs() < EXPOSURE_HOLD_STEP {
        return;
    }
    *last = progress;
    refresh_exposure(&ui.0, &mut game.0);
}

/// How far a crossing runs between re-exposures, as a fraction of it.
const EXPOSURE_HOLD_STEP: f64 = 0.002;

/// One frame of boot, so the window is up before anything slow happens.
fn boot(mut next: ResMut<NextState<AppState>>, mut ui: ResMut<Ui>, dev: Res<DevEntry>) {
    if dev.observe_immediately {
        ui.screen = Screen::Loading;
        next.set(AppState::Loading);
        return;
    }
    ui.screen = Screen::MainMenu;
    next.set(AppState::MainMenu);
}

fn run_dev_actions(dev: Res<DevEntry>, game: Res<Game>, mut out: MessageWriter<Requested>) {
    for action in &dev.actions {
        out.write(Requested(action.clone()));
    }
    // Finding one is the game, so this is not an Action and no key reaches it. The dev entry
    // picks the star; selecting it goes through the same action as a click on the list.
    if dev.target_swarm {
        let found = game
            .stars
            .iter()
            .find(|s| lc_world::sky::generate::swarm_for(s).is_some())
            .map(|s| s.id);
        if let Some(id) = found {
            out.write(Requested(Action::SelectTarget(Some(id))));
        }
    }
}

/// Photograph the sky through the real pipeline, then quit.
fn photograph(
    mut commands: Commands,
    dev: Res<DevEntry>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &dev.screenshot else { return };
    *frames += 1;
    if *frames == dev.after_frames {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if *frames > dev.after_frames + 30 {
        exit.write(AppExit::Success);
    }
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

    /// A crossing driven entirely through the message path, with no window and no input
    /// device: select, fly, and let the schedule run it.
    #[test]
    fn a_crossing_runs_through_the_schedule() {
        let mut app = harness();
        let id = app.world().resource::<Game>().stars[0].id;
        let before = {
            let game = app.world().resource::<Game>();
            game.distance_to(game.star(id).unwrap())
        };

        app.world_mut().write_message(Requested(Action::SelectTarget(Some(id))));
        app.world_mut().write_message(Requested(Action::FlyTo(None)));
        app.update();
        assert!(app.world().resource::<Game>().cruise.is_some(), "the crossing should have begun");

        for _ in 0..64 {
            app.update();
        }
        let game = app.world().resource::<Game>();
        assert!(game.distance_to(game.star(id).unwrap()) < before, "the ship did not move");
        assert!(game.beta.length() > 0.0, "and it is not under way");
        assert!(game.ship_clock_s < game.coordinate_time_s(), "the ship clock should lag");
    }

    /// The camera turns; it does not travel. Everything drawn is at a fixed radius around it.
    #[test]
    fn the_camera_only_ever_rotates() {
        let mut app = harness();
        app.add_systems(Update, aim_camera);
        let camera = app.world_mut().spawn((Camera3d::default(), Transform::default())).id();
        app.world_mut().write_message(Requested(Action::Look { yaw: 1.0, pitch: 0.4 }));
        app.update();
        let transform = *app.world().entity(camera).get::<Transform>().unwrap();
        assert_eq!(transform.translation, Vec3::ZERO, "the camera must stay at the origin");
        assert!(transform.rotation.is_finite() && transform.rotation.length() > 0.5);
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
