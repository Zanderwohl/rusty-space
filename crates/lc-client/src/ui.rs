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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuPage {
    #[default]
    Root,
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
}

impl Panel {
    pub const ALL: [Panel; 6] = [
        Panel::Escape,
        Panel::Settings,
        Panel::Debug,
        Panel::Telescope,
        Panel::System,
        Panel::Flight,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Panel::Escape => "Menu",
            Panel::Settings => "Settings",
            Panel::Debug => "Debug",
            Panel::Telescope => "Telescope",
            Panel::System => "System",
            Panel::Flight => "Flight",
        }
    }
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
}

/// How many notifications are kept. Older ones fall off rather than accumulating.
pub const NOTIFICATION_LIMIT: usize = 6;

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
    /// Open panels, most recently opened last. Order is what "back" walks.
    open: Vec<Panel>,
    pub selected: Option<StarId>,
    pub look: Look,
    /// Stops away from the automatic exposure.
    pub exposure_offset: f32,
    pub preset: usize,
    pub integration_s: f64,
    pub god_view: bool,
    /// Development only; the server owns the rate.
    pub time_rate: f64,
    pub notifications: Vec<Notification>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Boot,
            menu_page: MenuPage::Root,
            open: Vec::new(),
            selected: None,
            look: Look::default(),
            exposure_offset: 0.0,
            preset: 0,
            integration_s: 1.0e4,
            god_view: false,
            time_rate: TEST_TIME_RATE,
            notifications: Vec::new(),
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
        self.notifications.push(Notification { text: text.into(), at });
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
}
