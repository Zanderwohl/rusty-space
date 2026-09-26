//! The development entry: what the flags ask for, and the pins that hold it.
//!
//! None of it is reachable from the interface. It is here rather than in [`crate::app`] because
//! it is the part that grows with every flag, and because what it does is one responsibility:
//! put the world into a stated pose and photograph it.
//!
//! **A pin is written every frame, an action once.** An action races whatever else aims the
//! same thing — two runs of `--pitch` in three came back at pitch zero — so anything two shots
//! are meant to agree about is a pin.

use bevy::math::DVec3;
use bevy::prelude::*;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;

/// What the flags asked for: skip the menu, and optionally photograph the sky and quit.
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
    /// How far off `--at` stands, in the body's radii.
    pub standoff_radii: Option<f64>,
    /// The angle `--at` stands at between the star and itself, seen from the body, degrees.
    /// Past ninety it is looking at the night side.
    pub phase_deg: Option<f64>,
    /// Dress the `--at` body as a generated planet, by its name: `--wear "Wolf 359 c"`. Only the
    /// sphere is borrowed; a generated system is otherwise a crossing away.
    /// On an airless body a name no planet has is a seed for another airless look.
    pub wear: Option<String>,
    /// Put the ship straight onto a station, by [`crate::navigation::Course::parse`] spelling.
    /// The same courses the interface offers, without the crossing in between.
    pub station: Option<String>,
    /// Seed what the ship knows from truth, the way the charting office used to.
    ///
    /// A ship knows nothing on creation now, so it photographs nothing: every body shot is a
    /// ship staring at an empty panel, and `--focus` names a body the panel does not list.
    /// `observatory::issue_charts` is exactly the mechanism that used to do this, which is the
    /// second reason it survived the phase that stopped calling it. The charting office kept as
    /// a dev tool.
    pub charted: bool,
    /// Hold the beauty shots on one [`crate::beauty::Subject::kind`].
    pub beauty_kind: Option<String>,
    /// Save every beauty shot into this directory, numbered and named by kind.
    pub beauty_dir: Option<String>,
    /// Seconds between beauty shots.
    pub beauty_period_s: Option<f32>,
    /// What the map's camera is to hold onto, written every frame like the rest of the pin.
    pub map_focus: Option<WantedFocus>,

    /// Which mode of play to hold the main view in.
    ///
    /// A **pin**, for the reason [`DevEntry::map_camera`] is one: as an action it came back in
    /// the world's mode on one run in four, because a dev action is written once and whatever
    /// else was settling that frame won.
    pub view: Option<crate::ui::ViewMode>,

    /// Where the map's camera stands: bearing and elevation in degrees, stand-off in
    /// astronomical units. A light-year is 63 241 of them.
    ///
    /// A **pin**, written every frame after the dispatcher, for the reason `--demo-cam` is
    /// one: `--turn` and `--pitch` are actions and race whatever else aims the view, and two
    /// runs of the same command came back framed differently. Two shots meant to be compared
    /// have to be the same shot.
    pub map_camera: Option<(f64, f64, f64)>,

    /// `--demo-cam-at x:y:z:m`: orbit this point of the ship's frame, meters, from this far,
    /// unclamped. The boom's stops are the whole ship's, so without it nothing closer than half
    /// a GSV can be photographed. Aim with `--demo-cam`.
    pub camera_at: Option<(glam::DVec3, f64)>,

    /// Degrees to lift the ship out of the ecliptic, about the star, keeping its distance.
    ///
    /// Every station the interface offers is in the plane, and every population's pole is the
    /// ecliptic pole, so from any of them a belt is edge-on and a shell is a band. This is the
    /// only way to photograph one as the ring it is. It leaves the ship ballistic rather than
    /// holding the station it was placed from — the waypoint would put it straight back in the
    /// plane — so use it with `--rate 0`.
    pub lift_deg: Option<f64>,
    /// Draw the player's ship as this preset, by [`crate::parts::fixture`]'s spelling.
    pub form: Option<String>,
    /// Start the editor's draft as this, by [`crate::draft::staged`]'s spelling, so a shot has
    /// marks on it.
    pub draft: Option<String>,
    /// Stage `--demo refit` on the player's ship, its clock frozen by `--refit-at` or looping.
    pub refit: Option<crate::construction::Clock>,
    /// `--rate` was given, so a scene leaves the clock alone.
    pub rate_given: bool,
    pub screenshot: Option<String>,
    /// Frames to let the sky settle before the shutter. Pipelines compile lazily.
    pub after_frames: u32,
    /// How many consecutive frames to photograph. More than one for diagnosing a flicker.
    pub burst: u32,
    /// Frames to time after [`DevEntry::after_frames`] of warm-up. See `bench`.
    pub bench: Option<u32>,
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
    /// Sent once the shard has welcomed this client. The only way to photograph an answer.
    pub console: Option<String>,
    /// Press the editor's Apply once the shard has welcomed this client and Apply is open.
    pub apply: bool,
    /// Pull the selected part's size handle out to this many times its length once the shard has
    /// stated the ship, stopping where the budget does. The only way to photograph that stop.
    pub pull: Option<f64>,
    /// Cancel the round once it is this fraction through. The only way to photograph a ship left
    /// between its forms.
    pub cancel_at: Option<f64>,
    /// Run once on reaching the sky. Actions rather than flags, so a development entry can
    /// reach anything the interface can and needs no plumbing of its own.
    pub actions: Vec<Action>,
}

/// Put the ship on a station named by `--station`, without flying it there.
///
/// The crossing is what `--station` skips: a course to Neptune is two months of coordinate
/// time, and a screenshot of a place should not have to wait for it.
pub(crate) fn place_on_station(
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
    let label = waypoint.label(&game.home_labels());
    // Not over a `--focus`: this flag says where the ship is, that one says what is selected.
    if !dev.actions.iter().any(|action| matches!(action, Action::FocusTarget(_))) {
        ui.focus = course.target();
    }

    if let Some(degrees) = dev.lift_deg.filter(|d| d.abs() > 0.0) {
        let at = lifted(at, system.origin_ly, degrees);
        // Back at the star, which is the center of whatever the lift was for looking down at.
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

/// Do what the flags asked for, once there is a ship to do it to.
///
/// **Polled, and it waits for the connection.** These used to run on entering the world, which
/// is before a welcome can possibly have arrived — so `--fly` put the ship on a crossing, the
/// welcome landed a dozen frames later and replaced the ship wholesale, and the crossing was
/// gone with no order ever having reached the server. The flag looked like it worked for about
/// a third of a second.
pub(crate) fn run_dev_actions(
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
            // Selected and stared at: selecting alone only describes it.
            out.write(Requested(Action::SelectTarget(Some(id))));
            out.write(Requested(Action::StareSelected));
        }
    }
}

/// Found among the stars whose name `name` begins with.
fn generated_paint(stars: &[lc_world::sky::CatalogStar], name: &str) -> Option<Worn> {
    use lc_world::climate::{Inputs, derived, variety};
    stars
        .iter()
        .filter(|s| s.provenance.name.as_deref().is_some_and(|n| name.starts_with(n)))
        .find_map(|s| {
            let p = lc_world::sky::generate::planets_of(s).into_iter().find(|p| p.name == name)?;
            let inputs = Inputs {
                atmosphere: p.atmosphere,
                top: p.top,
                equilibrium_k: p.equilibrium_k,
                water_fraction: Some(p.water_fraction),
                life: Some(p.life),
                star_teff_k: s.star.teff_k,
            };
            let bare = (p.atmosphere, p.top) == (lc_world::worlds::Atmosphere::None, lc_world::worlds::Top::Rock);
            let surface = lc_world::surface::Surface::classify(p.radius_m, p.mass_kg, p.equilibrium_k);
            let airless = bare.then(|| lc_world::airless::derived(surface, p.radius_m, variety(&p.name)));
            Some(Worn { climate: derived(&inputs, variety(&p.name)), giant: p.giant(s), airless, world: p.world(s) })
        })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Worn {
    climate: Option<lc_world::climate::Climate>,
    giant: Option<lc_world::giant::Giant>,
    airless: Option<lc_world::airless::Airless>,
    world: lc_world::worlds::World,
}

/// See [`DevEntry::wear`]. Every frame, because the bodies are rebuilt every frame; after them
/// and before anything resolves them.
pub(crate) fn dress_worn(
    dev: Res<DevEntry>,
    game: Res<Game>,
    mut bodies: ResMut<crate::starfield::Bodies>,
    mut worn: Local<Option<Option<Worn>>>,
) {
    let (Some(name), Some(at)) = (&dev.wear, &dev.at_body) else { return };
    let Some(body) = bodies.drawn.iter_mut().find(|d| &d.name == at) else { return };
    let paint = *worn.get_or_insert_with(|| {
        let found = generated_paint(&game.stars, name)
            .filter(|w| w.climate.is_some() || w.giant.is_some() || w.airless.is_some())
            .or_else(|| {
                body.airless?;
                let airless = lc_world::airless::derived(body.surface, body.radius_m, lc_world::climate::variety(name));
                Some(Worn { climate: None, giant: None, airless: Some(airless), world: body.world })
            });
        match &found {
            Some(w) => info!("wearing {name}: {w:?}"),
            None => warn!("no generated planet named {name} has paint to wear, and {at} is not airless"),
        }
        found
    });
    let Some(paint) = paint else { return };
    {
        body.climate = paint.climate;
        body.giant = paint.giant;
        body.airless = paint.airless;
        body.world = paint.world;
    }
}

/// Stand off from a named body, once its system has loaded.
pub(crate) fn place_at_body(
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
    let stand_off = body.radius_m * dev.standoff_radii.unwrap_or(12.0) / crate::system::M_PER_LY;
    let origin = game.system.as_ref().map(|s| s.origin_ly).unwrap_or_default();
    let from_star = (body.position_ly - origin)
        .normalize_or_zero();
    // Off to the side and a little sunward, so the body shows a terminator. Straight out from
    // the star is the night side, which is a correct view of nothing.
    let across = from_star.cross(DVec3::Z).normalize_or_zero();
    let phase = dev.phase_deg.unwrap_or(63.4).to_radians();
    let offset = (across * phase.sin() - from_star * phase.cos()).normalize_or_zero();
    game.place_at(body.position_ly + offset * stand_off);
    if let Some(look) = crate::ui::Look::aimed_at(-offset) {
        ui.look = look;
    }
    ui.notify(format!("standing off {want}"), game.coordinate_time_s());
    *done = true;
}

/// Turn to face the cast, once there is one to face.
///
/// Polled rather than run on entering the world, like `--at`: a contact comes from a shard that
/// has to connect first, and there is nobody in the list on the frame the sky appears.
pub(crate) fn frame_the_cast(
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
pub(crate) fn open_the_radio(
    dev: Res<DevEntry>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<Requested>,
    mut done: Local<bool>,
) {
    let Some(words) = dev.say.as_ref() else { return };
    if *done {
        return;
    }
    let Some(contact) = uplink.contacts.first() else { return };
    *done = true;
    out.write(Requested(Action::OpenChat(contact.ship_id)));
    out.write(Requested(Action::Say {
        to: Some(contact.ship_id),
        aim: lc_proto::Aim::Omni,
        secrecy: lc_proto::Secrecy::Open,
        body: words.clone(),
        idem: None,
    }));
}

pub(crate) fn type_at_the_console(
    dev: Res<DevEntry>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<Requested>,
    mut done: Local<bool>,
) {
    let Some(line) = dev.console.as_ref() else { return };
    if *done || uplink.joined().is_none() {
        return;
    }
    *done = true;
    out.write(Requested(Action::OpenPanel(crate::ui::Panel::Console)));
    out.write(Requested(Action::RunCommand(line.clone())));
}

/// `--apply` and `--cancel-at`. Once the round is under way the view is pinned to the world,
/// where the ship is drawn building.
pub(crate) fn apply_and_cancel(
    mut dev: ResMut<DevEntry>,
    ui: Res<Ui>,
    game: Res<crate::app::Game>,
    mut out: MessageWriter<Requested>,
    mut done: Local<(bool, bool)>,
) {
    let session = &game.0;
    if dev.apply
        && !done.0
        && let Some(draft) = &ui.form.draft
        && crate::ledger::gate(draft, &ui.form.applying, crate::ledger::Situation::of(session)).is_ok()
    {
        done.0 = true;
        out.write(Requested(Action::ApplyDraft));
    }
    let now = session.coordinate_time_s();
    let running = session.ship.fitting().and_then(|f| f.refit()).filter(|_| session.ship.is_refitting(now));
    if done.0 && running.is_some() {
        dev.view = Some(crate::ui::ViewMode::World);
    }
    if let (Some(at), Some(plan), false) = (dev.cancel_at, running, done.1)
        && crate::ledger::standing(plan, now).fraction >= at
    {
        done.1 = true;
        out.write(Requested(Action::CancelRefit));
    }
}

/// Hold the camera still, so two runs photograph the same view.
///
/// Written every frame rather than once, which is the whole point: anything that aims the
/// camera — an arriving crossing, a snap to a target, a hand on the mouse — is overruled on the
/// frame after it, so there is nothing left for a shot to race.
pub(crate) fn pin_camera(dev: Res<DevEntry>, mut ui: ResMut<Ui>) {
    let Some((yaw_deg, pitch_deg, booms)) = dev.camera else { return };
    ui.look.yaw = yaw_deg.to_radians();
    ui.look.pitch = pitch_deg.to_radians();
    ui.boom_lengths = booms;
}

/// What `--map-focus` asked for, before there is a system to resolve a star against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WantedFocus {
    Ship,
    Primary,
    /// The primary, in the frame that turns with the ship.
    Local,
    Star,
    Free,
}

impl WantedFocus {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "ship" => Some(Self::Ship),
            "primary" => Some(Self::Primary),
            "local" => Some(Self::Local),
            "star" => Some(Self::Star),
            "free" => Some(Self::Free),
            _ => None,
        }
    }
}

/// Hold the map's focus where the development flags asked for it.
///
/// **A pin has to pin the whole camera.** `--map` held the angles and the stand-off and left
/// the focus alone, so a hand on the mouse could pan a shot that was supposed to be
/// reproducible — and two runs of the same command then framed differently, which is exactly
/// what `--demo-cam` exists to prevent. `--map` on its own holds the ship.
pub(crate) fn pin_map_focus(dev: Res<DevEntry>, game: Res<Game>, mut ui: ResMut<Ui>) {
    let wanted = match (dev.map_focus, dev.map_camera) {
        (Some(wanted), _) => wanted,
        (None, Some(_)) => WantedFocus::Ship,
        (None, None) => return,
    };
    ui.map.focus = match wanted {
        WantedFocus::Ship => crate::ui::MapFocus::Observer,
        WantedFocus::Primary => crate::ui::MapFocus::Primary(crate::ui::Frame::Fixed),
        WantedFocus::Local => crate::ui::MapFocus::Primary(crate::ui::Frame::Local),
        WantedFocus::Free => crate::ui::MapFocus::Free,
        WantedFocus::Star => match game.0.system.as_ref() {
            Some(system) => {
                crate::ui::MapFocus::Item(em_map::ItemKey::from_id("star", system.star.get()))
            }
            None => return,
        },
    };
}

/// Hold the main view in the mode the development flags asked for.
pub(crate) fn pin_view(dev: Res<DevEntry>, mut ui: ResMut<Ui>) {
    if let Some(view) = dev.view {
        ui.view = view;
    }
}

/// The same, for the map. See [`DevEntry::map_camera`].
pub(crate) fn pin_map_camera(dev: Res<DevEntry>, mut ui: ResMut<Ui>) {
    let Some((azimuth_deg, elevation_deg, au)) = dev.map_camera else { return };
    ui.map.orbit.azimuth = azimuth_deg.to_radians();
    // Through `turn` from zero rather than written, so the two clamps apply: a pinned
    // elevation of zero is a view in the plane, and the camera is never in the plane.
    ui.map.orbit.elevation = 0.0;
    ui.map.orbit.turn(0.0, elevation_deg.to_radians());
    ui.map.orbit.set_distance_m(au * em_map::snapshot::M_PER_AU);
}

/// `shot.png` and 2 becomes `shot.2.png`.
fn numbered(path: &str, index: u32) -> String {
    match path.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem}.{index}.{extension}"),
        None => format!("{path}.{index}"),
    }
}

/// Photograph the sky through the real pipeline, then quit.
pub(crate) fn photograph(
    mut commands: Commands,
    dev: Res<DevEntry>,
    unready: Option<Res<crate::refit_hull::Unready>>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &dev.screenshot else { return };
    // Warm-up counts from when the meshes have landed, and a burst, once begun, is not held.
    if *frames < dev.after_frames && unready.is_some_and(|u| u.0) {
        return;
    }
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


#[cfg(test)]
mod tests {
    use super::*;

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
}
