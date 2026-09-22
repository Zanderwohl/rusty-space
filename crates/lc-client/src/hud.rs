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
    /// The ship's own clock, `T'`. Behind [`Hud::clock`] by whatever the ship has flown.
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
    /// Stored energy, for a ship with modules.
    pub energy: Option<Energy>,
}

/// The energy readout: a bar, and the numbers beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Energy {
    /// Stored over capacity, `[0, 1]`.
    pub fraction: f32,
    /// `23.4 / 30.0 ME`, and what is spoken for: a plan's commitment or a refit under way.
    pub amount: String,
}

/// What an arc reads as: the two apsides, or the periapsis alone on an escape.
///
/// Apsides rather than elements. Nobody looks at an eccentricity and knows whether they are
/// about to hit the planet.
/// `primary` is what this ship calls the body, not its key.
pub fn arc(coast: &crate::coast::Coast, primary: &str) -> String {
    let near = span(coast.periapsis_m());
    match coast.apoapsis_m() {
        Some(far) => format!("{near} by {} about {primary}", span(far)),
        None => format!("escaping {primary} past {near}"),
    }
}

/// A distance in whatever unit makes it readable.
fn span(meters: f64) -> String {
    match meters {
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
/// its quarry does something new. And between the hulls rather than between centers, because
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
    let centers_m =
        session.ship.motion.position_ly.distance(quarry.position_ly) * crate::system::M_PER_LY;
    let clear_m = (centers_m - 0.5 * (session.ship.length_m + quarry.length_m)).max(0.0);
    format!("{doing} {} — {} between hulls — {how}", quarry.name, near(clear_m))
}

/// A short distance, finely enough to see a kilometer-and-a-quarter wander.
fn near(meters: f64) -> String {
    match meters {
        m if m < 1.0e3 => format!("{m:.0} m"),
        m if m < 1.0e5 => format!("{:.1} km", m / 1.0e3),
        m => span(m),
    }
}

pub fn lines(session: &Session, ui: &UiState) -> Hud {
    let name = presets::all().get(ui.preset).map(|(n, _)| *n).unwrap_or("custom");
    Hud {
        clock: format!("T + {:.2} years", session.coordinate_time_s() / YEAR_S),
        ship_clock: format!("T' + {:.2} years", session.ship.motion.clock_s / YEAR_S),
        // What this ship believes, never the catalogue: a click on any light in the sky is not
        // a range to it.
        target: ui.selected.map(|id| {
            let range = crate::range::short(session.knowledge.belief(id), session.ship.motion.position_ly);
            format!("{} — {range}", session.name_of(id))
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
        coasting: session.coast().map(|coast| arc(&coast, &session.body_label(&coast.primary))),
        // Anything but the design rate is said on screen rather than left to look normal —
        // whether the player set it offline or a shard staging a scene stated it. The clock
        // running sixty times over is exactly when a readout of how fast earns its place.
        warning: (ui.time_rate != 1.0).then(|| crate::ui::rate_label(ui.time_rate)),
        energy: energy(session),
    }
}

fn energy(session: &Session) -> Option<Energy> {
    let now = session.coordinate_time_s();
    let ship = &session.ship;
    let fitting = ship.fitting()?;
    let module_j = fitting.balance.module_energy_j();
    let (stored, capacity) = (fitting.stored_j_at(&ship.motion, now), fitting.capacity_j_at(now));
    let mut line = format!("{:.1} / {:.1} ME", stored / module_j, capacity / module_j);
    if fitting.solar_w() > 0.0 {
        let net = fitting.solar_w() - fitting.balance.drain_w(&fitting.loadout_at(now));
        line += &format!(" {}", crate::refit_panel::me_per_year(net, module_j));
    }
    let committed = fitting.committed_j_at(&ship.motion, now);
    if committed > 0.0 {
        line += &format!(" ({:.2} committed)", committed / module_j);
    }
    if ship.is_refitting(now) {
        line += " — REFITTING";
    }
    let fraction = if capacity > 0.0 { (stored / capacity).clamp(0.0, 1.0) as f32 } else { 0.0 };
    Some(Energy { fraction, amount: line })
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::action::{Action, apply};

    fn fixture() -> (UiState, Session) {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        // Charted, because a ship leaves port with charts and most of these tests are about
        // something else. `session::tests` is where an unsurveyed sky is the subject.
        session.issue_charts(30.0);
        (UiState::default(), session)
    }

    /// The pursuit reads where a crossing's progress would, and in hull clearance.
    #[test]
    fn a_pursuit_says_who_and_how_much_space_is_between_the_hulls() {
        let (_, mut s) = fixture();
        s.ship.length_m = 500.0;
        let at_ly = s.ship.motion.position_ly + glam::DVec3::X * 3_750.0 / crate::system::M_PER_LY;
        let quarry = crate::uplink::Contact::seen(
            lc_proto::Presence {
                ship_id: lc_proto::ShipId(7),
                name: "Anvil".into(),
                length_m: 5_000.0,
                at_ly: at_ly.to_array(),
                beta: [0.0; 3],
                facing: [1.0, 0.0, 0.0],
                jet_power_w: 0.0,
                emitted_t: 0,
                arrive_t: 0,
            },
            None,
        );
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

    /// The one thing the readout exists for, and only as far as this ship knows it: the range
    /// and its error. Whose word it is on is the telescope panel's to say.
    #[test]
    fn a_selected_target_says_how_far_it_is() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert!(lines(&s, &ui).target.is_none());
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        let target = lines(&s, &ui).target.expect("a target line");
        // The sample provider's nearest star is 4.2 light-years out, charted to a percent.
        assert!(target.contains(" ± ") && target.ends_with(" ly"), "{target}");
    }

    #[test]
    fn a_fitted_ship_shows_its_energy_as_a_bar_and_numbers() {
        use lc_world::fitting::{Balance, Fitting, Loadout};
        let (ui, mut s) = fixture();
        assert!(lines(&s, &ui).energy.is_none(), "an unfitted ship has no energy readout");
        s.ship.fit(Some(Fitting::full(Loadout::STARTING, Balance::DEFAULT, s.coordinate_time_s())));
        let energy = lines(&s, &ui).energy.expect("an energy readout");
        assert!((energy.fraction - 1.0).abs() < 1.0e-6, "{}", energy.fraction);
        assert_eq!(energy.amount, "30.0 / 30.0 ME");
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

    /// A rate that is not the world's has to say so: a sixty-times clock that looked normal
    /// would make every duration on screen a lie.
    ///
    /// The default is the design rate and therefore says nothing, which is the point of it —
    /// a warning that is always on is a warning nobody reads.
    #[test]
    fn a_non_canonical_clock_rate_is_announced() {
        let (mut ui, mut s) = fixture();
        assert!(lines(&s, &ui).warning.is_none(), "the default is the world's own rate");
        apply(Action::SetTimeRate(60.0), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui).warning.unwrap(), "1 year / minute", "a fast clock is flagged");
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
        let aboard: f64 = l.ship_clock.trim_start_matches("T' + ").trim_end_matches(" years").parse().unwrap();
        assert!(aboard < coordinate, "ship {aboard} should be behind coordinate {coordinate}");
    }

    /// A star nobody has named still has something to call it: the designation its own
    /// discovery wrote down. Nothing falls back to a catalogue.
    #[test]
    fn an_unnamed_star_still_gets_a_label() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        s.knowledge = lc_world::knowledge::Knowledge::new(lc_world::knowledge::Witness(0));
        s.point_at(Some(id));
        s.advance(1.0);
        s.tick_instruments(1.0);
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        let target = lines(&s, &ui).target.unwrap();
        // A second's stare has detected nothing yet, so there is neither a name nor a range.
        assert!(target.ends_with("not detected"), "{target}");
        assert!(
            !target.starts_with("Authored"),
            "a catalogue name is not a name: {target}"
        );
    }
}
