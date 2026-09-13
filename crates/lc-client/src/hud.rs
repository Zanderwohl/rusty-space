//! The always-visible readout.
//!
//! The game is about looking at the past, so how stale the picture is belongs on screen at
//! all times rather than in a panel a player can close.

use em_spectra::presets;

use crate::session::Session;
use crate::ui::UiState;

const YEAR_S: f64 = 31_557_600.0;

/// Lines the head-up display shows. Strings, so a test can read them.
#[derive(Clone, Debug, PartialEq)]
pub struct Hud {
    /// Coordinate time.
    pub clock: String,
    /// The selected target and how old its light is, if anything is selected.
    pub target: Option<String>,
    /// Band mapping in force.
    pub mapping: String,
    /// Exposure relative to automatic.
    pub exposure: String,
    /// Set when the clock is running at something other than the canonical rate.
    pub warning: Option<String>,
}

pub fn lines(session: &Session, ui: &UiState) -> Hud {
    let name = presets::all().get(ui.preset).map(|(n, _)| *n).unwrap_or("custom");
    Hud {
        clock: format!("T + {:.2} years", session.coordinate_time_s() / YEAR_S),
        target: ui.selected.and_then(|id| {
            let star = session.star(id)?;
            let age = session
                .sky()
                .into_iter()
                .find(|s| s.id == id)
                .map(|s| s.light_age_s / YEAR_S)
                .unwrap_or(0.0);
            let label = star.name.clone().unwrap_or_else(|| format!("star {:x}", id.get()));
            Some(format!("{label} — light is {age:.2} years old"))
        }),
        mapping: name.to_uppercase(),
        exposure: match ui.exposure_offset {
            o if o.abs() < 1e-6 => "auto".to_string(),
            o => format!("{o:+.1} stops"),
        },
        // The time rate is a development control and the server owns it; say so on screen
        // rather than letting a fast clock look normal.
        warning: (ui.time_rate != 1.0).then(|| format!("clock at {:.0}x", ui.time_rate)),
    }
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::action::{Action, apply};

    fn fixture() -> (UiState, Session) {
        (UiState::default(), Session::new(&AuthoredStars::sample(), 3))
    }

    #[test]
    fn the_clock_is_always_shown() {
        let (ui, mut s) = fixture();
        assert!(lines(&s, &ui).clock.contains("T + 0.00 years"));
        s.advance(3600.0);
        assert!(lines(&s, &ui).clock.contains("T + 1.00 years"));
    }

    /// The one thing the readout exists for.
    #[test]
    fn a_selected_target_says_how_old_its_light_is() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert!(lines(&s, &ui).target.is_none());
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        let target = lines(&s, &ui).target.expect("a target line");
        assert!(target.contains("light is"), "{target}");
        // The sample provider's nearest star is 4.2 light-years out.
        assert!(target.contains("4.2"), "{target}");
    }

    #[test]
    fn the_band_mapping_is_named_not_numbered() {
        let (mut ui, mut s) = fixture();
        assert_eq!(lines(&s, &ui).mapping, "NATURAL");
        apply(Action::SetBandPreset(2), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).mapping, "THERMAL");
    }

    #[test]
    fn exposure_reads_auto_until_it_is_moved() {
        let (mut ui, mut s) = fixture();
        assert_eq!(lines(&s, &ui).exposure, "auto");
        apply(Action::ExposureUp, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).exposure, "+0.5 stops");
        apply(Action::ExposureDown, &mut ui, &mut s);
        apply(Action::ExposureDown, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).exposure, "-0.5 stops");
        apply(Action::ExposureAuto, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).exposure, "auto");
    }

    #[test]
    fn a_non_canonical_clock_rate_is_announced() {
        let (mut ui, mut s) = fixture();
        assert!(lines(&s, &ui).warning.is_none());
        apply(Action::SetTimeRate(64.0), &mut ui, &mut s);
        assert!(lines(&s, &ui).warning.unwrap().contains("64"));
    }

    #[test]
    fn an_unnamed_star_still_gets_a_label() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        if let Some(star) = s.stars.iter_mut().find(|x| x.id == id) {
            star.name = None;
        }
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        assert!(lines(&s, &ui).target.unwrap().contains("light is"));
    }
}
