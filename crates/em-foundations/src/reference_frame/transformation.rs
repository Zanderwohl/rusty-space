use glam::{DMat4, DVec3};
use crate::reference_frame::ReferenceFrame;

/// A transformation from reference frame A to reference frame B.
///
/// Simulation space only — right-handed and Z-up, in `f64`. There is deliberately no
/// `f32` entry point here: narrowing belongs at the render boundary, where the position
/// can first be made camera-relative. A `point_f32(Vec3) -> Vec3` used to sit on this
/// type, which invited feeding Y-up render-space data into Z-up simulation maths.
pub struct Transformation {
    pub(in crate::reference_frame) mat: DMat4,
}

impl Transformation {
    /// Transforms a point from A to B
    pub fn point(&self, point: DVec3) -> DVec3 {
        self.mat.transform_point3(point)
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
