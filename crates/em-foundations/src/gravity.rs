use glam::DVec3;

/// Gravitational acceleration from a single attractor, in m/s², pointing along `-a_to_b`.
///
/// `local_gravity_mu` is μ = G × M in m³/s²; `a_to_b` is the displacement from the
/// attracted body to the attractor, in metres.
pub fn one_body_acceleration(local_gravity_mu: f64, a_to_b: DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(local_gravity_mu / (distance * distance * distance));
    directionless * a_to_b
}
