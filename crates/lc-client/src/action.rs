//! Every UI action, as a value.
//!
//! No system acts directly. A key press, a button, a menu item and a test all produce the
//! same [`Action`], and [`apply`] is the only thing that changes [`crate::ui::UiState`] or
//! reaches into a [`Session`]. Rebinding is then a table rather than a rewrite, and a UI can
//! be driven from a test with no window.

use em_spectra::presets;
use lc_world::sky::StarId;

use crate::session::Session;
use crate::ui::{Look, MenuPage, Panel, UiState};

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

    // --- looking ----------------------------------------------------------------------
    /// Turn by a relative amount, radians.
    Look { yaw: f64, pitch: f64 },
    LookAtSelected,

    // --- flight -----------------------------------------------------------------------
    /// Cross to a star. `None` means whatever is selected.
    FlyTo(Option<StarId>),
    /// Cross to the nearest star that is actually interstellar.
    FlyToNearest,
    AbortFlight,
    /// Proper acceleration for the next crossing, in g.
    SetDriveAccel(f64),

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

/// Below this a "star" is the one the ship is already at, not a destination.
pub const INTERSTELLAR_LY: f64 = 0.01;

/// Radians per keypress of look.
pub const LOOK_STEP: f64 = 0.05;

/// What the drive will accept. The low end is a burn a crew could live in for decades; the
/// high end is where a crossing stops being something you watch happen.
pub const MIN_ACCEL_G: f64 = 0.1;
pub const MAX_ACCEL_G: f64 = 1000.0;

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

        Action::Look { yaw, pitch } => ui.look.turn(yaw, pitch),
        Action::LookAtSelected => match aim(ui, session) {
            Some(look) => ui.look = look,
            None => effects.push(Effect::Notify("nothing is selected to look at".into())),
        },

        Action::FlyTo(id) => fly(ui, session, id, &mut effects),
        Action::FlyToNearest => {
            // Not simply the first: the catalogue carries the Sun at about an astronomical
            // unit, and "nearest star" has to mean one worth crossing to.
            let nearest = session
                .stars
                .iter()
                .find(|s| session.distance_to(s) > INTERSTELLAR_LY)
                .map(|s| s.id);
            match nearest {
                Some(id) => {
                    apply_to(ui, session, Action::SelectTarget(Some(id)), &mut effects);
                    fly(ui, session, Some(id), &mut effects);
                }
                None => effects.push(Effect::Notify("nothing interstellar in range".into())),
            }
        }
        Action::AbortFlight => {
            if session.cruise.is_some() {
                session.abort_flight();
                effects.push(Effect::Notify("drive cut".into()));
            }
        }
        Action::SetDriveAccel(g) => {
            session.drive.accel_g = g.clamp(MIN_ACCEL_G, MAX_ACCEL_G);
            effects.push(Effect::Notify(format!("drive set to {:.0} g", session.drive.accel_g)));
        }

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

/// Run a nested action, keeping its effects. Only for actions composed of other actions.
fn apply_to(ui: &mut UiState, session: &mut Session, action: Action, effects: &mut Vec<Effect>) {
    effects.extend(apply(action, ui, session));
}

fn aim(ui: &UiState, session: &Session) -> Option<Look> {
    let star = session.star(ui.selected?)?;
    Look::aimed_at(session.offset_to(star))
}

fn fly(ui: &mut UiState, session: &mut Session, id: Option<StarId>, effects: &mut Vec<Effect>) {
    let Some(id) = id.or(ui.selected) else {
        effects.push(Effect::Notify("no destination selected".into()));
        return;
    };
    let Some(star) = session.star(id) else {
        effects.push(Effect::Notify("that star is not loaded".into()));
        return;
    };
    let name = star.name.clone().unwrap_or_else(|| "an unnamed star".into());
    let Some(cruise) = session.fly_to(id) else { return };
    let (years, aboard) = (
        cruise.duration_s() / crate::flight::JULIAN_YEAR_S,
        cruise.proper_duration_s() / crate::flight::JULIAN_YEAR_S,
    );
    // Looking somewhere else during a crossing is allowed; starting one pointed at the
    // destination is what anybody wants by default.
    if let Some(look) = Look::aimed_at(session.offset_to(session.star(id).unwrap())) {
        ui.look = look;
    }
    effects.push(Effect::Notify(format!("{name}: {years:.2} years out, {aboard:.2} aboard")));
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

/// Re-place the exposure window and re-apply the user's offset on top of it.
///
/// Public because the window has to be replaced whenever the scene's brightness moves on its
/// own — flying toward a star changes it by tens of stops without anyone touching a control.
pub fn refresh_exposure(ui: &UiState, session: &mut Session) {
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

    #[test]
    fn looking_accumulates_and_the_pitch_stops_at_the_pole() {
        let (mut ui, mut s) = fixture();
        apply(Action::Look { yaw: 0.3, pitch: 0.2 }, &mut ui, &mut s);
        apply(Action::Look { yaw: 0.3, pitch: 0.2 }, &mut ui, &mut s);
        assert!((ui.look.yaw - 0.6).abs() < 1e-12);
        assert!((ui.look.pitch - 0.4).abs() < 1e-12);
        for _ in 0..200 {
            apply(Action::Look { yaw: 0.0, pitch: 0.5 }, &mut ui, &mut s);
        }
        assert!(ui.look.pitch <= Look::PITCH_LIMIT, "{}", ui.look.pitch);
        assert!(ui.look.forward().is_finite());
    }

    #[test]
    fn yaw_wraps_rather_than_growing_without_bound() {
        let (mut ui, mut s) = fixture();
        for _ in 0..1000 {
            apply(Action::Look { yaw: 1.0, pitch: 0.0 }, &mut ui, &mut s);
        }
        assert!(ui.look.yaw >= 0.0 && ui.look.yaw < std::f64::consts::TAU, "{}", ui.look.yaw);
    }

    #[test]
    fn looking_at_the_selection_points_the_camera_at_it() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert_eq!(apply(Action::LookAtSelected, &mut ui, &mut s).len(), 1, "nothing selected");
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        apply(Action::LookAtSelected, &mut ui, &mut s);
        let want = s.offset_to(s.star(id).unwrap()).normalize();
        assert!((ui.look.forward() - want).length() < 1e-12, "{:?} vs {want:?}", ui.look.forward());
    }

    #[test]
    fn flying_with_nothing_selected_says_so_and_starts_nothing() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::FlyTo(None), &mut ui, &mut s);
        assert!(matches!(effects.as_slice(), [Effect::Notify(t)] if t.contains("no destination")));
        assert!(s.cruise.is_none());
    }

    #[test]
    fn flying_uses_the_selection_and_reports_both_clocks() {
        let (mut ui, mut s) = fixture();
        apply(Action::SelectTarget(Some(s.stars[0].id)), &mut ui, &mut s);
        let effects = apply(Action::FlyTo(None), &mut ui, &mut s);
        assert!(s.cruise.is_some());
        let text = effects
            .iter()
            .find_map(|e| match e {
                Effect::Notify(t) if t.contains("aboard") => Some(t.clone()),
                _ => None,
            })
            .expect("a crossing report");
        assert!(text.contains("years out"), "{text}");
    }

    /// Starting a crossing turns the camera to the destination; nothing else may.
    #[test]
    fn starting_a_crossing_aims_the_camera_at_it() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::Look { yaw: 2.0, pitch: -0.5 }, &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        let want = s.offset_to(s.star(id).unwrap()).normalize();
        assert!((ui.look.forward() - want).length() < 1e-12);
    }

    #[test]
    fn cutting_a_drive_that_is_not_running_says_nothing() {
        let (mut ui, mut s) = fixture();
        assert!(apply(Action::AbortFlight, &mut ui, &mut s).is_empty());
        apply(Action::FlyTo(Some(s.stars[0].id)), &mut ui, &mut s);
        assert_eq!(apply(Action::AbortFlight, &mut ui, &mut s).len(), 1);
        assert!(s.cruise.is_none());
    }

    #[test]
    fn the_drive_setting_is_clamped_to_something_flyable() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetDriveAccel(1e9), &mut ui, &mut s);
        assert_eq!(s.drive.accel_g, MAX_ACCEL_G);
        apply(Action::SetDriveAccel(-4.0), &mut ui, &mut s);
        assert_eq!(s.drive.accel_g, MIN_ACCEL_G);
    }

    /// The setting has to reach the crossing, not just the readout.
    #[test]
    fn a_harder_drive_plans_a_shorter_crossing() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::SetDriveAccel(1.0), &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        let slow = s.cruise.as_ref().unwrap().duration_s();
        apply(Action::SetDriveAccel(50.0), &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        assert!(s.cruise.as_ref().unwrap().duration_s() < slow);
    }
}
