use bevy::math::DVec3;


// TODO: Rename better to differentiate functions
pub fn newton_gravity_yeet(gravitational_constant: f64, mass_a: f64, displacement_a: DVec3, mass_b: f64, displacement_b: &DVec3) -> DVec3 {
    let a_to_b = displacement_a - displacement_b;
    let distance = a_to_b.length();
    let directionless = -(gravitational_constant * (mass_a * mass_b)) / (distance * distance * distance);
    directionless * a_to_b
}

/// Masses in grams, displacement in meters.
/// Returns force in newtons TODO: really?
pub fn newton_gravity(gravitational_constant: f64, mass_a: f64, mass_b: f64, a_to_b: &DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(gravitational_constant * (mass_a * mass_b)) / (distance * distance * distance);
    directionless * a_to_b
}

/// Mu value is mass of attractor in grams * gravity constant
/// Displacement in meters
/// Returns acceleration in meters * seconds^-2
pub fn one_body_acceleration(local_gravity_mu: f64, a_to_b: DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(local_gravity_mu / (distance * distance *  distance));
    directionless * a_to_b
}
