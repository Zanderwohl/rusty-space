use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::shape::{self, Field, ShapeKind, Unit, simpson, smoothstep};
use crate::compiled::{Aabb, Compiled, OpSpec, Shape, Target};
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// What a shape does to a field, as written: `(Density(1), Add(10))`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contribution(pub Target, pub OpSpec);

/// A Plummer sphere, `ρ ∝ (1 + r²/a²)^(-5/2)`, truncated at `radius_ly` over `edge_ly`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plummer {
    pub frame: FeatureId,
    pub center_ly: (f64, f64, f64),
    pub core_ly: Num,
    pub radius_ly: Num,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

impl FeatureKind for Plummer {
    const KIND: &'static str = "plummer";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "center_ly", "core_ly", "radius_ly", "edge_ly", "contributes"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            center_ly: (0.0, 0.0, 0.0),
            core_ly: Num::Lit(2.0),
            radius_ly: Num::Lit(3.0),
            edge_ly: Num::Lit(1.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.core_ly, &self.radius_ly, &self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("a = {} ly, r = {} ly", self.core_ly, self.radius_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?;
        let v = &out.variables;
        let (core, radius, edge) = (self.core_ly.resolve(v)?, self.radius_ly.resolve(v)?, self.edge_ly.resolve(v)?);
        if !(core > 0.0 && radius > 0.0 && (0.0..=radius).contains(&edge)) {
            return Err(Problem::Invalid("needs core > 0, radius > 0 and 0 ≤ edge ≤ radius".into()));
        }
        let (x, y, z) = self.center_ly;
        let shape = PlummerShape { center: frame.to_galaxy(DVec3::new(x, y, z)), core, radius, edge };
        let volume = shape.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(shape), volume)])
    }
}

impl ShapeKind for Plummer {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![
            Field::Vec3("center", &mut self.center_ly),
            Field::Num("core", &mut self.core_ly, Unit::Ly),
            Field::Num("radius", &mut self.radius_ly, Unit::Ly),
            Field::Num("edge", &mut self.edge_ly, Unit::Ly),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct PlummerShape {
    pub center: DVec3,
    pub core: f64,
    pub radius: f64,
    pub edge: f64,
}

impl PlummerShape {
    fn weight_at(&self, r: f64) -> f64 {
        if r >= self.radius {
            return 0.0;
        }
        let q = 1.0 + (r * r) / (self.core * self.core);
        let profile = 1.0 / (q * q * libm::sqrt(q));
        // Not `shape::taper`: its arithmetic differs in the last bit, and published Phase I and II
        // stars must not move.
        let taper = if self.edge > 0.0 { smoothstep(((self.radius - r) / self.edge).min(1.0)) } else { 1.0 };
        profile * taper
    }

    /// `∫ weight dV` in ly³: the peak density that puts one star in the cluster is `1 / volume`.
    pub fn volume(&self) -> f64 {
        simpson(|r| 4.0 * std::f64::consts::PI * r * r * self.weight_at(r), 0.0, self.radius, 512)
    }
}

impl Shape for PlummerShape {
    fn distance(&self, p: DVec3) -> f64 {
        p.distance(self.center) - self.radius
    }

    fn weight(&self, p: DVec3) -> f64 {
        self.weight_at(p.distance(self.center))
    }

    fn lipschitz(&self) -> f64 {
        1.0
    }

    fn bounds(&self) -> Aabb {
        Aabb::around(self.center, DVec3::splat(self.radius))
    }
}
