//! Union, intersection and difference of earlier shapes, with a blend width in ly.
//!
//! The distance combines the operands' by the quadratic smooth minimum (Quílez), whose gradient
//! is a convex combination of the operands', so the result keeps the largest of their Lipschitz
//! bounds. A smooth maximum can exceed the true distance near a crease, so the weight is cut to
//! where the combined distance is negative, over the blend width: the support is the smoothed
//! one, and the blend is what makes the edge soft.

use std::fmt;
use std::sync::Arc;

use glam::DVec3;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Serialize, Serializer};

use super::plummer::Contribution;
use super::shape::{self, require, resolve, smoothstep};
use crate::compiled::{Aabb, Compiled, Shape};
use crate::feature::{FeatureId, FeatureKind, Output, Problem};
use crate::num::Num;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoolOp {
    Union,
    Intersect,
    /// The first operand less every other.
    Difference,
}

/// One output of an earlier feature: a bare id for a feature's only output, or
/// `(id, index, of)` for one of several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeRef(pub Output);

impl Serialize for ShapeRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let o = self.0;
        if (o.index, o.of) == (0, 1) {
            return s.serialize_u32(o.feature.0);
        }
        let mut t = s.serialize_tuple(3)?;
        t.serialize_element(&o.feature.0)?;
        t.serialize_element(&o.index)?;
        t.serialize_element(&o.of)?;
        t.end()
    }
}

impl<'de> Deserialize<'de> for ShapeRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ShapeRef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a feature id or (id, index, of)")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ShapeRef, E> {
                let id = u32::try_from(v).map_err(|_| E::custom("feature id out of range"))?;
                Ok(ShapeRef(Output::only(FeatureId(id))))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<ShapeRef, E> {
                self.visit_u64(u64::try_from(v).map_err(|_| E::custom("negative feature id"))?)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ShapeRef, A::Error> {
                let mut next = |i| seq.next_element::<u32>()?.ok_or_else(|| de::Error::invalid_length(i, &self));
                let (feature, index, of) = (next(0)?, next(1)?, next(2)?);
                Ok(ShapeRef(Output { feature: FeatureId(feature), index, of }))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boolean {
    pub op: BoolOp,
    pub blend_ly: Num,
    pub of: Vec<ShapeRef>,
    pub contributes: Vec<Contribution>,
}

/// `min(a, b)` rounded over `k`: never above `min`, equal to it once `|a - b| ≥ k`.
fn smin(a: f64, b: f64, k: f64) -> f64 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn smax(a: f64, b: f64, k: f64) -> f64 {
    -smin(-a, -b, k)
}

#[derive(Debug)]
struct Combined {
    op: BoolOp,
    blend: f64,
    operands: Vec<Arc<dyn Shape>>,
    bounds: Aabb,
}

impl Combined {
    /// 1 inside the smoothed support by more than the blend width, 0 outside it.
    fn mask(&self, d: f64) -> f64 {
        if self.blend > 0.0 { smoothstep(-d / self.blend) } else if d < 0.0 { 1.0 } else { 0.0 }
    }
}

impl Shape for Combined {
    fn distance(&self, p: DVec3) -> f64 {
        let mut d = self.operands.iter().map(|s| s.distance(p));
        let first = d.next().unwrap_or(f64::INFINITY);
        match self.op {
            BoolOp::Union => d.fold(first, |a, b| smin(a, b, self.blend)),
            BoolOp::Intersect => d.fold(first, |a, b| smax(a, b, self.blend)),
            BoolOp::Difference => d.fold(first, |a, b| smax(a, -b, self.blend)),
        }
    }

    fn weight(&self, p: DVec3) -> f64 {
        if !self.bounds.contains(p) {
            return 0.0;
        }
        let mut w = self.operands.iter().map(|s| s.weight(p));
        let first = w.next().unwrap_or(0.0);
        match self.op {
            BoolOp::Union => w.fold(first, f64::max),
            BoolOp::Intersect | BoolOp::Difference => {
                let w = if self.op == BoolOp::Intersect { w.fold(first, f64::min) } else { first };
                if w == 0.0 { 0.0 } else { w * self.mask(self.distance(p)) }
            }
        }
    }

    fn lipschitz(&self) -> f64 {
        self.operands.iter().map(|s| s.lipschitz()).fold(0.0, f64::max)
    }

    fn bounds(&self) -> Aabb {
        self.bounds
    }
}

impl FeatureKind for Boolean {
    const KIND: &'static str = "boolean";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["op", "blend_ly", "of", "contributes"];

    fn new() -> Self {
        Self { op: BoolOp::Intersect, blend_ly: Num::Lit(1.0), of: Vec::new(), contributes: Vec::new() }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        let mut deps: Vec<FeatureId> = self.of.iter().map(|r| r.0.feature).collect();
        deps.extend(self.contributes.iter().filter_map(|c| c.0.population()));
        deps.sort();
        deps.dedup();
        deps
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.blend_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        let op = match self.op {
            BoolOp::Union => "union",
            BoolOp::Intersect => "intersection",
            BoolOp::Difference => "difference",
        };
        format!("{op} of {}", self.of.len())
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let blend = resolve(&self.blend_ly, out)?;
        require(blend >= 0.0, "needs a blend ≥ 0")?;
        require(self.of.len() >= 2, "needs at least two shapes")?;
        let operands = self
            .of
            .iter()
            .map(|r| shape::operand(out, r.0))
            .collect::<Result<Vec<_>, _>>()?;
        let bounds = match self.op {
            BoolOp::Union => operands.iter().fold(Aabb::EMPTY, |b, s| b.union(s.bounds())),
            BoolOp::Intersect => operands.iter().skip(1).fold(operands[0].bounds(), |b, s| b.intersection(s.bounds())),
            BoolOp::Difference => operands[0].bounds(),
        };
        require(!bounds.is_empty(), "the shapes do not overlap")?;
        let shape = Combined { op: self.op, blend, operands, bounds };
        let volume = shape::volume_if_counted(&shape, &self.contributes);
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(shape), volume)])
    }
}
