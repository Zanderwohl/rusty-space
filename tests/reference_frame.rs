//! `ReferenceFrame` produces proper rotations — orthonormal, determinant +1, never a
//! mirror — and no NaN at the pitch singularity.

use exotic_matters::foundations::reference_frame::ReferenceFrame;
use bevy::math::{DMat4, DVec3};

fn angles() -> Vec<f64> {
    (0..24).map(|i| -std::f64::consts::PI + i as f64 * std::f64::consts::TAU / 24.0).collect()
}

fn assert_proper_basis(f: &ReferenceFrame, what: &str) {
    let (x, y, z) = (f.local_forward(), f.local_right(), f.local_up());

    for (v, name) in [(x, "forward"), (y, "right"), (z, "up")] {
        assert!(v.is_finite(), "{what}: {name} is not finite: {v:?}");
        assert!((v.length() - 1.0).abs() < 1e-12, "{what}: {name} not unit: |{v:?}| = {}", v.length());
    }
    assert!(x.dot(y).abs() < 1e-12, "{what}: forward.right = {}", x.dot(y));
    assert!(x.dot(z).abs() < 1e-12, "{what}: forward.up = {}", x.dot(z));
    assert!(y.dot(z).abs() < 1e-12, "{what}: right.up = {}", y.dot(z));

    // det = forward . (right x up); +1 right-handed, -1 a mirror.
    let det = x.dot(y.cross(z));
    assert!((det - 1.0).abs() < 1e-12, "{what}: basis determinant {det} (expected +1; -1 means mirrored)");
}

#[test]
fn yaw_pitch_basis_is_a_proper_rotation() {
    for yaw in angles() {
        for pitch in angles().into_iter().filter(|p| p.abs() < std::f64::consts::FRAC_PI_2 - 0.05) {
            let f = ReferenceFrame::from_position_yaw_pitch(DVec3::ZERO, yaw, pitch);
            assert_proper_basis(&f, &format!("yaw={yaw:.3} pitch={pitch:.3}"));
        }
    }
}

/// At yaw = pitch = 0 the documented convention is +X forward, +Y right, +Z up.
#[test]
fn identity_orientation_matches_documented_convention() {
    let f = ReferenceFrame::from_position_yaw_pitch(DVec3::ZERO, 0.0, 0.0);
    assert!((f.local_forward() - DVec3::X).length() < 1e-12, "forward = {:?}, want +X", f.local_forward());
    assert!((f.local_right() - DVec3::Y).length() < 1e-12, "right = {:?}, want +Y", f.local_right());
    assert!((f.local_up() - DVec3::Z).length() < 1e-12, "up = {:?}, want +Z", f.local_up());
}

/// Straight up and down: no NaN from normalizing a zero cross product.
#[test]
fn pitch_singularity_does_not_produce_nan() {
    for pitch in [std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
        for yaw in angles() {
            let f = ReferenceFrame::from_position_yaw_pitch(DVec3::ZERO, yaw, pitch);
            assert_proper_basis(&f, &format!("singularity yaw={yaw:.3} pitch={pitch:.3}"));
        }
    }
}

#[test]
fn yaw_and_pitch_round_trip() {
    for yaw in angles() {
        for pitch in [-1.2, -0.5, 0.0, 0.5, 1.2] {
            let f = ReferenceFrame::from_position_yaw_pitch(DVec3::ZERO, yaw, pitch);
            assert!((f.pitch() - pitch).abs() < 1e-9, "pitch round trip: {} vs {pitch}", f.pitch());
            let dy = ((f.yaw() - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU))
                - std::f64::consts::PI;
            assert!(dy.abs() < 1e-9, "yaw round trip: {} vs {yaw} (drift {dy})", f.yaw());
        }
    }
}

/// Both `look_at_universal` variants agree.
#[test]
fn look_at_variants_agree() {
    let origin = DVec3::new(1.0e6, -2.0e6, 5.0e5);
    let frame: ReferenceFrame = DMat4::from_translation(origin).into();
    for target in [
        DVec3::new(2.0e6, 0.0, 0.0),
        DVec3::new(-3.0e6, 4.0e6, 1.0e6),
        DVec3::new(0.0, 0.0, 9.0e6),
    ] {
        let a = frame.look_at_universal(target, DVec3::Z);
        let b = frame.look_at_universal_roll_rads(target, 0.0);
        assert_proper_basis(&a, "look_at_universal");
        assert_proper_basis(&b, "look_at_universal_roll_rads");
        // Both point forward at the target.
        let want = (target - origin).normalize();
        assert!((a.local_forward() - want).length() < 1e-9, "look_at_universal forward off target");
        assert!((b.local_forward() - want).length() < 1e-9, "roll variant forward off target");
    }
}

/// Translating in local coordinates and back returns to the start.
#[test]
fn local_translation_round_trips() {
    let f = ReferenceFrame::from_position_yaw_pitch(DVec3::new(10.0, 20.0, 30.0), 0.7, 0.3);
    let delta = DVec3::new(5.0, -3.0, 2.0);
    let moved = f.clone().translated_local(delta).translated_local(-delta);
    assert!(
        (moved.position() - f.position()).length() < 1e-9,
        "local translate round trip: {:?} vs {:?}", moved.position(), f.position()
    );
}
