use bevy::math::{DMat4, DQuat, DVec3, Vec3};
use crate::foundations::reference_frame::ReferenceFrame;

/// Represents a transformation from Reference Frame A to Reference Frame B
pub struct Transformation {
    pub(in crate::foundations::reference_frame) mat: DMat4,
}

impl Transformation {
    /// Transforms a point from A to B
    pub fn point(&self, point: DVec3) -> DVec3 {
        self.mat.transform_point3(point)
    }

    /// Transforms a point from A to B
    pub fn point_f32(&self, point: Vec3) -> Vec3 {
        self.mat.transform_point3(DVec3::from(point)).as_vec3()
    }

    /// Transforms a velocity or direction vector from A to B,
    /// ignoring displacement (using w = 0)
    pub fn velocity(&self, velocity: DVec3) -> DVec3 {
        self.mat.transform_vector3(velocity)
    }

    /// Transforms the Pose (displacement and rotation) from A to B
    pub fn pose(&self, pose: ReferenceFrame) -> ReferenceFrame {
        ReferenceFrame {
            mat: self.mat * pose.mat
        }
    }
}
