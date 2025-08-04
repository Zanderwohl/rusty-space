
use bevy::math::{Vec3, DVec3};
use num_traits::ToPrimitive;

pub trait GlamVec {
    // Convert from z-axis-up to y-axis-up coordinate system
    // In z-axis-up: (x, y, z) where z is up
    // In y-axis-up: (x, z, -y) where y is up
    fn as_bevy(&self) -> Vec3;

    fn as_bevy_scaled(&self, scale: f64) -> Vec3;
}

impl GlamVec for DVec3 {
    // Convert from z-axis-up to y-axis-up coordinate system
    // In z-axis-up: (x, y, z) where z is up
    // In y-axis-up: (x, z, -y) where y is up
    fn as_bevy(&self) -> Vec3 {
        let v = self.as_vec3();
        Vec3::new(v.x, v.z, -v.y)
    }

    fn as_bevy_scaled(&self, scale: f64) -> Vec3 {
        (self * scale).as_bevy()
    }
}
