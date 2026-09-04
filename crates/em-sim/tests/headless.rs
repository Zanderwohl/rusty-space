//! The bundled solar system propagating with no engine. Nothing here may touch Bevy, an
//! ECS, a window or a file.

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

/// Arena positions match the elements evaluated directly.
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

/// Analytic evaluation: the route to a time cannot affect the result.
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

/// Stepping a year lands where evaluating that instant directly does.
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

/// Keplerian bodies are evaluated, not integrated, so specific energy is exact.
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
        appearance: Default::default(),
    };
    assert!(s.insert(def("Earth")).is_ok());
    assert_eq!(s.insert(def("Earth")), Err(SystemError::DuplicateName("Earth".into())));
    assert!(s.insert(def("Mars")).is_ok());
}

/// Removal swaps another body into the freed slot and bumps the generation; ids still
/// resolve.
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

/// Velocity Verlet holds a circular orbit without leaking energy. Bounds are for a
/// one-day step over one year.
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
        motive: Motive::fixed(DVec3::ZERO), rotation: None, appearance: Default::default(),
    }).unwrap();
    s.insert(BodyDef {
        info: BodyInfo { id: "Probe".into(), mass: 1.0, ..Default::default() },
        motive: Motive::newtonian(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0)),
        rotation: None, appearance: Default::default(),
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

/// A body naming a primary that is not present must be refused, not quietly broken.
///
/// Such a body keeps propagating: mu collapses from `G(M+m)` to `G*m_self` — for Earth
/// about 3e-6 of the intended value — and the orbit anchors at the world origin. It
/// still draws at a plausible-looking position, and `em-sim` has no logging, so nothing
/// downstream can report it.
mod unresolved_primary {
    use em_foundations::time::Instant;
    use em_sim::body::BodyInfo;
    use em_sim::motive::Motive;
    use em_sim::motive::kepler::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive,
                                 KeplerRotation, KeplerShape, MeanAnomalyAtJ2000};
    use em_sim::system::{BodyDef, System, SystemError};

    fn orbiting(name: &str, primary: &str) -> BodyDef {
        BodyDef {
            info: BodyInfo { id: name.to_string(), mass: 5.97e24, ..Default::default() },
            motive: Motive::from_keplerian(KeplerMotive {
                primary_id: primary.to_string(),
                shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                    eccentricity: 0.0167,
                    semi_major_axis: 1.496e11,
                }),
                rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                    inclination: 0.0,
                    longitude_of_ascending_node: 0.0,
                    argument_of_periapsis: 0.0,
                }),
                epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly: 0.0 }),
                anomalistic_period: None,
                gravitational_parameter: None,
            }),
            rotation: None,
            appearance: Default::default(),
        }
    }

    #[test]
    fn the_arena_reports_it() {
        let mut system = System::new(6.6743015e-11);
        system.insert(orbiting("Earth", "NoSuchStar")).expect("insert");
        em_sim::propagate::evaluate_at(&mut system, Instant::J2000);

        let unresolved = system.unresolved_primaries();
        assert_eq!(unresolved.len(), 1, "expected one unresolved primary");
        assert_eq!(unresolved[0], ("Earth".to_string(), "NoSuchStar".to_string()));
    }

    #[test]
    fn a_resolved_hierarchy_reports_nothing() {
        let mut system = System::new(6.6743015e-11);
        system.insert(BodyDef {
            info: BodyInfo { id: "Sol".into(), mass: 1.989e30, ..Default::default() },
            motive: Motive::fixed(glam::DVec3::ZERO),
            rotation: None,
            appearance: Default::default(),
        }).expect("insert");
        system.insert(orbiting("Earth", "Sol")).expect("insert");
        em_sim::propagate::evaluate_at(&mut system, Instant::J2000);

        assert!(system.unresolved_primaries().is_empty());
    }

    /// Loading is where this can still be refused, so it is refused there.
    #[test]
    fn loading_a_file_with_one_fails() {
        let mut contents = em_sim::presets::solar_system();
        // Repoint one moon at a body that is not in the file.
        for body in contents.bodies.iter_mut() {
            if let em_sim::universe::SomeBody::KeplerEntry(e) = body {
                if e.info.id == "Luna" {
                    e.params.primary_id = "Vulcan".into();
                    break;
                }
            }
        }
        match System::from_contents(&contents) {
            Err(SystemError::UnresolvedPrimary { body, primary }) => {
                assert_eq!(body, "Luna");
                assert_eq!(primary, "Vulcan");
            }
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("a dangling primary was accepted"),
        }
    }
}
