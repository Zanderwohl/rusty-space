//! Epoch and time-scale consistency.
//!
//! `Instant` counts SECONDS since J2000. `Instant::J2000` previously held the Julian Day
//! *number* 2451545.0, putting the J2000 epoch 28.37 days late and shifting every body
//! that declares a J2000 epoch (11 of the bundled planets).

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
    }
}

/// `KeplerEpoch::J2000 { M }` must be exactly equivalent to
/// `KeplerEpoch::MeanAnomaly { epoch: J2000, M }`. They diverged by 2451545 seconds.
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

/// `epoch()` and `time_at_periapsis_passage()` handle the J2000 variant on separate code
/// paths. They must agree, or bodies drift off their own rendered trajectory lines.
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

/// At periapsis passage the body must actually be at periapsis distance.
/// This ties `time_at_periapsis_passage`, `mean_anomaly` and `displacement` together.
#[test]
fn body_is_at_periapsis_at_periapsis_passage() {
    const MU: f64 = 1.32712440018e20;
    let m = motive(KeplerEpoch::J2000(MeanAnomalyAtJ2000 { mean_anomaly: 357.529 }));

    let t_p = m.time_at_periapsis_passage(MU);
    let r = m.displacement(t_p, MU).unwrap().length();
    let expected = m.periapsis();

    // The series-based true anomaly limits accuracy here; tighten in Phase 3.
    let rel = (r - expected).abs() / expected;
    assert!(rel < 1e-3, "at periapsis passage r = {r:e}, expected periapsis {expected:e} (rel {rel:e})");
}
