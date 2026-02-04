use bevy::math::DVec3;


/// Mu value is mass of attractor in grams * gravity constant
/// Displacement in meters
/// Returns acceleration in meters * seconds^-2
pub fn one_body_acceleration(local_gravity_mu: f64, a_to_b: DVec3) -> DVec3 {
    let distance = a_to_b.length();
    let directionless = -(local_gravity_mu / (distance * distance *  distance));
    directionless * a_to_b
}
