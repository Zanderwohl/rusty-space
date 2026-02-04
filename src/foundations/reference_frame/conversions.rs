use bevy::math::{DMat4, DQuat, DVec3};
use crate::foundations::reference_frame::ReferenceFrame;
use crate::foundations::reference_frame::transformation::Transformation;

impl ReferenceFrame {
    pub fn from_dvec3(dvec: DVec3) -> Self {
        dvec.into()
    }
}

impl From<DMat4> for ReferenceFrame {
    fn from(mat: DMat4) -> Self {
        Self {
            mat
        }
    }
}

impl Into<DMat4> for ReferenceFrame {
    fn into(self) -> DMat4 {
        self.mat.clone()
    }
}

impl From<DQuat> for ReferenceFrame {
    fn from(d_quat: DQuat) -> Self {
        Self::new(DVec3::ZERO, d_quat)
    }
}

impl From<DVec3> for ReferenceFrame {
    fn from(d_vec3: DVec3) -> Self {
        Self::new(d_vec3, DQuat::IDENTITY)
    }
}

impl From<DMat4> for Transformation {
    fn from(mat: DMat4) -> Self {
        Self { mat }
    }
}
