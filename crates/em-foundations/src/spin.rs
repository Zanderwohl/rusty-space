//! Spin gravity for rotating habitats. SI units: m, rad/s, m/s, m/s².

use std::f64::consts::PI;

/// Tangential velocity (m/s) at `radius` metres from the axis, given rotations per minute.
pub fn tangential_velocity(radius: f64, rpm: f64) -> f64 {
    2.0 * PI * radius * rpm / 60.0
}

/// Centripetal acceleration (m/s²) — the perceived gravity in a rotating habitat.
pub fn centripetal_acceleration(tangential_velocity: f64, radius: f64) -> f64 {
    tangential_velocity * tangential_velocity / radius
}

/// Angular velocity (rad/s) from tangential velocity and radius.
pub fn angular_velocity(tangential_velocity: f64, radius: f64) -> f64 {
    tangential_velocity / radius
}

/// Coriolis acceleration (m/s², positive = spinward) for radial motion in a rotating frame.
///
/// `radial_velocity` is positive inward.
pub fn coriolis_acceleration(angular_velocity: f64, radial_velocity: f64) -> f64 {
    2.0 * angular_velocity * radial_velocity
}
