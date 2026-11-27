//! A galaxy's smooth components, centered on their frame with its +Z as their axis: the
//! exponential disc, the Sérsic bulge and the power-law halo. Each is truncated at
//! `radius_ly` over `edge_ly` so its support is finite.

use std::f64::consts::PI;
use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::plummer::Contribution;
use super::shape::{self, Field, Local, Placed, ShapeKind, Unit, ellipsoid_bound, require, resolve, simpson, taper};
use crate::compiled::Compiled;
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// Scale heights from the midplane to where an exponential disc's support ends: `e^-7` is
/// about a thousandth of the midplane density.
const DISC_CUT_SCALE_HEIGHTS: f64 = 7.0;

/// `ρ ∝ exp(-R/h_R) · exp(-|z|/h_z)`, the double exponential fitted to the Milky Way's discs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExponentialDisc {
    pub frame: FeatureId,
    pub scale_length_ly: Num,
    pub scale_height_ly: Num,
    pub radius_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct Disc {
    scale_length: f64,
    scale_height: f64,
    radius: f64,
    edge: f64,
}

impl Disc {
    fn height(&self) -> f64 {
        self.scale_height * DISC_CUT_SCALE_HEIGHTS
    }

    fn radial(&self, r: f64) -> f64 {
        libm::exp(-r / self.scale_length) * taper(r / self.radius, self.edge / self.radius)
    }

    fn vertical(&self, z: f64) -> f64 {
        libm::exp(-z.abs() / self.scale_height) * taper(z.abs() / self.height(), 1.0 / DISC_CUT_SCALE_HEIGHTS)
    }
}

impl Local for Disc {
    /// The exact distance to the cylinder that holds the support.
    fn distance(&self, p: DVec3) -> f64 {
        let (dr, dz) = (p.truncate().length() - self.radius, p.z.abs() - self.height());
        let outside = (dr.max(0.0).powi(2) + dz.max(0.0).powi(2)).sqrt();
        outside + dr.max(dz).min(0.0)
    }

    fn weight(&self, p: DVec3) -> f64 {
        self.radial(p.truncate().length()) * self.vertical(p.z)
    }

    fn half_extent(&self) -> DVec3 {
        DVec3::new(self.radius, self.radius, self.height())
    }

    fn volume(&self) -> f64 {
        let radial = simpson(|r| 2.0 * PI * r * self.radial(r), 0.0, self.radius, 1024);
        2.0 * radial * simpson(|z| self.vertical(z), 0.0, self.height(), 512)
    }
}

impl FeatureKind for ExponentialDisc {
    const KIND: &'static str = "exponential_disc";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "scale_length_ly", "scale_height_ly", "radius_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            scale_length_ly: Num::Lit(15.0),
            scale_height_ly: Num::Lit(1.5),
            radius_ly: Num::Lit(50.0),
            edge_ly: Num::Lit(5.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.scale_length_ly, &self.scale_height_ly, &self.radius_ly, &self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("h_R = {} ly, h_z = {} ly", self.scale_length_ly, self.scale_height_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let local = Disc {
            scale_length: resolve(&self.scale_length_ly, out)?,
            scale_height: resolve(&self.scale_height_ly, out)?,
            radius: resolve(&self.radius_ly, out)?,
            edge: resolve(&self.edge_ly, out)?,
        };
        require(
            local.scale_length > 0.0 && local.scale_height > 0.0 && local.radius > 0.0 && (0.0..=local.radius).contains(&local.edge),
            "needs positive scale lengths and radius, and 0 ≤ edge ≤ radius",
        )?;
        let placed = Placed::new(frame.origin, frame.axes, local);
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for ExponentialDisc {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Num("scale length", &mut self.scale_length_ly, Unit::Ly),
            Field::Num("scale height", &mut self.scale_height_ly, Unit::Ly),
            Field::Num("radius", &mut self.radius_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}

/// A spheroid whose density depends on `m² = R² + (z/q)²`: the profile, its flattening `q` and
/// its truncation. Shared by the bulge and the halo.
struct Spheroid<F> {
    profile: F,
    axis_ratio: f64,
    radius: f64,
    edge: f64,
}

impl<F> std::fmt::Debug for Spheroid<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spheroid").field("axis_ratio", &self.axis_ratio).field("radius", &self.radius).field("edge", &self.edge).finish()
    }
}

impl<F: Fn(f64) -> f64> Spheroid<F> {
    fn at(&self, m: f64) -> f64 {
        (self.profile)(m) * taper(m / self.radius, self.edge / self.radius)
    }

    fn semi_axes(&self) -> DVec3 {
        DVec3::new(self.radius, self.radius, self.radius * self.axis_ratio)
    }
}

impl<F: Fn(f64) -> f64 + Send + Sync + 'static> Local for Spheroid<F> {
    fn distance(&self, p: DVec3) -> f64 {
        ellipsoid_bound(p, self.semi_axes())
    }

    fn weight(&self, p: DVec3) -> f64 {
        let m = DVec3::new(p.x, p.y, p.z / self.axis_ratio).length();
        self.at(m)
    }

    fn half_extent(&self) -> DVec3 {
        self.semi_axes()
    }

    /// The shell between `m` and `m + dm` has volume `4πq m² dm`.
    fn volume(&self) -> f64 {
        simpson(|m| 4.0 * PI * self.axis_ratio * m * m * self.at(m), 0.0, self.radius, 2048)
    }
}

fn spheroid_checks(axis_ratio: f64, radius: f64, edge: f64) -> Result<(), Problem> {
    require(
        axis_ratio > 0.0 && axis_ratio <= 1.0 && radius > 0.0 && (0.0..=radius * axis_ratio).contains(&edge),
        "needs 0 < axis ratio ≤ 1, radius > 0 and 0 ≤ edge ≤ the short semi-axis",
    )
}

/// The Sérsic law in three dimensions, `ρ ∝ exp(-b_n (m/r_e)^(1/n))`, with `b_n` from Ciotti and
/// Bertin (1999). Index 4 is de Vaucouleurs' classical bulge, and 1 is exponential.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sersic {
    pub frame: FeatureId,
    pub effective_ly: Num,
    pub index: Num,
    pub axis_ratio: Num,
    pub radius_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

impl FeatureKind for Sersic {
    const KIND: &'static str = "sersic";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "effective_ly", "index", "axis_ratio", "radius_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            effective_ly: Num::Lit(3.0),
            index: Num::Lit(4.0),
            axis_ratio: Num::Lit(0.7),
            radius_ly: Num::Lit(12.0),
            edge_ly: Num::Lit(2.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.effective_ly, &self.index, &self.axis_ratio, &self.radius_ly, &self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("n = {}, r_e = {} ly", self.index, self.effective_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let (re, n) = (resolve(&self.effective_ly, out)?, resolve(&self.index, out)?);
        let (axis_ratio, radius, edge) = (resolve(&self.axis_ratio, out)?, resolve(&self.radius_ly, out)?, resolve(&self.edge_ly, out)?);
        // The b_n expansion is accurate to 1e-4 above n = 0.5.
        require(re > 0.0 && (0.5..=10.0).contains(&n), "needs r_e > 0 and 0.5 ≤ n ≤ 10")?;
        spheroid_checks(axis_ratio, radius, edge)?;
        let b = 2.0 * n - 1.0 / 3.0 + 4.0 / (405.0 * n) + 46.0 / (25515.0 * n * n);
        let profile = move |m: f64| libm::exp(-b * libm::pow(m / re, 1.0 / n));
        let placed = Placed::new(frame.origin, frame.axes, Spheroid { profile, axis_ratio, radius, edge });
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for Sersic {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Num("effective radius", &mut self.effective_ly, Unit::Ly),
            Field::Num("index", &mut self.index, Unit::Plain),
            Field::Num("axis ratio", &mut self.axis_ratio, Unit::Plain),
            Field::Num("radius", &mut self.radius_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}

/// A cored power law, `ρ ∝ (1 + (m/core)²)^(-slope/2)`: falls as `m^-slope` outside the core.
/// A stellar halo has a slope near 3.5.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PowerHalo {
    pub frame: FeatureId,
    pub core_ly: Num,
    pub slope: Num,
    pub axis_ratio: Num,
    pub radius_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

impl FeatureKind for PowerHalo {
    const KIND: &'static str = "power_halo";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "core_ly", "slope", "axis_ratio", "radius_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            core_ly: Num::Lit(5.0),
            slope: Num::Lit(3.5),
            axis_ratio: Num::Lit(0.8),
            radius_ly: Num::Lit(60.0),
            edge_ly: Num::Lit(10.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.core_ly, &self.slope, &self.axis_ratio, &self.radius_ly, &self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("slope {}, core {} ly", self.slope, self.core_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let (core, slope) = (resolve(&self.core_ly, out)?, resolve(&self.slope, out)?);
        let (axis_ratio, radius, edge) = (resolve(&self.axis_ratio, out)?, resolve(&self.radius_ly, out)?, resolve(&self.edge_ly, out)?);
        require(core > 0.0 && slope >= 0.0, "needs core > 0 and slope ≥ 0")?;
        spheroid_checks(axis_ratio, radius, edge)?;
        let profile = move |m: f64| libm::pow(1.0 + (m / core) * (m / core), -slope / 2.0);
        let placed = Placed::new(frame.origin, frame.axes, Spheroid { profile, axis_ratio, radius, edge });
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for PowerHalo {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Num("core", &mut self.core_ly, Unit::Ly),
            Field::Num("slope", &mut self.slope, Unit::Plain),
            Field::Num("axis ratio", &mut self.axis_ratio, Unit::Plain),
            Field::Num("radius", &mut self.radius_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}
