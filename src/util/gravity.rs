use std::ops::Mul;
use glam::DVec3;

/// Masses in grams, displacement in meters.
/// Returns force in newtons TODO: really?
fn newton_gravity(mass_a: f64, mass_b: f64, a_to_b: DVec3) -> DVec3 {
    let gravitational_constant = 1.0;
    let distance = a_to_b.length();
    let directionless = -(gravitational_constant * (mass_a * mass_b)) / (distance * distance * distance);
    directionless * a_to_b
}

/// Mu value is mass of attractor in grams * gravity constant
/// Displacement in meters
/// Returns acceleration in meters * seconds^-2
fn one_body_acceleration(local_gravity_mu: f64, a_to_b: DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(local_gravity_mu / (distance * distance *  distance));
    directionless * a_to_b
}
