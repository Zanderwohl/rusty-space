//! The Bevy layer: states, resources, and the systems that carry actions.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::DVec3;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::camera::Hdr;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};
use lc_world::knowledge::{Knowledge, Witness};
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
    /// The browser build could not reach its shard, or lost it. Terminal: there is no menu to
    /// fall back to, and reloading the page is the way back.
    Unreachable,
}

/// Whether this build has a main menu to start at and to leave the game for.
///
/// The browser build does not. The page that launches it has already signed the player in and
/// named the shard, so it opens straight into the world, and failing to reach that shard ends
/// at [`AppState::Unreachable`] rather than at a menu.
pub const HAS_MAIN_MENU: bool = cfg!(not(target_arch = "wasm32"));

/// The order a frame is built in. Every system in [`Update`] belongs to one of these.
///
/// The schedule used to declare none of this, and Bevy reported 133 pairs of systems with
/// conflicting access and no order between them. Most were harmless; one was not. [`survey`]
/// reads the camera the sky is about to be rendered with, and it was unordered against
/// [`aim_camera`], which writes it — so a reticle was drawn from whichever pose the executor
/// happened to leave in the component, and trailed the view by a frame whenever that was the
/// old one.
///
/// The same shape three times over since: the map's snapshot was taken in [`Stage::Act`] and
/// held the previous frame's eye while the contacts in it were this frame's, so this ship's own
/// mark trailed one frame behind everything around it. **Anything built from the scene belongs
/// after the scene is placed.**
///
/// [`survey`]: crate::pick
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// What the server has said since the last frame.
    Link,
    /// Input, orders, and the clock.
    Act,
    /// Where the camera points, and where everything is drawn.
    Scene,
    /// What the cursor is on, against the scene just placed.
    ///
    /// A click found here reaches [`dispatch`] on the *next* frame, which is the honest
    /// reading of it: the player clicked on the image they were looking at, and that image is
    /// the one this stage measures.
    Mark,
}

/// The scene is placed: the eye, the sky, the bodies, the hulls.
///
/// A set rather than the systems themselves. `place_eye` is registered twice — once for the
/// sky and once for the menu's backdrop — and Bevy refuses to order against a system type with
/// two instances in one schedule.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Placed;

#[derive(Resource, Deref, DerefMut)]
pub struct Ui(pub UiState);

#[derive(Resource, Deref, DerefMut)]
pub struct Game(pub Session);

/// Which sky to load, if any.
///
/// An **asset path**, not a filesystem path: the asset server reads a file on the desktop and
/// fetches over HTTP in a browser, and this code cannot tell which. A `.csv` is still accepted
/// on native builds with the `hyg` feature, which is how the catalog gets packed in the
/// first place.
#[derive(Resource, Default)]
pub struct Catalog(pub Option<String>);

/// The sky asset in flight, while [`AppState::Loading`] waits for it.
#[derive(Resource)]
struct LoadingSky(Handle<crate::sky_asset::Sky>);

/// How many stars the session keeps.
pub const SKY_LIMIT: usize = 6000;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            EguiPlugin::default(),
            RelativisticStarfieldMaterialPlugin,
            PopulationMaterialPlugin,
            em_render::plume_material::PlumeMaterialPlugin,
            BodySurfaceMaterialPlugin,
            crate::sky_asset::SkyAssetPlugin,
            crate::procedural::ProceduralTexturesPlugin,
            crate::library::LibraryPlugin,
            crate::faces::FacesPlugin,
            crate::map::MapPlugin,
            crate::bench::BenchPlugin,
            crate::haze::HazePlugin,
        ))
            // **Which camera egui draws on is not left to spawn order.**
            //
            // `bevy_egui` gives its primary context to the first camera an application
            // creates, and `spawn_camera` and the map's own setup are two `Startup` systems
            // with no order between them. Whichever won, the readout and every panel were
            // drawn onto that camera's target — and when the map won, the whole interface
            // went into a 512-pixel texture and the window showed the sky with nothing on it.
            //
            // The same lesson as `18-ui-style.md`'s Z-order note, one layer down: two systems
            // spawning into one frame have no order, so say which one you meant.
            .insert_resource(EguiGlobalSettings {
                auto_create_primary_context: false,
                ..default()
            })
            .init_state::<AppState>()
            .add_message::<Requested>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .init_resource::<Catalog>()
            .init_resource::<crate::dev::DevEntry>()
            .init_resource::<Looking>()
            .init_resource::<Bodies>()
            .init_resource::<crate::envelope::Envelopes>()
            .init_resource::<crate::hull::Eye>()
            .init_resource::<crate::hull::Hulls>()
            .init_resource::<crate::plume::Plumes>()
            .init_resource::<crate::resolved::Resolved>()
            .configure_sets(Update, (Stage::Link, Stage::Act, Stage::Scene, Stage::Mark).chain())
            .init_resource::<panels::HudFoot>()
            .init_resource::<crate::map_panel::WorldInset>()
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::Loading), begin_load)
            .add_systems(OnExit(AppState::InGame), crate::map_panel::release_world_frame)
            .add_systems(OnEnter(AppState::InGame), spawn_sky)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(
                Update,
                (
                    boot.run_if(in_state(AppState::Boot)),
                    finish_load.run_if(in_state(AppState::Loading)),
                    // Not gated on a state: `--menu --shot` photographs the menu, and the
                    // system does nothing unless a path was asked for.
                    crate::dev::photograph,
                    crate::dev::run_dev_actions.run_if(in_state(AppState::InGame)),
                    crate::dev::place_at_body.run_if(in_state(AppState::InGame)),
                    crate::dev::place_on_station.run_if(in_state(AppState::InGame)),
                    (
                        read_keys,
                        // Only while the world is the view being flown. In the map's mode the
                        // world is a thumbnail in the corner, and a drag over the map turning
                        // the ship behind it would be the two modes fighting over one pointer.
                        (grab_cursor, look_around, crate::input::read_wheel).chain().run_if(flying),
                    )
                        .chain()
                        .run_if(in_state(AppState::InGame)),
                    dispatch,
                    // After the dispatcher and after the look, because those are what it is
                    // overruling: a pin that ran before them would be undone by a hand on the
                    // mouse or by a crossing aiming itself, on the same frame.
                    crate::dev::frame_the_cast.run_if(in_state(AppState::InGame)),
                    crate::dev::open_the_radio.run_if(in_state(AppState::InGame)),
                    // After the framing, because a pin overrules everything including that.
                    crate::dev::pin_camera.run_if(in_state(AppState::InGame)),
                    crate::dev::pin_view.run_if(in_state(AppState::InGame)),
                    crate::dev::pin_map_camera.run_if(in_state(AppState::InGame)),
                    crate::dev::pin_map_focus.run_if(in_state(AppState::InGame)),
                    // The clock is deliberately not gated on any panel or overlay. See
                    // lightcone/docs/13-client-shell.md: the game does not pause.
                    advance_clock.run_if(in_state(AppState::InGame)),
                    // After the clock, so a contact is drawn at the same instant as the ship.
                    crate::uplink::reckon_contacts.run_if(in_state(AppState::InGame)),
                    observe.run_if(in_state(AppState::InGame)),
                    hold_exposure.run_if(in_state(AppState::InGame)),
                )
                    .chain()
                    .in_set(Stage::Act),
            )
            .add_systems(
                Update,
                (
                    // Before anything is placed for it: how much of the window the world's
                    // camera has decides what a pixel of it is worth.
                    crate::map_panel::frame_world,
                    // Everything below is drawn relative to the eye, and one placed against
                    // last frame's would shear the whole scene against the ship every time the
                    // view turned.
                    crate::hull::place_eye,
                    aim_camera,
                    update_sky,
                    update_bodies,
                    // After the bodies, because it meters them; before the surfaces, because
                    // they are shaded against what it places.
                    crate::resolved::sample_scene,
                    crate::resolved::update_resolved,
                    crate::envelope::update_envelopes,
                    // Last, because a hull is metered as part of the scene the exposure was
                    // just placed for.
                    crate::hull::update_hulls,
                    // And the exhaust after the ship, so it is placed against the same frame.
                    crate::plume::update_plumes,
                )
                    .chain()
                    .in_set(Stage::Scene)
                    .in_set(Placed)
                    .run_if(in_state(AppState::InGame)),
            )
            // The menu's backdrop is the same starfield pass, so it needs the same two
            // systems. Nothing else: there are no bodies and nothing to resolve.
            .add_systems(
                Update,
                (crate::hull::place_eye, aim_camera, update_sky)
                    .chain()
                    .in_set(Stage::Scene)
                    .run_if(in_state(AppState::MainMenu)),
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
                    // Before anything is laid out: it changes how every glyph is
                    // rasterized, and a pass that ran first would be measured hinted.
                    crate::faces::unhint,
                    // First of the drawing, so a frame that has the faces is drawn in them
                    // rather than the frame after it.
                    crate::faces::settle,
                    panels::loading.run_if(in_state(AppState::Loading)),
                    (panels::hud, crate::map_panel::draw, panels::open_panels,
                        crate::reader::draw)
                        .run_if(in_state(AppState::InGame)),
                    panels::unreachable.run_if(in_state(AppState::Unreachable)),
                )
                    .chain(),
            );
        if HAS_MAIN_MENU {
            app.add_plugins(crate::menu::MainMenuPlugin);
        } else {
            app.add_systems(
                Update,
                strand
                    .in_set(Stage::Act)
                    .run_if(not(in_state(AppState::Boot)))
                    .run_if(not(in_state(AppState::Unreachable))),
            )
            .add_systems(OnEnter(AppState::Unreachable), crate::input::release_cursor);
        }
    }
}

/// Whether the world is the view being flown, rather than the map's thumbnail.
fn flying(ui: Res<Ui>) -> bool {
    ui.view == crate::ui::ViewMode::World
}

/// The camera the sky is drawn for.
///
/// HDR with bloom is not decoration here: the tone map deliberately pushes anything above the
/// displayed window past the knee, so overflow has to become a halo somewhere. `min_radius`
/// keeps a faint star to a couple of pixels rather than letting bloom eat the field.
///
/// Named, rather than found by `With<Camera3d>`, because the map draws with a second one. Every
/// query for the eye takes `.single()`, which returns `Err` on two matches and is handled with
/// an early return — so an unnamed second camera does not produce a wrong picture, it produces
/// no picture, with nothing in the build to say why.
#[derive(Component)]
pub struct SkyCamera;

/// Camera near plane, in render units of one astronomical unit. Fifteen meters.
///
/// Anything nearer than this is clipped, so it is the closest a ship can come to a surface.
pub const NEAR_PLANE: f32 = 1.0e-10;

/// The camera the interface is drawn on.
///
/// Its own camera, and not the sky's. `bevy_egui` lays the interface out inside the viewport of
/// the camera holding the context, and the sky's viewport is the corner square while the map is
/// the view — so the whole readout went down into the corner with it, over a black window.
#[derive(Component)]
pub struct UiCamera;

fn spawn_camera(mut commands: Commands) {
    // Last, over the sky, and clearing nothing: it has the interface to draw and no scene.
    //
    // **`Hdr`, although it draws no scene.** Two cameras on one window share the texture they
    // draw into only when their format, sample count and usages all match, and `Hdr` is what
    // decides the format. Without it the interface got a texture of its own that nothing ever
    // cleared: every frame's readout was laid over the last until the words were a smear, with
    // the loading screen still underneath it a thousand frames later.
    commands.spawn((
        Camera2d,
        UiCamera,
        // See `EguiGlobalSettings` above for why this is said rather than left to spawn order.
        PrimaryEguiContext,
        Hdr,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
    ));

    commands.spawn((
        Camera3d::default(),
        SkyCamera,
        // A system spans a hundred thousand astronomical units and the render unit is one, so
        // the default thousand-unit far plane would clip everything past Saturn.
        //
        // The near plane is fifteen meters. It has to be, because a low orbit is a fraction of
        // a planetary radius above the surface: at 1e-5 units the near plane stood a million
        // and a half meters off, which is further than a low orbit of Earth, and the sphere
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
/// The camera never translates. Distance to a star is tens of trillions of kilometers and no
/// float holds that next to a render unit, so the ship stays at the render origin and the sky
/// moves around it; what changes when the ship flies is the direction to each star.
fn aim_camera(ui: Res<Ui>, mut transform: Single<&mut Transform, With<SkyCamera>>) {
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

/// One frame of boot, so the window is up before anything slow happens.
fn boot(
    mut next: ResMut<NextState<AppState>>,
    mut ui: ResMut<Ui>,
    dev: Res<crate::dev::DevEntry>,
    ticket: Res<crate::Ticket>,
) {
    // Once, here, because this is where the ticket is. It does not change while the client
    // runs: a sign-in that produced a different one would be a different session.
    #[cfg(feature = "godview")]
    {
        ui.may_see_everything = crate::map_source::may_see_everything(ticket.0.as_deref());
    }
    #[cfg(not(feature = "godview"))]
    let _ = &ticket;

    if dev.observe_immediately || !HAS_MAIN_MENU {
        ui.screen = Screen::Loading;
        next.set(AppState::Loading);
        return;
    }
    ui.screen = Screen::MainMenu;
    next.set(AppState::MainMenu);
}

/// Leave for the error screen once the shard is out of reach, for a build with nowhere else to go.
fn strand(
    uplink: Res<crate::uplink::Uplink>,
    address: Res<crate::uplink::ServerAddress>,
    mut ui: ResMut<Ui>,
    mut next: ResMut<NextState<AppState>>,
) {
    if let Some(why) = crate::uplink::out_of_reach(&uplink.state, address.0.as_deref()) {
        warn!("stranded: {why}");
        ui.screen = Screen::Unreachable;
        next.set(AppState::Unreachable);
    }
}

/// Starts the sky loading, or finishes immediately when there is nothing to load.
fn begin_load(
    dev: Res<crate::dev::DevEntry>,
    catalog: Res<Catalog>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    uplink: Res<crate::uplink::Uplink>,
    mut next: ResMut<NextState<AppState>>,
) {
    match catalog.0.as_deref() {
        // Packing is native tooling and reads a file directly; see `skypack`. Absent from a
        // browser build, where the feature is off and `csv` is not in the tree at all.
        #[cfg(feature = "hyg")]
        Some(path) if path.ends_with(".csv") => {
            match lc_world::sky::hyg::HygProvider::load(path) {
                Ok(p) => enter_game(&mut game, &mut ui, &mut next, &p, &uplink, dev.charted),
                Err(e) => {
                    ui.notify(format!("catalog: {e}"), 0.0);
                    enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink, dev.charted);
                }
            }
        }
        Some(path) => {
            commands.insert_resource(LoadingSky(assets.load(path.to_owned())));
        }
        None => enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink, dev.charted),
    }
}

/// Waits for the sky, then builds the session.
///
/// A frozen window is not a loading screen, so this is a polled system rather than a blocking
/// read: the loading panel keeps drawing while the fetch is in flight.
fn finish_load(
    dev: Res<crate::dev::DevEntry>,
    loading: Option<Res<LoadingSky>>,
    skies: Res<Assets<crate::sky_asset::Sky>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut ui: ResMut<Ui>,
    uplink: Res<crate::uplink::Uplink>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(loading) = loading else { return };
    if let Some(sky) = skies.get(&loading.0) {
        if sky.skipped > 0 {
            ui.notify(format!("{} sky records were unusable", sky.skipped), 0.0);
        }
        enter_game(&mut game, &mut ui, &mut next, sky, &uplink, dev.charted);
        commands.remove_resource::<LoadingSky>();
    } else if let Some(state) = assets.get_load_state(&loading.0)
        && state.is_failed()
    {
        // A sky that will not load is worth saying out loud rather than silently becoming
        // three hand-written stars.
        ui.notify("sky failed to load; using the sample", 0.0);
        enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink, dev.charted);
        commands.remove_resource::<LoadingSky>();
    }
}

fn enter_game(
    game: &mut Game,
    ui: &mut Ui,
    next: &mut NextState<AppState>,
    provider: &dyn StarProvider,
    uplink: &crate::uplink::Uplink,
    charted: bool,
) {
    let count = provider.len();
    // What the craft knows, and what its telescope is doing, survive the session being
    // replaced. With a shard they are a copy of what it has already said, and it does not say
    // a page twice: dropping them here would lose everything learned before the sky loaded.
    let knowledge = std::mem::replace(&mut game.0.knowledge, Knowledge::new(Witness(0)));
    let observatory = game.0.observatory.clone();
    game.0 = Session::new(provider, SKY_LIMIT);
    // The session was just replaced, and with it everything the server had said about where
    // and when this ship is. Put it back, or the client flies locally from the origin while
    // the interface still says LINKED. See `uplink::Placement`.
    uplink.place(&mut game.0);
    if game.0.remote {
        game.0.knowledge = knowledge;
        game.0.observatory = observatory;
    } else if charted {
        // Loaded is not known, and nothing issues charts on a path a player reaches. The flag
        // is the old charting office kept as a dev tool, because a ship that knows nothing
        // photographs nothing and `--focus` needs a body the panel lists.
        game.0.issue_charts(crate::session::CHARTED_LY);
    }
    let known = game.0.knowledge.len();
    ui.notify(format!("{count} stars loaded, {known} known"), 0.0);
    ui.screen = Screen::InGame;
    next.set(AppState::InGame);
}

/// Carry every request through the one dispatcher.
fn dispatch(
    mut requests: MessageReader<Requested>,
    mut ui: ResMut<Ui>,
    mut game: ResMut<Game>,
    mut uplink: ResMut<crate::uplink::Uplink>,
    time: Res<Time>,
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
                Effect::Stage(scenario) => {
                    uplink.say(lc_proto::Inbound::Stage { scenario });
                    uplink.asked(time.elapsed_secs_f64());
                }
                Effect::Grant(joules) => {
                    uplink.say(lc_proto::Inbound::Grant { joules });
                    uplink.asked(time.elapsed_secs_f64());
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
                        uplink.asked(time.elapsed_secs_f64());
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
/// Buy coordinate time with the real seconds that have passed.
///
/// The *virtual* clock, which Bevy clamps to a quarter of a second a frame, and the clamp is
/// load-bearing for a reason that is not the usual one. Nothing here is integrated, so a slow
/// frame does not threaten the physics — but the shard this client talks to is a thread on a
/// twenty-hertz timer that does not make up ticks it misses, so when the machine is busy the
/// *world* falls behind real time too. Losing the same quarter-second the server lost keeps the
/// two roughly together.
///
/// Measured, because the obvious change is the wrong one: taking `Time<Real>` here made the
/// client track real time perfectly and pull away from a server that could not, turning three
/// corrections of half a day into one of ninety-one days. What would actually fix it is slewing
/// this rate to the server's observed progress rather than trusting a nominal one — the first
/// tier of `lightcone/docs/17-reconciliation.md`, which does not exist yet.
fn advance_clock(time: Res<Time>, ui: Res<Ui>, mut game: ResMut<Game>) {
    game.advance(time.delta_secs_f64() * ui.time_rate.max(0.0));
}

/// Run whatever the telescope is committed to.
///
/// Not "take a measurement per frame": a duty turns elapsed coordinate time into exposure, so
/// what is recorded is the same whether the client is drawing at ninety frames a second or
/// ten. See [`Session::tick_instruments`](crate::session::Session::tick_instruments).
fn observe(ui: Res<Ui>, mut game: ResMut<Game>) {
    // With a shard the telescope is the shard's, and runs whether or not this client does.
    if game.remote {
        return;
    }
    let integration = ui.integration_s;
    game.tick_instruments(integration);
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
        assert_eq!(app.world().resource::<Game>().described, Some(id));
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
        let camera = app.world_mut().spawn((Camera3d::default(), SkyCamera, Transform::default())).id();
        app.world_mut().write_message(Requested(Action::Look { yaw: 1.0, pitch: 0.4 }));
        app.update();
        let transform = *app.world().entity(camera).get::<Transform>().unwrap();
        assert_eq!(transform.translation, Vec3::ZERO, "the camera must stay at the origin");
        assert!(transform.rotation.is_finite() && transform.rotation.length() > 0.5);
    }

    /// A second camera must not be able to stop the sky.
    ///
    /// Every system that wants the eye takes `.single()`, which returns `Err` on two matches —
    /// and every one of them handles that with an early return. So before [`SkyCamera`] existed,
    /// adding the map's camera did not draw a wrong picture, it drew no picture: the view froze,
    /// the stars sized to zero and nothing was pickable, with nothing in the build to say why.
    ///
    /// The map camera is stood in for by a bare `Camera3d`, because what is being asserted is
    /// that the marker is what disambiguates and not anything the map happens to carry.
    #[test]
    fn a_second_camera_does_not_stop_the_sky_turning() {
        let mut app = harness();
        app.add_systems(Update, aim_camera);
        let sky = app.world_mut().spawn((Camera3d::default(), SkyCamera, Transform::default())).id();
        app.world_mut().spawn((Camera3d::default(), Transform::default()));

        let before = *app.world().entity(sky).get::<Transform>().unwrap();
        app.world_mut().write_message(Requested(Action::Look { yaw: 1.0, pitch: 0.4 }));
        app.update();

        let after = *app.world().entity(sky).get::<Transform>().unwrap();
        assert_ne!(after.rotation, before.rotation, "the turn never reached the sky camera");
    }

    /// Where a mark placed in [`Stage::Mark`] found the camera.
    #[derive(Resource, Default)]
    struct Seen(Option<Vec3>);

    fn probe(camera: Query<&Transform, With<SkyCamera>>, mut seen: ResMut<Seen>) {
        seen.0 = camera.single().ok().map(|t| *t.forward());
    }

    /// The bug this pins: a reticle is drawn over a rendered sky, so it has to be measured
    /// against the camera that sky was rendered with. Nothing ordered [`survey`] after
    /// [`aim_camera`], so the executor was free to run it first and the marks trailed the view
    /// by exactly one frame.
    ///
    /// [`survey`]: crate::pick
    #[test]
    fn a_mark_is_placed_against_this_frames_camera_and_not_last_frames() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<AppState>()
            .add_message::<Requested>()
            .add_message::<AppExit>()
            .insert_resource(Ui(UiState::default()))
            .insert_resource(Game(Session::new(&AuthoredStars::sample(), 3)))
            .init_resource::<crate::uplink::Uplink>()
            .init_resource::<Seen>()
            .configure_sets(Update, (Stage::Link, Stage::Act, Stage::Scene, Stage::Mark).chain())
            .add_systems(Update, dispatch.in_set(Stage::Act))
            .add_systems(Update, aim_camera.in_set(Stage::Scene))
            .add_systems(Update, probe.in_set(Stage::Mark));
        app.insert_state(AppState::InGame);
        let camera = app.world_mut().spawn((Camera3d::default(), SkyCamera, Transform::default())).id();
        app.update();
        let before = app.world().resource::<Seen>().0.expect("the mark stage should have run");

        app.world_mut().write_message(Requested(Action::Look { yaw: 1.2, pitch: 0.3 }));
        app.update();

        let aimed = *app.world().entity(camera).get::<Transform>().unwrap().forward();
        let seen = app.world().resource::<Seen>().0.unwrap();
        assert_ne!(seen, before, "the turn never reached the camera at all");
        assert_eq!(seen, aimed, "the mark was placed against last frame's camera");
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
