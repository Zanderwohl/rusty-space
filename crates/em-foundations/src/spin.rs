//! Physics calculations for rotating habitats / spin gravity.
//!
//! All units are SI:
//! - Radius: meters (m)
//! - Angular velocity: radians per second (rad/s)
//! - Velocity: meters per second (m/s)
//! - Acceleration: meters per second squared (m/s²)

use std::f64::consts::PI;

/// Calculate tangential velocity for a rotating body.
///
/// # Arguments
/// * `radius` - Distance from center of rotation (meters)
/// * `rpm` - Rotations per minute
///
/// # Returns
/// Tangential velocity in meters per second
pub fn tangential_velocity(radius: f64, rpm: f64) -> f64 {
    2.0 * PI * radius * rpm / 60.0
}

/// Calculate centripetal acceleration (perceived "gravity" in a rotating habitat).
///
/// # Arguments
/// * `tangential_velocity` - Tangential velocity in m/s
/// * `radius` - Distance from center of rotation (meters)
///
/// # Returns
/// Centripetal acceleration in m/s²
pub fn centripetal_acceleration(tangential_velocity: f64, radius: f64) -> f64 {
    tangential_velocity * tangential_velocity / radius
}

/// Calculate angular velocity from tangential velocity and radius.
///
/// # Arguments
/// * `tangential_velocity` - Tangential velocity in m/s
/// * `radius` - Distance from center of rotation (meters)
///
/// # Returns
/// Angular velocity in rad/s
pub fn angular_velocity(tangential_velocity: f64, radius: f64) -> f64 {
    tangential_velocity / radius
}

/// Calculate Coriolis acceleration for an object moving radially in a rotating frame.
///
/// # Arguments
/// * `angular_velocity` - Angular velocity of the rotating frame (rad/s)
/// * `radial_velocity` - Velocity component toward/away from center (m/s, positive = inward)
///
/// # Returns
/// Coriolis acceleration in m/s² (positive = spinward/prograde direction)
pub fn coriolis_acceleration(angular_velocity: f64, radial_velocity: f64) -> f64 {
    2.0 * angular_velocity * radial_velocity
}
