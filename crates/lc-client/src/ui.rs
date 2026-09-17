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
}

impl Panel {
    pub const ALL: [Panel; 11] = [
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
            Panel::Chat => "Radio",
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
    /// Across a scene, where the cast is kilometres apart, the difference is microseconds and
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
/// a minute. Labelled by period rather than by factor because a factor is not something anyone
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

/// Development default for the clock multiplier: a Julian year a minute rather than an hour.
///
/// A four light-year crossing then takes four minutes of real time instead of four hours,
/// which is the difference between watching the sky move and taking it on faith. The server
/// owns the rate in a real session and this multiplier does not exist there.
pub const TEST_TIME_RATE: f64 = 60.0;

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
    /// Development only; the server owns the rate.
    pub time_rate: f64,
    pub notifications: Vec<Notification>,
    /// The loadout the refit panel's sliders are set to, or `None` to follow the ship.
    pub refit_draft: Option<lc_world::fitting::Loadout>,
    /// Which conversation the radio window is showing.
    ///
    /// Here rather than local to the panel because a click in the events box has to be able to
    /// change it, and that click is an [`crate::action::Action`] like any other.
    pub chat_with: Option<lc_proto::ShipId>,
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
            time_rate: TEST_TIME_RATE,
            notifications: Vec::new(),
            refit_draft: None,
            chat_with: None,
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

    /// Somebody said something. Shown in the events box in the colour the interface reserves
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
