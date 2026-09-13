//! What the interface is showing. Data only: [`crate::action::apply`] is what changes it.

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
}

impl Panel {
    pub const ALL: [Panel; 5] =
        [Panel::Escape, Panel::Settings, Panel::Debug, Panel::Telescope, Panel::System];

    pub fn title(&self) -> &'static str {
        match self {
            Panel::Escape => "Menu",
            Panel::Settings => "Settings",
            Panel::Debug => "Debug",
            Panel::Telescope => "Telescope",
            Panel::System => "System",
        }
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

#[derive(Clone, Debug)]
pub struct UiState {
    pub screen: Screen,
    pub menu_page: MenuPage,
    /// Open panels, most recently opened last. Order is what "back" walks.
    open: Vec<Panel>,
    pub selected: Option<StarId>,
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
            exposure_offset: 0.0,
            preset: 0,
            integration_s: 1.0e4,
            god_view: false,
            time_rate: 1.0,
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
