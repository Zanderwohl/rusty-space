use std::f64::consts::FRAC_PI_2;
use bevy::math::{Vec3, DVec3, DQuat, Quat};

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

pub trait GlamQuat {
    /// Convert a quaternion from Z-up (simulation) to Y-up (Bevy) coordinate system.
    ///
    /// The transformation is: sim(x,y,z) → bevy(x,z,-y)
    /// This is a -90 degree rotation around the X axis.
    /// Quaternion transformation uses conjugation: q_bevy = R * q_sim * R_inv
    fn as_bevy(&self) -> Quat;
}

impl GlamQuat for DQuat {
    fn as_bevy(&self) -> Quat {
        // Rotation that transforms Z-up to Y-up: -90 degrees around X axis
        // This sends +Z → +Y and +Y → -Z
        let coord_transform = DQuat::from_axis_angle(DVec3::X, -FRAC_PI_2);
        let transformed = coord_transform * *self * coord_transform.inverse();
        transformed.as_quat()
    }
}
