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
    /// The ship's own clock. Behind [`Hud::clock`] by whatever the ship has flown.
    pub ship_clock: String,
    /// The selected target and how old its light is, if anything is selected.
    pub target: Option<String>,
    /// Band mapping in force.
    pub mapping: String,
    /// Exposure relative to automatic.
    pub exposure: String,
    /// Set while a crossing is under way.
    pub flight: Option<String>,
    /// Set while the ship is falling rather than flying.
    pub coasting: Option<String>,
    /// Set when the clock is running at something other than the canonical rate.
    pub warning: Option<String>,
}

/// What an arc reads as: the two apsides, or the periapsis alone on an escape.
///
/// Apsides rather than elements. Nobody looks at an eccentricity and knows whether they are
/// about to hit the planet.
pub fn arc(coast: &crate::coast::Coast) -> String {
    let near = span(coast.periapsis_m());
    match coast.apoapsis_m() {
        Some(far) => format!("{near} by {} about {}", span(far), coast.primary),
        None => format!("escaping {} past {near}", coast.primary),
    }
}

/// A distance in whatever unit makes it readable.
fn span(metres: f64) -> String {
    match metres {
        m if m < 1.0e6 => format!("{:.0} km", m / 1.0e3),
        m if m < 1.0e9 => format!("{:.0} thousand km", m / 1.0e6),
        m if m < 1.0e11 => format!("{:.2} million km", m / 1.0e9),
        m => format!("{:.2} AU", m / 1.495_978_707e11),
    }
}

/// What a standing intercept reads as, in the place a crossing's progress would be: who, whether
/// the approach is still being flown, and how much space there is between the two hulls.
///
/// No percentage, because there is nothing to be a percentage of — a pursuit re-plans whenever
/// its quarry does something new. And between the hulls rather than between centres, because
/// that is what a closeness is set in; see `lc_world::pursuit::Closeness`.
pub fn pursuit(
    session: &Session,
    pursuit: lc_proto::Pursuit,
    quarry: Option<&crate::uplink::Contact>,
) -> String {
    let doing = match session.ship.motion.still_closing(session.coordinate_time_s()) {
        true => "closing on",
        false => "alongside",
    };
    let how = match pursuit.closeness {
        lc_proto::Closeness::Company => "in company",
        lc_proto::Closeness::Intimate => "close in",
    };
    let Some(quarry) = quarry else {
        return format!("{doing} ship {} — {how}", pursuit.quarry.0);
    };
    let centres_m =
        session.ship.motion.position_ly.distance(quarry.position_ly) * crate::system::M_PER_LY;
    let clear_m = (centres_m - 0.5 * (session.ship.length_m + quarry.length_m)).max(0.0);
    format!("{doing} {} — {} between hulls — {how}", quarry.name, near(clear_m))
}

/// A short distance, finely enough to see a kilometre-and-a-quarter wander.
fn near(metres: f64) -> String {
    match metres {
        m if m < 1.0e3 => format!("{m:.0} m"),
        m if m < 1.0e5 => format!("{:.1} km", m / 1.0e3),
        m => span(m),
    }
}

pub fn lines(session: &Session, ui: &UiState) -> Hud {
    let name = presets::all().get(ui.preset).map(|(n, _)| *n).unwrap_or("custom");
    Hud {
        clock: format!("T + {:.2} years", session.coordinate_time_s() / YEAR_S),
        ship_clock: format!("ship {:.2} years", session.ship.motion.clock_s / YEAR_S),
        target: ui.selected.and_then(|id| {
            let star = session.star(id)?;
            // Distance directly, not by way of sky(): shading six thousand stars once a frame
            // to read one number off the result is the same answer at six thousand times the
            // cost. A light-year of distance is a year of staleness by definition.
            let age = session.distance_to(star);
            let label = star.name.clone().unwrap_or_else(|| format!("star {:x}", id.get()));
            Some(format!("{label} — light is {age:.2} years old"))
        }),
        mapping: name.to_uppercase(),
        exposure: match ui.exposure_offset {
            o if o.abs() < 1e-6 => "auto".to_string(),
            o => format!("{o:+.1} stops"),
        },
        flight: session.cruise().as_ref().map(|c| {
            let left = (c.duration_s() - (session.coordinate_time_s() - c.start_s)).max(0.0);
            format!(
                "{:?} — {:.0}% — {:.4}c — {:.2} years to go",
                c.at(session.coordinate_time_s()).phase,
                c.progress(session.coordinate_time_s()) * 100.0,
                session.ship.motion.beta.length(),
                left / YEAR_S,
            )
        }),
        coasting: session.coast().map(arc),
        // Anything but the design rate is said on screen rather than left to look normal —
        // whether the player set it offline or a shard staging a scene stated it. The clock
        // running sixty times over is exactly when a readout of how fast earns its place.
        warning: (ui.time_rate != 1.0).then(|| crate::ui::rate_label(ui.time_rate)),
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

    /// The pursuit reads where a crossing's progress would, and in hull clearance.
    #[test]
    fn a_pursuit_says_who_and_how_much_space_is_between_the_hulls() {
        let (_, mut s) = fixture();
        s.ship.length_m = 500.0;
        let quarry = crate::uplink::Contact {
            ship_id: lc_proto::ShipId(7),
            name: "Anvil".into(),
            length_m: 5_000.0,
            position_ly: s.ship.motion.position_ly + glam::DVec3::X * 3_750.0 / crate::system::M_PER_LY,
            beta: glam::DVec3::ZERO,
            facing: glam::DVec3::X,
            jet_power_w: 0.0,
            emitted_s: 0.0,
        };
        let close = lc_proto::Pursuit { quarry: quarry.ship_id, closeness: lc_proto::Closeness::Intimate };
        assert_eq!(pursuit(&s, close, Some(&quarry)), "alongside Anvil — 1.0 km between hulls — close in");
        assert_eq!(pursuit(&s, close, None), "alongside ship 7 — close in");
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

    /// The development default is itself non-canonical, and has to say so: a sixty-times
    /// clock that looked normal would make every duration on screen a lie.
    #[test]
    fn a_non_canonical_clock_rate_is_announced() {
        let (mut ui, mut s) = fixture();
        assert_eq!(lines(&s, &ui).warning.unwrap(), "1 year / minute", "the test rate must be flagged");
        apply(Action::SetTimeRate(64.0), &mut ui, &mut s);
        let off_ladder = lines(&s, &ui).warning.expect("one off the ladder must be flagged");
        assert_eq!(off_ladder, "1 year / 56 seconds");
        // And a rate *below* the design one is flagged just as loudly. A scene runs slowly so
        // an orbit can be looked at, and a slow clock is no more normal than a fast one.
        apply(Action::SetTimeRate(0.05), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).warning.unwrap(), "7 minutes / second");
        apply(Action::SetTimeRate(1.0), &mut ui, &mut s);
        assert!(lines(&s, &ui).warning.is_none(), "the canonical rate needs no warning");
    }

    /// The readout's job during a crossing: the two clocks disagree and it has to show both.
    #[test]
    fn a_crossing_reports_itself_and_the_ship_clock_falls_behind() {
        let (ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert!(lines(&s, &ui).flight.is_none());
        s.fly_to(id);
        s.advance(3600.0);
        let l = lines(&s, &ui);
        let flight = l.flight.expect("a flight line");
        assert!(flight.contains('c') && flight.contains("to go"), "{flight}");
        let coordinate: f64 = l.clock.trim_start_matches("T + ").trim_end_matches(" years").parse().unwrap();
        let aboard: f64 = l.ship_clock.trim_start_matches("ship ").trim_end_matches(" years").parse().unwrap();
        assert!(aboard < coordinate, "ship {aboard} should be behind coordinate {coordinate}");
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
