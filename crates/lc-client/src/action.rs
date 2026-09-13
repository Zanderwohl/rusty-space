//! Every UI action, as a value.
//!
//! No system acts directly. A key press, a button, a menu item and a test all produce the
//! same [`Action`], and [`apply`] is the only thing that changes [`crate::ui::UiState`] or
//! reaches into a [`Session`]. Rebinding is then a table rather than a rewrite, and a UI can
//! be driven from a test with no window.

use em_spectra::presets;
use lc_world::sky::StarId;

use crate::session::Session;
use crate::ui::{MenuPage, Panel, UiState};

/// Everything the interface can be asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    // --- navigation -------------------------------------------------------------------
    OpenPanel(Panel),
    ClosePanel(Panel),
    TogglePanel(Panel),
    CloseTopPanel,
    GoToMenuPage(MenuPage),
    StartGame,
    Quit,

    // --- instruments ------------------------------------------------------------------
    SetBandPreset(usize),
    NextBandPreset,
    PreviousBandPreset,
    ExposureUp,
    ExposureDown,
    ExposureAuto,

    // --- observing --------------------------------------------------------------------
    SelectTarget(Option<StarId>),
    SetIntegration(f64),

    // --- development ------------------------------------------------------------------
    ToggleGodView,
    SetTimeRate(f64),
    WriteSnapshot,
}

/// What an action needs from outside: the few things the core cannot do itself.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Quit,
    StartGame,
    WriteSnapshot,
    Notify(String),
}

/// Stops of exposure per keypress.
pub const EXPOSURE_STEP: f32 = 0.5;

/// Apply an action. The only path that mutates UI state.
pub fn apply(action: Action, ui: &mut UiState, session: &mut Session) -> Vec<Effect> {
    let mut effects = Vec::new();
    match action {
        Action::OpenPanel(p) => ui.open(p),
        Action::ClosePanel(p) => ui.close(p),
        Action::TogglePanel(p) => ui.toggle(p),
        Action::CloseTopPanel => {
            // Nothing is modal, so "back" closes the most recently opened panel and opens
            // the escape menu only when there is nothing left to close.
            if ui.close_top().is_none() {
                ui.open(Panel::Escape);
            }
        }
        Action::GoToMenuPage(page) => ui.menu_page = page,
        Action::StartGame => effects.push(Effect::StartGame),
        Action::Quit => effects.push(Effect::Quit),

        Action::SetBandPreset(i) => set_preset(ui, session, i, &mut effects),
        Action::NextBandPreset => {
            let next = (ui.preset + 1) % presets::all().len();
            set_preset(ui, session, next, &mut effects);
        }
        Action::PreviousBandPreset => {
            let count = presets::all().len();
            let prev = (ui.preset + count - 1) % count;
            set_preset(ui, session, prev, &mut effects);
        }

        Action::ExposureUp => adjust_exposure(ui, session, EXPOSURE_STEP),
        Action::ExposureDown => adjust_exposure(ui, session, -EXPOSURE_STEP),
        Action::ExposureAuto => {
            ui.exposure_offset = 0.0;
            session.auto_expose();
        }

        Action::SelectTarget(id) => {
            ui.selected = id;
            session.point_at(id);
            match id.and_then(|i| session.star(i)).and_then(|s| s.name.clone()) {
                Some(name) => effects.push(Effect::Notify(format!("watching {name}"))),
                None if id.is_some() => effects.push(Effect::Notify("watching an unnamed star".into())),
                None => {}
            }
        }
        Action::SetIntegration(seconds) => ui.integration_s = seconds.max(0.0),

        Action::ToggleGodView => {
            if cfg!(feature = "godview") {
                ui.god_view = !ui.god_view;
            } else {
                // Not merely hidden: the code is not in this build.
                effects.push(Effect::Notify("god view is not compiled into this build".into()));
            }
        }
        Action::SetTimeRate(rate) => ui.time_rate = rate.max(0.0),
        Action::WriteSnapshot => effects.push(Effect::WriteSnapshot),
    }
    effects
}

fn set_preset(ui: &mut UiState, session: &mut Session, index: usize, effects: &mut Vec<Effect>) {
    let all = presets::all();
    let Some((name, mapping)) = all.get(index) else {
        effects.push(Effect::Notify(format!("no band preset {index}")));
        return;
    };
    ui.preset = index;
    session.mapping = *mapping;
    // The window follows the mapping: a different band is a different brightness.
    session.auto_expose();
    apply_exposure_offset(ui, session);
    effects.push(Effect::Notify(format!("band mapping: {name}")));
}

fn adjust_exposure(ui: &mut UiState, session: &mut Session, stops: f32) {
    ui.exposure_offset = (ui.exposure_offset + stops).clamp(-12.0, 12.0);
    session.auto_expose();
    apply_exposure_offset(ui, session);
}

fn apply_exposure_offset(ui: &UiState, session: &mut Session) {
    session.tone = session.tone.exposed(ui.exposure_offset);
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;

    fn fixture() -> (UiState, Session) {
        (UiState::default(), Session::new(&AuthoredStars::sample(), 3))
    }

    #[test]
    fn panels_open_close_and_toggle_independently() {
        let (mut ui, mut s) = fixture();
        apply(Action::OpenPanel(Panel::Telescope), &mut ui, &mut s);
        apply(Action::OpenPanel(Panel::Debug), &mut ui, &mut s);
        assert!(ui.is_open(Panel::Telescope) && ui.is_open(Panel::Debug));
        // Nothing is modal: opening one does not close another.
        apply(Action::TogglePanel(Panel::Telescope), &mut ui, &mut s);
        assert!(!ui.is_open(Panel::Telescope) && ui.is_open(Panel::Debug));
    }

    #[test]
    fn back_closes_the_newest_panel_and_then_opens_the_escape_menu() {
        let (mut ui, mut s) = fixture();
        apply(Action::OpenPanel(Panel::Telescope), &mut ui, &mut s);
        apply(Action::OpenPanel(Panel::System), &mut ui, &mut s);
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(!ui.is_open(Panel::System) && ui.is_open(Panel::Telescope));
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(!ui.is_open(Panel::Telescope));
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(ui.is_open(Panel::Escape), "with nothing left, back means the menu");
    }

    #[test]
    fn a_band_preset_reaches_the_session_and_re_exposes() {
        let (mut ui, mut s) = fixture();
        let before = s.tone.reference;
        let effects = apply(Action::SetBandPreset(2), &mut ui, &mut s);
        assert_eq!(ui.preset, 2);
        assert_eq!(s.mapping, presets::all()[2].1);
        assert_ne!(s.tone.reference, before, "a different band is a different brightness");
        assert!(matches!(effects.as_slice(), [Effect::Notify(_)]));
    }

    #[test]
    fn preset_cycling_wraps_both_ways() {
        let (mut ui, mut s) = fixture();
        let count = presets::all().len();
        for _ in 0..count {
            apply(Action::NextBandPreset, &mut ui, &mut s);
        }
        assert_eq!(ui.preset, 0, "a full cycle returns to the start");
        apply(Action::PreviousBandPreset, &mut ui, &mut s);
        assert_eq!(ui.preset, count - 1);
    }

    #[test]
    fn an_unknown_preset_is_reported_rather_than_applied() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::SetBandPreset(99), &mut ui, &mut s);
        assert_eq!(ui.preset, 0);
        assert!(matches!(effects.as_slice(), [Effect::Notify(m)] if m.contains("99")));
    }

    #[test]
    fn exposure_steps_in_stops_and_returns_to_auto() {
        let (mut ui, mut s) = fixture();
        let auto = s.tone.reference;
        apply(Action::ExposureUp, &mut ui, &mut s);
        assert!((ui.exposure_offset - EXPOSURE_STEP).abs() < 1e-6);
        assert!(s.tone.reference < auto, "opening up lowers the reference");
        apply(Action::ExposureAuto, &mut ui, &mut s);
        assert_eq!(ui.exposure_offset, 0.0);
        assert!((s.tone.reference - auto).abs() < auto * 1e-6);
    }

    #[test]
    fn exposure_is_bounded() {
        let (mut ui, mut s) = fixture();
        for _ in 0..200 {
            apply(Action::ExposureUp, &mut ui, &mut s);
        }
        assert!(ui.exposure_offset <= 12.0);
        assert!(s.tone.reference.is_finite() && s.tone.reference > 0.0);
    }

    #[test]
    fn selecting_a_target_points_the_telescope_and_says_so() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        let effects = apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        assert_eq!(ui.selected, Some(id));
        assert_eq!(s.pointing, Some(id));
        assert!(matches!(effects.as_slice(), [Effect::Notify(_)]));
        apply(Action::SelectTarget(None), &mut ui, &mut s);
        assert_eq!(s.pointing, None);
    }

    #[test]
    fn god_view_is_absent_rather_than_hidden() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::ToggleGodView, &mut ui, &mut s);
        if cfg!(feature = "godview") {
            assert!(ui.god_view);
        } else {
            assert!(!ui.god_view);
            assert!(matches!(effects.as_slice(), [Effect::Notify(m)] if m.contains("not compiled")));
        }
    }

    #[test]
    fn effects_are_returned_rather_than_performed() {
        let (mut ui, mut s) = fixture();
        assert_eq!(apply(Action::Quit, &mut ui, &mut s), vec![Effect::Quit]);
        assert_eq!(apply(Action::WriteSnapshot, &mut ui, &mut s), vec![Effect::WriteSnapshot]);
        assert_eq!(apply(Action::StartGame, &mut ui, &mut s), vec![Effect::StartGame]);
    }

    /// A session driven entirely by actions, which is what makes the UI testable.
    #[test]
    fn a_whole_interaction_runs_with_no_window() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        for action in [
            Action::StartGame,
            Action::SelectTarget(Some(id)),
            Action::OpenPanel(Panel::Telescope),
            Action::SetBandPreset(2),
            Action::ExposureDown,
            Action::SetIntegration(1e4),
        ] {
            apply(action, &mut ui, &mut s);
        }
        assert!(ui.is_open(Panel::Telescope));
        assert_eq!(ui.preset, 2);
        assert_eq!(ui.integration_s, 1e4);
        s.observe(ui.integration_s);
        assert!(!s.curve.is_empty(), "the telescope should have recorded something");
    }
}
