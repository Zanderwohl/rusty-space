use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::compiled::{Aabb, Compiled, Op, OpSpec, Shape, Target, Term};
use crate::feature::{FeatureId, FeatureKind, Output, Problem};
use crate::num::Num;

/// What a shape does to a field, as written: `(Density(1), Add(10))`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contribution(pub Target, pub OpSpec);

/// A Plummer sphere, `ρ ∝ (1 + r²/a²)^(-5/2)`, truncated at `radius_ly` over `edge_ly`.
///
/// **`Add` on a density counts stars**, not stars per ly³: the cluster's total is what a
/// designer knows, and the peak density follows from it and the profile.
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
        let mut deps = vec![self.frame];
        deps.extend(self.contributes.iter().filter_map(|c| c.0.population()));
        deps.sort();
        deps.dedup();
        deps
    }

    fn variables(&self) -> Vec<String> {
        [&self.core_ly, &self.radius_ly, &self.edge_ly]
            .into_iter()
            .filter_map(Num::variable)
            .chain(self.contributes.iter().filter_map(|c| c.1.variable()))
            .map(String::from)
            .collect()
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
        let mut terms = Vec::new();
        for Contribution(target, op) in &self.contributes {
            super::check_target(out, *target)?;
            let op = match (target, op.resolve(v)?) {
                (Target::Density(_), Op::Add(stars)) => Op::Add(stars / shape.volume()),
                (_, op) => op,
            };
            terms.push(Term { source: id, target: *target, op, shape: Output::only(id) });
        }
        out.shapes.insert(Output::only(id), Arc::new(shape));
        out.terms.extend(terms);
        Ok(())
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
        let taper = if self.edge > 0.0 { smoothstep(((self.radius - r) / self.edge).min(1.0)) } else { 1.0 };
        profile * taper
    }

    /// `∫ weight dV` in ly³: the peak density that puts one star in the cluster is `1 / volume`.
    pub fn volume(&self) -> f64 {
        // Simpson's rule on a smooth integrand that vanishes at both ends.
        const STEPS: usize = 512;
        let h = self.radius / STEPS as f64;
        let f = |r: f64| 4.0 * std::f64::consts::PI * r * r * self.weight_at(r);
        let inner: f64 = (1..STEPS).map(|i| f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 }).sum();
        (f(0.0) + inner + f(self.radius)) * h / 3.0
    }
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
