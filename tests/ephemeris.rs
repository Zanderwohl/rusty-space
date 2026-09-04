//! Golden ephemeris: the bundled solar system checked against JPL Horizons.
//!
//! Reference vectors are GEOMETRIC cartesian states in the ecliptic frame of J2000,
//! retrieved from the Horizons API. The exact query is recorded in
//! `docs/horizons-golden-vectors.md` so these can be regenerated.
//!
//! Unlike the rest of the suite, this checks the element DATA in
//! `body/universe/solar_system.rs`, not just internal consistency. It is the only test
//! that can catch a wrong semi-major axis or mean anomaly.
//!
//! Note the model is a fixed two-body Kepler orbit per body, so it cannot match DE441
//! exactly: planetary perturbations are not modelled and the elements are mean, not
//! osculating. Tolerances below are set from the model's actual capability.

use em_sim::presets::solar_system;
use em_sim::universe::SomeBody;
use exotic_matters::foundations::time::Instant;
use bevy::math::DVec3;

const KM: f64 = 1000.0;
const AU: f64 = 1.495978707e11;

struct Reference {
    body: &'static str,
    julian_day: f64,
    /// Position relative to the body's primary, in km (ecliptic J2000).
    position_km: [f64; 3],
}

/// JPL Horizons, DE441, GEOMETRIC states, "Ecliptic of J2000.0".
/// Planets are heliocentric (CENTER='500@10'); Luna is geocentric (CENTER='500@399').
const REFERENCES: &[Reference] = &[
    Reference { body: "mercury", julian_day: 2451545.0, position_km: [-1.946172635585372E+07, -6.691327526352400E+07, -3.679854343749542E+06] },
    Reference { body: "mercury", julian_day: 2460676.5, position_km: [-5.793970539114399E+07, -2.419359515794889E+07,  3.337188220186163E+06] },
    Reference { body: "venus",   julian_day: 2451545.0, position_km: [-1.074564940521906E+08, -4.885014975872536E+06,  6.135634299718402E+06] },
    Reference { body: "venus",   julian_day: 2460676.5, position_km: [ 6.783048185030288E+07,  8.410632833599219E+07, -2.758788226574615E+06] },
    Reference { body: "earth",   julian_day: 2451545.0, position_km: [-2.649903367743050E+07,  1.446972967925493E+08, -6.111494259536266E+02] },
    Reference { body: "earth",   julian_day: 2460676.5, position_km: [-2.673066229892559E+07,  1.446585671920011E+08, -7.643589382000268E+03] },
    Reference { body: "mars",    julian_day: 2451545.0, position_km: [ 2.080481406418420E+08, -2.007052628025221E+06, -5.156288959268022E+06] },
    Reference { body: "mars",    julian_day: 2460676.5, position_km: [-7.804309481287776E+07,  2.281718450076631E+08,  6.695342168331429E+06] },
    Reference { body: "Jupiter", julian_day: 2451545.0, position_km: [ 5.985676246570644E+08,  4.396046799481729E+08, -1.522686167298746E+07] },
    Reference { body: "Jupiter", julian_day: 2460676.5, position_km: [ 1.579803698060460E+08,  7.437186577256843E+08, -6.623904060887098E+06] },
    Reference { body: "Uranus",  julian_day: 2451545.0, position_km: [ 2.158974819528798E+09, -2.054625536468218E+09, -3.562550131686962E+07] },
    Reference { body: "Uranus",  julian_day: 2460676.5, position_km: [ 1.661079227785963E+09,  2.407700523277836E+09, -1.259589127124381E+07] },
    Reference { body: "Neptune", julian_day: 2451545.0, position_km: [ 2.515046523944309E+09, -3.738714567646374E+09,  1.903221685677218E+07] },
    Reference { body: "Neptune", julian_day: 2460676.5, position_km: [ 4.469973780179410E+09, -9.487334113276581E+07, -1.010533335986975E+08] },
    Reference { body: "luna",    julian_day: 2451545.0, position_km: [-2.916083841877129E+05, -2.749797416731504E+05,  3.627119662699287E+04] },
    Reference { body: "luna",    julian_day: 2460676.5, position_km: [ 1.520523605713538E+05, -3.488036665045074E+05, -3.066409317095052E+04] },
];

/// Model position of `body` relative to its primary, in metres.
///
/// Uses the bundled preset directly from `em-sim` — no engine, no file, no ECS.
fn modelled_position(body_id: &str, jd: f64) -> Option<DVec3> {
    let contents = solar_system();
    let g = contents.physics.gravitational_constant;

    let mass_of = |id: &str| -> Option<f64> {
        contents.bodies.iter().find_map(|b| match b {
            SomeBody::KeplerEntry(k) if k.info.id == id => Some(k.info.mass),
            SomeBody::FixedEntry(f) if f.info.id == id => Some(f.info.mass),
            SomeBody::NewtonEntry(n) if n.info.id == id => Some(n.info.mass),
            _ => None,
        })
    };

    let entry = contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == body_id => Some(k),
        _ => None,
    })?;

    let primary_mass = mass_of(&entry.params.primary_id)?;
    // Must match the propagator: relative two-body motion uses mu = G(M + m).
    let mu = g * (primary_mass + entry.info.mass);
    entry.params.displacement(Instant::from_julian_day(jd), mu)
}

/// Relative-position tolerance per body, as of Phase 0.
///
/// Since Phase 3 replaced the Bessel series with a Halley solve of Kepler's equation,
/// the residual is dominated by the two-body approximation itself — unmodelled planetary
/// perturbations, and mean rather than osculating elements — not by the anomaly solver.
///
/// Mercury and Luna grow most between the two epochs, which indicates mean-motion error
/// rather than a static offset: Mercury goes 1.7e-5 -> 1.2e-2 over 25 years (104 orbits),
/// where a two-body model omits both planetary perturbation and relativistic precession.
///
/// Measured relative error at the time these were set (worst of the two epochs):
///
///   mercury 1.1e-5   venus 7.3e-5   earth 6.7e-5   mars 1.0e-4
///   jupiter 2.6e-3   uranus 4.3e-4  neptune 2.6e-4  luna 2.7e-2
///
/// Luna's elements were refitted against 3653 Horizons samples over 2000-2050, taking it
/// from 6.3e-1 to 4.8e-2. It is now near the floor for a precessing-ellipse model: the
/// fit's own residual is 7 989 km RMS (~1.2 deg), and evection alone, which this model
/// cannot represent, is 1.27 deg. Better accuracy needs a real lunar theory, not better
/// elements. See docs/horizons-golden-vectors.md.
///
/// Note ids are matched case-insensitively: `solar_system.rs` spells most ids lowercase
/// but capitalises "Jupiter", "Uranus", "Neptune" and "Sedna".
fn tolerance(body: &str) -> f64 {
    match body.to_ascii_lowercase().as_str() {
        "venus" => 1.5e-4,
        "neptune" => 5.0e-4,
        "earth" => 1.5e-4,
        "uranus" => 8.0e-4,
        "jupiter" => 4.0e-3,
        "mars" => 2.0e-4,
        "mercury" => 5.0e-5,
        "luna" => 4.0e-2,
        other => panic!("no tolerance recorded for {other}"),
    }
}

/// Every bundled body must stay within its recorded budget of the JPL position.
///
/// A failure here means either the element data in `solar_system.rs` is wrong, the
/// perifocal-to-inertial rotation is wrong, or the anomaly solve regressed.
#[test]
fn bundled_bodies_match_jpl_within_budget() {
    let mut failures = Vec::new();
    for r in REFERENCES {
        let Some(model) = modelled_position(r.body, r.julian_day) else {
            failures.push(format!("{} at JD {}: no Keplerian entry found", r.body, r.julian_day));
            continue;
        };
        let truth = DVec3::new(r.position_km[0], r.position_km[1], r.position_km[2]) * KM;
        let rel = (model - truth).length() / truth.length();
        let budget = tolerance(r.body);
        if rel > budget {
            failures.push(format!(
                "{} at JD {}: relative error {rel:.3e} exceeds budget {budget:.3e} \
                 (off by {:.4e} m = {:.3e} AU)",
                r.body, r.julian_day, (model - truth).length(), (model - truth).length() / AU
            ));
        }
    }
    assert!(failures.is_empty(), "ephemeris budget exceeded:\n  {}", failures.join("\n  "));
}

/// At its own epoch a body should be at its best; error there isolates the element data
/// and the anomaly solve from any secular drift.
#[test]
fn error_does_not_grow_wildly_between_epochs() {
    for body in ["earth", "venus", "Neptune"] {
        let at: Vec<f64> = REFERENCES.iter().filter(|r| r.body == body).map(|r| {
            let model = modelled_position(r.body, r.julian_day).unwrap();
            let truth = DVec3::new(r.position_km[0], r.position_km[1], r.position_km[2]) * KM;
            (model - truth).length() / truth.length()
        }).collect();
        assert_eq!(at.len(), 2, "{body} should have two reference epochs");
        let growth = at[1] / at[0].max(1e-12);
        assert!(
            growth < 10.0,
            "{body}: error grew {growth:.1}x over 25 years ({:.2e} -> {:.2e}); \
             suggests a rate error (mean motion, epoch, or precession) rather than a \
             static offset", at[0], at[1]
        );
    }
}

/// Diagnostic: prints the full error table. Run with
/// `cargo test --test ephemeris -- --nocapture report_ephemeris_error`
#[test]
fn report_ephemeris_error() {
    println!("\n{:<9} {:>12} {:>14} {:>12} {:>10}", "body", "JD", "|error| (m)", "as AU", "rel");
    for r in REFERENCES {
        let Some(model) = modelled_position(r.body, r.julian_day) else {
            println!("{:<9} {:>12} {:>14}", r.body, r.julian_day, "NO MODEL");
            continue;
        };
        let truth = DVec3::new(r.position_km[0], r.position_km[1], r.position_km[2]) * KM;
        let err = (model - truth).length();
        println!(
            "{:<9} {:>12.1} {:>14.4e} {:>12.3e} {:>10.2e}",
            r.body, r.julian_day, err, err / AU, err / truth.length()
        );
    }
    println!();
}

/// Semi-major axis must be the real one, so radii read off it are right.
///
/// Before `anomalistic_period` existed, a precessing orbit had to smuggle its mean
/// motion into `a` — Kepler's third law gives the sidereal rate, but mean anomaly
/// advances at the anomalistic rate once periapsis moves. Luna's fitted `a` came out
/// 386 931 km against a true mean of 384 370 km, so every radius derived from it, the
/// apsides included, was 0.66% high.
#[test]
fn lunar_apsides_are_physical() {
    let contents = solar_system();
    let luna = contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == "luna" => Some(k),
        _ => None,
    }).expect("luna");

    let a = luna.params.semi_major_axis();
    let peri = luna.params.periapsis();
    let apo = luna.params.apoapsis().expect("closed orbit");

    // Mean perigee 363 300 km, mean apogee 405 500 km, mean a 384 400 km.
    let km = |m: f64| m / 1000.0;
    assert!((km(a) - 384_400.0).abs() < 1_000.0, "semi-major axis {} km", km(a));
    assert!((km(peri) - 363_300.0).abs() < 2_000.0, "perigee {} km", km(peri));
    assert!((km(apo) - 405_500.0).abs() < 2_000.0, "apogee {} km", km(apo));
    assert!(peri < a && a < apo);
}

/// The anomalistic period must actually drive the mean anomaly. If it were ignored,
/// Kepler's third law would give the sidereal month instead — a 1% rate error.
#[test]
fn luna_advances_at_the_anomalistic_rate() {
    let contents = solar_system();
    let g = contents.physics.gravitational_constant;
    let mass_of = |id: &str| contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == id => Some(k.info.mass),
        _ => None,
    }).unwrap();
    let luna = contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == "luna" => Some(k),
        _ => None,
    }).unwrap();

    let mu = g * (mass_of("earth") + luna.info.mass);
    let n = luna.params.mean_angular_motion(mu); // rad/s
    let period_days = std::f64::consts::TAU / n / 86400.0;

    assert!((period_days - 27.554533).abs() < 1e-3,
        "mean anomaly should advance at the anomalistic month (27.5545 d), got {period_days:.6} d");
    assert!((period_days - 27.321661).abs() > 0.1,
        "and must NOT be the sidereal month (27.3217 d)");
}

/// Velocity, checked against the same Horizons states as the positions above.
///
/// Position agreeing does not imply velocity does — until Phase 3c the Keplerian path
/// produced no velocity at all, so nothing here was exercised. These are the VX/VY/VZ
/// columns of the same query, in km/s.
#[test]
fn velocities_match_jpl() {
    struct V { body: &'static str, jd: f64, primary: &'static str, kms: [f64; 3], tol: f64 }
    let refs = [
        V { body: "earth", jd: 2451545.0, primary: "sol",
            kms: [-2.979426007043741E+01, -5.469294939770602E+00, 1.817836785027449E-04], tol: 1e-3 },
        V { body: "mars", jd: 2451545.0, primary: "sol",
            kms: [1.162672403766088E+00, 2.629606454546266E+01, 5.222970229952857E-01], tol: 5e-3 },
        V { body: "venus", jd: 2451545.0, primary: "sol",
            kms: [1.381906029263447E+00, -3.514029517644670E+01, -5.600423382820807E-01], tol: 2e-2 },
        V { body: "luna", jd: 2451545.0, primary: "earth",
            kms: [6.435313889889519E-01, -7.309839826871004E-01, -1.150646473918648E-02], tol: 5e-2 },
    ];

    let contents = solar_system();
    let g = contents.physics.gravitational_constant;
    let mass_of = |id: &str| contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == id => Some(k.info.mass),
        SomeBody::FixedEntry(f) if f.info.id == id => Some(f.info.mass),
        _ => None,
    }).unwrap();

    for r in refs {
        let entry = contents.bodies.iter().find_map(|b| match b {
            SomeBody::KeplerEntry(k) if k.info.id == r.body => Some(k),
            _ => None,
        }).expect(r.body);

        let mu = g * (mass_of(r.primary) + entry.info.mass);
        let (_, v) = entry.params
            .state_vectors(Instant::from_julian_day(r.jd), mu)
            .expect("Keplerian motive must produce a state vector");

        let truth = DVec3::new(r.kms[0], r.kms[1], r.kms[2]) * KM;
        let rel = (v - truth).length() / truth.length();
        assert!(rel < r.tol,
            "{} velocity off by {rel:.3e} (budget {:.0e}): got {:?} m/s, expected {:?} m/s",
            r.body, r.tol, v, truth);
    }
}

/// Speed must obey vis-viva against the body's own elements, at every point of the orbit.
/// This is internal consistency rather than accuracy, and holds regardless of element data.
#[test]
fn keplerian_speeds_obey_vis_viva() {
    let contents = solar_system();
    let g = contents.physics.gravitational_constant;
    let sol_mass = contents.bodies.iter().find_map(|b| match b {
        SomeBody::FixedEntry(f) if f.info.id == "sol" => Some(f.info.mass),
        SomeBody::KeplerEntry(k) if k.info.id == "sol" => Some(k.info.mass),
        _ => None,
    }).unwrap();

    for body in ["mercury", "earth", "mars", "Neptune"] {
        let entry = contents.bodies.iter().find_map(|b| match b {
            SomeBody::KeplerEntry(k) if k.info.id == body => Some(k),
            _ => None,
        }).expect(body);
        if entry.params.primary_id != "sol" { continue; }

        let mu = g * (sol_mass + entry.info.mass);
        let a = entry.params.semi_major_axis();

        for days in [0.0, 40.0, 500.0, 3000.0, 9000.0] {
            let t = Instant::from_julian_day(2451545.0 + days);
            let (r, v) = entry.params.state_vectors(t, mu).unwrap();
            let expected = (mu * (2.0 / r.length() - 1.0 / a)).sqrt();
            let rel = (v.length() - expected).abs() / expected;
            assert!(rel < 1e-9, "{body} at +{days} d: speed {} vs vis-viva {expected} (rel {rel:e})",
                v.length());
        }
    }
}

/// Every bundled body must carry physically sane data.
///
/// This exists because the hand-maintained preset accumulated exactly these mistakes:
/// Mars's mass was 6.4171 kg (an `e23` lost), Vesta had Luna's radius, and Eris, Sedna
/// and Dysnomia stored diameters in a field named `radius`.
#[test]
fn every_body_has_sane_physical_data() {
    let contents = solar_system();
    let ids: Vec<&str> = contents.bodies.iter().map(|b| match b {
        SomeBody::KeplerEntry(k) => k.info.id.as_str(),
        SomeBody::FixedEntry(f) => f.info.id.as_str(),
        SomeBody::NewtonEntry(n) => n.info.id.as_str(),
        SomeBody::CompoundEntry(c) => c.info.id.as_str(),
        SomeBody::CompoundMotiveEntry(c) => c.info.id.as_str(),
    }).collect();

    assert!(ids.len() > 100, "expected the full system, got {} bodies", ids.len());

    let mut seen = std::collections::HashSet::new();
    for id in &ids {
        assert!(seen.insert(*id), "duplicate body id {id:?}");
        assert!(!id.is_empty() && !id.contains(' '), "unusable body id {id:?}");
    }

    for b in &contents.bodies {
        let SomeBody::KeplerEntry(k) = b else { continue };
        let id = &k.info.id;

        // Every primary must exist, or the body silently orbits the origin.
        assert!(ids.contains(&k.params.primary_id.as_str()),
            "{id} orbits {:?}, which is not in the system", k.params.primary_id);

        assert!(k.info.mass >= 0.0 && k.info.mass.is_finite(), "{id} mass {}", k.info.mass);
        // A gravitating body needs a real mass. Mars had 6.4171 kg.
        if k.info.major {
            assert!(k.info.mass > 1e15, "{id} is Major but has mass {} kg", k.info.mass);
        }

        let a = k.params.semi_major_axis();
        let e = k.params.eccentricity();
        assert!(a.is_finite() && a != 0.0, "{id} semi-major axis {a}");
        assert!((0.0..20.0).contains(&e) && e.is_finite(), "{id} eccentricity {e}");
        if e < 1.0 {
            assert!(a > 0.0, "{id} is closed (e={e}) but has a = {a}");
            let peri = k.params.periapsis();
            let apo = k.params.apoapsis().expect("closed orbit has an apoapsis");
            assert!(peri > 0.0 && peri <= apo, "{id} apsides {peri} / {apo}");
        }

        let radius = k.appearance.radius();
        assert!(radius > 0.0 && radius.is_finite(), "{id} radius {radius}");
        if e < 1.0 {
            // A body cannot be larger than its own orbit. Vesta had Luna's radius; that
            // would not trip this, but a diameter-for-radius on a close moon would.
            assert!(radius < k.params.periapsis(),
                "{id} radius {radius:e} m exceeds its periapsis {:e} m", k.params.periapsis());
        }

        assert!(k.params.inclination().is_finite(), "{id} inclination");
        assert!(k.params.anomalistic_period.map_or(true, |p| p.to_seconds() > 0.0),
            "{id} has a non-positive anomalistic period");
    }
}

/// Moons must actually be bound to their planet: a satellite orbit has to sit well
/// inside the primary's Hill sphere, or it is not a satellite.
#[test]
fn moons_orbit_inside_their_primarys_hill_sphere() {
    let contents = solar_system();
    let g = contents.physics.gravitational_constant;
    let get = |id: &str| contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == id => Some((k.info.mass, Some(k))),
        SomeBody::FixedEntry(f) if f.info.id == id => Some((f.info.mass, None)),
        _ => None,
    });

    for b in &contents.bodies {
        let SomeBody::KeplerEntry(k) = b else { continue };
        if k.params.primary_id == "sol" { continue; }
        let (primary_mass, primary_entry) = get(&k.params.primary_id).expect("primary");
        let Some(pe) = primary_entry else { continue };
        let (sun_mass, _) = get("sol").unwrap();

        // Hill radius of the primary about the Sun.
        let hill = pe.params.semi_major_axis()
            * (1.0 - pe.params.eccentricity())
            * (primary_mass / (3.0 * sun_mass)).cbrt();
        let apo = k.params.apoapsis().unwrap_or(f64::INFINITY);
        assert!(apo < hill,
            "{} reaches {:.3e} m from {}, outside its {:.3e} m Hill sphere",
            k.info.id, apo, k.params.primary_id, hill);
        let _ = g;
    }
}
