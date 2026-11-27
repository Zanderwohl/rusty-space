//! Noise over an earlier shape: it breaks the symmetry every analytic profile has, either by
//! varying the shape's weight or by moving the points the shape is read at.

use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::boolean::ShapeRef;
use super::plummer::Contribution;
use super::shape::{self, require, resolve};
use crate::compiled::{Aabb, Compiled, Shape};
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::noise::Fbm;
use crate::num::Num;

pub const MAX_OCTAVES: u32 = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NoiseMode {
    /// The weight times `1 - depth · (1 - n) / 2`, for noise `n` in `[-1, 1]`: unchanged where
    /// the noise peaks, down to `1 - depth` where it is lowest. Clumps without moving the edge.
    Modulate { depth: Num },
    /// The shape read at `p + reach · N(p)`, for a vector of noise `N` with each component in
    /// `[-1, 1]`. Ragged edges and arms that wander.
    Warp { reach_ly: Num },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoiseKind {
    pub of: ShapeRef,
    pub mode: NoiseMode,
    /// The size of the largest features.
    pub scale_ly: Num,
    pub octaves: u32,
    /// Mixed with the feature's id, so two noise features differ unless told otherwise.
    pub seed: u32,
    pub contributes: Vec<Contribution>,
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Modulate(f64),
    Warp(f64),
}

#[derive(Debug)]
struct Noisy {
    operand: Arc<dyn Shape>,
    fbm: Fbm,
    mode: Mode,
    bounds: Aabb,
    lipschitz: f64,
}

impl Noisy {
    fn new(operand: Arc<dyn Shape>, fbm: Fbm, mode: Mode) -> Self {
        let (bounds, lipschitz) = match mode {
            Mode::Modulate(_) => (operand.bounds(), operand.lipschitz()),
            // A component's gradient is bounded by `fbm.lipschitz()`, so the Jacobian's norm is
            // within √3 of it.
            Mode::Warp(reach) => {
                let b = operand.bounds();
                let grow = DVec3::splat(reach * 3f64.sqrt());
                (Aabb { min: b.min - grow, max: b.max + grow }, operand.lipschitz() * (1.0 + reach * 3f64.sqrt() * fbm.lipschitz()))
            }
        };
        Self { operand, fbm, mode, bounds, lipschitz }
    }

    fn read_at(&self, p: DVec3) -> DVec3 {
        match self.mode {
            Mode::Modulate(_) => p,
            Mode::Warp(reach) => p + self.fbm.vector(p) * reach,
        }
    }
}

impl Shape for Noisy {
    /// Not a distance to the warped support, but zero or less on it and changing no faster than
    /// `lipschitz`, which is what the bake's cell test and the marcher need.
    fn distance(&self, p: DVec3) -> f64 {
        self.operand.distance(self.read_at(p))
    }

    fn weight(&self, p: DVec3) -> f64 {
        if !self.bounds.contains(p) {
            return 0.0;
        }
        match self.mode {
            Mode::Modulate(depth) => {
                let w = self.operand.weight(p);
                if w == 0.0 { 0.0 } else { w * (1.0 - depth * (1.0 - self.fbm.at(p)) * 0.5) }
            }
            // No point moves farther than `reach · √3`, so where the shape is farther than
            // that, scaled by its bound, nothing lands inside it and the noise need not be read.
            Mode::Warp(reach) if self.operand.distance(p) > reach * 3f64.sqrt() * self.operand.lipschitz() => 0.0,
            Mode::Warp(_) => self.operand.weight(self.read_at(p)),
        }
    }

    fn weight_and_distance(&self, p: DVec3) -> (f64, f64) {
        let q = self.read_at(p);
        let (w, d) = self.operand.weight_and_distance(q);
        let w = match self.mode {
            _ if !self.bounds.contains(p) || w == 0.0 => 0.0,
            Mode::Modulate(depth) => w * (1.0 - depth * (1.0 - self.fbm.at(p)) * 0.5),
            Mode::Warp(_) => w,
        };
        (w, d)
    }

    fn lipschitz(&self) -> f64 {
        self.lipschitz
    }

    fn bounds(&self) -> Aabb {
        self.bounds
    }
}

impl FeatureKind for NoiseKind {
    const KIND: &'static str = "noise";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["of", "mode", "scale_ly", "octaves", "seed", "contributes"];

    fn new() -> Self {
        Self {
            of: ShapeRef(crate::Output::only(FeatureId(0))),
            mode: NoiseMode::Modulate { depth: Num::Lit(0.6) },
            scale_ly: Num::Lit(8.0),
            octaves: 4,
            seed: 0,
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        let mut deps = vec![self.of.0.feature];
        deps.extend(self.contributes.iter().filter_map(|c| c.0.population()));
        deps.sort();
        deps.dedup();
        deps
    }

    fn variables(&self) -> Vec<String> {
        let amount = match &self.mode {
            NoiseMode::Modulate { depth } => depth,
            NoiseMode::Warp { reach_ly } => reach_ly,
        };
        shape::variables(&[&self.scale_ly, amount], &self.contributes)
    }

    fn summary(&self) -> String {
        let mode = match self.mode {
            NoiseMode::Modulate { .. } => "modulates",
            NoiseMode::Warp { .. } => "warps",
        };
        format!("{mode} {} at {} ly", self.of.0.feature, self.scale_ly)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let scale = resolve(&self.scale_ly, out)?;
        require(scale > 0.0 && (1..=MAX_OCTAVES).contains(&self.octaves), &format!("needs scale > 0 and 1 to {MAX_OCTAVES} octaves"))?;
        let mode = match &self.mode {
            NoiseMode::Modulate { depth } => {
                let depth = resolve(depth, out)?;
                require((0.0..=1.0).contains(&depth), "needs 0 ≤ depth ≤ 1")?;
                Mode::Modulate(depth)
            }
            NoiseMode::Warp { reach_ly } => {
                let reach = resolve(reach_ly, out)?;
                require(reach >= 0.0, "needs reach ≥ 0")?;
                Mode::Warp(reach)
            }
        };
        let operand = shape::operand(out, self.of.0)?;
        let fbm = Fbm { scale, octaves: self.octaves, seed: (u64::from(id.0) << 32) | u64::from(self.seed) };
        let shape = Noisy::new(operand, fbm, mode);
        let volume = shape::volume_if_counted(&shape, &self.contributes);
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(shape), volume)])
    }
}
