//! What a ship holds after the client has actually started one up.
//!
//! The unit tests use three authored stars. This runs the real catalogue through the real
//! session, because "the charts cover twenty light-years" is a claim about 119 625 rows and
//! the cost of a sweep over them is not visible at three.

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

    // Held on the charting office's word, at a percent of the range.
    let far = session
        .knowledge
        .beliefs()
        .max_by(|a, b| {
            let d =
                |b: &lc_world::knowledge::Belief| b.distance.from(glam::DVec3::ZERO).unwrap_or(0.0);
            d(a).total_cmp(&d(b))
        })
        .expect("something is charted");
    assert!(!far.measured_here);
    assert_eq!(far.hops, 1);
    let truth = session.star(far.star).unwrap().position_ly;
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
    let pass_s = match &session.duty {
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

    // What a sweep does not find is what the sun it is sitting beside is in front of.
    let missed = session
        .stars
        .iter()
        .filter(|s| !session.knowledge.knows(s.id))
        .count();
    assert!(
        missed > 0,
        "a sky with no blind spots is a bug, not a good telescope"
    );
}
