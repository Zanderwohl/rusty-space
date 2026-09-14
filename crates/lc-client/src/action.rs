//! Every UI action, as a value.
//!
//! No system acts directly. A key press, a button, a menu item and a test all produce the
//! same [`Action`], and [`apply`] is the only thing that changes [`crate::ui::UiState`] or
//! reaches into a [`Session`]. Rebinding is then a table rather than a rewrite, and a UI can
//! be driven from a test with no window.

use em_spectra::{Band, presets};
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
    /// Point at the nearest star that is actually interstellar.
    SelectNearest,
    SetIntegration(f64),
    /// Which band the light curve measures. Independent of the display mapping.
    SetCurveBand(Band),

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
    /// Step one rung along [`crate::ui::RATE_LADDER`].
    TimeRateUp,
    TimeRateDown,
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
        Action::SelectNearest => match nearest_interstellar(session) {
            Some(id) => apply_to(ui, session, Action::SelectTarget(Some(id)), &mut effects),
            None => effects.push(Effect::Notify("nothing interstellar in range".into())),
        },
        Action::SetIntegration(seconds) => ui.integration_s = seconds.max(0.0),
        Action::SetCurveBand(band) => {
            if !session.telescope.sees(band) {
                effects.push(Effect::Notify(format!("the sensor cannot reach {band:?}")));
            } else if session.curve.band() != band {
                session.curve.set_band(band);
                effects.push(Effect::Notify(format!("curve: {band:?}")));
            }
        }

        Action::Look { yaw, pitch } => ui.look.turn(yaw, pitch),
        Action::LookAtSelected => match aim(ui, session) {
            Some(look) => ui.look = look,
            None => effects.push(Effect::Notify("nothing is selected to look at".into())),
        },

        Action::FlyTo(id) => fly(ui, session, id, &mut effects),
        Action::FlyToNearest => {
            apply_to(ui, session, Action::SelectNearest, &mut effects);
            let id = ui.selected;
            if id.is_some() {
                fly(ui, session, id, &mut effects);
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
        Action::TimeRateUp | Action::TimeRateDown => {
            ui.time_rate = crate::ui::rate_step(ui.time_rate, action == Action::TimeRateUp);
            effects.push(Effect::Notify(format!("clock: {}", crate::ui::rate_label(ui.time_rate))));
        }
        Action::WriteSnapshot => effects.push(Effect::WriteSnapshot),
    }
    effects
}

/// Run a nested action, keeping its effects. Only for actions composed of other actions.
fn apply_to(ui: &mut UiState, session: &mut Session, action: Action, effects: &mut Vec<Effect>) {
    effects.extend(apply(action, ui, session));
}

/// The nearest star worth pointing at.
///
/// Not simply the first: the catalogue carries the Sun at about an astronomical unit, and
/// "nearest star" has to mean one that is somewhere else.
fn nearest_interstellar(session: &Session) -> Option<StarId> {
    session.stars.iter().find(|s| session.distance_to(s) > INTERSTELLAR_LY).map(|s| s.id)
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
    session.retune();
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
    use crate::ui::{RATE_LADDER, rate_label};

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

    #[test]
    fn the_clock_steps_along_the_ladder_and_stops_at_the_ends() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(RATE_LADDER[0].0), &mut ui, &mut s);
        apply(Action::TimeRateDown, &mut ui, &mut s);
        assert_eq!(ui.time_rate, RATE_LADDER[0].0, "the bottom rung is the bottom");
        for _ in 0..20 {
            apply(Action::TimeRateUp, &mut ui, &mut s);
        }
        assert_eq!(ui.time_rate, RATE_LADDER[RATE_LADDER.len() - 1].0, "and the top is the top");
    }

    #[test]
    fn stepping_from_a_rate_off_the_ladder_still_moves_the_right_way() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(200.0), &mut ui, &mut s);
        apply(Action::TimeRateUp, &mut ui, &mut s);
        assert!(ui.time_rate > 200.0, "went to {}", ui.time_rate);
        apply(Action::SetTimeRate(200.0), &mut ui, &mut s);
        apply(Action::TimeRateDown, &mut ui, &mut s);
        assert!(ui.time_rate < 200.0, "went to {}", ui.time_rate);
    }

    /// Every rung has to be reachable by stepping, or a key cannot get to it.
    #[test]
    fn every_rung_is_reachable_by_stepping_up_from_the_bottom() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(RATE_LADDER[0].0), &mut ui, &mut s);
        for (rate, _) in RATE_LADDER.iter().skip(1) {
            apply(Action::TimeRateUp, &mut ui, &mut s);
            assert_eq!(ui.time_rate, *rate, "the ladder skipped a rung");
        }
    }

    #[test]
    fn a_rate_is_named_by_what_it_feels_like() {
        assert_eq!(rate_label(60.0), "1 year / minute");
        assert_eq!(rate_label(360.0), "1 year / 10 s");
        assert!(rate_label(123.0).contains("123"), "an unnamed rate still reads");
    }

    /// The exit criterion, driven entirely through actions with no window.
    #[test]
    fn a_swarm_reads_as_a_deficit_in_the_visible_and_an_excess_in_the_thermal() {
        use lc_world::sky::{AuthoredStars, StarProvider};
        let provider = AuthoredStars::sample();
        let Some(star) = provider.stars().iter().find(|s| {
            lc_world::sky::generate::swarm_for(s).is_some()
        }) else {
            // The sample sky is three stars and may carry no swarm. The catalogue test covers
            // the populated case; this one has nothing to say.
            return;
        };
        let mut s = Session::new(&provider, 3);
        let mut ui = UiState::default();
        apply(Action::SelectTarget(Some(star.id)), &mut ui, &mut s);

        let watch = |s: &mut Session, ui: &mut UiState, band: Band| {
            apply(Action::SetCurveBand(band), ui, s);
            for _ in 0..40 {
                s.advance(30.0);
                s.observe(1.0e4);
            }
            let samples = s.curve.samples().to_vec();
            samples.iter().map(|(_, d)| *d).sum::<f64>() / samples.len() as f64
        };
        assert!(watch(&mut s, &mut ui, Band::V) > 0.0, "the visible must be a shadow");
        assert!(watch(&mut s, &mut ui, Band::ThermalIr) < 0.0, "and the thermal a source");
    }

    #[test]
    fn changing_the_curve_band_discards_the_old_measurements() {
        let (mut ui, mut s) = fixture();
        apply(Action::SelectTarget(Some(s.stars[0].id)), &mut ui, &mut s);
        s.observe(1.0e4);
        assert!(!s.curve.is_empty());
        apply(Action::SetCurveBand(Band::K), &mut ui, &mut s);
        assert!(s.curve.is_empty(), "two bands are not one series");
        assert_eq!(s.curve.band(), Band::K);
    }

    #[test]
    fn a_band_the_sensor_cannot_reach_is_refused_rather_than_measured() {
        let (mut ui, mut s) = fixture();
        s.telescope = s.telescope.with_bands(em_spectra::BandMask::SILICON);
        let effects = apply(Action::SetCurveBand(Band::Radio), &mut ui, &mut s);
        assert!(matches!(effects.as_slice(), [Effect::Notify(t)] if t.contains("cannot reach")));
        assert_ne!(s.curve.band(), Band::Radio);
    }

    /// The telescope must not be limited to the stars the sky happens to model.
    #[test]
    fn any_star_in_the_list_can_be_watched() {
        let (mut ui, mut s) = fixture();
        let last = s.stars.last().unwrap().id;
        apply(Action::SelectTarget(Some(last)), &mut ui, &mut s);
        assert!(s.target(last).is_some(), "pointing at a star should model it");
        assert!(s.observe(1.0e4).is_some());
    }
}
