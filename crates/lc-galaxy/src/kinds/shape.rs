//! What the shape kinds share: placement in a frame, the soft edge, a shape's volume, and
//! turning its contributions into terms.

use std::fmt::Debug;
use std::sync::Arc;

use glam::{DMat3, DVec3};

use super::plummer::Contribution;
use crate::compiled::{Aabb, BOX_HALF_LY, Compiled, Op, Shape, Target, Term, in_box};
use crate::feature::{FeatureId, Output, Problem};
use crate::num::Num;

/// A shape in its own axes, before it is placed.
pub(crate) trait Local: Debug + Send + Sync + 'static {
    /// Signed distance in ly, as [`Shape::distance`].
    fn distance(&self, p: DVec3) -> f64;
    fn weight(&self, p: DVec3) -> f64;
    fn lipschitz(&self) -> f64 {
        1.0
    }
    /// Half the size of a box about the local origin that holds the support.
    fn half_extent(&self) -> DVec3;
    /// `∫ weight dV`, ly³.
    fn volume(&self) -> f64;
}

/// A [`Local`] at `origin` with `axes` as its columns. The axes are orthonormal, so distances
/// carry over unchanged.
#[derive(Debug)]
pub(crate) struct Placed<S> {
    origin: DVec3,
    axes: DMat3,
    to_local: DMat3,
    local: S,
}

impl<S: Local> Placed<S> {
    pub fn new(origin: DVec3, axes: DMat3, local: S) -> Self {
        Self { origin, axes, to_local: axes.transpose(), local }
    }

    pub fn volume(&self) -> f64 {
        self.local.volume()
    }
}

impl<S: Local> Shape for Placed<S> {
    fn distance(&self, p: DVec3) -> f64 {
        self.local.distance(self.to_local * (p - self.origin))
    }

    fn weight(&self, p: DVec3) -> f64 {
        self.local.weight(self.to_local * (p - self.origin))
    }

    fn lipschitz(&self) -> f64 {
        self.local.lipschitz()
    }

    fn bounds(&self) -> Aabb {
        let abs = DMat3::from_cols(self.axes.x_axis.abs(), self.axes.y_axis.abs(), self.axes.z_axis.abs());
        Aabb::around(self.origin, abs * self.local.half_extent())
    }
}

/// `axes` turned by `deg` about its own +Z.
pub(crate) fn turned(axes: DMat3, deg: f64) -> DMat3 {
    let (s, c) = (libm::sin(deg.to_radians()), libm::cos(deg.to_radians()));
    axes * DMat3::from_cols(DVec3::new(c, s, 0.0), DVec3::new(-s, c, 0.0), DVec3::Z)
}

pub(crate) fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1 up to `m = 1 - width`, falling by smoothstep to 0 at `m = 1`. A `width` of 0 is a hard
/// edge.
pub(crate) fn taper(m: f64, width: f64) -> f64 {
    if m >= 1.0 {
        0.0
    } else if width <= 0.0 {
        1.0
    } else {
        smoothstep((1.0 - m) / width)
    }
}

/// Simpson's rule over `steps` intervals, rounded up to even.
pub(crate) fn simpson(f: impl Fn(f64) -> f64, a: f64, b: f64, steps: usize) -> f64 {
    let n = steps.max(2).next_multiple_of(2);
    let h = (b - a) / n as f64;
    let inner: f64 = (1..n).map(|i| f(a + i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 }).sum();
    (f(a) + inner + f(b)) * h / 3.0
}

/// A lower bound on the distance to the ellipsoid with semi-axes `r`, with Lipschitz bound 1:
/// dividing by `r` shrinks lengths by at most `1 / min(r)`.
pub(crate) fn ellipsoid_bound(p: DVec3, r: DVec3) -> f64 {
    ((p / r).length() - 1.0) * r.min_element()
}

/// `∫ taper(m) · 4πm² dm` over the unit ball: an ellipsoid's volume per unit `abc`.
pub(crate) fn ball_volume(width: f64) -> f64 {
    simpson(|m| 4.0 * std::f64::consts::PI * m * m * taper(m, width), 0.0, 1.0, 256)
}

/// `∫ taper(m) · 2πm dm` over the unit disc: a tube's cross-section per unit `wh`.
pub(crate) fn disc_area(width: f64) -> f64 {
    simpson(|m| 2.0 * std::f64::consts::PI * m * taper(m, width), 0.0, 1.0, 256)
}

pub(crate) fn resolve(n: &Num, out: &Compiled) -> Result<f64, Problem> {
    n.resolve(&out.variables)
}

pub(crate) fn require(ok: bool, why: &str) -> Result<(), Problem> {
    if ok { Ok(()) } else { Err(Problem::Invalid(why.into())) }
}

pub(crate) fn depends_on(frame: FeatureId, contributes: &[Contribution]) -> Vec<FeatureId> {
    let mut deps = vec![frame];
    deps.extend(contributes.iter().filter_map(|c| c.0.population()));
    deps.sort();
    deps.dedup();
    deps
}

pub(crate) fn variables(nums: &[&Num], contributes: &[Contribution]) -> Vec<String> {
    nums.iter()
        .filter_map(|n| n.variable())
        .chain(contributes.iter().filter_map(|c| c.1.variable()))
        .map(String::from)
        .collect()
}

/// Inserts `shapes` as the outputs of feature `id` with a term per contribution on each.
///
/// **`Add` on a density counts stars** across all of the feature's outputs: the total is what a
/// designer knows, and the peak density follows from it and the volumes.
pub(crate) fn contribute(
    id: FeatureId,
    out: &mut Compiled,
    contributes: &[Contribution],
    shapes: Vec<(Arc<dyn Shape>, f64)>,
) -> Result<(), Problem> {
    for (shape, _) in &shapes {
        let b = shape.bounds();
        require(in_box(b.min) && in_box(b.max), &format!("reaches outside the ±{BOX_HALF_LY} ly box"))?;
    }
    let volume: f64 = shapes.iter().map(|(_, v)| v).sum();
    let of = shapes.len() as u32;
    let mut ops = Vec::new();
    for Contribution(target, op) in contributes {
        super::check_target(out, *target)?;
        let op = match (target, op.resolve(&out.variables)?) {
            (Target::Density(_), Op::Add(stars)) => {
                require(volume > 0.0, "has no volume to put stars in")?;
                Op::Add(stars / volume)
            }
            (_, op) => op,
        };
        ops.push((*target, op));
    }
    for (index, (shape, _)) in shapes.into_iter().enumerate() {
        let output = Output { feature: id, index: index as u32, of };
        for &(target, op) in &ops {
            out.terms.push(Term { source: id, target, op, shape: output });
        }
        out.shapes.insert(output, shape);
    }
    Ok(())
}

/// How a shape field is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Ly,
    Deg,
    Plain,
}

/// One editable field of a shape kind, for an editor that shows every kind the same way.
pub enum Field<'a> {
    Num(&'static str, &'a mut Num, Unit),
    Vec3(&'static str, &'a mut (f64, f64, f64)),
    Count(&'static str, &'a mut u32),
}

/// A kind that places a shape in a frame and contributes through it.
pub trait ShapeKind {
    fn frame_mut(&mut self) -> &mut FeatureId;
    fn contributes_mut(&mut self) -> &mut Vec<Contribution>;
    /// Everything else, in the order an editor lists it.
    fn fields(&mut self) -> Vec<Field<'_>>;
    /// The control points of a kind drawn through them, in its frame.
    fn points_mut(&mut self) -> Option<&mut Vec<super::path::PathPoint>> {
        None
    }
}
