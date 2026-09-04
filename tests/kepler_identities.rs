//! Algebraic identities every Kepler formula in `foundations::kepler` must satisfy.

use exotic_matters::foundations::kepler::*;
use bevy::math::DVec3;

const TOL: f64 = 1e-9;

/// Eccentricities to sweep; stops short of 1.0 (parabolic).
fn eccentricities() -> Vec<f64> {
    vec![0.0, 0.0067, 0.0167, 0.0934, 0.2056, 0.2488, 0.436, 0.7, 0.9, 0.95]
}

fn true_anomalies() -> Vec<f64> {
    (0..72).map(|i| i as f64 * std::f64::consts::TAU / 72.0).collect()
}

fn assert_close(actual: f64, expected: f64, tol: f64, what: &str) {
    let scale = expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tol * scale,
        "{what}: got {actual:e}, expected {expected:e} (rel err {:e})",
        (actual - expected).abs() / scale
    );
}

// === Conic geometry ===

/// `p = a(1 - e^2)`, not the semi-minor axis `a*sqrt(1-e^2)`.
#[test]
fn semi_parameter_is_the_semi_latus_rectum() {
    let a = 1.496e11;
    for e in eccentricities() {
        let p = semi_parameter::definition(a, e);
        assert_close(p, a * (1.0 - e * e), TOL, &format!("semi_parameter at e={e}"));
        assert_close(
            p,
            semi_latus_rectum::conic_definition(a, e),
            TOL,
            &format!("semi_parameter vs semi_latus_rectum at e={e}"),
        );
    }
}

/// `p` and `b = a*sqrt(1-e^2)` agree only at e=0.
#[test]
fn semi_parameter_differs_from_semi_minor_axis() {
    let a = 1.496e11;
    for e in eccentricities().into_iter().filter(|e| *e > 0.0) {
        let p = semi_parameter::definition(a, e);
        let b = semi_minor_axis::conic_definition(a, e);
        assert!(p < b, "at e={e}, semi-parameter {p:e} should be below semi-minor axis {b:e}");
    }
}

/// `r_p = a(1-e)` and `r_a = a(1+e)`, however they are derived.
#[test]
fn apsides_agree_across_every_derivation() {
    let a = 1.496e11;
    for e in eccentricities() {
        let expect_peri = a * (1.0 - e);
        let expect_apo = a * (1.0 + e);

        assert_close(periapsis::definition(a, e), expect_peri, TOL, &format!("periapsis::definition e={e}"));
        assert_close(
            apsides::periapsis::from_parameters(a, e),
            expect_peri, TOL, &format!("apsides::periapsis::from_parameters e={e}"),
        );
        if e > 0.0 {
            // q*e == p
            let focal_parameter = semi_parameter::definition(a, e) / e;
            assert_close(
                apsides::periapsis::definition(focal_parameter, e),
                expect_peri, TOL, &format!("apsides::periapsis::definition e={e}"),
            );
            assert_close(
                apsides::apoapsis::definition(focal_parameter, e),
                expect_apo, TOL, &format!("apsides::apoapsis::definition e={e}"),
            );
        }

        let apo = apoapsis::definition(a, e).expect("closed orbit has an apoapsis");
        assert_close(apo, expect_apo, TOL, &format!("apoapsis::definition e={e}"));
        assert_close(
            apsides::apoapsis::from_parameters(a, e),
            expect_apo, TOL, &format!("apsides::apoapsis::from_parameters e={e}"),
        );
    }
}

/// Periapsis never exceeds apoapsis.
#[test]
fn periapsis_never_exceeds_apoapsis() {
    let a = 1.496e11;
    for e in eccentricities() {
        let rp = periapsis::definition(a, e);
        let ra = apoapsis::definition(a, e).unwrap();
        assert!(rp <= ra, "at e={e}: periapsis {rp:e} > apoapsis {ra:e}");
    }
}

#[test]
fn open_orbits_have_no_apoapsis() {
    for e in [1.0, 1.5, 3.0] {
        assert!(apoapsis::definition(1.0e11, e).is_none(), "e={e} should have no apoapsis");
    }
}

/// `a -> b -> a` round trip.
#[test]
fn semi_major_axis_round_trips_through_semi_minor() {
    let a = 1.496e11;
    for e in eccentricities() {
        let b = semi_minor_axis::conic_definition(a, e);
        assert_close(
            semi_major_axis::from_semi_minor_and_eccentricity(b, e),
            a, TOL, &format!("a -> b -> a at e={e}"),
        );
    }
}

#[test]
fn semi_major_axis_from_semi_latus_rectum_round_trips() {
    let a = 1.496e11;
    for e in eccentricities() {
        let p = semi_latus_rectum::conic_definition(a, e);
        assert_close(
            semi_major_axis::from_eccentricity_and_semi_latus_rectum(e, p),
            a, TOL, &format!("a -> p -> a at e={e}"),
        );
    }
}

/// Both radius overloads agree, and the infallible wrapper does not transpose arguments.
#[test]
fn radius_overloads_agree() {
    let a = 1.496e11;
    for e in eccentricities() {
        for nu in true_anomalies() {
            let expected = a * (1.0 - e * e) / (1.0 + e * nu.cos());
            let from2 = local::radius::from_semi_major_axis(a, e, nu).unwrap();
            assert_close(from2, expected, TOL, &format!("from_elements2 e={e} nu={nu}"));
            assert_close(
                local::radius::from_semi_major_axis_infallible(a, e, nu),
                expected, TOL, &format!("from_elements2_infallible e={e} nu={nu}"),
            );
            if e > 0.0 {
                let focal_parameter = a * (1.0 - e * e) / e;
                assert_close(
                    local::radius::from_focal_parameter(focal_parameter, e, nu),
                    expected, TOL, &format!("from_elements1 e={e} nu={nu}"),
                );
            }
        }
    }
}

/// A circular orbit has constant radius `a` at every true anomaly.
#[test]
fn circular_orbit_has_constant_radius() {
    let a = 7.0e6;
    for nu in true_anomalies() {
        assert_close(local::radius::from_semi_major_axis(a, 0.0, nu).unwrap(), a, TOL, "circular radius");
    }
}

// === Anomalies ===

/// `nu -> E -> r` reproduces the radius computed straight from `nu`, in every quadrant.
#[test]
fn eccentric_anomaly_round_trips_to_radius() {
    let a = 1.496e11;
    for e in eccentricities() {
        for nu in true_anomalies() {
            let ea = eccentric_anomaly::from_true_anomaly(e, nu);
            let r_from_ea = local::radius::from_eccentric_anomaly(a, e, ea);
            let r_from_nu = local::radius::from_semi_major_axis(a, e, nu).unwrap();
            assert_close(r_from_ea, r_from_nu, 1e-7, &format!("r(E) vs r(nu) at e={e} nu={nu}"));
        }
    }
}

/// `E -> nu -> E` (mod tau).
#[test]
fn true_and_eccentric_anomaly_round_trip() {
    for e in eccentricities() {
        for ea in true_anomalies() {
            let nu = true_anomaly::at_time(ea, e);
            let back = eccentric_anomaly::from_true_anomaly(e, nu);
            let diff = ((back - ea + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU))
                - std::f64::consts::PI;
            assert!(diff.abs() < 1e-7, "E -> nu -> E at e={e} E={ea}: drift {diff:e}");
        }
    }
}

/// The equation of the centre matches a converged Kepler solve for small `e`.
#[test]
fn equation_of_the_centre_matches_exact_solve_for_small_e() {
    for e in [0.0, 0.0067, 0.0167, 0.05] {
        for m in true_anomalies() {
            let approx = true_anomaly::from_mean_anomaly(m, e);
            let exact = exact_true_anomaly(m, e);
            let diff = ((approx - exact + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU))
                - std::f64::consts::PI;
            // Series truncated at e^3, so the tolerance scales with e^4.
            let tol = 50.0 * e.powi(4) + 1e-12;
            assert!(diff.abs() < tol, "eq. of centre at e={e} M={m}: err {diff:e} > {tol:e}");
        }
    }
}

/// Reference Kepler solve, implemented independently of the library.
fn exact_true_anomaly(m: f64, e: f64) -> f64 {
    let mut ea = m;
    for _ in 0..200 {
        let f = ea - e * ea.sin() - m;
        let fp = 1.0 - e * ea.cos();
        let step = f / fp;
        ea -= step;
        if step.abs() < 1e-15 { break; }
    }
    2.0 * f64::atan2(
        (1.0 + e).sqrt() * (ea / 2.0).sin(),
        (1.0 - e).sqrt() * (ea / 2.0).cos(),
    )
}

// === Third law ===

#[test]
fn third_law_round_trips() {
    let mu = 1.327e20; // Sun
    for a in [5.79e10_f64, 1.496e11, 2.279e11, 5.9e12] {
        let period = period::third_law(a, mu);
        assert_close(semi_major_axis::third_law(mu, period), a, 1e-9, "mu,T -> a");
        assert_close(gravitational_parameter::third_law(period, a), mu, 1e-9, "T,a -> mu");
    }
}

// === Vector quantities ===

/// Both eccentricity-vector implementations agree and reproduce the scalar eccentricity.
#[test]
fn eccentricity_vector_implementations_agree() {
    let mu = 3.986e14; // Earth
    let cases = [
        (DVec3::new(7.0e6, 0.0, 0.0), DVec3::new(0.0, 7.546e3, 0.0)),           // circular
        (DVec3::new(7.0e6, 0.0, 0.0), DVec3::new(0.0, 9.0e3, 0.0)),             // elliptical
        (DVec3::new(6.8e6, 1.2e6, 3.0e5), DVec3::new(-1.1e3, 7.4e3, 1.9e3)),    // inclined
    ];
    for (r, v) in cases {
        let a = eccentricity_vector::definition(mu, r, v);
        let b = eccentricity::vector::definition(r, v, mu);
        assert!(
            (a - b).length() < 1e-9 * a.length().max(1.0),
            "eccentricity vector impls disagree: {a:?} vs {b:?}"
        );

        // Magnitude equals the eccentricity implied by energy and angular momentum.
        let h = r.cross(v);
        let energy = v.length_squared() / 2.0 - mu / r.length();
        let e_scalar = (1.0 + 2.0 * energy * h.length_squared() / (mu * mu)).max(0.0).sqrt();
        assert_close(a.length(), e_scalar, 1e-7, "|e_vec| vs scalar e");
    }
}

/// A circular orbit has zero eccentricity vector.
#[test]
fn circular_orbit_has_zero_eccentricity_vector() {
    let mu: f64 = 3.986e14;
    let r = DVec3::new(7.0e6, 0.0, 0.0);
    let v = DVec3::new(0.0, (mu / 7.0e6).sqrt(), 0.0);
    let e = eccentricity_vector::definition(mu, r, v);
    assert!(e.length() < 1e-9, "circular orbit eccentricity vector {e:?} should vanish");
}

// === Energy ===

/// Specific orbital energy is `-mu/2a`, negative for a bound orbit.
#[test]
fn specific_energy_is_negative_and_matches_vis_viva() {
    let mu: f64 = 3.986e14;
    let a: f64 = 7.0e6;
    for e in [0.0, 0.1, 0.5, 0.9] {
        let r = a * (1.0 - e); // periapsis
        let v = (mu * (2.0 / r - 1.0 / a)).sqrt(); // vis-viva
        let eps = energy::mechanical::specific(v, mu, r);
        assert!(eps < 0.0, "bound orbit at e={e} has non-negative energy {eps:e}");
        assert_close(eps, -mu / (2.0 * a), 1e-9, &format!("specific energy at e={e}"));
    }
}

/// Escape velocity is exactly the zero-energy boundary.
#[test]
fn escape_velocity_has_zero_specific_energy() {
    let mu: f64 = 3.986e14;
    let r: f64 = 7.0e6;
    let v_esc = (2.0 * mu / r).sqrt();
    let eps = energy::mechanical::specific(v_esc, mu, r);
    assert!(eps.abs() < 1e-3, "escape velocity should give ~zero energy, got {eps:e}");
}

/// `true_anomaly::from_state_vectors` returns an angle for every orbit, including the
/// degenerate ones with no periapsis.
mod true_anomaly_from_state {
    use exotic_matters::foundations::kepler::{eccentricity_vector, state, true_anomaly};
    use bevy::math::DVec3;

    const MU_EARTH: f64 = 3.986004418e14;

    /// State vector from elements, so a test can state an orbit in the terms the function
    /// recovers.
    fn state_of(eccentricity: f64, inclination: f64, true_anomaly: f64) -> (DVec3, DVec3, DVec3) {
        let elements = state::Elements {
            semi_major_axis: 7.0e6,
            eccentricity,
            inclination,
            longitude_of_ascending_node: 0.4,
            argument_of_periapsis: if eccentricity > 0.0 { 0.9 } else { 0.0 },
            true_anomaly,
        };
        let (r, v) = state::to_state(MU_EARTH, &elements).expect("a closed orbit has a state");
        let e_vec = eccentricity_vector::definition(MU_EARTH, r, v);
        (r, v, e_vec)
    }

    /// A circular orbit has no eccentricity vector; dividing by its length gives NaN. Passed
    /// as an exact zero here. Recovered from a state it is ~1e-17 and points nowhere, giving
    /// a plausible wrong angle — that case is `it_agrees_with_the_full_element_recovery`.
    #[test]
    fn circular_orbits_do_not_produce_nan() {
        for &inclination in &[0.0, 0.5, std::f64::consts::PI / 2.0] {
            for &nu in &[0.0, 1.0, 3.0, 5.5] {
                let (r, v, recovered) = state_of(0.0, inclination, nu);
                for (what, e) in [("exactly zero", DVec3::ZERO), ("recovered", recovered)] {
                    let got = true_anomaly::from_state_vectors(r, v, e);
                    assert!(
                        got.is_finite(),
                        "circular ({what}) i={inclination} nu={nu} gave {got}"
                    );
                }
            }
        }
    }

    /// The angle is right, not merely finite: for a circular orbit it is the argument of
    /// latitude, which `from_state` also reports as the true anomaly.
    #[test]
    fn it_agrees_with_the_full_element_recovery() {
        for &e in &[0.0, 0.01, 0.3, 0.8] {
            for &inclination in &[0.0, 0.3, 1.2] {
                for &nu in &[0.2, 1.7, 3.4, 5.9] {
                    let (r, v, e_vec) = state_of(e, inclination, nu);
                    let got = true_anomaly::from_state_vectors(r, v, e_vec);
                    let expected = state::from_state(MU_EARTH, r, v)
                        .expect("elements recover")
                        .true_anomaly;
                    let diff = (got - expected).abs();
                    let diff = diff.min(std::f64::consts::TAU - diff);
                    assert!(
                        diff < 1e-9,
                        "e={e} i={inclination} nu={nu}: {got} vs {expected}"
                    );
                }
            }
        }
    }

    /// An eccentric orbit comes back where it started.
    #[test]
    fn eccentric_orbits_round_trip() {
        for &e in &[0.05, 0.4, 0.85] {
            for &nu in &[0.1, 2.0, 4.5, 6.0] {
                let (r, v, e_vec) = state_of(e, 0.7, nu);
                let got = true_anomaly::from_state_vectors(r, v, e_vec);
                let diff = (got - nu).abs();
                let diff = diff.min(std::f64::consts::TAU - diff);
                assert!(diff < 1e-9, "e={e}: stored nu={nu}, got {got}");
            }
        }
    }

    /// Degenerate inputs produce no NaN either.
    #[test]
    fn degenerate_inputs_are_finite() {
        let z = DVec3::ZERO;
        assert!(true_anomaly::from_state_vectors(z, z, z).is_finite(), "all zero");
        // Purely radial: no orbital plane to measure an angle in.
        let r = DVec3::new(7.0e6, 0.0, 0.0);
        let v = DVec3::new(1.0e3, 0.0, 0.0);
        assert!(true_anomaly::from_state_vectors(r, v, z).is_finite(), "radial");
    }
}
