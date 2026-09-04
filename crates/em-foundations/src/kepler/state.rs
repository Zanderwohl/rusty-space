//! Orbital elements and Cartesian state, and the conversion each way.
//!
//! Angles in radians, distances metres, velocities m/s, `mu` in m³/s².

use glam::{DMat3, DVec3};

use super::anomaly;
use crate::common::unit_circle_xy;

/// Classical orbital elements, in radians and metres.
///
/// `true_anomaly` places the body on the orbit; propagation needs an epoch and mean
/// motion, owned by the layer above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elements {
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub longitude_of_ascending_node: f64,
    pub argument_of_periapsis: f64,
    pub true_anomaly: f64,
}

/// Below this eccentricity an orbit is treated as circular and the argument of periapsis
/// is undefined. Above f64 noise, far below any real orbit's eccentricity.
pub const CIRCULAR_TOLERANCE: f64 = 1e-11;

/// Below this inclination (radians) an orbit is equatorial and the node is undefined.
pub const EQUATORIAL_TOLERANCE: f64 = 1e-11;

/// Position in the perifocal (PQW) frame: +P toward periapsis, +Q 90° along the
/// direction of motion, +W along angular momentum. `semi_latus_rectum` is `p = a(1 - e²)`.
#[inline]
pub fn perifocal_position(semi_latus_rectum: f64, eccentricity: f64, true_anomaly: f64) -> DVec3 {
    let (sin_nu, cos_nu) = true_anomaly.sin_cos();
    let r = semi_latus_rectum / (1.0 + eccentricity * cos_nu);
    DVec3::new(r * cos_nu, r * sin_nu, 0.0)
}

/// Velocity in the perifocal frame: `sqrt(mu/p) * (-sin v, e + cos v, 0)`.
#[inline]
pub fn perifocal_velocity(
    gravitational_parameter: f64,
    semi_latus_rectum: f64,
    eccentricity: f64,
    true_anomaly: f64,
) -> DVec3 {
    let (sin_nu, cos_nu) = true_anomaly.sin_cos();
    let k = (gravitational_parameter / semi_latus_rectum).sqrt();
    DVec3::new(-k * sin_nu, k * (eccentricity + cos_nu), 0.0)
}

/// Perifocal to reference frame: the 3-1-3 sequence `Rz(Omega) · Rx(i) · Rz(omega)`.
#[inline]
pub fn perifocal_to_inertial(
    longitude_of_ascending_node: f64,
    inclination: f64,
    argument_of_periapsis: f64,
) -> DMat3 {
    DMat3::from_rotation_z(longitude_of_ascending_node)
        * DMat3::from_rotation_x(inclination)
        * DMat3::from_rotation_z(argument_of_periapsis)
}

/// Position and velocity implied by a set of elements.
///
/// `None` for parabolic (`e == 1`): `a` is infinite and `p` cannot be recovered from it.
pub fn to_state(gravitational_parameter: f64, elements: &Elements) -> Option<(DVec3, DVec3)> {
    let e = elements.eccentricity;
    if (e - 1.0).abs() < f64::EPSILON {
        return None;
    }
    let p = elements.semi_major_axis * (1.0 - e * e);
    if p <= 0.0 || !p.is_finite() {
        return None;
    }

    let r_pqw = perifocal_position(p, e, elements.true_anomaly);
    let v_pqw = perifocal_velocity(gravitational_parameter, p, e, elements.true_anomaly);
    let rot = perifocal_to_inertial(
        elements.longitude_of_ascending_node,
        elements.inclination,
        elements.argument_of_periapsis,
    );

    Some((rot * r_pqw, rot * v_pqw))
}

/// Elements implied by a Cartesian state.
///
/// Degenerate orbits take the usual conventions, and still reproduce the input state
/// through [`to_state`]:
///
/// - **Equatorial** (`i ~ 0`): `Omega = 0`, argument of periapsis measured from +X
///   (the longitude of periapsis).
/// - **Circular** (`e ~ 0`): `omega = 0`, true anomaly measured from the ascending node
///   (the argument of latitude).
/// - **Circular and equatorial**: both 0, true anomaly is the true longitude from +X.
///
/// `None` for zero radius or zero angular momentum (a radial trajectory).
pub fn from_state(
    gravitational_parameter: f64,
    position: DVec3,
    velocity: DVec3,
) -> Option<Elements> {
    let mu = gravitational_parameter;
    let r = position.length();
    if r == 0.0 || !r.is_finite() {
        return None;
    }

    let h = position.cross(velocity);
    let h_len = h.length();
    if h_len == 0.0 {
        return None; // radial: inclination and node are meaningless
    }

    // Eccentricity vector, in the form avoiding a second cross product.
    let v2 = velocity.length_squared();
    let e_vec = ((v2 - mu / r) * position - position.dot(velocity) * velocity) / mu;
    let e = e_vec.length();

    // Specific energy fixes `a`; parabolic orbits have zero energy and no finite `a`.
    let energy = v2 / 2.0 - mu / r;
    let semi_major_axis = if energy.abs() < f64::EPSILON {
        f64::INFINITY
    } else {
        -mu / (2.0 * energy)
    };

    let inclination = (h.z / h_len).clamp(-1.0, 1.0).acos();
    let equatorial = inclination < EQUATORIAL_TOLERANCE
        || (std::f64::consts::PI - inclination) < EQUATORIAL_TOLERANCE;
    let circular = e < CIRCULAR_TOLERANCE;

    // Node vector z_hat x h, toward the ascending node.
    let node = DVec3::new(-h.y, h.x, 0.0);
    let node_len = node.length();

    let (longitude_of_ascending_node, argument_of_periapsis, true_anomaly) =
        match (equatorial, circular) {
            (false, false) => {
                let raan = f64::atan2(node.y, node.x);
                let mut argp = (node.dot(e_vec) / (node_len * e)).clamp(-1.0, 1.0).acos();
                if e_vec.z < 0.0 {
                    argp = -argp;
                }
                let mut nu = (e_vec.dot(position) / (e * r)).clamp(-1.0, 1.0).acos();
                if position.dot(velocity) < 0.0 {
                    nu = -nu;
                }
                (raan, argp, nu)
            }
            (false, true) => {
                // Circular, inclined: argument of latitude, from the node.
                let raan = f64::atan2(node.y, node.x);
                let mut u = (node.dot(position) / (node_len * r)).clamp(-1.0, 1.0).acos();
                if position.z < 0.0 {
                    u = -u;
                }
                (raan, 0.0, u)
            }
            (true, false) => {
                // Equatorial, eccentric: longitude of periapsis, from +X.
                let mut lon_peri = (e_vec.x / e).clamp(-1.0, 1.0).acos();
                if e_vec.y < 0.0 {
                    lon_peri = -lon_peri;
                }
                let mut nu = (e_vec.dot(position) / (e * r)).clamp(-1.0, 1.0).acos();
                if position.dot(velocity) < 0.0 {
                    nu = -nu;
                }
                // Retrograde equatorial runs the other way.
                if h.z < 0.0 {
                    lon_peri = -lon_peri;
                    nu = -nu;
                }
                (0.0, lon_peri, nu)
            }
            (true, true) => {
                // Circular equatorial: true longitude from +X.
                let mut lon = (position.x / r).clamp(-1.0, 1.0).acos();
                if position.y < 0.0 {
                    lon = -lon;
                }
                if h.z < 0.0 {
                    lon = -lon;
                }
                (0.0, 0.0, lon)
            }
        };

    Some(Elements {
        semi_major_axis,
        eccentricity: e,
        inclination: if equatorial && h.z < 0.0 { std::f64::consts::PI } else { inclination },
        longitude_of_ascending_node: anomaly::wrap_tau(longitude_of_ascending_node),
        argument_of_periapsis: anomaly::wrap_tau(argument_of_periapsis),
        true_anomaly: anomaly::wrap_tau(true_anomaly),
    })
}

/// Semi-minor axis from the elements, `b = a sqrt(1 - e²)`.
#[inline]
pub fn semi_minor_axis(elements: &Elements) -> f64 {
    elements.semi_major_axis * unit_circle_xy(elements.eccentricity)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU_EARTH: f64 = 3.986004418e14;
    const MU_SUN: f64 = 1.32712440018e20;

    fn close(a: f64, b: f64, tol: f64, what: &str) {
        let scale = b.abs().max(1.0);
        assert!((a - b).abs() <= tol * scale, "{what}: {a} vs {b}");
    }

    fn assert_round_trips(mu: f64, el: Elements) {
        let (r, v) = to_state(mu, &el).expect("elements should produce a state");
        let back = from_state(mu, r, v).expect("state should produce elements");

        close(back.semi_major_axis, el.semi_major_axis, 1e-9, "a");
        close(back.eccentricity, el.eccentricity, 1e-9, "e");
        close(back.inclination, el.inclination, 1e-9, "i");
        for (got, want, name) in [
            (back.longitude_of_ascending_node, el.longitude_of_ascending_node, "raan"),
            (back.argument_of_periapsis, el.argument_of_periapsis, "argp"),
            (back.true_anomaly, el.true_anomaly, "nu"),
        ] {
            let d = anomaly::wrap_pi(got - want).abs();
            assert!(d < 1e-8, "{name}: {got} vs {want} (drift {d:e})");
        }

        let (r2, v2) = to_state(mu, &back).unwrap();
        assert!((r2 - r).length() / r.length() < 1e-9, "position: {r2:?} vs {r:?}");
        assert!((v2 - v).length() / v.length() < 1e-9, "velocity: {v2:?} vs {v:?}");
    }

    #[test]
    fn elements_round_trip_through_state() {
        for e in [0.0001, 0.01, 0.0167, 0.2, 0.5, 0.8, 0.95] {
            for inc in [0.01, 0.4, 1.0, 1.57, 2.5, 3.0] {
                for nu in [0.1, 1.0, 2.5, 3.5, 4.7, 6.0] {
                    assert_round_trips(MU_EARTH, Elements {
                        semi_major_axis: 7.0e6,
                        eccentricity: e,
                        inclination: inc,
                        longitude_of_ascending_node: 1.2,
                        argument_of_periapsis: 2.3,
                        true_anomaly: nu,
                    });
                }
            }
        }
    }

    #[test]
    fn round_trips_at_planetary_scale() {
        assert_round_trips(MU_SUN, Elements {
            semi_major_axis: 1.496e11,
            eccentricity: 0.0167086,
            inclination: 8.7e-7,
            longitude_of_ascending_node: 3.05,
            argument_of_periapsis: 1.99,
            true_anomaly: 1.75,
        });
    }

    /// Degenerate cases pick a convention; the state must still survive the trip.
    #[test]
    fn degenerate_orbits_still_reproduce_their_state() {
        let cases = [
            ("circular inclined", 0.0, 0.9),
            ("eccentric equatorial", 0.3, 0.0),
            ("circular equatorial", 0.0, 0.0),
        ];
        for (name, e, inc) in cases {
            let el = Elements {
                semi_major_axis: 7.0e6,
                eccentricity: e,
                inclination: inc,
                longitude_of_ascending_node: if inc == 0.0 { 0.0 } else { 1.2 },
                argument_of_periapsis: if e == 0.0 { 0.0 } else { 2.3 },
                true_anomaly: 1.1,
            };
            let (r, v) = to_state(MU_EARTH, &el).unwrap();
            let back = from_state(MU_EARTH, r, v).unwrap_or_else(|| panic!("{name}"));
            let (r2, v2) = to_state(MU_EARTH, &back).unwrap();
            assert!((r2 - r).length() / r.length() < 1e-9, "{name} position");
            assert!((v2 - v).length() / v.length() < 1e-9, "{name} velocity");
        }
    }

    /// Circular orbits have constant speed `sqrt(mu/r)`.
    #[test]
    fn circular_orbit_has_circular_speed() {
        let a: f64 = 7.0e6;
        for nu in [0.0, 1.0, 3.0, 5.0] {
            let el = Elements {
                semi_major_axis: a, eccentricity: 0.0, inclination: 0.5,
                longitude_of_ascending_node: 0.0, argument_of_periapsis: 0.0,
                true_anomaly: nu,
            };
            let (r, v) = to_state(MU_EARTH, &el).unwrap();
            close(r.length(), a, 1e-12, "radius");
            close(v.length(), (MU_EARTH / a).sqrt(), 1e-12, "speed");
            assert!(r.dot(v).abs() / (r.length() * v.length()) < 1e-12, "r must be perpendicular to v");
        }
    }

    /// Vis-viva holds at every point of an eccentric orbit.
    #[test]
    fn speed_obeys_vis_viva() {
        let a: f64 = 7.0e6;
        let e = 0.4;
        for nu in [0.0, 0.7, 1.9, 3.14159, 4.4, 5.9] {
            let el = Elements {
                semi_major_axis: a, eccentricity: e, inclination: 0.3,
                longitude_of_ascending_node: 1.0, argument_of_periapsis: 2.0,
                true_anomaly: nu,
            };
            let (r, v) = to_state(MU_EARTH, &el).unwrap();
            let expected = (MU_EARTH * (2.0 / r.length() - 1.0 / a)).sqrt();
            close(v.length(), expected, 1e-12, "vis-viva speed");
        }
    }

    /// Periapsis is the fastest and closest point, apoapsis the slowest and furthest.
    #[test]
    fn apsides_are_the_speed_extremes() {
        let (a, e) = (7.0e6, 0.4);
        let mk = |nu| Elements {
            semi_major_axis: a, eccentricity: e, inclination: 0.0,
            longitude_of_ascending_node: 0.0, argument_of_periapsis: 0.0, true_anomaly: nu,
        };
        let (rp, vp) = to_state(MU_EARTH, &mk(0.0)).unwrap();
        let (ra, va) = to_state(MU_EARTH, &mk(std::f64::consts::PI)).unwrap();

        close(rp.length(), a * (1.0 - e), 1e-12, "periapsis radius");
        close(ra.length(), a * (1.0 + e), 1e-12, "apoapsis radius");
        assert!(vp.length() > va.length(), "periapsis must be faster");
        // Conserved angular momentum: r_p v_p == r_a v_a.
        close(rp.length() * vp.length(), ra.length() * va.length(), 1e-12, "specific angular momentum");
    }

    #[test]
    fn radial_and_degenerate_states_are_rejected() {
        let r = DVec3::new(7.0e6, 0.0, 0.0);
        assert!(from_state(MU_EARTH, r, DVec3::new(1000.0, 0.0, 0.0)).is_none());
        assert!(from_state(MU_EARTH, DVec3::ZERO, DVec3::new(0.0, 1000.0, 0.0)).is_none());
    }

    #[test]
    fn parabolic_elements_have_no_state() {
        let el = Elements {
            semi_major_axis: 7.0e6, eccentricity: 1.0, inclination: 0.0,
            longitude_of_ascending_node: 0.0, argument_of_periapsis: 0.0, true_anomaly: 0.5,
        };
        assert!(to_state(MU_EARTH, &el).is_none());
    }

    /// A proper rotation, not a mirror.
    #[test]
    fn perifocal_rotation_is_proper() {
        for (raan, inc, argp) in [(0.0, 0.0, 0.0), (1.2, 0.4, 2.3), (5.0, 3.0, 1.0)] {
            let m = perifocal_to_inertial(raan, inc, argp);
            close(m.determinant(), 1.0, 1e-12, "determinant");
        }
    }
}
