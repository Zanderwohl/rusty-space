use glam::DVec3;

/// Compute gravitational acceleration from a single body.
///
/// # Parameters
/// - `local_gravity_mu`: Gravitational parameter μ = G × M, in m³/s²
///   (where M is the attractor mass in **kilograms** and G ≈ 6.674×10⁻¹¹ m³ kg⁻¹ s⁻²)
/// - `a_to_b`: Displacement vector from the attracted body to the attractor, in **meters**
///
/// # Returns
/// Acceleration vector in **m/s²**, pointing toward the attractor (negative of a_to_b direction)
pub fn one_body_acceleration(local_gravity_mu: f64, a_to_b: DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(local_gravity_mu / (distance * distance * distance));
    directionless * a_to_b
}
