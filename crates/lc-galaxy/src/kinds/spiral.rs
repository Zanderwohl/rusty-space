//! Logarithmic spiral arms in the frame's plane, one output per arm.

use std::f64::consts::TAU;
use std::sync::Arc;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use super::plummer::Contribution;
use super::shape::{self, Field, Local, Placed, ShapeKind, Unit, disc_area, require, resolve, simpson, smoothstep, taper};
use crate::compiled::{Compiled, Shape};
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// The most arms one feature makes.
pub const MAX_ARMS: u32 = 8;

/// Arm `k` of `arms` runs `R = inner · exp((θ - θ_k) tan(pitch))` from `inner_ly` to `outer_ly`,
/// with `θ_k = phase + 2πk / arms`. Its cross-section is an ellipse of the two half sizes, with
/// a soft edge as [`super::Ellipsoid`] has, and it fades in and out over a half width at either
/// end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogSpiral {
    pub frame: FeatureId,
    pub arms: u32,
    pub pitch_deg: Num,
    pub phase_deg: Num,
    pub inner_ly: Num,
    pub outer_ly: Num,
    pub half_width_ly: Num,
    pub half_height_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct Arm {
    /// Where the arm starts, radians.
    start: f64,
    /// `tan(pitch)`.
    rise: f64,
    pitch: f64,
    inner: f64,
    outer: f64,
    tube: DVec2,
    width: f64,
}

impl Arm {
    /// The arm's distance across, measured as the radial offset to its nearest winding times
    /// `cos(pitch)`: exact on the arm's axis and a bound off it. Over every winding, not only
    /// those between `inner` and `outer`; the ends are the annulus's business.
    fn across(&self, p: DVec3) -> f64 {
        let r = p.truncate().length().max(1e-9);
        let phi = libm::atan2(p.y, p.x);
        // The winding `j` crosses this direction at `inner · exp((phi - start + 2πj) · rise)`.
        let base = phi - self.start;
        let j = ((libm::log(r / self.inner) / self.rise - base) / TAU).floor();
        let at = |j: f64| self.inner * libm::exp((base + TAU * j) * self.rise);
        (r - at(j)).abs().min((at(j + 1.0) - r).abs()) * libm::cos(self.pitch)
    }

    fn section(&self, p: DVec3) -> f64 {
        (DVec2::new(self.across(p), p.z) / self.tube).length()
    }

    fn ends(&self, r: f64) -> f64 {
        if r <= self.inner || r >= self.outer {
            return 0.0;
        }
        smoothstep((r - self.inner) / self.tube.x) * smoothstep((self.outer - r) / self.tube.x)
    }
}

impl Local for Arm {
    /// The tube's ellipse bound intersected with the annulus between the ends.
    fn distance(&self, p: DVec3) -> f64 {
        let r = p.truncate().length();
        let tube = (self.section(p) - 1.0) * self.tube.min_element();
        tube.max(self.inner - r).max(r - self.outer)
    }

    fn weight(&self, p: DVec3) -> f64 {
        let ends = self.ends(p.truncate().length());
        if ends == 0.0 { 0.0 } else { ends * taper(self.section(p), self.width) }
    }

    /// The tangential part of the gradient of `across` is `(R_j / R) sin(pitch)`, and the
    /// nearest winding is never more than twice as far out as the point.
    fn lipschitz(&self) -> f64 {
        let (s, c) = (libm::sin(self.pitch), libm::cos(self.pitch));
        (c * c + 4.0 * s * s).sqrt()
    }

    fn half_extent(&self) -> DVec3 {
        let r = self.outer + self.tube.x;
        DVec3::new(r, r, self.tube.y)
    }

    /// The cross-section times the arm's length, `dR / sin(pitch)` per step out. Ignores the
    /// curvature across the arm, which is small while the half width is small beside `R`.
    fn volume(&self) -> f64 {
        let length = simpson(|r| self.ends(r), self.inner, self.outer, 1024) / libm::sin(self.pitch);
        length * self.tube.element_product() * disc_area(self.width)
    }
}

impl FeatureKind for LogSpiral {
    const KIND: &'static str = "log_spiral";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] =
        &["frame", "arms", "pitch_deg", "phase_deg", "inner_ly", "outer_ly", "half_width_ly", "half_height_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            arms: 2,
            pitch_deg: Num::Lit(12.0),
            phase_deg: Num::Lit(0.0),
            inner_ly: Num::Lit(8.0),
            outer_ly: Num::Lit(45.0),
            half_width_ly: Num::Lit(3.0),
            half_height_ly: Num::Lit(1.5),
            edge_ly: Num::Lit(1.5),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        let nums = [&self.pitch_deg, &self.phase_deg, &self.inner_ly, &self.outer_ly, &self.half_width_ly, &self.half_height_ly, &self.edge_ly];
        shape::variables(&nums, &self.contributes)
    }

    fn summary(&self) -> String {
        format!("{} arms at {}°", self.arms, self.pitch_deg)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let (pitch, phase) = (resolve(&self.pitch_deg, out)?, resolve(&self.phase_deg, out)?);
        let (inner, outer) = (resolve(&self.inner_ly, out)?, resolve(&self.outer_ly, out)?);
        let tube = DVec2::new(resolve(&self.half_width_ly, out)?, resolve(&self.half_height_ly, out)?);
        let edge = resolve(&self.edge_ly, out)?;
        require((1..=MAX_ARMS).contains(&self.arms), &format!("needs 1 to {MAX_ARMS} arms"))?;
        require(pitch > 0.0 && pitch < 90.0, "needs a pitch between 0° and 90°")?;
        require(inner > 0.0 && outer > inner, "needs 0 < inner < outer")?;
        require(tube.min_element() > 0.0 && (0.0..=tube.min_element()).contains(&edge), "needs positive half sizes and 0 ≤ edge ≤ the smaller")?;
        // `across` sees only the nearest winding, so windings closer than the tube's width would
        // cut it short and `volume` would overcount. The gap is narrowest at `inner`.
        let gap = inner * libm::expm1(TAU * libm::tan(pitch.to_radians())) * libm::cos(pitch.to_radians());
        require(gap >= 2.0 * tube.x, "winds so tightly that an arm overlaps its own next turn")?;
        let shapes = (0..self.arms)
            .map(|k| {
                let arm = Arm {
                    start: phase.to_radians() + TAU * f64::from(k) / f64::from(self.arms),
                    rise: libm::tan(pitch.to_radians()),
                    pitch: pitch.to_radians(),
                    inner,
                    outer,
                    tube,
                    width: edge / tube.min_element(),
                };
                let placed = Placed::new(frame.origin, frame.axes, arm);
                let volume = placed.volume();
                (Arc::new(placed) as Arc<dyn Shape>, volume)
            })
            .collect();
        shape::contribute(id, out, &self.contributes, shapes)
    }
}

impl ShapeKind for LogSpiral {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Count("arms", &mut self.arms),
            Field::Num("pitch", &mut self.pitch_deg, Unit::Deg),
            Field::Num("phase", &mut self.phase_deg, Unit::Deg),
            Field::Num("inner", &mut self.inner_ly, Unit::Ly),
            Field::Num("outer", &mut self.outer_ly, Unit::Ly),
            Field::Num("half width", &mut self.half_width_ly, Unit::Ly),
            Field::Num("half height", &mut self.half_height_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}

