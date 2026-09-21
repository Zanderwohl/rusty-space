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
    // Thirty kilometers a second, mostly across the line rather than along it.
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

/// **A hard sideways re-aim sheds what is across the line, rather than losing it.**
///
/// Ninety degrees off at relativistic speed is the case a straight-line plan could not express:
/// almost all of the velocity is across the new line, and shedding it is a burn of its own. It
/// now is one — the crossing begins with a match, which takes real time and covers real ground,
/// and the speed is still there the instant after the order.
#[test]
fn a_hard_sideways_re_aim_sheds_across_the_line_rather_than_losing_it() {
    let mut craft = crossing_craft();
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("premise") };
    let at = cruise.duration_s() * 0.10;
    craft.advance(at, at);
    let moving = craft.motion.beta.length();
    assert!(moving > 0.5, "premise: genuinely fast, at {moving}c");

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

    // A frame later the ship is still going what it was going. Momentum does not vanish
    // because a destination was chosen.
    craft.advance(at + 1.0, 1.0);
    let just_after = craft.motion.beta.length();
    assert!(
        just_after > moving * 0.99,
        "the ship was at {moving}c and a frame later was at {just_after}c",
    );

    // And the match is real: it takes time, and by the end of it the ship is on the line.
    let Motive::Crossing(plan) = &craft.motion.motive else { panic!("still crossing") };
    // At the end, not near it: a brake still has speed at 99.9% of the way through.
    let arrived = plan.at(at + plan.duration_s());
    assert!(
        arrived.beta.length() < 1.0e-6,
        "a crossing still ends at rest: {}c",
        arrived.beta.length(),
    );
    // And once the match is done the ship flies a line: its heading at two different moments
    // mid-flight is the same heading. Taken from the trajectory rather than from `from_ly`,
    // which is where the ship was *ordered*, not where the line begins.
    let one = plan.at(at + plan.duration_s() * 0.5).beta.normalize();
    let two = plan.at(at + plan.duration_s() * 0.6).beta.normalize();
    assert!(
        one.dot(two) > 0.999_999,
        "the heading wandered mid-flight: {one:?} then {two:?}",
    );
}

/// Reported from play: inside a system, "go to the star" carried the ship **outward**.
///
/// A crossing stops about sixty astronomical units short of a star, so a ship six AU out is
/// already ten times nearer than the crossing would leave it. Flying it would be going away
/// from the thing you asked to go to.
#[test]
fn a_crossing_to_a_star_you_are_already_inside_does_not_fly_you_outward() {
    const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;
    let star = DVec3::ZERO;
    let mut craft = Craft::at(CraftId(1), Kind::Ship, star + DVec3::X * 6.0 * AU_LY);
    let was = craft.motion.position_ly;

    let outcome = motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: 0.0,
            change: Change::Cross { to_ly: star, drive: Kind::Ship.drive() },
        },
    );

    assert!(outcome.is_err(), "it accepted a crossing to a star it was already inside");
    craft.advance(60.0, 60.0);
    let moved = (craft.motion.position_ly - was).length();
    assert!(moved < 1.0e-9, "the ship moved {moved} light-years anyway");
}

/// And a crossing to somewhere genuinely far is still a crossing.
#[test]
fn a_crossing_to_a_star_further_than_the_standoff_still_flies() {
    let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
    motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: 0.0,
            change: Change::Cross { to_ly: DVec3::new(4.0, 0.0, 0.0), drive: Kind::Ship.drive() },
        },
    )
    .expect("four light-years is a crossing");
    assert!(matches!(craft.motion.motive, Motive::Crossing(_)));
}

/// The audit's other half: an in-system course keeps the speed the ship already has, rather
/// than planning from rest and dropping it to nothing.
#[test]
fn an_in_system_course_keeps_the_speed_the_ship_has() {
    use lc_world::navigation::Course;
    use lc_world::sky::{AuthoredStars, StarProvider};
    use lc_world::system::LocalSystem;

    let star = AuthoredStars::sample().stars().first().cloned().expect("a star");
    let system = std::sync::Arc::new(LocalSystem::for_star(&star).expect("a system"));
    let mut craft = Craft::at(CraftId(1), Kind::Ship, star.position_ly);
    craft.enter(Some(system), 0.0);
    craft.motion.beta = DVec3::new(0.0, 0.0006, 0.0);
    craft.motion.set_adrift(0.0);
    let moving = craft.motion.beta.length();

    motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event {
            ship: ShipId(0),
            at_t: 0.0,
            change: Change::SetCourse { course: Course::LeaveSystem, drive: Kind::Ship.drive() },
        },
    )
    .expect("leaving a system is a course");

    craft.advance(1.0, 1.0);
    let after = craft.motion.beta.length();
    assert!(after > moving * 0.5, "a course from {moving}c left the ship at {after}c");
}
