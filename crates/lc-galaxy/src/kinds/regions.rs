//! Shapes with a center or an orientation of their own: the ellipsoid, the ring, the bar and
//! the King cluster.
//!
//! An `edge_ly` on an ellipsoid or a ring is measured along its shortest semi-axis and scales
//! with the axis along the others, so the profile stays a function of one ellipsoidal radius.

use std::f64::consts::PI;
use std::sync::Arc;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use super::plummer::Contribution;
use super::shape::{self, Field, Local, Placed, ShapeKind, Unit, ball_volume, disc_area, ellipsoid_bound, require, resolve, simpson, taper, turned};
use crate::compiled::Compiled;
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// Uniform inside, with a soft edge: a void, a nebula's envelope, a star-forming complex.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ellipsoid {
    pub frame: FeatureId,
    pub center_ly: (f64, f64, f64),
    pub radii_ly: (f64, f64, f64),
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct Blob {
    radii: DVec3,
    /// The edge as a share of the ellipsoidal radius.
    width: f64,
}

impl Local for Blob {
    fn distance(&self, p: DVec3) -> f64 {
        ellipsoid_bound(p, self.radii)
    }

    fn weight(&self, p: DVec3) -> f64 {
        taper((p / self.radii).length(), self.width)
    }

    fn half_extent(&self) -> DVec3 {
        self.radii
    }

    fn volume(&self) -> f64 {
        self.radii.element_product() * ball_volume(self.width)
    }
}

impl FeatureKind for Ellipsoid {
    const KIND: &'static str = "ellipsoid";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "center_ly", "radii_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self { frame: FeatureId(0), center_ly: (0.0, 0.0, 0.0), radii_ly: (4.0, 3.0, 2.0), edge_ly: Num::Lit(1.0), contributes: Vec::new() }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("radii {} ly", super::fmt_v3(self.radii_ly))
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let radii = DVec3::from(self.radii_ly);
        let edge = resolve(&self.edge_ly, out)?;
        require(radii.min_element() > 0.0 && (0.0..=radii.min_element()).contains(&edge), "needs positive radii and 0 ≤ edge ≤ the shortest")?;
        let center = frame.to_galaxy(DVec3::from(self.center_ly));
        let placed = Placed::new(center, frame.axes, Blob { radii, width: edge / radii.min_element() });
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for Ellipsoid {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![Field::Vec3("center", &mut self.center_ly), Field::Vec3("radii", &mut self.radii_ly), Field::Num("edge", &mut self.edge_ly, Unit::Ly)]
    }
}

/// A torus in the frame's plane, about its origin: a resonance ring, or a gap between rings
/// with `Subtract`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ring {
    pub frame: FeatureId,
    pub radius_ly: Num,
    pub half_width_ly: Num,
    pub half_height_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct Torus {
    radius: f64,
    /// Semi-axes of the cross-section: radial, then vertical.
    tube: DVec2,
    width: f64,
}

impl Torus {
    /// The cross-section's coordinates, scaled so its edge is the unit circle.
    fn section(&self, p: DVec3) -> DVec2 {
        DVec2::new(p.truncate().length() - self.radius, p.z) / self.tube
    }
}

impl Local for Torus {
    /// The ellipse bound of [`ellipsoid_bound`] in the meridian plane, where the nearest point
    /// on the torus lies.
    fn distance(&self, p: DVec3) -> f64 {
        (self.section(p).length() - 1.0) * self.tube.min_element()
    }

    fn weight(&self, p: DVec3) -> f64 {
        taper(self.section(p).length(), self.width)
    }

    fn half_extent(&self) -> DVec3 {
        let r = self.radius + self.tube.x;
        DVec3::new(r, r, self.tube.y)
    }

    /// Pappus: the section's centroid is on the circle of `radius`, because the profile is
    /// symmetric about it.
    fn volume(&self) -> f64 {
        2.0 * PI * self.radius * self.tube.element_product() * disc_area(self.width)
    }
}

impl FeatureKind for Ring {
    const KIND: &'static str = "ring";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "radius_ly", "half_width_ly", "half_height_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            radius_ly: Num::Lit(20.0),
            half_width_ly: Num::Lit(3.0),
            half_height_ly: Num::Lit(1.0),
            edge_ly: Num::Lit(1.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.radius_ly, &self.half_width_ly, &self.half_height_ly, &self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("r = {} ly", self.radius_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let radius = resolve(&self.radius_ly, out)?;
        let tube = DVec2::new(resolve(&self.half_width_ly, out)?, resolve(&self.half_height_ly, out)?);
        let edge = resolve(&self.edge_ly, out)?;
        require(
            tube.min_element() > 0.0 && tube.x < radius && (0.0..=tube.min_element()).contains(&edge),
            "needs 0 < half width < radius, half height > 0 and 0 ≤ edge ≤ the smaller half",
        )?;
        let placed = Placed::new(frame.origin, frame.axes, Torus { radius, tube, width: edge / tube.min_element() });
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for Ring {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Num("radius", &mut self.radius_ly, Unit::Ly),
            Field::Num("half width", &mut self.half_width_ly, Unit::Ly),
            Field::Num("half height", &mut self.half_height_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}

/// A Ferrers bar, `ρ ∝ (1 - m²)²` inside the ellipsoid of its semi-axes, about the frame's
/// origin and turned `angle_deg` about its +Z. It falls smoothly to nothing at its surface, so it
/// has no edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub frame: FeatureId,
    pub half_length_ly: Num,
    pub half_width_ly: Num,
    pub half_height_ly: Num,
    pub angle_deg: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct Ferrers {
    semi: DVec3,
}

impl Local for Ferrers {
    fn distance(&self, p: DVec3) -> f64 {
        ellipsoid_bound(p, self.semi)
    }

    fn weight(&self, p: DVec3) -> f64 {
        let m2 = (p / self.semi).length_squared();
        if m2 >= 1.0 { 0.0 } else { (1.0 - m2) * (1.0 - m2) }
    }

    fn half_extent(&self) -> DVec3 {
        self.semi
    }

    /// `4πabc ∫₀¹ m² (1 - m²)² dm = 4πabc · 8/105`.
    fn volume(&self) -> f64 {
        4.0 * PI * self.semi.element_product() * 8.0 / 105.0
    }
}

impl FeatureKind for Bar {
    const KIND: &'static str = "bar";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "half_length_ly", "half_width_ly", "half_height_ly", "angle_deg", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            half_length_ly: Num::Lit(12.0),
            half_width_ly: Num::Lit(4.0),
            half_height_ly: Num::Lit(2.0),
            angle_deg: Num::Lit(25.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.half_length_ly, &self.half_width_ly, &self.half_height_ly, &self.angle_deg], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("{} × {} ly at {}°", self.half_length_ly, self.half_width_ly, self.angle_deg)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let semi = DVec3::new(resolve(&self.half_length_ly, out)?, resolve(&self.half_width_ly, out)?, resolve(&self.half_height_ly, out)?);
        require(semi.min_element() > 0.0, "needs positive semi-axes")?;
        let axes = turned(frame.axes, resolve(&self.angle_deg, out)?);
        let placed = Placed::new(frame.origin, axes, Ferrers { semi });
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for Bar {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Num("half length", &mut self.half_length_ly, Unit::Ly),
            Field::Num("half width", &mut self.half_width_ly, Unit::Ly),
            Field::Num("half height", &mut self.half_height_ly, Unit::Ly),
            Field::Num("angle", &mut self.angle_deg, Unit::Deg),
        ]
    }
}

/// King's (1962) cluster, his equation 27: the space density whose projection is his
/// surface-density law, reaching zero at the tidal radius.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct King {
    pub frame: FeatureId,
    pub center_ly: (f64, f64, f64),
    pub core_ly: Num,
    pub tidal_ly: Num,
    pub contributes: Vec<Contribution>,
}

#[derive(Debug)]
struct KingSphere {
    core: f64,
    tidal: f64,
    /// The unnormalized density at the center, so the weight peaks at 1.
    peak: f64,
}

impl KingSphere {
    fn new(core: f64, tidal: f64) -> Self {
        let mut s = Self { core, tidal, peak: 1.0 };
        s.peak = s.unnormalized(0.0);
        s
    }

    /// `(arccos x / x - √(1 - x²)) / x²` with `x² = (1 + r²/r_c²) / (1 + r_t²/r_c²)`.
    fn unnormalized(&self, r: f64) -> f64 {
        if r >= self.tidal {
            return 0.0;
        }
        let x = ((1.0 + (r / self.core).powi(2)) / (1.0 + (self.tidal / self.core).powi(2))).sqrt();
        ((libm::acos(x) / x - (1.0 - x * x).max(0.0).sqrt()) / (x * x)).max(0.0)
    }

    fn at(&self, r: f64) -> f64 {
        self.unnormalized(r) / self.peak
    }
}

impl Local for KingSphere {
    fn distance(&self, p: DVec3) -> f64 {
        p.length() - self.tidal
    }

    fn weight(&self, p: DVec3) -> f64 {
        self.at(p.length())
    }

    fn half_extent(&self) -> DVec3 {
        DVec3::splat(self.tidal)
    }

    fn volume(&self) -> f64 {
        simpson(|r| 4.0 * PI * r * r * self.at(r), 0.0, self.tidal, 2048)
    }
}

impl FeatureKind for King {
    const KIND: &'static str = "king";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "center_ly", "core_ly", "tidal_ly", "contributes"];

    fn new() -> Self {
        Self { frame: FeatureId(0), center_ly: (0.0, 0.0, 0.0), core_ly: Num::Lit(1.0), tidal_ly: Num::Lit(6.0), contributes: Vec::new() }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.core_ly, &self.tidal_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("r_c = {} ly, r_t = {} ly", self.core_ly, self.tidal_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let (core, tidal) = (resolve(&self.core_ly, out)?, resolve(&self.tidal_ly, out)?);
        require(core > 0.0 && tidal > core, "needs 0 < core < tidal radius")?;
        let center = frame.to_galaxy(DVec3::from(self.center_ly));
        let placed = Placed::new(center, frame.axes, KingSphere::new(core, tidal));
        let volume = placed.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(placed), volume)])
    }
}

impl ShapeKind for King {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![Field::Vec3("center", &mut self.center_ly), Field::Num("core", &mut self.core_ly, Unit::Ly), Field::Num("tidal radius", &mut self.tidal_ly, Unit::Ly)]
    }
}
