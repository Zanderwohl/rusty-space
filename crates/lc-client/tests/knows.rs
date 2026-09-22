//! What a ship holds after the client has started one up.
//!
//! Runs the real catalog (119 625 rows) through the real session: the unit tests' three
//! authored stars show neither the charted volume nor the cost of a sweep.

use lc_client::session::{CHARTED_LY, Session};
use lc_world::knowledge::survey::{Duty, Sweep};
use lc_world::sky::{StarProvider, hyg::HygProvider};

fn catalogue() -> Option<HygProvider> {
    HygProvider::load("../../assets/catalogs/hygdata_v42.csv").ok()
}

#[test]
fn a_ship_leaves_port_with_the_charts_of_its_own_volume_and_no_more() {
    let Some(provider) = catalogue() else { return };
    let mut session = Session::new(&provider, 6000);
    assert!(session.knowledge.is_empty());
    session.issue_charts(CHARTED_LY);

    let charted = session.knowledge.len();
    let inside = provider
        .stars()
        .iter()
        .filter(|s| s.position_ly.length() <= CHARTED_LY && s.position_ly.length() > 0.0)
        .count();
    assert_eq!(
        charted, inside,
        "every star inside the charted volume and nothing outside it"
    );
    assert!(
        charted < session.stars.len() / 4,
        "{charted} of {}",
        session.stars.len()
    );

    // Held on the charts' word, at a percent of the range.
    let far = session
        .knowledge
        .beliefs()
        .max_by(|a, b| {
            let d =
                |b: &lc_world::knowledge::Belief| b.distance.from(glam::DVec3::ZERO).unwrap_or(0.0);
            d(a).total_cmp(&d(b))
        })
        .expect("something is charted");
    assert!(!far.triangulated);
    assert_eq!(far.hops, 1);
    let truth = session.star(far.star().unwrap()).unwrap().position_ly;
    let believed = far.distance.position_ly().unwrap();
    assert!(
        believed.distance(truth) > 0.0,
        "a measurement is not the truth"
    );
    assert!(believed.distance(truth) < 0.05 * truth.length());
}

#[test]
fn a_sweep_finds_stars_the_charts_never_reached() {
    let Some(provider) = catalogue() else { return };
    let mut session = Session::new(&provider, 6000);
    session.issue_charts(CHARTED_LY);
    let charted = session.knowledge.len();

    session.take_up(Duty::Sweep(Sweep::all_sky(session.coordinate_time_s())));
    let pass_s = match &session.observatory.duty {
        Duty::Sweep(sweep) => sweep.pass_s(),
        _ => unreachable!(),
    };
    // Two passes, in steps a twentieth of one: about a fortnight of in-game time.
    for _ in 0..40 {
        session.advance(pass_s / lc_client::session::TIME_RATE / 20.0);
        session.tick_instruments(1.0);
    }
    let found = session.knowledge.len();
    assert!(
        found > charted,
        "{found} known after two passes, {charted} charted"
    );

    let missed = session
        .stars
        .iter()
        .filter(|s| !session.knowledge.knows(s.id))
        .count();
    assert!(
        missed > 0,
        "a sky with no blind spots is a bug, not a good telescope"
    );
    // Some of what it missed is hidden in a brighter source's glare, not merely not reached yet.
    let optics = lc_world::knowledge::survey::Optics::of(session.telescope);
    let band = optics.band().expect("a sensor with a band");
    let mut sky = lc_world::knowledge::observatory::Sky::new(std::sync::Arc::new(session.stars.clone()));
    let sources = sky.sources(band, session.ship.motion.position_ly);
    let glared = session
        .stars
        .iter()
        .enumerate()
        .filter(|(_, s)| !session.knowledge.knows(s.id))
        .filter(|(i, _)| lc_world::knowledge::survey::hidden_by(&sources, *i, optics.resolution_rad(band)).is_some())
        .count();
    assert!(glared > 0, "of {missed} missed, none was hidden by glare");
}
