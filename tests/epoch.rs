//! Epoch and time-scale consistency. `Instant` counts seconds since J2000; mixing it with
//! a Julian Day number puts the epoch 28.37 days late.

use exotic_matters::foundations::time::{Instant, JD_SECONDS_PER_JULIAN_DAY};
use exotic_matters::body::motive::kepler_motive::{
    EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape,
    MeanAnomalyAtEpoch, MeanAnomalyAtJ2000,
};

const J2000_JULIAN_DAY: f64 = 2451545.0;

#[test]
fn j2000_is_zero_seconds() {
    assert_eq!(
        Instant::J2000.to_j2000_seconds(), 0.0,
        "Instant counts seconds since J2000, so the J2000 epoch itself is 0 seconds"
    );
}

#[test]
fn j2000_constant_matches_its_julian_day() {
    assert!(
        (Instant::J2000.to_julian_day() - J2000_JULIAN_DAY).abs() < 1e-9,
        "Instant::J2000 is JD {}, expected {J2000_JULIAN_DAY}", Instant::J2000.to_julian_day()
    );
    assert!(
        (Instant::from_julian_day(J2000_JULIAN_DAY).to_j2000_seconds()).abs() < 1e-9,
        "JD {J2000_JULIAN_DAY} must map to 0 seconds since J2000"
    );
}

#[test]
fn julian_day_round_trips() {
    for jd in [2451545.0, 2451544.5, 2433282.5, 2440587.5, 2460800.5, 2400000.5] {
        let back = Instant::from_julian_day(jd).to_julian_day();
        assert!((back - jd).abs() < 1e-6, "JD round trip: {jd} -> {back}");
    }
}

#[test]
fn one_julian_day_is_86400_seconds() {
    assert_eq!(JD_SECONDS_PER_JULIAN_DAY, 86400.0);
    let a = Instant::from_julian_day(J2000_JULIAN_DAY);
    let b = Instant::from_julian_day(J2000_JULIAN_DAY + 1.0);
    assert!(((b - a).to_seconds() - 86400.0).abs() < 1e-9);
}

fn motive(epoch: KeplerEpoch) -> KeplerMotive {
    KeplerMotive {
        primary_id: "sol".into(),
        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
            eccentricity: 0.0167,
            semi_major_axis: 1.496e11,
        }),
        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            argument_of_periapsis: 102.9,
        }),
        epoch,
        anomalistic_period: None,
        gravitational_parameter: None,
    }
}

/// `KeplerEpoch::J2000 { M }` is equivalent to `MeanAnomaly { epoch: J2000, M }`.
#[test]
fn j2000_epoch_variant_matches_explicit_j2000_epoch() {
    const MU: f64 = 1.32712440018e20;
    let mean_anomaly = 357.529;

    let implicit = motive(KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly }));
    let explicit = motive(KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
        epoch: Instant::from_seconds_since_j2000(0.0),
        mean_anomaly,
    }));

    assert_eq!(
        implicit.epoch.epoch().to_j2000_seconds(),
        explicit.epoch.epoch().to_j2000_seconds(),
        "the J2000 epoch variant must resolve to the same instant as an explicit J2000 epoch"
    );

    for days in [0.0, 1.0, 100.0, 365.25, 3652.5] {
        let t = Instant::from_seconds_since_j2000(days * JD_SECONDS_PER_JULIAN_DAY);
        let a = implicit.displacement(t, MU).unwrap();
        let b = explicit.displacement(t, MU).unwrap();
        assert!(
            (a - b).length() < 1.0,
            "positions diverge at +{days} d: {a:?} vs {b:?} (|d| = {} m)", (a - b).length()
        );
    }
}

/// `epoch()` and `time_at_periapsis_passage()` take separate paths for the J2000 variant
/// and must agree, or bodies drift off their own trajectory lines.
#[test]
fn periapsis_passage_agrees_between_epoch_variants() {
    const MU: f64 = 1.32712440018e20;
    let mean_anomaly = 357.529;

    let implicit = motive(KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly }));
    let explicit = motive(KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
        epoch: Instant::from_seconds_since_j2000(0.0),
        mean_anomaly,
    }));

    let a = implicit.time_at_periapsis_passage(MU).to_j2000_seconds();
    let b = explicit.time_at_periapsis_passage(MU).to_j2000_seconds();
    assert!((a - b).abs() < 1.0, "periapsis passage: {a} vs {b} (differ by {} s)", (a - b).abs());
}

/// At periapsis passage the body is at periapsis distance, tying
/// `time_at_periapsis_passage`, `mean_anomaly` and `displacement` together.
#[test]
fn body_is_at_periapsis_at_periapsis_passage() {
    const MU: f64 = 1.32712440018e20;
    let m = motive(KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly: 357.529 }));

    let t_p = m.time_at_periapsis_passage(MU);
    let r = m.displacement(t_p, MU).unwrap().length();
    let expected = m.periapsis();

    // Tolerance set by the series-based true anomaly.
    let rel = (r - expected).abs() / expected;
    assert!(rel < 1e-3, "at periapsis passage r = {r:e}, expected periapsis {expected:e} (rel {rel:e})");
}

/// A `TrueAnomaly` epoch propagates rather than panicking. Both decoders build it from a
/// save whose `epoch_type` is `'TrueAnomaly'`.
mod true_anomaly_epoch {
    use em_foundations::time::Instant;
    use em_sim::motive::kepler::*;

    const MU_SOL: f64 = 1.32712440018e20;

    fn orbit(eccentricity: f64, true_anomaly_deg: f64) -> KeplerMotive {
        KeplerMotive {
            primary_id: "Sol".into(),
            shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                eccentricity,
                semi_major_axis: 1.495978707e11,
            }),
            rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                argument_of_periapsis: 0.0,
            }),
            epoch: KeplerEpoch::TrueAnomaly(TrueAnomalyAtEpoch {
                epoch: Instant::J2000,
                true_anomaly: true_anomaly_deg,
            }),
            anomalistic_period: None,
            gravitational_parameter: None,
        }
    }

    #[test]
    fn it_propagates_at_all() {
        for &e in &[0.0, 0.3, 0.9] {
            for &nu in &[0.0, 45.0, 180.0, 300.0] {
                let k = orbit(e, nu);
                let p = k.displacement(Instant::J2000, MU_SOL);
                assert!(p.is_some(), "e={e} nu={nu}: no displacement");
                assert!(p.unwrap().is_finite(), "e={e} nu={nu}: non-finite");
            }
        }
    }

    /// At the epoch instant the body's true anomaly is the stored one, not merely some
    /// number.
    #[test]
    fn the_body_is_where_the_stored_true_anomaly_says() {
        for &e in &[0.0, 0.15, 0.6, 0.9] {
            for &nu in &[0.0, 30.0, 90.0, 200.0, 359.0] {
                let k = orbit(e, nu);
                let got = k.true_anomaly(Instant::J2000, MU_SOL).to_degrees().rem_euclid(360.0);
                let want = nu.rem_euclid(360.0);
                let diff = (got - want).abs().min(360.0 - (got - want).abs());
                assert!(diff < 1e-6, "e={e}: stored nu={want}, got {got}");
            }
        }
    }

    /// Periapsis is true anomaly zero, whichever epoch form states it.
    #[test]
    fn periapsis_agrees_with_the_mean_anomaly_form() {
        let e = 0.4;
        let nu = 120.0_f64;
        let by_true = orbit(e, nu);

        // The same orbit, stated as the implied mean anomaly.
        let m = em_foundations::kepler::anomaly::mean_from_eccentric(
            em_foundations::kepler::anomaly::eccentric_from_true(nu.to_radians(), e),
            e,
        );
        let mut by_mean = orbit(e, 0.0);
        by_mean.epoch = KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
            epoch: Instant::J2000,
            mean_anomaly: m.to_degrees(),
        });

        let a = by_true.time_at_periapsis_passage(MU_SOL).to_j2000_seconds();
        let b = by_mean.time_at_periapsis_passage(MU_SOL).to_j2000_seconds();
        assert!((a - b).abs() < 1e-6, "periapsis {a} vs {b}");
    }

    /// An open orbit stated by true anomaly works too.
    #[test]
    fn hyperbolic_orbits_propagate() {
        let k = orbit(1.5, 20.0);
        let p = k.displacement(Instant::J2000, MU_SOL);
        assert!(p.is_some_and(|p| p.is_finite()), "hyperbolic true-anomaly epoch");
    }
}
