//! The deliverable of the whole refactor: a solar system that propagates with no engine.
//!
//! Nothing here touches Bevy, an ECS, a window or a file. If any of it ever needs to, the
//! separation has regressed.

use em_foundations::time::{Instant, TimeDelta};
use em_sim::id::BodyId;
use em_sim::presets::solar_system;
use em_sim::propagate;
use em_sim::system::{System, SystemError};

fn built() -> System {
    System::from_contents(&solar_system()).expect("the bundled system must build")
}

#[test]
fn the_bundled_system_builds_without_an_engine() {
    let s = built();
    assert!(s.len() > 200, "expected the full system, got {}", s.len());
    assert!(s.by_name("Sol").is_some());
    assert!(s.by_name("Earth").is_some());
    assert!(s.by_name("Pluto Barycenter").is_some());
}

#[test]
fn every_parent_resolves() {
    let mut s = built();
    propagate::evaluate_at(&mut s, Instant::J2000);
    for i in s.indices() {
        if s.name(i) == "Sol" { continue; }
        assert!(s.parent(i).is_some() || s.motive(i).is_newtonian(Instant::J2000),
            "{} has no parent and is not Newtonian", s.name(i));
    }
}

/// Positions must match what the elements say directly — the arena is plumbing, not a
/// second implementation of the orbital mechanics.
#[test]
fn arena_positions_match_the_elements() {
    use em_sim::motive::MotiveSelection;
    let mut s = built();
    let t = Instant::from_julian_day(2460676.5);
    propagate::evaluate_at(&mut s, t);

    for name in ["Earth", "Luna", "Io", "Titan", "Charon", "Pluto"] {
        let i = s.by_name(name).unwrap();
        let (_, sel) = s.motive(i).motive_at(t);
        let MotiveSelection::Keplerian(k) = sel else { continue };
        let expected_local = k.displacement(t, s.mu(i)).unwrap();
        let parent = s.parent(i).unwrap();
        let expected = s.position(parent) + expected_local;
        let got = s.position(i);
        assert!((got - expected).length() < 1.0, "{name}: arena {got:?} vs elements {expected:?}");
    }
}

/// Evaluation is analytic, so any time costs the same and the route taken cannot matter.
#[test]
fn evaluation_is_order_independent() {
    let mut a = built();
    let mut b = built();
    let t1 = Instant::from_julian_day(2460676.5);

    propagate::evaluate_at(&mut a, t1);

    propagate::evaluate_at(&mut b, Instant::J2000);
    propagate::evaluate_at(&mut b, Instant::from_julian_day(2470000.0));
    propagate::evaluate_at(&mut b, t1);

    for i in a.indices() {
        let (pa, pb) = (a.position(i), b.position(i));
        assert!((pa - pb).length() < 1e-6, "{} depends on how it was reached", a.name(i));
    }
}

/// A year of the real system, stepped rather than evaluated, must land where evaluating
/// directly does.
#[test]
fn stepping_a_year_stays_on_the_orbits() {
    let mut stepped = built();
    let mut direct = built();

    let dt = TimeDelta::from_days(1.0);
    for _ in 0..365 {
        propagate::step(&mut stepped, dt);
    }
    propagate::evaluate_at(&mut direct, stepped.time());

    for name in ["Mercury", "Earth", "Jupiter", "Luna", "Io", "Titan"] {
        let i = stepped.by_name(name).unwrap();
        let j = direct.by_name(name).unwrap();
        let drift = (stepped.position(i) - direct.position(j)).length();
        let scale = direct.position(j).length().max(1.0);
        assert!(drift / scale < 1e-9,
            "{name} drifted {drift:e} m over a stepped year, {:.2e} relative", drift / scale);
    }
}

/// Keplerian bodies conserve orbital energy exactly, because they are evaluated rather
/// than integrated. That is what buys arbitrary time-scrubbing.
#[test]
fn keplerian_energy_is_exactly_conserved() {
    let mut s = built();
    let earth = s.by_name("Earth").unwrap();

    let mut energies = Vec::new();
    for days in [0.0, 30.0, 120.0, 300.0, 3000.0] {
        propagate::evaluate_at(&mut s, Instant::from_julian_day(2451545.0 + days));
        let parent = s.parent(earth).unwrap();
        let r = (s.position(earth) - s.position(parent)).length();
        let v = (s.velocity(earth) - s.velocity(parent)).length();
        energies.push(v * v / 2.0 - s.mu(earth) / r);
    }
    let first = energies[0];
    for e in &energies {
        assert!((e - first).abs() / first.abs() < 1e-9, "specific energy wandered: {energies:?}");
    }
}

#[test]
fn duplicate_ids_are_refused() {
    use em_sim::body::BodyInfo;
    use em_sim::motive::Motive;
    use em_sim::system::BodyDef;
    use glam::DVec3;

    let mut s = System::new(6.6743015e-11);
    let def = |id: &str| BodyDef {
        info: BodyInfo { id: id.to_string(), mass: 1.0, ..Default::default() },
        motive: Motive::fixed(DVec3::ZERO),
        rotation: None,
    };
    assert!(s.insert(def("Earth")).is_ok());
    assert_eq!(s.insert(def("Earth")), Err(SystemError::DuplicateName("Earth".into())));
    assert!(s.insert(def("Mars")).is_ok());
}

/// Removing a body swaps another into its slot, so cached indices must be re-resolved.
/// The generation is how a caller knows to.
#[test]
fn removal_bumps_the_generation_and_ids_still_resolve() {
    let mut s = built();
    let before = s.generation();
    let luna = BodyId::from_name("Luna");

    assert!(s.remove(BodyId::from_name("Mercury")));
    assert_ne!(s.generation(), before, "generation must advance on a structural change");
    assert!(s.index_of(luna).is_some(), "Luna must still be findable by id");
    assert!(s.by_name("Mercury").is_none());
}

/// A Newtonian body under a single star must hold a circular orbit, and velocity Verlet
/// must not leak energy doing it.
#[test]
fn a_newtonian_body_holds_a_circular_orbit() {
    use em_sim::body::BodyInfo;
    use em_sim::motive::Motive;
    use em_sim::system::BodyDef;
    use glam::DVec3;

    const G: f64 = 6.6743015e-11;
    const M: f64 = 1.988416e30;
    let r: f64 = 1.496e11;
    let v = (G * M / r).sqrt();

    let mut s = System::new(G);
    s.insert(BodyDef {
        info: BodyInfo { id: "Star".into(), mass: M, major: true, ..Default::default() },
        motive: Motive::fixed(DVec3::ZERO), rotation: None,
    }).unwrap();
    s.insert(BodyDef {
        info: BodyInfo { id: "Probe".into(), mass: 1.0, ..Default::default() },
        motive: Motive::newtonian(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0)),
        rotation: None,
    }).unwrap();

    let probe = s.by_name("Probe").unwrap();
    let energy = |s: &System| {
        let p = s.position(probe);
        s.velocity(probe).length_squared() / 2.0 - G * M / p.length()
    };

    propagate::step(&mut s, TimeDelta::ZERO);
    let e0 = energy(&s);
    let mut worst = 0.0f64;
    for _ in 0..365 {
        propagate::step(&mut s, TimeDelta::from_days(1.0));
        worst = worst.max((s.position(probe).length() - r).abs() / r);
    }
    let drift = (energy(&s) - e0).abs() / e0.abs();
    assert!(worst < 1e-3, "radius wandered {worst:e} over a year");
    assert!(drift < 1e-6, "velocity Verlet leaked {drift:e} of the orbital energy");
}
