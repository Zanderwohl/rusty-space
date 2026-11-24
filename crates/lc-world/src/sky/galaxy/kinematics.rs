//! A star's velocity: its frame's rotation curve where it is, plus a dispersion that grows with
//! age, drawn from its id. Shared by the bake and the loader so both compute the same velocity.

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use crate::rng;
use crate::sky::StarId;

const SALT_VELOCITY: u64 = 0x7665_6c6f;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Rotation {
    Static,
    /// `v(R) = speed · R / √(R² + core²)`, prograde about the frame's +Z.
    Flat { speed_km_s: f64, core_ly: f64 },
}

/// A population's frame and velocity dispersion.
#[derive(Clone, Debug, PartialEq)]
pub struct Kinematics {
    pub origin: DVec3,
    /// Columns are the frame's axes in galaxy coordinates.
    pub axes: DMat3,
    pub rotation: Rotation,
    /// `σ = zero_age_km_s · (1 + age_gyr)^exponent`, per axis.
    pub dispersion_km_s: f64,
    pub dispersion_exponent: f64,
}

impl Kinematics {
    /// Circular velocity at `p`, km/s, in galaxy axes.
    pub fn circular_km_s(&self, p: DVec3) -> DVec3 {
        let Rotation::Flat { speed_km_s, core_ly } = self.rotation else { return DVec3::ZERO };
        let local = self.axes.transpose() * (p - self.origin);
        let r = libm::hypot(local.x, local.y);
        if r == 0.0 {
            return DVec3::ZERO;
        }
        let v = speed_km_s * r / libm::sqrt(r * r + core_ly * core_ly);
        self.axes * (DVec3::new(-local.y, local.x, 0.0) * (v / r))
    }

    pub fn velocity_km_s(&self, p: DVec3, age_gyr: f64, id: StarId) -> DVec3 {
        let sigma = self.dispersion_km_s * libm::pow(1.0 + age_gyr, self.dispersion_exponent);
        let h = rng::hash(&[id.get(), SALT_VELOCITY]);
        self.circular_km_s(p) + DVec3::new(gaussian(h, 0), gaussian(h, 1), gaussian(h, 2)) * sigma
    }
}

/// Box–Muller with `libm`, so every platform draws the same velocity.
fn gaussian(h: u64, n: u64) -> f64 {
    let u = |k: u64| rng::uniform(rng::mix(h ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15)));
    let u1 = u(2 * n + 1).max(f64::MIN_POSITIVE);
    let u2 = u(2 * n + 2);
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(std::f64::consts::TAU * u2)
}
