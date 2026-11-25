//! What a timeline compiles to: the fields the bake, the previews and the statistics read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::Arc;

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use crate::feature::{FeatureId, Output, Problem};
use crate::num::Num;

/// The packing fraction `nV` where nothing sets one: the hard-core distance is about half the
/// local mean spacing, at 1.39 candidates per surviving star. See `34-galaxy-authoring.md`.
pub const DEFAULT_PACKING: f64 = 0.5;

/// `-ln(1 - nV)` diverges at 1; this keeps the candidate rate finite.
pub const MAX_PACKING: f64 = 0.9;

/// Half the side of the coordinate box every galaxy feature must lie in, ly. See
/// `34-galaxy-authoring.md`, "The coordinate box".
pub const BOX_HALF_LY: f64 = 36_500.0;

pub fn in_box(p: DVec3) -> bool {
    p.abs().max_element() <= BOX_HALF_LY
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: DVec3::splat(f64::INFINITY), max: DVec3::splat(f64::NEG_INFINITY) };

    pub fn around(center: DVec3, half: DVec3) -> Self {
        Self { min: center - half, max: center + half }
    }

    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    pub fn union(self, other: Aabb) -> Aabb {
        Aabb { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn intersects(&self, other: &Aabb) -> bool {
        !(self.min.cmpgt(other.max).any() || other.min.cmpgt(self.max).any())
    }

    pub fn contains(&self, p: DVec3) -> bool {
        p.cmpge(self.min).all() && p.cmplt(self.max).all()
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec3 {
        self.max - self.min
    }
}

pub trait Shape: Debug + Send + Sync {
    /// Signed distance to the support in ly, negative inside. May underestimate; never
    /// overestimates by more than `lipschitz` allows, since the ray marcher steps by it.
    fn distance(&self, p: DVec3) -> f64;
    /// 1 at the profile's peak, falling to 0 at the edge of the support.
    fn weight(&self, p: DVec3) -> f64;
    /// Bound on `|∇ distance|`: 1 for an exact distance.
    fn lipschitz(&self) -> f64;
    fn bounds(&self) -> Aabb;
}

pub use lc_world::sky::galaxy::Rotation;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub origin: DVec3,
    /// Columns are the frame's axes in galaxy coordinates.
    pub axes: DMat3,
    pub rotation: Rotation,
}

impl Frame {
    pub fn to_local(&self, p: DVec3) -> DVec3 {
        self.axes.transpose() * (p - self.origin)
    }

    pub fn to_galaxy(&self, local: DVec3) -> DVec3 {
        self.origin + self.axes * local
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Imf {
    /// Kroupa (2001): slope 1.3 below 0.5 M☉, 2.3 above.
    Kroupa,
    /// Salpeter (1955): slope 2.35 throughout.
    Salpeter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Age {
    Uniform { min_gyr: f64, max_gyr: f64 },
    /// Gaussian about `gyr`, clipped to the age of the universe.
    Burst { gyr: f64, spread_gyr: f64 },
}

/// `[Fe/H] = zero_age + per_gyr·age + per_kly·R + scatter·N(0,1)`, with `R` the cylindrical
/// radius in the population's frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feh {
    pub zero_age: f64,
    pub per_gyr: f64,
    pub per_kly: f64,
    pub scatter: f64,
}

/// `σ = zero_age_km_s · (1 + age_gyr)^exponent`, per axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dispersion {
    pub zero_age_km_s: f64,
    pub exponent: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Population {
    pub frame: FeatureId,
    pub imf: Imf,
    pub age: Age,
    pub feh: Feh,
    pub dispersion: Dispersion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// Stars per ly³ of a population.
    Density(FeatureId),
    /// The packing fraction `nV` that sets the hard-core distance.
    Exclusion,
    /// An offset in dex to a population's `[Fe/H]`.
    FeH(FeatureId),
    /// One of a star's system fields, whatever its population. See
    /// `lightcone/docs/35-system-contents.md`.
    System(SystemField),
}

/// A region-level shift to the system generator, read at each star.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemField {
    /// The chance an eligible star builds an ocean world first, `[0, 1]`.
    Ocean,
    /// Disc solid mass, dex.
    Solids,
    /// Belt and Kuiper-belt survival, dex.
    Belts,
    /// Oort-cloud efficiency, dex.
    Cloud,
    /// The share of habitable worlds something lives on, `[0, 1]`.
    Life,
}

impl SystemField {
    pub const ALL: [SystemField; 5] = [SystemField::Ocean, SystemField::Solids, SystemField::Belts, SystemField::Cloud, SystemField::Life];

    /// Its value where nothing contributes.
    pub fn neutral(self) -> f64 {
        match self {
            SystemField::Life => lc_world::sky::generate::tuning::LIFE_FRACTION,
            _ => 0.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SystemField::Ocean => "ocean-first",
            SystemField::Solids => "disc solids",
            SystemField::Belts => "belts",
            SystemField::Cloud => "Oort cloud",
            SystemField::Life => "life",
        }
    }

    pub fn in_dex(self) -> bool {
        matches!(self, SystemField::Solids | SystemField::Belts | SystemField::Cloud)
    }

    pub fn range(self) -> (f64, f64) {
        let dex = lc_world::sky::generate::profile::DEX_LIMIT;
        match self {
            SystemField::Ocean | SystemField::Life => (0.0, 1.0),
            _ => (-dex, dex),
        }
    }
}

impl Target {
    pub fn population(&self) -> Option<FeatureId> {
        match self {
            Target::Density(p) | Target::FeH(p) => Some(*p),
            Target::Exclusion | Target::System(_) => None,
        }
    }
}

/// As written in a feature; [`Op`] is the same with its variables resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OpSpec {
    Add(Num),
    Scale(Num),
    Subtract,
    Set(Num),
}

impl OpSpec {
    pub fn variable(&self) -> Option<&str> {
        match self {
            OpSpec::Add(n) | OpSpec::Scale(n) | OpSpec::Set(n) => n.variable(),
            OpSpec::Subtract => None,
        }
    }

    pub fn resolve(&self, variables: &BTreeMap<String, f64>) -> Result<Op, Problem> {
        Ok(match self {
            OpSpec::Add(n) => Op::Add(n.resolve(variables)?),
            OpSpec::Scale(n) => Op::Scale(n.resolve(variables)?),
            OpSpec::Subtract => Op::Subtract,
            OpSpec::Set(n) => Op::Set(n.resolve(variables)?),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add(f64),
    Scale(f64),
    Subtract,
    Set(f64),
}

impl Op {
    pub fn apply(self, value: f64, w: f64) -> f64 {
        match self {
            Op::Add(a) => value + a * w,
            Op::Scale(c) => value * (1.0 + c * w),
            Op::Subtract => value * (1.0 - w),
            Op::Set(v) => value + (v - value) * w,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Term {
    pub source: FeatureId,
    pub target: Target,
    pub op: Op,
    pub shape: Output,
}

#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub variables: BTreeMap<String, f64>,
    pub frames: BTreeMap<FeatureId, Frame>,
    pub populations: BTreeMap<FeatureId, Population>,
    pub shapes: BTreeMap<Output, Arc<dyn Shape>>,
    /// In timeline order, which is the order they apply in: terms do not commute.
    pub terms: Vec<Term>,
    pub problems: BTreeMap<FeatureId, Problem>,
    /// Features whose problem left them showing their last good output.
    pub held: BTreeSet<FeatureId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Stars per ly³, one per population in `Compiled::populations` order.
    pub density: Vec<f64>,
    pub packing: f64,
    /// Dex, one per population in the same order.
    pub feh_offset: Vec<f64>,
    /// Indexed by `SystemField as usize`, clamped to each field's range.
    pub system: [f64; 5],
}

impl Sample {
    pub fn profile(&self) -> lc_world::sky::generate::SystemProfile {
        let v = |f: SystemField| self.system[f as usize];
        lc_world::sky::generate::SystemProfile::quantize(
            v(SystemField::Ocean),
            v(SystemField::Life),
            v(SystemField::Solids),
            v(SystemField::Belts),
            v(SystemField::Cloud),
        )
    }
}

impl Sample {
    pub fn total(&self) -> f64 {
        self.density.iter().sum()
    }
}

/// For the inspector.
#[derive(Clone, Debug)]
pub struct Contribution {
    pub source: FeatureId,
    pub target: Target,
    pub before: f64,
    pub after: f64,
}

impl Compiled {
    pub fn population_index(&self, id: FeatureId) -> Option<usize> {
        self.populations.keys().position(|k| *k == id)
    }

    pub fn sample(&self, p: DVec3) -> Sample {
        self.sample_with(p, |_, _, _| {})
    }

    fn sample_with(&self, p: DVec3, mut seen: impl FnMut(&Term, f64, f64)) -> Sample {
        let n = self.populations.len();
        let mut s = Sample {
            density: vec![0.0; n],
            packing: DEFAULT_PACKING,
            feh_offset: vec![0.0; n],
            system: SystemField::ALL.map(SystemField::neutral),
        };
        for term in &self.terms {
            let Some(shape) = self.shapes.get(&term.shape) else { continue };
            let slot = match term.target {
                Target::Density(pop) => self.population_index(pop).map(|i| &mut s.density[i]),
                Target::FeH(pop) => self.population_index(pop).map(|i| &mut s.feh_offset[i]),
                Target::Exclusion => Some(&mut s.packing),
                Target::System(f) => Some(&mut s.system[f as usize]),
            };
            let Some(slot) = slot else { continue };
            let w = shape.weight(p);
            if w == 0.0 {
                continue;
            }
            let before = *slot;
            *slot = term.op.apply(before, w);
            seen(term, before, *slot);
        }
        for d in &mut s.density {
            *d = d.max(0.0);
        }
        s.packing = s.packing.clamp(0.0, MAX_PACKING);
        for f in SystemField::ALL {
            let (lo, hi) = f.range();
            s.system[f as usize] = s.system[f as usize].clamp(lo, hi);
        }
        s
    }

    pub fn contributions(&self, p: DVec3) -> Vec<Contribution> {
        let mut out = Vec::new();
        self.sample_with(p, |t, before, after| {
            out.push(Contribution { source: t.source, target: t.target, before, after })
        });
        out
    }

    /// Terms that can raise a density from zero, whose supports bound where stars can be.
    fn raising(&self) -> impl Iterator<Item = &Term> {
        self.terms.iter().filter(|t| matches!((t.target, t.op), (Target::Density(_), Op::Add(_) | Op::Set(_))))
    }

    /// Where any population's density can be nonzero.
    pub fn bounds(&self) -> Aabb {
        self.raising()
            .filter_map(|t| self.shapes.get(&t.shape))
            .fold(Aabb::EMPTY, |b, s| b.union(s.bounds()))
    }

    /// Whether any density-raising shape's support reaches within `radius` of `p`. The bake's
    /// test for skipping a cell, exact up to each shape's Lipschitz bound.
    pub fn may_reach(&self, p: DVec3, radius: f64) -> bool {
        self.raising()
            .filter_map(|t| self.shapes.get(&t.shape))
            .any(|s| s.distance(p) < radius * s.lipschitz())
    }
}
