//! An earlier shape, kept only where a field passes a threshold: young stars only where the arm
//! is dense, a cluster only inside the disc's bright part.
//!
//! The field is read as the timeline stands at the mask, so a feature after it cannot change
//! what it keeps; the mask takes the terms before it with it.

use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::boolean::ShapeRef;
use super::plummer::Contribution;
use super::shape::{self, require, resolve, smoothstep};
use crate::compiled::{Aabb, Compiled, Op, Shape, Target};
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// What a mask reads.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MaskField {
    /// A population's stars per ly³, from every term before the mask. At least one of them must
    /// raise it, or the mask is a problem: moving or suppressing the rows that feed a density
    /// would otherwise empty the mask without a word.
    Density(FeatureId),
    /// Another shape's weight, 0 to 1.
    Weight(ShapeRef),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskKind {
    pub of: ShapeRef,
    pub field: MaskField,
    /// Where the field reaches this, the shape is kept.
    pub above: Num,
    /// The field rises from `above - width` to `above` across the mask's edge; 0 is a hard edge.
    pub width: Num,
    /// Keep where the field is below instead.
    pub invert: bool,
    pub contributes: Vec<Contribution>,
}

/// The field as the timeline stood at the mask.
#[derive(Debug)]
enum Field {
    Density(Vec<(Op, Arc<dyn Shape>)>),
    Weight(Arc<dyn Shape>),
}

impl Field {
    fn at(&self, p: DVec3) -> f64 {
        match self {
            Field::Density(terms) => terms
                .iter()
                // Outside its box a shape weighs nothing; the box test is far cheaper than a
                // warped or combined shape's weight.
                .filter(|(_, s)| s.bounds().contains(p))
                .fold(0.0, |n, (op, s)| {
                    let w = s.weight(p);
                    if w == 0.0 { n } else { op.apply(n, w) }
                })
                .max(0.0),
            Field::Weight(s) => s.weight(p),
        }
    }
}

#[derive(Debug)]
struct Masked {
    operand: Arc<dyn Shape>,
    field: Field,
    above: f64,
    width: f64,
    invert: bool,
}

impl Shape for Masked {
    /// The operand's: the mask only lowers its weight, so its support stays inside the
    /// operand's.
    fn distance(&self, p: DVec3) -> f64 {
        self.operand.distance(p)
    }

    fn weight(&self, p: DVec3) -> f64 {
        let w = self.operand.weight(p);
        if w == 0.0 {
            return 0.0;
        }
        let v = self.field.at(p);
        let pass = if self.width > 0.0 { smoothstep((v - (self.above - self.width)) / self.width) } else if v >= self.above { 1.0 } else { 0.0 };
        w * if self.invert { 1.0 - pass } else { pass }
    }

    fn lipschitz(&self) -> f64 {
        self.operand.lipschitz()
    }

    fn bounds(&self) -> Aabb {
        self.operand.bounds()
    }
}

impl FeatureKind for MaskKind {
    const KIND: &'static str = "mask";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["of", "field", "above", "width", "invert", "contributes"];

    fn new() -> Self {
        Self {
            of: ShapeRef(crate::Output::only(FeatureId(0))),
            field: MaskField::Density(FeatureId(0)),
            above: Num::Lit(0.1),
            width: Num::Lit(0.05),
            invert: false,
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        let mut deps = vec![self.of.0.feature];
        deps.push(match self.field {
            MaskField::Density(p) => p,
            MaskField::Weight(r) => r.0.feature,
        });
        deps.extend(self.contributes.iter().filter_map(|c| c.0.population()));
        deps.sort();
        deps.dedup();
        deps
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.above, &self.width], &self.contributes)
    }

    fn summary(&self) -> String {
        let field = match self.field {
            MaskField::Density(p) => format!("density of {p}"),
            MaskField::Weight(r) => format!("weight of {}", r.0.feature),
        };
        format!("{} where {field} {} {}", self.of.0.feature, if self.invert { "<" } else { "≥" }, self.above)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let (above, width) = (resolve(&self.above, out)?, resolve(&self.width, out)?);
        require(width >= 0.0, "needs a width ≥ 0")?;
        let operand = shape::operand(out, self.of.0)?;
        let field = match self.field {
            MaskField::Density(pop) => {
                super::population(out, pop)?;
                // Only the terms that move this population's density, as they stand now.
                let terms: Vec<(Op, Arc<dyn Shape>)> = out
                    .terms
                    .iter()
                    .filter(|t| t.target == Target::Density(pop))
                    .filter_map(|t| Some((t.op, out.shapes.get(&t.shape)?.clone())))
                    .collect();
                require(terms.iter().any(|(op, _)| matches!(op, Op::Add(_) | Op::Set(_))), &format!("no feature before this adds to {pop}'s density"))?;
                Field::Density(terms)
            }
            MaskField::Weight(r) => Field::Weight(shape::operand(out, r.0)?),
        };
        let shape = Masked { operand, field, above, width, invert: self.invert };
        let volume = shape::volume_if_counted(&shape, &self.contributes);
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(shape), volume)])
    }
}
