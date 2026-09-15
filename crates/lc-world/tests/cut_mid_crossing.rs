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

/// Ordering a crossing while one is already under way, to somewhere further along.
///
/// `Cruise::plan` was a **rest-to-rest** brachistochrone: it took a start, an end, a time and a
/// drive, and no velocity at all. So replacing a crossing in flight planned a fresh profile from
/// standstill and the speed already built up was silently discarded — which read in the
/// interface as the velocity dropping to 0.00c the instant a new destination was chosen.
#[test]
fn a_crossing_ordered_in_flight_keeps_the_speed_already_built() {
    let mut craft = crossing_craft();
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("premise") };
    let at = cruise.duration_s() * 0.10;
    craft.advance(at, at);

    let moving = craft.motion.beta.length();
    assert!(moving > 0.01, "premise: actually under way at {moving}c");

    // Further along the way it is already going, which is what re-aiming usually is.
    let further = DVec3::new(9.0, 0.0, 0.0);
    motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: at,
            change: Change::Cross { to_ly: further, drive: Kind::Ship.drive() },
        },
    )
    .expect("a crossing from a moving start");

    // A frame later, not immediately: the fold leaves `beta` stale until something advances.
    craft.advance(at + 1.0, 1.0);
    let after = craft.motion.beta.length();
    assert!(
        after >= moving,
        "the ship was at {moving}c and the new crossing left it at {after}c",
    );
}

/// From an orbit, which is the other case: a small velocity in whatever direction it happens to
/// be. It is carried rather than discarded, and the sideways part of it is far too small to
/// matter.
#[test]
fn a_crossing_ordered_from_an_orbit_keeps_its_speed() {
    let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
    // Thirty kilometres a second, mostly across the line rather than along it.
    craft.motion.beta = DVec3::new(0.00002, 0.00009, 0.0);
    craft.motion.set_adrift(0.0);
    let moving = craft.motion.beta.length();

    motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: 0.0,
            change: Change::Cross { to_ly: DVec3::new(4.0, 0.0, 0.0), drive: Kind::Ship.drive() },
        },
    )
    .expect("a crossing from an orbit");

    craft.advance(1.0, 1.0);
    assert!(
        craft.motion.beta.length() > moving * 0.1,
        "an orbit's speed was thrown away: {moving}c became {}c",
        craft.motion.beta.length(),
    );
}

/// **The limit of a straight-line plan, pinned so it is a known shape rather than a surprise.**
///
/// Re-aiming ninety degrees at relativistic speed: the component along the new line is carried
/// exactly, and the component across it is not, because shedding it curves the path and a
/// `Cruise` is a straight line between two points. Fixing that means giving the plan a matching
/// segment of its own.
#[test]
fn a_hard_sideways_re_aim_still_loses_what_is_across_the_line() {
    let mut craft = crossing_craft();
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("premise") };
    let at = cruise.duration_s() * 0.10;
    craft.advance(at, at);
    let moving = craft.motion.beta.length();

    // Ninety degrees off: almost all of the velocity is across the new line.
    motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: at,
            change: Change::Cross { to_ly: DVec3::new(0.0, 3.0, 0.0), drive: Kind::Ship.drive() },
        },
    )
    .expect("accepted");

    craft.advance(at + 1.0, 1.0);
    let after = craft.motion.beta.length();
    assert!(after < moving, "premise: the across-the-line part is what is lost");
    assert!(
        after > 0.0,
        "even a sideways re-aim keeps what little is along the new line: {after}c",
    );
}
