use glam::{DMat4, DVec3};
use crate::reference_frame::ReferenceFrame;

/// A transformation from reference frame A to reference frame B.
///
/// Simulation space only: right-handed, Z-up, `f64`. No `f32` entry point — narrowing
/// belongs at the render boundary, where positions can first be made camera-relative;
/// a former `point_f32` invited Y-up render data into Z-up maths.
pub struct Transformation {
    pub(in crate::reference_frame) mat: DMat4,
}

impl Transformation {
    pub fn point(&self, point: DVec3) -> DVec3 {
        self.mat.transform_point3(point)
    }

    /// Transforms a velocity or direction from A to B, ignoring displacement (w = 0).
    pub fn velocity(&self, velocity: DVec3) -> DVec3 {
        self.mat.transform_vector3(velocity)
    }

    pub fn pose(&self, pose: ReferenceFrame) -> ReferenceFrame {
        ReferenceFrame {
            mat: self.mat * pose.mat
        }
    }
}
