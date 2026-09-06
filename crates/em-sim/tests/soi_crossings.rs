//! Detecting where a path meets the edge of a sphere of influence.
//!
//! The `soi_test` preset exists for this: three spacecraft chosen so that one must cross a
//! boundary twice a revolution, one must cross a *moving* boundary, and one must never
//! cross at all.

use em_foundations::time::{Instant, TimeDelta};
use em_sim::id::BodyIndex;
use em_sim::influence;
use em_sim::presets::soi_test;
use em_sim::propagate;
use em_sim::system::System;

fn built() -> System {
    let mut system = System::from_contents(&soi_test()).expect("the test system builds");
    propagate::evaluate_at(&mut system, Instant::J2000);
    system
}

fn body(system: &System, name: &str) -> BodyIndex {
    system.by_name(name).unwrap_or_else(|| panic!("no body named {name}"))
}

/// The premise: Earth has a sphere at all, and the craft straddle it.
#[test]
fn the_test_system_is_shaped_as_advertised() {
    let s = built();
    let earth = influence::soi_now(&s, body(&s, "Earth")).expect("Earth orbits Sol, so it has one");
    assert!((earth.bounding_radius() - 9.246e8).abs() < 1.0e7, "{:e}", earth.bounding_radius());

    let luna = influence::soi_now(&s, body(&s, "Luna")).expect("Luna orbits Earth");
    assert!((luna.bounding_radius() - 6.62e7).abs() < 1.0e6, "{:e}", luna.bounding_radius());
}

/// An orbit that straddles the boundary crosses it exactly twice a revolution — out at
/// apoapsis, back in before periapsis — and the two alternate.
#[test]
fn a_straddling_orbit_crosses_twice_per_revolution() {
    let s = built();
    let (craft, earth) = (body(&s, "SC-APO"), body(&s, "Earth"));

    // The craft's period is ~74.8 d; three revolutions should give six crossings.
    let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(3.0 * 74.8));
    let found = influence::crossings(&s, craft, earth, window);
    assert_eq!(found.len(), 6, "expected six crossings, got {}", found.len());

    for pair in found.windows(2) {
        assert!(pair[0].time < pair[1].time, "crossings must come out in time order");
        assert_ne!(pair[0].entering, pair[1].entering,
            "an exit must be followed by an entry and vice versa");
    }
}

/// Every reported crossing actually sits on the boundary, and the flag matches the
/// direction of travel through it.
#[test]
fn a_crossing_lands_on_the_boundary_facing_the_right_way() {
    let s = built();
    let (craft, earth) = (body(&s, "SC-APO"), body(&s, "Earth"));
    let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(160.0));

    for crossing in influence::crossings(&s, craft, earth, window) {
        let soi = influence::soi_at(&s, earth, crossing.time).unwrap();
        let offset = crossing.position - soi.centre;
        let error = offset.length() - soi.radius_toward(offset);
        // A metre on a 9.25e8 m boundary; the bisection tolerance is a millisecond of
        // flight, and the craft moves under a km/s out here.
        assert!(error.abs() < 1.0, "off the boundary by {error:e} m");

        // Straddle the moment and check the sign really flips the way the flag claims.
        let nudge = TimeDelta::from_seconds(60.0);
        let before = influence::boundary_distance(&s, craft, earth, crossing.time - nudge).unwrap();
        let after = influence::boundary_distance(&s, craft, earth, crossing.time + nudge).unwrap();
        if crossing.entering {
            assert!(before > 0.0 && after < 0.0, "entering: {before:e} -> {after:e}");
        } else {
            assert!(before < 0.0 && after > 0.0, "leaving: {before:e} -> {after:e}");
        }
    }
}

/// The point of doing this analytically: a window in the past costs what one in the future
/// costs, and `previous_crossing` is not a different code path.
#[test]
fn the_search_runs_backwards_as_well_as_forwards() {
    let s = built();
    let (craft, earth) = (body(&s, "SC-APO"), body(&s, "Earth"));
    let from = Instant::J2000 + TimeDelta::from_days(100.0);
    let horizon = TimeDelta::from_days(120.0);

    let next = influence::next_crossing(&s, craft, earth, from, horizon).expect("one ahead");
    let previous = influence::previous_crossing(&s, craft, earth, from, horizon).expect("one behind");

    assert!(next.time > from);
    assert!(previous.time < from);
    // And they bracket `from` with nothing in between.
    let between = influence::crossings(&s, craft, earth, (previous.time, next.time));
    assert!(between.is_empty() || between.iter().all(|c| c.time <= previous.time || c.time >= next.time),
        "previous and next must be adjacent");
}

/// Luna's sphere is not sitting still — it is going round Earth at a kilometre a second.
/// Catching this one means the search really is evaluating the boundary at each instant
/// rather than freezing it at the current time.
#[test]
fn a_moving_sphere_is_crossed_too() {
    let s = built();
    let (craft, luna) = (body(&s, "SC-LUN"), body(&s, "Luna"));
    let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(30.0));

    let found = influence::crossings(&s, craft, luna, window);
    assert_eq!(found.len(), 2, "one entry and one exit, got {found:?}");
    assert!(found[0].entering, "the first must be the way in");
    assert!(!found[1].entering, "the second must be the way out");

    // It is a flyby, not an impact: closest approach clears the surface comfortably.
    let midpoint = found[0].time + (found[1].time - found[0].time) / 2.0;
    let craft_position = propagate::position_at(&s, craft, midpoint).unwrap();
    let luna_position = propagate::position_at(&s, luna, midpoint).unwrap();
    let separation = (craft_position - luna_position).length();
    assert!(separation > 2.0 * s.radius(luna), "would hit the Moon: {separation:e} m");
    assert!(separation < 6.62e7, "should be inside the sphere: {separation:e} m");
}

/// The control. A detector that invents roots fails here and nowhere else.
#[test]
fn an_orbit_that_never_leaves_reports_nothing() {
    let s = built();
    let (craft, earth) = (body(&s, "SC-INN"), body(&s, "Earth"));
    let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(365.0));

    assert!(influence::crossings(&s, craft, earth, window).is_empty());
    // And it really is inside the whole time, rather than never evaluated.
    let inside = influence::boundary_distance(&s, craft, earth, Instant::J2000).unwrap();
    assert!(inside < 0.0, "the control craft should start inside: {inside:e}");
}

/// What the app asks for: the spheres a craft could actually reach.
#[test]
fn candidates_are_the_primary_and_its_other_children() {
    let s = built();
    let craft = body(&s, "SC-APO");
    let names: Vec<&str> = influence::crossing_candidates(&s, craft)
        .into_iter().map(|i| s.name(i)).collect();

    assert!(names.contains(&"Earth"), "the sphere it is inside: {names:?}");
    assert!(names.contains(&"Luna"), "a sibling it could reach: {names:?}");
    assert!(!names.contains(&"SC-APO"), "not itself: {names:?}");
    assert!(!names.contains(&"Sol"), "Sol is a root and has no surface: {names:?}");
}
