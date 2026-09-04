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

use exotic_matters::body::universe::save::SomeBody;
use exotic_matters::body::universe::solar_system::solar_system;
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
fn modelled_position(body_id: &str, jd: f64) -> Option<DVec3> {
    let file = solar_system();
    let g = file.contents.physics.gravitational_constant;

    let mut mass_of = |id: &str| -> Option<f64> {
        file.contents.bodies.iter().find_map(|b| match b {
            SomeBody::KeplerEntry(k) if k.info.id == id => Some(k.info.mass),
            SomeBody::FixedEntry(f) if f.info.id == id => Some(f.info.mass),
            SomeBody::NewtonEntry(n) if n.info.id == id => Some(n.info.mass),
            _ => None,
        })
    };

    let entry = file.contents.bodies.iter().find_map(|b| match b {
        SomeBody::KeplerEntry(k) if k.info.id == body_id => Some(k.clone()),
        _ => None,
    })?;

    let primary_mass = mass_of(&entry.params.primary_id)?;
    // Must match the propagator: relative two-body motion uses mu = G(M + m).
    let mu = g * (primary_mass + entry.info.mass);
    entry.params.displacement(Instant::from_julian_day(jd), mu)
}

/// Relative-position tolerance per body, as of Phase 0.
///
/// These are LOOSE, and deliberately so: they record what the model can currently do,
/// not what it should do. The dominant error is `true_anomaly::fourier_expansion`, the
/// Bessel series used to get true anomaly from mean anomaly. Its error against an exact
/// Kepler solve is ~0.96 deg for Earth and ~5.4 deg for Mars, and an along-track angular
/// error of `d` radians at radius `r` displaces the body by `r*d` — which is exactly the
/// magnitude seen below.
///
/// Measured relative error at the time these were set (worst of the two epochs):
///
///   mercury 1.8e-1   venus 1.1e-2   earth 2.0e-2   mars 6.6e-2
///   jupiter 4.7e-2   uranus 4.2e-2  neptune 7.7e-3  luna 4.8e-2
///
/// PHASE 3 will replace the series with a Halley solve of Kepler's equation. When it
/// lands, tighten every budget here — the residual should then be dominated by the
/// two-body approximation (unmodelled planetary perturbations, mean rather than
/// osculating elements) rather than by the solver.
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
        "venus" => 2.0e-2,
        "neptune" => 1.5e-2,
        "earth" => 3.0e-2,
        "uranus" => 6.0e-2,
        "jupiter" => 6.0e-2,
        "mars" => 9.0e-2,
        "mercury" => 2.5e-1,
        "luna" => 7.0e-2,
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
