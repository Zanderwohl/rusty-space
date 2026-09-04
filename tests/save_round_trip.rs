//! Saving and loading is lossless.
//!
//! A dropped or misread column changes an orbit without failing anything. These tests
//! compare what comes back out field by field, then propagate a decade forward and compare
//! positions — an element read into the wrong slot shows up only there.

use em_sim::appearance::Appearance;
use em_sim::body::{BodyInfo, BodyRotation, RotationMode};
use em_sim::motive::kepler::KeplerMotive;
use em_sim::motive::MotiveSelection;
use em_sim::system::System;
use em_sim::universe::{SomeBody, UniverseFileContents};
use em_foundations::time::Instant;
use exotic_matters::body::universe::save_sqlite;
use std::path::PathBuf;

/// A temp path that cleans up after itself even when the test fails.
struct ScratchFile(PathBuf);

impl ScratchFile {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "em-round-trip-{tag}-{}-{:?}.em",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Both corpora are needed: the bundled save is `CompoundMotiveEntry` bodies with elements
/// buried in a motive timeline; only the generated preset carries `KeplerEntry` bodies and
/// an `anomalistic_period`. See the coverage check at the end.
fn corpora() -> Vec<(&'static str, UniverseFileContents)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/systems/solar_system.em");
    vec![
        ("bundled save", save_sqlite::load_from_em(&path).expect("the bundled save should load")),
        ("generated preset", em_sim::presets::solar_system()),
    ]
}

/// Save and load it back.
fn round_trip(contents: &UniverseFileContents, tag: &str) -> UniverseFileContents {
    let scratch = ScratchFile::new(tag);
    save_sqlite::save_to_em(&scratch.0, contents).expect("save should succeed");
    save_sqlite::load_from_em(&scratch.0).expect("and load back")
}

fn info(body: &SomeBody) -> &BodyInfo {
    match body {
        SomeBody::FixedEntry(e) => &e.info,
        SomeBody::NewtonEntry(e) => &e.info,
        SomeBody::KeplerEntry(e) => &e.info,
        SomeBody::CompoundEntry(e) => &e.info,
        SomeBody::CompoundMotiveEntry(e) => &e.info,
    }
}

fn appearance(body: &SomeBody) -> &Appearance {
    match body {
        SomeBody::FixedEntry(e) => &e.appearance,
        SomeBody::NewtonEntry(e) => &e.appearance,
        SomeBody::KeplerEntry(e) => &e.appearance,
        SomeBody::CompoundEntry(e) => &e.appearance,
        SomeBody::CompoundMotiveEntry(e) => &e.appearance,
    }
}

fn rotation(body: &SomeBody) -> Option<&BodyRotation> {
    match body {
        SomeBody::FixedEntry(e) => e.rotation.as_ref(),
        SomeBody::NewtonEntry(e) => e.rotation.as_ref(),
        SomeBody::KeplerEntry(e) => e.rotation.as_ref(),
        SomeBody::CompoundEntry(_) => None,
        SomeBody::CompoundMotiveEntry(e) => e.rotation.as_ref(),
    }
}

/// Every Keplerian element set in a body, in timeline order: one for a `KeplerEntry`, one
/// per Keplerian segment for a `CompoundMotiveEntry`.
fn kepler_elements(body: &SomeBody) -> Vec<&KeplerMotive> {
    match body {
        SomeBody::KeplerEntry(e) => vec![&e.params],
        SomeBody::CompoundMotiveEntry(e) => e
            .motive
            .iter_events()
            .filter_map(|(_, _, sel)| match sel {
                MotiveSelection::Keplerian(k) => Some(k),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Nothing about a body may change by being written down and read back.
#[test]
fn a_saved_body_comes_back_unchanged() {
    // What the corpora actually exercised, checked at the end.
    let (mut seen_anomalistic, mut seen_mu, mut seen_precessing, mut seen_rotation) = (0, 0, 0, 0);

    for (label, before) in corpora() {
        let after = round_trip(&before, "bodies");

        assert_eq!(
            before.bodies.len(),
            after.bodies.len(),
            "{label}: {} bodies went in, {} came out",
            before.bodies.len(),
            after.bodies.len()
        );

        for (a, b) in before.bodies.iter().zip(after.bodies.iter()) {
            let id = a.id();
            assert_eq!(id, b.id(), "{label}: bodies came back in a different order");

            let (ia, ib) = (info(a), info(b));
            assert_eq!(ia.name, ib.name, "{label} {id}: name");
            assert_eq!(ia.mass, ib.mass, "{label} {id}: mass");
            assert_eq!(ia.major, ib.major, "{label} {id}: major");
            assert_eq!(ia.designation, ib.designation, "{label} {id}: designation");

            // Set, not sequence: `tag_members` has no order column and consumers treat
            // tags as a set. The round trip reorders them.
            let (mut ta, mut tb) = (ia.tags.clone(), ib.tags.clone());
            ta.sort();
            tb.sort();
            assert_eq!(ta, tb, "{label} {id}: tags");

            assert_eq!(
                appearance(a).radius(),
                appearance(b).radius(),
                "{label} {id}: appearance radius"
            );

            // Rotation went unwritten before the table existed; losing it drops tidal
            // locking and spin silently.
            match (rotation(a), rotation(b)) {
                (None, None) => {}
                (Some(ra), Some(rb)) => {
                    seen_rotation += 1;
                    match (&ra.mode, &rb.mode) {
                        (
                            RotationMode::Spinning { orientation_at_epoch: oa, angular_velocity: va, .. },
                            RotationMode::Spinning { orientation_at_epoch: ob, angular_velocity: vb, .. },
                        ) => {
                            assert!(
                                (*oa).abs_diff_eq(*ob, 1.0e-12),
                                "{label} {id}: spin orientation {oa:?} became {ob:?}"
                            );
                            assert_eq!(va, vb, "{label} {id}: angular velocity");
                        }
                        (
                            RotationMode::TidallyLocked { primary_id: pa, .. },
                            RotationMode::TidallyLocked { primary_id: pb, .. },
                        ) => assert_eq!(pa, pb, "{label} {id}: tidal lock primary"),
                        _ => panic!("{label} {id}: rotation mode changed across the round trip"),
                    }
                }
                (x, y) => panic!(
                    "{label} {id}: rotation went from {:?} to {:?}",
                    x.is_some(),
                    y.is_some()
                ),
            }

            let (ka, kb) = (kepler_elements(a), kepler_elements(b));
            assert_eq!(ka.len(), kb.len(), "{label} {id}: number of Keplerian segments");
            for (n, (ka, kb)) in ka.iter().zip(kb.iter()).enumerate() {
                let at = format!("{label} {id}[{n}]");
                assert_eq!(ka.primary_id, kb.primary_id, "{at}: primary");
                assert_eq!(ka.eccentricity(), kb.eccentricity(), "{at}: eccentricity");
                assert_eq!(ka.semi_major_axis(), kb.semi_major_axis(), "{at}: semi-major axis");
                assert_eq!(ka.inclination(), kb.inclination(), "{at}: inclination");
                assert_eq!(ka.is_precessing(), kb.is_precessing(), "{at}: precession");
                if ka.is_precessing() {
                    seen_precessing += 1;
                }

                // Both are `Option`, so a lost column reads as "derive from Kepler's
                // third law" and silently inflates the semi-major axis.
                match (ka.anomalistic_period, kb.anomalistic_period) {
                    (None, None) => {}
                    (Some(pa), Some(pb)) => {
                        seen_anomalistic += 1;
                        assert!(
                            (pa.to_seconds() - pb.to_seconds()).abs() < 1.0e-6,
                            "{at}: anomalistic period {} s became {} s",
                            pa.to_seconds(),
                            pb.to_seconds()
                        );
                    }
                    _ => panic!("{at}: anomalistic period appeared or vanished"),
                }
                if ka.gravitational_parameter.is_some() {
                    seen_mu += 1;
                }
                assert_eq!(
                    ka.gravitational_parameter, kb.gravitational_parameter,
                    "{at}: explicit mu"
                );
            }
        }
    }

    // The assertions above only count if the corpora reach them.
    assert!(seen_anomalistic > 100, "only {seen_anomalistic} anomalistic periods compared");
    assert!(seen_mu > 100, "only {seen_mu} explicit mus compared");
    assert!(seen_precessing > 100, "only {seen_precessing} precessing orbits compared");
    assert!(seen_rotation > 50, "only {seen_rotation} rotations compared");
}

/// The clock and physics constants round-trip too.
#[test]
fn the_files_settings_come_back_unchanged() {
    for (label, before) in corpora() {
        let after = round_trip(&before, "settings");

        assert_eq!(
            before.physics.gravitational_constant, after.physics.gravitational_constant,
            "{label}: gravitational constant"
        );
        assert_eq!(before.time.step, after.time.step, "{label}: step");
        assert_eq!(before.time.gui_speed, after.time.gui_speed, "{label}: gui speed");
        assert_eq!(
            before.time.max_frame_time, after.time.max_frame_time,
            "{label}: max frame time"
        );
        assert!(
            (before.time.time_julian_days - after.time.time_julian_days).abs() < 1.0e-9,
            "{label}: saved epoch {} became {}",
            before.time.time_julian_days,
            after.time.time_julian_days
        );
    }
}

/// A round-tripped system puts its bodies in the same places. Catches a column read into
/// the wrong slot, which a field comparison would accept.
#[test]
fn a_round_tripped_system_propagates_to_the_same_places() {
    for (label, before) in corpora() {
        let after = round_trip(&before, "propagate");

        let mut a = System::from_contents(&before).expect("original builds");
        let mut b = System::from_contents(&after).expect("round-tripped builds");
        assert_eq!(a.len(), b.len(), "{label}: body count");

        // A decade out, so a slightly wrong element has room to show.
        let target = Instant::from_julian_day(2451545.0 + 3652.5);
        em_sim::propagate::evaluate_at(&mut a, target);
        em_sim::propagate::evaluate_at(&mut b, target);

        let mut worst = 0.0f64;
        let mut worst_body = String::new();
        for i in a.indices() {
            let id = a.id(i);
            let j = b.index_of(id).unwrap_or_else(|| panic!("{label}: {} is missing", a.name(i)));
            let drift = (a.position(i) - b.position(j)).length();
            assert!(
                drift.is_finite(),
                "{label}: {} propagated to a non-finite position",
                a.name(i)
            );
            if drift > worst {
                worst = drift;
                worst_body = a.name(i).to_string();
            }
        }

        // One metre over ten years: far above the noise of storing days as REAL, far
        // below anything the round trip could actually lose.
        assert!(
            worst < 1.0,
            "{label}: {worst_body} drifted {worst} m over a decade after a round trip"
        );
    }
}
