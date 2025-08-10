
use bevy::math::{Vec3, DVec3};

pub trait GlamVec {
    // Convert from z-axis-up to y-axis-up coordinate system
    // In z-axis-up: (x, y, z) where z is up
    // In y-axis-up: (x, z, -y) where y is up
    fn as_bevy(&self) -> Vec3;

    fn as_regular(&self) -> DVec3;

    fn as_bevy_scaled(&self, scale: f64) -> Vec3;

    fn as_bevy_scaled_dvec(&self, scale: f64) -> DVec3;

    fn as_bevy_scaled_cheated(&self, scale: f64, cheat: DVec3) -> Vec3;
}

impl GlamVec for DVec3 {
    // Convert from z-axis-up to y-axis-up coordinate system
    // In z-axis-up: (x, y, z) where z is up
    // In y-axis-up: (x, z, -y) where y is up
    fn as_bevy(&self) -> Vec3 {
        let v = self.as_vec3();
        Vec3::new(v.x, v.z, -v.y)
    }

    fn as_regular(&self) -> DVec3 {
        DVec3::new(self.y, -self.z, self.x)
    }

    fn as_bevy_scaled(&self, scale: f64) -> Vec3 {
        (self * scale).as_bevy()
    }

    fn as_bevy_scaled_dvec(&self, scale: f64) -> DVec3 {
        DVec3::new(self.x, self.z, -self.y) * scale
    }

    fn as_bevy_scaled_cheated(&self, scale: f64, cheat: DVec3) -> Vec3 {
        let bevyed = DVec3::new(self.x, self.z, -self.y);
        let scaled = bevyed * scale;
        let cheated = scaled - cheat;
        cheated.as_vec3()
    }
}
