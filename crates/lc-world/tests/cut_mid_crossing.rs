//! Cutting the drive part-way through an interstellar crossing.
//!
//! Reported from play: fly toward a star, cut at two percent, and the burn does not stop. The
//! crossing is the one motive that is not about a local system, so the cut has no conic to fall
//! onto and has to become a drift — which is the case nothing else exercises.

use std::time::Instant as Clock;

use glam::DVec3;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::flight::Cruise;
use lc_world::motion::{self, Change, Event, Motive, ShipId};

fn crossing_craft() -> Craft {
    let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
    // Four light-years out along +X, which is the shape of a real crossing.
    let to = DVec3::new(4.0, 0.0, 0.0);
    let drive = Kind::Ship.drive();
    craft.motion.begin_crossing(Cruise::plan(DVec3::ZERO, to, 0.0, drive), None);
    craft.motion.drive = drive;
    craft
}

#[test]
fn a_cut_part_way_through_a_crossing_stops_the_burn() {
    let mut craft = crossing_craft();
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("premise: crossing") };
    let duration = cruise.duration_s();

    // Two percent of the way, which is where the report cut.
    let at = duration * 0.02;
    craft.advance(at, at);
    assert!(
        matches!(craft.motion.motive, Motive::Crossing(_)),
        "premise: still under way at two percent",
    );
    let under_way = craft.motion.position_ly;

    let _ = motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event { ship: ShipId(0), at_t: at, change: Change::CutDrive },
    );

    assert!(
        !matches!(craft.motion.motive, Motive::Crossing(_)),
        "the cut left the ship on its crossing: {:?}",
        craft.motion.motive,
    );

    // And it keeps the velocity it had, which is what a cut is — not a stop.
    assert!(craft.motion.beta.length() > 0.0, "the cut stopped the ship dead");
    assert!(
        (craft.motion.position_ly - under_way).length() < 1.0e-9,
        "the cut moved the ship",
    );
}

/// And the frames after it are frames, not searches.
#[test]
fn the_frames_after_a_cut_in_flight_are_cheap() {
    let mut craft = crossing_craft();
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("premise") };
    let at = cruise.duration_s() * 0.02;
    craft.advance(at, at);
    let _ = motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event { ship: ShipId(0), at_t: at, change: Change::CutDrive },
    );

    let step = 8766.0 / 60.0;
    let started = Clock::now();
    let mut now = at;
    for _ in 0..60 {
        now += step;
        craft.advance(now, step);
    }
    let each = started.elapsed() / 60;
    println!("a frame after a cut in flight costs {each:?}");
    assert!(each < std::time::Duration::from_millis(4), "{each:?} is most of a frame");
}
