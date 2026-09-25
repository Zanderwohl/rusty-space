//! What the interface is showing. Data only: [`crate::action::apply`] is what changes it.

use glam::DVec3;
use lc_world::sky::StarId;

/// Where the application is. Not what is on top of it — nothing is modal, so panels are a
/// separate set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Boot,
    MainMenu,
    Loading,
    InGame,
    Unreachable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuPage {
    #[default]
    Root,
    /// Signing in. Reached by pressing Observe without an identity, and left by getting one or
    /// by giving up.
    ///
    /// A page rather than a state, like everything else here: the sky keeps drifting behind it
    /// and nothing is suspended, which is true rather than merely convenient.
    SignIn,
    NewWorld,
    Load,
    Settings,
    About,
}

/// A window. The escape menu is one of these rather than a state, because the clock does not
/// stop behind it and a thing that blocks the world would be claiming otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Escape,
    Settings,
    Debug,
    Telescope,
    System,
    Flight,
    Tuning,
    /// Scenes to stage. Development only, and it does nothing without a shard started for it.
    Scenarios,
    /// Rebuilding the ship: how many of each module, and the hull.
    Refit,
    /// Development only: things no player can do, such as being handed energy.
    DevActions,
    /// One conversation at a time, chosen from a list. Every ship this one has heard from is
    /// in it, whether or not it is still in sight.
    Chat,
    /// Something to read: the shelf, or a book off it. Drawn by [`crate::reader`] rather than
    /// with the others, because it is the one surface that is not a readout — it has its own
    /// frame, its own palette and its own keys.
    Reader,
    /// Drawn by [`crate::console`], not as a window.
    Console,
}

impl Panel {
    pub const ALL: [Panel; 13] = [
        Panel::Escape,
        Panel::Settings,
        Panel::Debug,
        Panel::Telescope,
        Panel::System,
        Panel::Flight,
        Panel::Tuning,
        Panel::Scenarios,
        Panel::Refit,
        Panel::DevActions,
        Panel::Chat,
        Panel::Reader,
        Panel::Console,
    ];

    /// A panel by the name a development flag would use.
    pub fn named(name: &str) -> Option<Self> {
        Panel::ALL.into_iter().find(|p| p.title().to_lowercase().starts_with(name))
    }

    pub fn title(&self) -> &'static str {
        match self {
            Panel::Escape => "Menu",
            Panel::Settings => "Settings",
            Panel::Debug => "Debug",
            Panel::Telescope => "Telescope",
            Panel::System => "System",
            Panel::Flight => "Flight",
            Panel::Tuning => "Starfield tuning",
            Panel::Scenarios => "Scenarios",
            Panel::Refit => "Refit",
            Panel::DevActions => "Dev actions",
            Panel::Chat => "Communications",
            Panel::Reader => "Reader",
            Panel::Console => "Console",
        }
    }
}

/// Which mode of play the main view is showing.
///
/// The map is not a window over the world: it is the other thing the same screen can be, and
/// whichever one is not in force is the thumbnail in the corner. Windows float above either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// The sky, through the ship's own camera.
    #[default]
    World,
    /// Where everything is: a reference plane, decade rings, and what stands off it.
    Map,
    /// The ship on a turntable, to be reshaped. See `lightcone/docs/29-ship-form.md`.
    Form,
}

impl ViewMode {
    /// What the corner square shows, and where a click on it goes.
    pub fn other(self) -> Self {
        match self {
            ViewMode::World => ViewMode::Map,
            ViewMode::Map | ViewMode::Form => ViewMode::World,
        }
    }

    /// Where `M` goes: into the map, or out of it to the world.
    pub fn map_key(self) -> Self {
        match self {
            ViewMode::Map => ViewMode::World,
            ViewMode::World | ViewMode::Form => ViewMode::Map,
        }
    }
}

/// What the map's camera is centered on.
///
/// Three states and not an `Option`, which is what this was and what made "center on the
/// ship" a button that did nothing at all. `None` has to mean *leave the camera where it is*,
/// because that is what a pan needs; following the observer is a third thing, and folding it
/// into the same `None` meant the ship was the one object on the map you could not center on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapFocus {
    /// Wherever it was last left. A pan puts it here.
    Free,
    /// The observer, which is where a map should open.
    #[default]
    Observer,
    /// Whatever holds the ship — a moon's planet, a planet's star — followed as the ship
    /// crosses from one sphere of influence into the next.
    ///
    /// A mode and not the body it resolves to today, which is the whole of the difference:
    /// [`MapFocus::Item`] on Earth stays on Earth after the ship has left it.
    Primary(Frame),
    /// Something in the snapshot, followed as it moves.
    Item(em_map::ItemKey),
}

/// Which frame the map is drawn in while it is centered on the primary.
///
/// The reference line is the primary's center to the ship's. [`Frame::Local`] holds the camera
/// against that line, so the ship keeps its place on screen and everything else goes round it;
/// [`Frame::Fixed`] measures against the reference plane's own axes, and the ship is what moves.
///
/// Only on the primary. About the ship there is no line to hold — the two ends are the same
/// point — and about a named body the ship is not one of the ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Frame {
    #[default]
    Fixed,
    Local,
}

impl Frame {
    pub fn label(self) -> &'static str {
        match self {
            Frame::Fixed => "fixed",
            Frame::Local => "local",
        }
    }
}

/// What the map is showing, and from where.
///
/// One field on [`UiState`] rather than six, because every part of it moves together: a plane
/// toggle that left the camera's angles measured against the old basis would be a plane toggle
/// that tilted nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapView {
    pub orbit: em_map::Orbit,
    pub plane: em_map::Plane,
    /// What [`em_map::Plane::System`] resolves against: the plane this craft *believes* the
    /// local system's planets share, refreshed as it learns and as it moves between systems.
    ///
    /// The belief and not a bare pole, because the panel has to be able to say why the option is
    /// unavailable — a craft that has solved no orbits has no system plane, and offering one
    /// would be drawing a frame nobody measured.
    ///
    /// Held beside the camera for the reason the rest of this struct is held together: the
    /// angles are measured against this basis, so the two cannot be a frame apart.
    pub system_plane: lc_world::knowledge::SystemPlane,
    /// A key rather than a position: Saturn moves, and a camera pointed at where it was is a
    /// camera that drifts off it over an afternoon.
    pub focus: MapFocus,
    /// Where the reference line pointed when the camera was last turned with it, radians.
    ///
    /// Held so the turn can be the *change* in that bearing: the camera's azimuth stays the one
    /// number every drag and every ray is measured in, which is what keeps a rotating frame
    /// from needing a second copy of the camera.
    ///
    /// `None` whenever nothing is being tracked, so entering the local frame turns nothing and
    /// leaving it leaves the camera where it is.
    pub bearing: Option<f64>,
    pub source: crate::map_source::Source,
}

impl MapView {
    /// The reference plane as the geometry needs it: resolved against the system in view.
    ///
    /// Everything that casts a ray, measures a height or reads a bearing goes through this, so
    /// the camera and the plane it is angled against cannot disagree.
    ///
    /// **A system plane nobody has solved falls back to the galactic one**, which this craft
    /// knows from the catalog and which needs nothing measured. Falling back to `+Z` laid
    /// every unsolved system's rings in the ecliptic of J2000 -- Sol's plane, shown around a
    /// star nobody has surveyed, which is the exact mistake `25-system-knowledge.md` opens by
    /// describing.
    pub fn resolved_plane(&self) -> em_map::Plane {
        match (self.plane, self.believed_pole()) {
            (em_map::Plane::System, None) => em_map::Plane::Galactic,
            (plane, _) => plane,
        }
    }

    pub fn datum(&self) -> em_map::Datum {
        self.resolved_plane().about(self.believed_pole().unwrap_or(DVec3::Z))
    }

    /// The believed pole, or `None` when this craft has not solved one.
    ///
    /// `Circle` is not a pole: a craft that has only watched transits from one place knows the
    /// pole lies somewhere on a great circle, which is not enough to lay rings in.
    pub fn believed_pole(&self) -> Option<DVec3> {
        match self.system_plane {
            lc_world::knowledge::SystemPlane::Known { pole, .. } => Some(pole),
            _ => None,
        }
    }
}

/// Which craft the camera is behind.
///
/// One variant, and an enum anyway. What the camera does is going to grow — a chase view along
/// the velocity, a fixed point a scene is composed from, a free fly-around — and every one of
/// those is a different answer to "where is the eye", not a flag on top of this one. Growing it
/// here keeps that a new arm rather than a second mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraPerspective {
    /// Over the shoulder of a craft, as its own pilot would have it.
    ///
    /// **The eye moves; the observer does not.** Everything the client works out about *light*
    /// — retarded times, aberration, what a contact looked like when it left — is still solved
    /// from the player's own ship, because that is the craft the session has a worldline for.
    /// Across a scene, where the cast is kilometers apart, the difference is microseconds and
    /// there is nothing to see. Across the Oort cloud it would be hours, and this would be a
    /// lie. Watching from a craft you are not on is a development view until the observer can
    /// move too.
    Pov(lc_proto::ShipId),
}

/// Where the ship is looking: ecliptic angles, yaw about the pole from +X and pitch from the
/// plane. Two angles rather than a quaternion so there is no roll to accumulate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub yaw: f64,
    pub pitch: f64,
}

impl Look {
    /// Pitch stops just short of the pole, where yaw stops being defined.
    pub const PITCH_LIMIT: f64 = std::f64::consts::FRAC_PI_2 - 1e-3;

    pub fn turn(&mut self, d_yaw: f64, d_pitch: f64) {
        self.yaw = (self.yaw + d_yaw).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch + d_pitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Unit vector the ship is looking along, in simulation axes.
    pub fn forward(&self) -> DVec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        DVec3::new(cp * cy, cp * sy, sp)
    }

    /// Aim at a direction. A zero vector leaves the look where it was.
    pub fn aimed_at(direction: DVec3) -> Option<Self> {
        let d = direction.normalize_or_zero();
        if d == DVec3::ZERO {
            return None;
        }
        Some(Self { yaw: d.y.atan2(d.x), pitch: d.z.clamp(-1.0, 1.0).asin() })
    }
}

/// What the radio window is showing.
///
/// [`Channel::Public`] is not a conversation and is deliberately not stored as one: it is a
/// *view* over every other, answering "what has been going on" rather than "what did we two
/// say". Sealed messages are absent from it whichever end they came from — including this
/// ship's own, because a private message listed in a public log is a private message on a
/// screen somebody can read over your shoulder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Everything said to nobody in particular: broadcasts, sent and heard.
    #[default]
    Public,
    /// Traffic between other craft that this ship was in range of. Open ones can be read;
    /// encrypted ones are shown as the noise they are.
    Overheard,
    /// One craft's conversation, both halves.
    With(lc_proto::ShipId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub text: String,
    /// Coordinate seconds when it was raised.
    pub at: f64,
    /// The craft this is about, when it is somebody talking.
    ///
    /// What makes the line green and clickable. Everything else in this box is the interface
    /// reporting on itself — an order accepted, a clock corrected — and has nowhere to go.
    pub from: Option<lc_proto::ShipId>,
}

/// How many notifications are kept. Older ones fall off rather than accumulating.
pub const NOTIFICATION_LIMIT: usize = 6;

/// Clock multipliers a development build offers, and what each one means to watch.
///
/// The multiplier is against the design rate of one Julian year per real hour, so 60 is a year
/// a minute. Labeled by period rather than by factor because a factor is not something anyone
/// can feel, and these exist to be chosen by eye — a crossing to Proxima takes four and a half
/// hours at 1x, four and a half minutes at 60x, and forty-five seconds at 360x.
/// The ladder, as multiples of [`crate::session::TIME_RATE`].
///
/// The bottom three rungs are what orbits need. The design rate is already 8766 times real
/// time, which puts a low orbit's whole period inside a second: at that speed a ship in orbit
/// is a strobe, and nothing about the view can be read. Each rung is about sixty times the one
/// below, so the whole range from a spacewalk to a crossing is eight steps.
pub const RATE_LADDER: [(f64, &str); 9] = [
    (0.0, "stopped"),
    (REAL_TIME, "real time"),
    (0.006_844, "1 minute / second"),
    (0.410_678, "1 hour / second"),
    (1.0, "1 year / hour"),
    (6.0, "1 year / 10 min"),
    (60.0, "1 year / minute"),
    (360.0, "1 year / 10 s"),
    (3600.0, "1 year / second"),
];

pub const REAL_TIME: f64 = 3600.0 / 31_557_600.0;

/// The ladder's name for a rate, or the bare factor for one set from outside it.
pub fn rate_label(rate: f64) -> String {
    if let Some((_, name)) = RATE_LADDER.iter().find(|(r, _)| (r - rate).abs() < 1e-9) {
        return (*name).to_string();
    }
    if rate <= 0.0 {
        return "stopped".into();
    }
    // A period, like every rung of the ladder, rather than a factor. The fallback used to be
    // `{rate:.0}x the design rate`, which printed a shard running at a twentieth as **0x** —
    // a slow clock reading as a stopped one, which is the one thing this label exists to stop.
    //
    // The idiom turns over at the design rate for the same reason the ladder's does: below it a
    // year is too long to be a period anyone can hold, and what you want to know is how much
    // game time a second buys.
    if rate < 1.0 {
        format!("{} / second", span(rate * crate::session::TIME_RATE))
    } else {
        format!("1 year / {}", span(3600.0 / rate))
    }
}

/// A duration in the largest unit it is more than one of. Whole numbers: this is a readout to
/// be glanced at, and "1.7 hours" is not something anyone can feel either.
fn span(seconds: f64) -> String {
    const MINUTE: f64 = 60.0;
    const HOUR: f64 = 60.0 * MINUTE;
    const DAY: f64 = 24.0 * HOUR;
    let (size, unit) = match seconds {
        s if s >= DAY => (DAY, "day"),
        s if s >= HOUR => (HOUR, "hour"),
        s if s >= MINUTE => (MINUTE, "minute"),
        _ => (1.0, "second"),
    };
    let how_many = (seconds / size).round().max(1.0);
    if how_many == 1.0 {
        unit.to_string()
    } else {
        format!("{how_many:.0} {unit}s")
    }
}

/// The next rung up or down from `rate`, saturating at the ends.
pub fn rate_step(rate: f64, up: bool) -> f64 {
    let at = RATE_LADDER
        .iter()
        .position(|(r, _)| (r - rate).abs() < 1e-9)
        // A rate from outside the ladder steps to the nearest rung that moves the right way.
        .unwrap_or_else(|| RATE_LADDER.iter().filter(|(r, _)| *r < rate).count().saturating_sub(1));
    let next = if up { at + 1 } else { at.saturating_sub(1) };
    RATE_LADDER[next.min(RATE_LADDER.len() - 1)].0
}

/// The clock multiplier a session starts at: **the design rate**, one Julian year an hour.
///
/// The same number a shard runs at, and the same one [`crate::uplink::SERVER_RATE`] names. A
/// single-player session is a server with one player, so it has no business running at a
/// different speed from one — and the offline default used to be sixty times the server's,
/// which is the whole of the bug behind "clock corrected by 140 hours" firing every second at
/// a client that had joined without touching a key.
///
/// Speeding it up is what `--rate` is for, and what the ladder in the interface is for. Both
/// are development affordances, and a shard refuses them: the rate is the world's.
pub const DESIGN_TIME_RATE: f64 = 1.0;

#[derive(Clone, Debug)]
pub struct UiState {
    pub screen: Screen,
    pub menu_page: MenuPage,
    /// Which craft the camera is behind. `None` is the player's own, which is what it has
    /// always been and is the only perspective a shipped build offers.
    pub perspective: Option<CameraPerspective>,
    /// Open panels, most recently opened last. Order is what "back" walks.
    open: Vec<Panel>,
    pub selected: Option<StarId>,
    /// What is picked out of the local system's inventory, and which of its courses is armed.
    ///
    /// Here rather than in the panel because picking a body out of the sky will set the same
    /// two fields, and because naming a world later has to change what this is pointing at.
    pub focus: Option<crate::navigation::Target>,
    pub course: Option<crate::navigation::Course>,
    pub look: Look,
    /// Which mode of play the main view is showing. See [`ViewMode`].
    pub view: ViewMode,
    /// The map's camera, plane and source. See [`MapView`].
    pub map: MapView,
    /// The editor's camera, and the mode it was entered from.
    pub form: crate::form_view::FormView,
    /// Whether this client may ask for the god view.
    ///
    /// Read once from the ticket, at boot, because that is the only place the ticket is. Held
    /// here so `action::apply` can refuse as well as the panel declining to offer — a control
    /// that is merely absent is a control the next development flag reaches anyway.
    ///
    /// Advisory. See [`crate::map_source::may_see_everything`] for what it is not.
    pub may_see_everything: bool,
    /// How far the orbit camera stands off, in hull lengths.
    ///
    /// A multiple rather than a distance, so it means the same framing whatever the player is
    /// flying. Clamped every frame against what the viewport can show — see
    /// [`crate::hull::boom_limits`] — because both ends of the range are angles.
    pub boom_lengths: f64,
    /// How the two starfield passes are drawn. State rather than constants so they can be
    /// turned while the thing they affect is on screen, which is the only way to tune a look.
    pub distant: crate::starfield::PointStyle,
    pub local: crate::starfield::PointStyle,
    pub bodies: crate::starfield::PointStyle,
    /// How far a population's covering fraction is amplified for display. A belt blocks a
    /// millionth of the light and a millionth of a pixel is nothing.
    pub envelope_gain: f32,
    /// Stops away from the automatic exposure.
    pub exposure_offset: f32,
    pub preset: usize,
    pub integration_s: f64,
    pub god_view: bool,
    /// See [`crate::beauty`].
    pub beauty_shots: bool,
    /// Development only; the server owns the rate.
    pub time_rate: f64,
    pub notifications: Vec<Notification>,
    pub reading: Reading,
    /// The loadout the refit panel's sliders are set to, or `None` to follow the ship.
    pub refit_draft: Option<lc_world::fitting::Loadout>,
    /// Which conversation the radio window is showing.
    ///
    /// Here rather than local to the panel because a click in the events box has to be able to
    /// change it, and that click is an [`crate::action::Action`] like any other.
    pub chat_with: Channel,
}

/// Where the player is up to in a book.
///
/// **A character offset, never a page.** A page is a fact about this window at this size; the
/// offset is a fact about the book, and it is what gets written down. See
/// `lightcone/docs/21-library.md`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// The book being read, by the name its file has on the shelf.
    pub book: Option<String>,
    pub spine: usize,
    pub offset: usize,
    /// Which block the offset means, while this chapter is open.
    ///
    /// **A character offset cannot say.** A plate is made of no characters, so it shares its
    /// offset with the plate or the paragraph after it — and resolving the offset alone always
    /// lands on the first of them, which is a page that can be turned to and never past. Two
    /// plates in a row is not a rare shape: the first chapter of Huckleberry Finn opens with
    /// exactly that.
    ///
    /// The index is a fact about the parsed document and survives a resize, where a row index
    /// would not. `None` after a jump or a fresh open, when the offset is all there is.
    pub block: Option<usize>,
    /// Pages asked for and not yet turned.
    ///
    /// An action cannot turn a page, because turning one means laying it out and only the
    /// surface with the fonts in it can do that. So the action records the request and the
    /// reader spends it on the next frame, which is also what makes a page turn survive a
    /// resize arriving in the same frame.
    pub turn: i32,
    /// A place to jump to, spent the same way.
    pub goto: Option<(usize, usize)>,
    /// Set when the player *moved*: a page turned, a chapter jumped to. Cleared once the place
    /// has been written down.
    ///
    /// The offset moves for another reason too — a resize reflows the page and the same sentence
    /// lands at a slightly different character — and a drag doing that sixty times a second
    /// would spend a connection's whole message budget on bookmarks nobody asked to save. So the
    /// deliberate move is flagged, and it is the one that does not wait.
    pub asked: bool,
    pub contents: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Boot,
            menu_page: MenuPage::Root,
            open: Vec::new(),
            selected: None,
            focus: None,
            course: None,
            look: Look::default(),
            view: ViewMode::default(),
            map: MapView::default(),
            form: crate::form_view::FormView::default(),
            may_see_everything: false,
            perspective: None,
            boom_lengths: crate::hull::DEFAULT_BOOM_LENGTHS,
            distant: crate::starfield::DISTANT,
            local: crate::starfield::LOCAL,
            bodies: crate::starfield::BODIES,
            envelope_gain: crate::envelope::OPACITY_GAIN,
            exposure_offset: 0.0,
            preset: 0,
            integration_s: 1.0e4,
            god_view: false,
            beauty_shots: false,
            time_rate: DESIGN_TIME_RATE,
            notifications: Vec::new(),
            reading: Reading::default(),
            refit_draft: None,
            chat_with: Channel::default(),
        }
    }
}

impl UiState {
    pub fn is_open(&self, panel: Panel) -> bool {
        self.open.contains(&panel)
    }

    /// Open panels in the order they were opened.
    pub fn open_panels(&self) -> &[Panel] {
        &self.open
    }

    pub fn open(&mut self, panel: Panel) {
        if !self.is_open(panel) {
            self.open.push(panel);
        }
    }

    pub fn close(&mut self, panel: Panel) {
        self.open.retain(|p| *p != panel);
    }

    pub fn toggle(&mut self, panel: Panel) {
        if self.is_open(panel) {
            self.close(panel);
        } else {
            self.open(panel);
        }
    }

    /// Close the most recently opened panel, returning it.
    pub fn close_top(&mut self) -> Option<Panel> {
        self.open.pop()
    }

    pub fn notify(&mut self, text: impl Into<String>, at: f64) {
        self.raise(Notification { text: text.into(), at, from: None });
    }

    /// Somebody said something. Shown in the events box in the color the interface reserves
    /// for it, and clicking it opens the conversation.
    pub fn heard(&mut self, from: lc_proto::ShipId, text: impl Into<String>, at: f64) {
        self.raise(Notification { text: text.into(), at, from: Some(from) });
    }

    fn raise(&mut self, note: Notification) {
        self.notifications.push(note);
        let excess = self.notifications.len().saturating_sub(NOTIFICATION_LIMIT);
        self.notifications.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::knowledge::SystemPlane;

    /// **The map lays its rings in the plane the crew solved, not the one the generator used.**
    /// A craft that has solved nothing gets a usable frame rather than a degenerate one, and the
    /// panel is what refuses the option -- see `map_panel::unsolved`.
    #[test]
    fn the_maps_plane_follows_the_belief() {
        let pole = DVec3::new(0.2, -0.4, 0.89).normalize();
        let mut view = MapView {
            plane: em_map::Plane::System,
            system_plane: SystemPlane::Known { pole, sigma_rad: 0.01, zero: DVec3::X },
            ..MapView::default()
        };
        assert_eq!(view.believed_pole(), Some(pole));
        assert!((view.datum().normal() - pole).length() < 1.0e-12);

        // A pole somewhere on a circle is not a pole: it cannot be laid rings in.
        // A pole somewhere on a circle is not a pole. The map falls back to the galactic
        // plane, which needs nothing solved -- not to +Z, which is Sol's plane and would be a
        // measurement of one system shown around another.
        let galactic_normal = em_map::Plane::Galactic.about(DVec3::Z).normal();
        view.system_plane = SystemPlane::Circle(DVec3::X);
        assert_eq!(view.believed_pole(), None);
        assert_eq!(view.resolved_plane(), em_map::Plane::Galactic);
        assert_eq!(view.datum().normal(), galactic_normal, "a frame it knows, not Sol's");

        view.system_plane = SystemPlane::Unknown;
        assert_eq!(view.believed_pole(), None);
        assert_eq!(view.datum().normal(), galactic_normal);
        assert!((galactic_normal - DVec3::Z).length() > 0.5, "the two frames are not the same");

        // The galactic frame needs nothing solved and is the same in every system.
        view.plane = em_map::Plane::Galactic;
        let galactic = view.datum();
        view.system_plane = SystemPlane::Known { pole, sigma_rad: 0.01, zero: DVec3::X };
        assert_eq!(view.datum(), galactic, "a belief moved the galactic plane");
    }

    #[test]
    fn panels_stack_in_the_order_they_were_opened() {
        let mut ui = UiState::default();
        ui.open(Panel::Telescope);
        ui.open(Panel::Debug);
        ui.open(Panel::Telescope); // already open, not raised twice
        assert_eq!(ui.open_panels(), &[Panel::Telescope, Panel::Debug]);
        assert_eq!(ui.close_top(), Some(Panel::Debug));
        assert_eq!(ui.close_top(), Some(Panel::Telescope));
        assert_eq!(ui.close_top(), None);
    }

    #[test]
    fn closing_something_that_is_not_open_is_not_an_error() {
        let mut ui = UiState::default();
        ui.close(Panel::Settings);
        assert!(!ui.is_open(Panel::Settings));
    }

    #[test]
    fn notifications_are_bounded() {
        let mut ui = UiState::default();
        for k in 0..20 {
            ui.notify(format!("event {k}"), k as f64);
        }
        assert_eq!(ui.notifications.len(), NOTIFICATION_LIMIT);
        assert_eq!(ui.notifications.last().unwrap().text, "event 19", "the newest survive");
    }

    #[test]
    fn every_panel_has_a_title() {
        for p in Panel::ALL {
            assert!(!p.title().is_empty());
        }
    }
    /// **A slow clock must not read as a stopped one.** The fallback used to print a rate of a
    /// twentieth as "0x the design rate", which is the label's whole job failing: a scene that
    /// runs slowly so an orbit can be looked at would have said the world was frozen.
    #[test]
    fn a_rate_off_the_ladder_is_still_a_period() {
        assert_eq!(rate_label(0.05), "7 minutes / second");
        assert_eq!(rate_label(20.0), "1 year / 3 minutes");
        assert!(!rate_label(0.05).starts_with('0'), "a slow clock read as a stopped one");
        // Nothing may divide by a rate that is not one.
        assert_eq!(rate_label(0.0), "stopped");
        assert_eq!(rate_label(-1.0), "stopped");
    }

    /// Every rung the ladder names it names; the general form is only for what falls between.
    #[test]
    fn every_rung_keeps_the_name_it_was_given() {
        for (rate, name) in RATE_LADDER {
            assert_eq!(rate_label(rate), name, "{rate}");
        }
    }

}
