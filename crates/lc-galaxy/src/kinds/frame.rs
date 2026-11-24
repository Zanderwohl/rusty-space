use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use crate::compiled::{Compiled, Frame, Rotation};
use crate::feature::{FeatureId, FeatureKind, Problem};

/// A galaxy's frame: where its center is, which way its north points, and how it turns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameKind {
    pub origin_ly: (f64, f64, f64),
    /// Of the frame's +Z from the galaxy's +Z, about the galaxy's +X.
    pub tilt_deg: f64,
    /// About the galaxy's +Z, applied after the tilt.
    pub azimuth_deg: f64,
    pub rotation: Rotation,
}

impl FrameKind {
    pub fn axes(&self) -> DMat3 {
        let (st, ct) = (libm::sin(self.tilt_deg.to_radians()), libm::cos(self.tilt_deg.to_radians()));
        let (sa, ca) = (libm::sin(self.azimuth_deg.to_radians()), libm::cos(self.azimuth_deg.to_radians()));
        let tilt = DMat3::from_cols(DVec3::X, DVec3::new(0.0, ct, st), DVec3::new(0.0, -st, ct));
        let azimuth = DMat3::from_cols(DVec3::new(ca, sa, 0.0), DVec3::new(-sa, ca, 0.0), DVec3::Z);
        azimuth * tilt
    }
}

impl FeatureKind for FrameKind {
    const KIND: &'static str = "frame";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["origin_ly", "tilt_deg", "azimuth_deg", "rotation"];

    fn new() -> Self {
        Self { origin_ly: (0.0, 0.0, 0.0), tilt_deg: 0.0, azimuth_deg: 0.0, rotation: Rotation::Static }
    }

    fn summary(&self) -> String {
        format!("at {} ly", super::fmt_v3(self.origin_ly))
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        if let Rotation::Flat { speed_km_s, core_ly } = self.rotation
            && !(speed_km_s.is_finite() && core_ly > 0.0)
        {
            return Err(Problem::Invalid("a flat rotation curve needs a positive core".into()));
        }
        let (x, y, z) = self.origin_ly;
        out.frames.insert(id, Frame { origin: DVec3::new(x, y, z), axes: self.axes(), rotation: self.rotation.clone() });
        Ok(())
    }
}
