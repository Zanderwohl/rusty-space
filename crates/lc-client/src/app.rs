//! The Bevy layer: states, resources, and the systems that carry actions.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::DVec3;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::camera::Hdr;
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
    /// Turn to face the nearest contact once there is one, for a scene that has a cast.
    ///
    /// A scene is about a second ship, and the odds of the camera happening to point at it are
    /// the odds of a bearing picked at random. Once, not every frame: the camera is the
    /// player's from then on, which is the whole reason it is not pinned.
    pub frame_cast: bool,
    /// Yaw and pitch in degrees, and a boom in hull lengths, held there for the run.
    ///
    /// `--turn` and `--pitch` are *actions*, and they race whatever else aims the camera — the
    /// same command three times has come back twice at pitch zero. A pin does not race
    /// anything: it is written every frame, so the frame the shutter opens on is the one that
    /// was asked for.
    pub camera: Option<(f64, f64, f64)>,
    /// Put the ship beside a body of the local system, by name. There is no action for this and
    /// there never will be; it exists so a thing too small to fly to can be looked at.
    pub at_body: Option<String>,
    /// Put the ship straight onto a station, by [`crate::navigation::Course::parse`] spelling.
    /// The same courses the interface offers, without the crossing in between.
    pub station: Option<String>,
    /// Degrees to lift the ship out of the ecliptic, about the star, keeping its distance.
    ///
    /// Every station the interface offers is in the plane, and every population's pole is the
    /// ecliptic pole, so from any of them a belt is edge-on and a shell is a band. This is the
    /// only way to photograph one as the ring it is. It leaves the ship ballistic rather than
    /// holding the station it was placed from — the waypoint would put it straight back in the
    /// plane — so use it with `--rate 0`.
    pub lift_deg: Option<f64>,
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
    /// Say this to the first contact that appears, and show the conversation.
    ///
    /// The only way to photograph a transcript. Everything in the radio window arrives from a
    /// shard, so an action run on entering the sky has nobody to talk to yet — this is polled,
    /// like `--at` and the framing, until there is somebody in the contact list.
    pub say: Option<String>,
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
            em_render::plume_material::PlumeMaterialPlugin,
            BodySurfaceMaterialPlugin,
            crate::sky_asset::SkyAssetPlugin,
            crate::library::LibraryPlugin,
            crate::faces::FacesPlugin,
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
            .init_resource::<crate::hull::Eye>()
            .init_resource::<crate::hull::Hulls>()
            .init_resource::<crate::plume::Plumes>()
            .init_resource::<crate::resolved::Resolved>()
            .configure_sets(Update, (Stage::Link, Stage::Act, Stage::Scene, Stage::Mark).chain())
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::Loading), begin_load)
            .add_systems(OnEnter(AppState::InGame), spawn_sky)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(
                Update,
                (
                    boot.run_if(in_state(AppState::Boot)),
                    finish_load.run_if(in_state(AppState::Loading)),
                    // Not gated on a state: `--menu --shot` photographs the menu, and the
                    // system does nothing unless a path was asked for.
                    photograph,
                    run_dev_actions.run_if(in_state(AppState::InGame)),
                    place_at_body.run_if(in_state(AppState::InGame)),
                    place_on_station.run_if(in_state(AppState::InGame)),
                    (read_keys, grab_cursor, look_around, crate::input::read_wheel)
                        .chain()
                        .run_if(in_state(AppState::InGame)),
                    dispatch,
                    // After the dispatcher and after the look, because those are what it is
                    // overruling: a pin that ran before them would be undone by a hand on the
                    // mouse or by a crossing aiming itself, on the same frame.
                    frame_the_cast.run_if(in_state(AppState::InGame)),
                    open_the_radio.run_if(in_state(AppState::InGame)),
                    // After the framing, because a pin overrules everything including that.
                    pin_camera.run_if(in_state(AppState::InGame)),
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
                    // First of the stage. Everything below is drawn relative to the eye, and
                    // one placed against last frame's would shear the whole scene against the
                    // ship every time the view turned.
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
                    // rasterised, and a pass that ran first would be measured hinted.
                    crate::faces::unhint,
                    // First of the drawing, so a frame that has the faces is drawn in them
                    // rather than the frame after it.
                    crate::faces::settle,
                    panels::loading.run_if(in_state(AppState::Loading)),
                    (panels::hud, panels::open_panels, crate::reader::draw)
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

    if let Some(degrees) = dev.lift_deg.filter(|d| d.abs() > 0.0) {
        let at = lifted(at, system.origin_ly, degrees);
        // Back at the star, which is the centre of whatever the lift was for looking down at.
        if let Some(look) = crate::ui::Look::aimed_at(system.origin_ly - at) {
            ui.look = look;
        }
        game.0.place_at(at);
        ui.notify(format!("{label}, lifted {degrees:.0} degrees"), game.coordinate_time_s());
        *done = true;
        return;
    }

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

/// The same distance from the star, at `degrees` of latitude above the plane.
///
/// A rotation rather than a displacement, so a belt station stays in its belt and only the
/// latitude changes — which is the one thing being varied.
fn lifted(at: DVec3, star: DVec3, degrees: f64) -> DVec3 {
    let out = at - star;
    let radius = out.length();
    if radius <= 0.0 {
        return at;
    }
    // The simulation's pole is +Z; the in-plane part of the offset is what gets tipped.
    let flat = DVec3::new(out.x, out.y, 0.0);
    let Some(along) = flat.try_normalize() else { return at };
    let (sin, cos) = degrees.to_radians().sin_cos();
    star + (along * cos + DVec3::Z * sin) * radius
}

/// One frame of boot, so the window is up before anything slow happens.
fn boot(mut next: ResMut<NextState<AppState>>, mut ui: ResMut<Ui>, dev: Res<DevEntry>) {
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

/// Development entry: do what the flags asked for, once there is a ship to do it to.
///
/// **Polled, and it waits for the connection.** These used to run on entering the world, which
/// is before a welcome can possibly have arrived — so `--fly` put the ship on a crossing, the
/// welcome landed a dozen frames later and replaced the ship wholesale, and the crossing was
/// gone with no order ever having reached the server. The flag looked like it worked for about
/// a third of a second.
fn run_dev_actions(
    dev: Res<DevEntry>,
    game: Res<Game>,
    uplink: Res<crate::uplink::Uplink>,
    address: Res<crate::uplink::ServerAddress>,
    mut out: MessageWriter<Requested>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    // Settled, which is not the same as connected: a build with no shard to talk to is settled
    // the moment it knows there is none, and one that has been refused is never going to be
    // any readier than it is.
    let settled = match &uplink.state {
        crate::uplink::State::Joined(_) => true,
        crate::uplink::State::Offline => address.0.is_none(),
        crate::uplink::State::Connecting => false,
        crate::uplink::State::Refused(_) | crate::uplink::State::Lost(_) => true,
    };
    if !settled {
        return;
    }
    *done = true;
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

/// Development entry: turn to face the cast, once there is one to face.
///
/// Polled rather than run on entering the world, like `--at`: a contact comes from a shard that
/// has to connect first, and there is nobody in the list on the frame the sky appears.
fn frame_the_cast(
    dev: Res<DevEntry>,
    game: Res<Game>,
    uplink: Res<crate::uplink::Uplink>,
    mut ui: ResMut<Ui>,
    mut done: Local<bool>,
) {
    if *done || !dev.frame_cast {
        return;
    }
    if uplink.contacts.is_empty() {
        return;
    }
    // Everybody the camera is *not* on. Watching from another craft, the interesting thing is
    // the ship you left; watching from your own, it is the cast. Aiming at the hull the boom is
    // attached to would be aiming at the middle of the screen.
    let anchored = match ui.perspective {
        Some(crate::ui::CameraPerspective::Pov(ship_id)) => Some(ship_id),
        None => None,
    };
    let here = match anchored.and_then(|id| uplink.contacts.iter().find(|c| c.ship_id == id)) {
        Some(contact) => contact.position_ly,
        None => game.ship.motion.position_ly,
    };
    // The middle of them, by bearing rather than by position: a scene with one ship in it aims
    // at that ship, and one with four spread about an axis aims down the axis instead of at
    // whichever happens to be nearest — which would throw the other three off to one side.
    // Directions are summed rather than positions, or the furthest would count for the most.
    let mut bearing = DVec3::ZERO;
    if anchored.is_some() {
        bearing += (game.ship.motion.position_ly - here).normalize_or_zero();
    }
    for contact in uplink.contacts.iter().filter(|c| Some(c.ship_id) != anchored) {
        bearing += (contact.position_ly - here).normalize_or_zero();
    }
    if let Some(look) = crate::ui::Look::aimed_at(bearing) {
        ui.look = look;
    }
    *done = true;
}

/// Development entry: say something to the first contact, and open the conversation.
///
/// Polled rather than run on arrival, for the same reason the framing is: there is nobody in
/// the contact list on the frame the sky appears, because the shard has not answered yet.
fn open_the_radio(
    dev: Res<DevEntry>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<crate::input::Requested>,
    mut done: Local<bool>,
) {
    let Some(words) = dev.say.as_ref() else { return };
    if *done {
        return;
    }
    let Some(contact) = uplink.contacts.first() else { return };
    *done = true;
    out.write(crate::input::Requested(Action::OpenChat(contact.ship_id)));
    out.write(crate::input::Requested(Action::Say {
        to: Some(contact.ship_id),
        aim: lc_proto::Aim::Omni,
        secrecy: lc_proto::Secrecy::Open,
        body: words.clone(),
        idem: None,
    }));
}

/// Development entry: hold the camera still, so two runs photograph the same view.
///
/// Written every frame rather than once, which is the whole point: anything that aims the
/// camera — an arriving crossing, a snap to a target, a hand on the mouse — is overruled on the
/// frame after it, so there is nothing left for a shot to race.
fn pin_camera(dev: Res<DevEntry>, mut ui: ResMut<Ui>) {
    let Some((yaw_deg, pitch_deg, booms)) = dev.camera else { return };
    ui.look.yaw = yaw_deg.to_radians();
    ui.look.pitch = pitch_deg.to_radians();
    ui.boom_lengths = booms;
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
    uplink: Res<crate::uplink::Uplink>,
    mut next: ResMut<NextState<AppState>>,
) {
    match catalogue.0.as_deref() {
        // Packing is native tooling and reads a file directly; see `skypack`. Absent from a
        // browser build, where the feature is off and `csv` is not in the tree at all.
        #[cfg(feature = "hyg")]
        Some(path) if path.ends_with(".csv") => {
            match lc_world::sky::hyg::HygProvider::load(path) {
                Ok(p) => enter_game(&mut game, &mut ui, &mut next, &p, &uplink),
                Err(e) => {
                    ui.notify(format!("catalogue: {e}"), 0.0);
                    enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink);
                }
            }
        }
        Some(path) => {
            commands.insert_resource(LoadingSky(assets.load(path.to_owned())));
        }
        None => enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink),
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
    uplink: Res<crate::uplink::Uplink>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(loading) = loading else { return };
    if let Some(sky) = skies.get(&loading.0) {
        if sky.skipped > 0 {
            ui.notify(format!("{} sky records were unusable", sky.skipped), 0.0);
        }
        enter_game(&mut game, &mut ui, &mut next, sky, &uplink);
        commands.remove_resource::<LoadingSky>();
    } else if let Some(state) = assets.get_load_state(&loading.0)
        && state.is_failed()
    {
        // A sky that will not load is worth saying out loud rather than silently becoming
        // three hand-written stars.
        ui.notify("sky failed to load; using the sample", 0.0);
        enter_game(&mut game, &mut ui, &mut next, &AuthoredStars::sample(), &uplink);
        commands.remove_resource::<LoadingSky>();
    }
}

fn enter_game(
    game: &mut Game,
    ui: &mut Ui,
    next: &mut NextState<AppState>,
    provider: &dyn StarProvider,
    uplink: &crate::uplink::Uplink,
) {
    let count = provider.len();
    game.0 = Session::new(provider, SKY_LIMIT);
    // The session was just replaced, and with it everything the server had said about where
    // and when this ship is. Put it back, or the client flies locally from the origin while
    // the interface still says LINKED. See `uplink::Placement`.
    uplink.place(&mut game.0);
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
                Effect::AutoAck { with, on } => uplink.chat.set_auto_ack(with, on),
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

    /// `--lift` is a rotation about the star, not a displacement: the ship keeps its distance
    /// and only its latitude changes. A belt station that moved radially as well would stop
    /// being a station in that belt, which is the whole point of lifting from one.
    #[test]
    fn a_lift_keeps_the_ship_at_its_own_radius() {
        let star = DVec3::new(3.0, -1.0, 0.5);
        let at = star + DVec3::new(2.0, 1.0, 0.0);
        let radius = (at - star).length();
        for degrees in [0.0, 15.0, 45.0, 90.0, -30.0] {
            let moved = lifted(at, star, degrees);
            assert!(
                ((moved - star).length() - radius).abs() < 1.0e-12,
                "{degrees} degrees changed the radius",
            );
            let latitude = ((moved - star).z / radius).asin().to_degrees();
            assert!((latitude - degrees).abs() < 1.0e-9, "{latitude} for {degrees}");
        }
        // A ship already on the pole has no plane direction to tip, and is left where it is.
        let polar = star + DVec3::Z;
        assert_eq!(lifted(polar, star, 30.0), polar);
        assert_eq!(lifted(star, star, 30.0), star);
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

    /// Where a mark placed in [`Stage::Mark`] found the camera.
    #[derive(Resource, Default)]
    struct Seen(Option<Vec3>);

    fn probe(camera: Query<&Transform, With<Camera3d>>, mut seen: ResMut<Seen>) {
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
        let camera = app.world_mut().spawn((Camera3d::default(), Transform::default())).id();
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
