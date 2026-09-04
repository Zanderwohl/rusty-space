//! Gravity kernel correctness, and the integrator ordering the propagator must use.
//!
//! The stepping rule is specified here against the same `gravity::one_body_acceleration`
//! kernel the propagator uses, rather than by calling the propagator.

use exotic_matters::foundations::gravity;
use bevy::math::DVec3;

const MU_EARTH: f64 = 3.986004418e14;

// ---------------------------------------------------------------------------
// The gravity kernel
// ---------------------------------------------------------------------------

#[test]
fn acceleration_points_at_the_attractor() {
    // `a_to_b` runs from the attracted body toward the attractor.
    let a_to_b = DVec3::new(7.0e6, 0.0, 0.0);
    let acc = gravity::one_body_acceleration(MU_EARTH, a_to_b);
    assert!(acc.x < 0.0, "acceleration should oppose a_to_b, got {acc:?}");
    assert!(acc.y.abs() < 1e-12 && acc.z.abs() < 1e-12);
}

#[test]
fn acceleration_obeys_the_inverse_square_law() {
    let r = 7.0e6;
    let a1 = gravity::one_body_acceleration(MU_EARTH, DVec3::new(r, 0.0, 0.0)).length();
    let a2 = gravity::one_body_acceleration(MU_EARTH, DVec3::new(2.0 * r, 0.0, 0.0)).length();
    assert!((a1 / a2 - 4.0).abs() < 1e-9, "doubling r should quarter a; ratio was {}", a1 / a2);
    assert!((a1 - MU_EARTH / (r * r)).abs() < 1e-6, "magnitude should be mu/r^2");
}

// ---------------------------------------------------------------------------
// Integrator ordering
// ---------------------------------------------------------------------------

fn specific_energy(pos: DVec3, vel: DVec3, mu: f64) -> f64 {
    vel.length_squared() / 2.0 - mu / pos.length()
}

/// Semi-implicit (symplectic) Euler: velocity first, then position with the NEW velocity.
fn step_symplectic(pos: &mut DVec3, vel: &mut DVec3, mu: f64, dt: f64) {
    *vel += gravity::one_body_acceleration(mu, *pos) * dt;
    *pos += *vel * dt;
}

/// Forward (explicit) Euler: both updates use start-of-step state. Evaluating the
/// acceleration up front is what makes it explicit; evaluating it at the new position
/// instead gives the other semi-implicit variant, as stable as `step_symplectic`.
fn step_forward(pos: &mut DVec3, vel: &mut DVec3, mu: f64, dt: f64) {
    let acc = gravity::one_body_acceleration(mu, *pos);
    *pos += *vel * dt;
    *vel += acc * dt;
}

fn circular_orbit(r: f64, mu: f64) -> (DVec3, DVec3) {
    (DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, (mu / r).sqrt(), 0.0))
}

/// Over many orbits, symplectic Euler's energy error stays bounded rather than growing.
#[test]
fn symplectic_euler_conserves_energy_over_many_orbits() {
    let r: f64 = 7.0e6;
    let (mut pos, mut vel) = circular_orbit(r, MU_EARTH);
    let e0 = specific_energy(pos, vel, MU_EARTH);

    let period = std::f64::consts::TAU * (r * r * r / MU_EARTH).sqrt();
    let dt = period / 2000.0;
    let steps = (20.0 * period / dt) as usize; // 20 orbits

    let mut worst: f64 = 0.0;
    for _ in 0..steps {
        step_symplectic(&mut pos, &mut vel, MU_EARTH, dt);
        worst = worst.max((specific_energy(pos, vel, MU_EARTH) - e0).abs() / e0.abs());
    }
    assert!(worst < 5e-3, "symplectic energy drift {worst:e} should stay bounded over 20 orbits");
}

/// The orbit must stay an orbit: radius bounded, no outward spiral.
#[test]
fn symplectic_euler_keeps_the_orbit_closed() {
    let r: f64 = 7.0e6;
    let (mut pos, mut vel) = circular_orbit(r, MU_EARTH);

    let period = std::f64::consts::TAU * (r * r * r / MU_EARTH).sqrt();
    let dt = period / 2000.0;
    let steps = (20.0 * period / dt) as usize;

    for _ in 0..steps {
        step_symplectic(&mut pos, &mut vel, MU_EARTH, dt);
        let dev = (pos.length() - r).abs() / r;
        assert!(dev < 1e-2, "radius wandered {dev:e} from circular");
    }
}

/// Forward Euler gains energy without bound on the same problem: the guard on the ordering.
#[test]
fn forward_euler_is_measurably_worse() {
    let r: f64 = 7.0e6;
    let period = std::f64::consts::TAU * (r * r * r / MU_EARTH).sqrt();
    let dt = period / 2000.0;
    let steps = (20.0 * period / dt) as usize;

    let e0 = {
        let (p, v) = circular_orbit(r, MU_EARTH);
        specific_energy(p, v, MU_EARTH)
    };

    let final_drift = |mut step: Box<dyn FnMut(&mut DVec3, &mut DVec3, f64, f64)>| {
        let (mut pos, mut vel) = circular_orbit(r, MU_EARTH);
        for _ in 0..steps {
            step(&mut pos, &mut vel, MU_EARTH, dt);
        }
        (specific_energy(pos, vel, MU_EARTH) - e0).abs() / e0.abs()
    };

    let sym = final_drift(Box::new(step_symplectic));
    let fwd = final_drift(Box::new(step_forward));

    assert!(
        fwd > sym * 10.0,
        "forward Euler drift {fwd:e} should dwarf symplectic drift {sym:e}; \
         if these are close, the propagator ordering may have regressed"
    );
}
