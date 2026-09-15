//! The Bevy layer: states, resources, and the systems that carry actions.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::DVec3;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use lc_world::sky::{AuthoredStars, StarProvider};

use em_render::body_surface_material::BodySurfaceMaterialPlugin;
use em_render::population_material::PopulationMaterialPlugin;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterialPlugin;
use em_render::render_space::sim_to_render;

use crate::action::{Action, Effect, apply, refresh_exposure};
use crate::input::{Looking, Requested, grab_cursor, look_around, read_keys};
use crate::panels;
use crate::session::Session;
use crate::starfield::{Bodies, spawn_sky, update_bodies, update_sky};
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

/// Which sky to load, if any.
///
/// An **asset path**, not a filesystem path: the asset server reads a file on the desktop and
/// fetches over HTTP in a browser, and this code cannot tell which. A `.csv` is still accepted
/// on native builds with the `hyg` feature, which is how the catalogue gets packed in the
/// first place.
#[derive(Resource, Default)]
pub struct Catalogue(pub Option<String>);

/// The sky asset in flight, while [`AppState::Loading`] waits for it.
#[derive(Resource)]
struct LoadingSky(Handle<crate::sky_asset::Sky>);

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
    /// Put the ship beside a body of the local system, by name. There is no action for this and
    /// there never will be; it exists so a thing too small to fly to can be looked at.
    pub at_body: Option<String>,
    /// Put the ship straight onto a station, by [`crate::navigation::Course::parse`] spelling.
    /// The same courses the interface offers, without the crossing in between.
    pub station: Option<String>,
    pub screenshot: Option<String>,
    /// Frames to let the sky settle before the shutter. Pipelines compile lazily.
    pub after_frames: u32,
    /// How many consecutive frames to photograph. More than one for diagnosing a flicker.
    pub burst: u32,
    /// A menu page to open on arrival. The only way to photograph one that draws over the
    /// root, which an action running on entering the sky cannot reach.
    pub menu_page: Option<crate::ui::MenuPage>,
    /// Open the password form on arrival, for the same reason.
    pub open_password_form: bool,
    /// Run once on reaching the sky. Actions rather than flags, so a development entry can
    /// reach anything the interface can and needs no plumbing of its own.
    pub actions: Vec<Action>,
}

/// How many stars the session keeps.
pub const SKY_LIMIT: usize = 6000;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            EguiPlugin::default(),
            RelativisticStarfieldMaterialPlugin,
            PopulationMaterialPlugin,
            BodySurfaceMaterialPlugin,
            crate::sky_asset::SkyAssetPlugin,
            crate::menu::MainMenuPlugin,
        ))
            .init_state::<AppState>()
            .add_message::<Requested>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .init_resource::<Catalogue>()
            .init_resource::<DevEntry>()
            .init_resource::<Looking>()
            .init_resource::<Bodies>()
            .init_resource::<crate::envelope::Envelopes>()
            .init_resource::<crate::resolved::Resolved>()
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::Loading), begin_load)
            .add_systems(OnEnter(AppState::InGame), (spawn_sky, run_dev_actions))
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(
                Update,
                (
                    boot.run_if(in_state(AppState::Boot)),
                    finish_load.run_if(in_state(AppState::Loading)),
                    // Not gated on a state: `--menu --shot` photographs the menu, and the
                    // system does nothing unless a path was asked for.
                    photograph,
                    place_at_body.run_if(in_state(AppState::InGame)),
                    place_on_station.run_if(in_state(AppState::InGame)),
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
            .add_systems(
                Update,
                (
                    aim_camera,
                    update_sky,
                    update_bodies,
                    // After the bodies, because it meters them; before the surfaces, because
                    // they are shaded against what it places.
                    crate::resolved::sample_scene,
                    crate::resolved::update_resolved,
                    crate::envelope::update_envelopes,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            // The menu's backdrop is the same starfield pass, so it needs the same two
            // systems. Nothing else: there are no bodies and nothing to resolve.
            .add_systems(
                Update,
                (aim_camera, update_sky).chain().run_if(in_state(AppState::MainMenu)),
            )
            // Absent unless something inserted one: the browser build reads it off the page
            // before the app is built, and the desktop mints one from its device grant.
            .init_resource::<crate::Ticket>()
            .add_plugins(crate::pick::PickPlugin)
            .add_plugins(crate::uplink::UplinkPlugin)
            // Desktop only: a browser build arrives with a session.
            .add_plugins(SigninPlugins)
            .add_systems(
                EguiPrimaryContextPass,
                (
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
/// Camera near plane, in render units of one astronomical unit. Fifteen metres.
///
/// Anything nearer than this is clipped, so it is the closest a ship can come to a surface.
pub const NEAR_PLANE: f32 = 1.0e-10;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // A system spans a hundred thousand astronomical units and the render unit is one, so
        // the default thousand-unit far plane would clip everything past Saturn.
        //
        // The near plane is fifteen metres. It has to be, because a low orbit is a fraction of
        // a planetary radius above the surface: at 1e-5 units the near plane stood a million
        // and a half metres off, which is further than a low orbit of Earth, and the sphere
        // was clipped away to nothing while its billboard still drew. Reversed float depth
        // costs nothing for the range — precision is relative, not absolute.
        Projection::Perspective(PerspectiveProjection {
            near: NEAR_PLANE,
            far: 1.0e9,
            ..default()
        }),
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
    let Some(cruise) = &game.cruise() else {
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

/// Put the ship on a station named by `--station`, without flying it there.
///
/// The crossing is what `--station` skips: a course to Neptune is two months of coordinate
/// time, and a screenshot of a place should not have to wait for it.
fn place_on_station(
    dev: Res<DevEntry>,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(spec) = &dev.station else { return };
    let Some(course) = crate::navigation::Course::parse(spec) else {
        ui.notify(format!("no such course: {spec}"), 0.0);
        *done = true;
        return;
    };
    let now = game.coordinate_time_s();
    let Some(system) = game.system.clone() else { return };
    let here = game.ship.motion.position_ly;
    let Some(waypoint) = course.resolve(&system, here, now) else { return };
    // Aimed at where the ship already is, so `--station` lands on the near side of an orbit
    // rather than wherever the clock had it.
    let waypoint = waypoint.nearest_to(here, &system, now);
    let Some(at) = waypoint.place_at(&system, now) else { return };
    let label = waypoint.label();
    ui.focus = course.target();
    if let Some(look) =
        waypoint.focus(&system, now).and_then(|f| crate::ui::Look::aimed_at(f - at))
    {
        ui.look = look;
    }
    game.0.place_at(at);
    game.0.ship.motion.begin_holding(waypoint);
    ui.notify(format!("on station: {label}"), game.coordinate_time_s());
    *done = true;
}

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

/// Development entry: stand off from a named body, once its system has loaded.
fn place_at_body(
    dev: Res<DevEntry>,
    bodies: Res<crate::starfield::Bodies>,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(want) = &dev.at_body else { return };
    let Some(body) = bodies.drawn.iter().find(|d| &d.name == want) else { return };
    // Far enough out that the body is a disc rather than a wall. Rings reach a couple of
    // planetary radii, so this has to clear them.
    let stand_off = body.radius_m * 12.0 / crate::system::M_PER_LY;
    let origin = game.system.as_ref().map(|s| s.origin_ly).unwrap_or_default();
    let from_star = (body.position_ly - origin)
        .normalize_or_zero();
    // Off to the side and a little sunward, so the body shows a terminator. Straight out from
    // the star is the night side, which is a correct view of nothing.
    let across = from_star.cross(DVec3::Z).normalize_or_zero();
    let offset = (across * 0.9 - from_star * 0.45).normalize_or_zero();
    game.place_at(body.position_ly + offset * stand_off);
    if let Some(look) = crate::ui::Look::aimed_at(-offset) {
        ui.look = look;
    }
    ui.notify(format!("standing off {want}"), game.coordinate_time_s());
    *done = true;
}

/// Photograph the sky through the real pipeline, then quit.
/// `shot.png` and 2 becomes `shot.2.png`.
fn numbered(path: &str, index: u32) -> String {
    match path.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem}.{index}.{extension}"),
        None => format!("{path}.{index}"),
    }
}

fn photograph(
    mut commands: Commands,
    dev: Res<DevEntry>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &dev.screenshot else { return };
    *frames += 1;
    let burst = dev.burst.max(1);
    if (dev.after_frames..dev.after_frames + burst).contains(&*frames) {
        // Consecutive frames of one run, which is the only way to see a flicker: two runs
        // stopped at frame n and frame n+1 have accumulated different wall time and are not
        // consecutive at all.
        let index = *frames - dev.after_frames;
        let at = if burst > 1 { numbered(path, index) } else { path.clone() };
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(at));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if *frames > dev.after_frames + burst + 30 {
        exit.write(AppExit::Success);
    }
}

/// Starts the sky loading, or finishes immediately when there is nothing to load.
fn begin_load(
    catalogue: Res<Catalogue>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    mut next: ResMut<NextState<AppState>>,
) {
    match catalogue.0.as_deref() {
        // Packing is native tooling and reads a file directly; see `skypack`. Absent from a
        // browser build, where the feature is off and `csv` is not in the tree at all.
        #[cfg(feature = "hyg")]
        Some(path) if path.ends_with(".csv") => {
            match lc_world::sky::hyg::HygProvider::load(path) {
                Ok(p) => enter_game(&mut game, &mut ui, &mut next, &p),
                Err(e) => {
                    ui.notify(format!("catalogue: {e}"), 0.0);
                    enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample());
                }
            }
        }
        Some(path) => {
            commands.insert_resource(LoadingSky(assets.load(path.to_owned())));
        }
        None => enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample()),
    }
}

/// Waits for the sky, then builds the session.
///
/// A frozen window is not a loading screen, so this is a polled system rather than a blocking
/// read: the loading panel keeps drawing while the fetch is in flight.
fn finish_load(
    loading: Option<Res<LoadingSky>>,
    skies: Res<Assets<crate::sky_asset::Sky>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(loading) = loading else { return };
    if let Some(sky) = skies.get(&loading.0) {
        if sky.skipped > 0 {
            ui.notify(format!("{} sky records were unusable", sky.skipped), 0.0);
        }
        enter_game(&mut game, &mut ui, &mut next, sky);
        commands.remove_resource::<LoadingSky>();
    } else if let Some(state) = assets.get_load_state(&loading.0)
        && state.is_failed()
    {
        // A sky that will not load is worth saying out loud rather than silently becoming
        // three hand-written stars.
        ui.notify("sky failed to load; using the sample", 0.0);
        enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample());
        commands.remove_resource::<LoadingSky>();
    }
}

fn enter_game(
    game: &mut Game,
    ui: &mut Ui,
    next: &mut NextState<AppState>,
    provider: &dyn StarProvider,
) {
    let count = provider.len();
    game.0 = Session::new(provider, SKY_LIMIT);
    ui.notify(format!("{count} stars loaded"), 0.0);
    ui.screen = Screen::InGame;
    next.set(AppState::InGame);
}

/// Carry every request through the one dispatcher.
fn dispatch(
    mut requests: MessageReader<Requested>,
    mut ui: ResMut<Ui>,
    mut game: ResMut<Game>,
    mut uplink: ResMut<crate::uplink::Uplink>,
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
                Effect::Send(order) => {
                    // A ship the server has not named is a ship this client does not have, so
                    // there is nothing to send an order for.
                    if let Some(ship_id) = uplink.joined().map(|joined| joined.ship_id) {
                        uplink.say(lc_proto::Inbound::Act(lc_proto::Intent {
                            ship_id,
                            order,
                            // Advisory, and clamped on arrival. Saying now is the honest
                            // claim: the client is acting on all it has been told so far.
                            issued_at_client_t: (game.coordinate_time_s() * 1e6) as i64,
                        }));
                    }
                }
                // Handled in `signin_ui`, which has the socket, the browser and the vault.
                // Nothing here, rather than nothing anywhere: in a browser the page that
                // launched the game already has a session and these never fire.
                Effect::SignIn
                | Effect::CancelSignIn
                | Effect::SignInWithPassword { .. }
                | Effect::SignOut => {}
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
            // The dispatcher routes orders to it. Absent, not connected: these tests are the
            // single-process game, which is what a session with no server still is.
            .init_resource::<crate::uplink::Uplink>()
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
        assert!(app.world().resource::<Game>().cruise().is_some(), "the crossing should have begun");

        for _ in 0..64 {
            app.update();
        }
        let game = app.world().resource::<Game>();
        assert!(game.distance_to(game.star(id).unwrap()) < before, "the ship did not move");
        assert!(game.ship.motion.beta.length() > 0.0, "and it is not under way");
        assert!(game.ship.motion.clock_s < game.coordinate_time_s(), "the ship clock should lag");
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

/// The desktop sign-in, or nothing at all in a browser.
///
/// A plugin group of one rather than a `#[cfg]` inside `build`, so the browser build does not
/// carry a branch about a thing it cannot do.
struct SigninPlugins;

impl Plugin for SigninPlugins {
    #[cfg(not(target_arch = "wasm32"))]
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::signin_ui::SigninPlugin);
    }

    #[cfg(target_arch = "wasm32")]
    fn build(&self, _app: &mut App) {}
}
