//! A refit drawn as a pure function of its plan and the round's clock.
//!
//! Nothing is kept between frames and nothing is sent: [`Frame::at`] reads [`Plan::at`] for what
//! is finished and what is under way, and derives the rest, so a paused clock draws the same frame
//! twice. See 32 §Building, as a function of time.
//!
//! A form partway through a round need not place, since a part can hang from one the round has
//! taken apart or not built yet. Such a parent stands in at the end of the round it belongs to:
//! the start's while taking apart, the target's once moving and building.
//!
//! On the placeholders a working part is drawn solid at its volume at `t`, inside a cage at the
//! sliver's outer size that thickens as the truss goes up and thins as the scaffold comes down.
//! On the hull meshes (`crate::refit_hull`) each point reads its own bands, through [`Sweep`].
//!
//! `--demo refit` is a client fixture beside `--form`, not a `Scenario`: it needs only the
//! player's own ship. In the game a [`Refit`] follows the round the shard states, the player's in
//! `Fitted` and another craft's in its `Presence`, through [`follow`]: a round that stops being
//! stated before it is done was canceled, and is drawn by [`Frame::canceled`] from there.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use glam::{DMat3, DQuat, DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::capacity::Capacities;
use lc_world::form::place::{Pose, Side};
use lc_world::form::sdf::Piece;
use lc_world::form::{Form, Kind, Mount, Part, PartId, Placement, Primitive, SparMode};
use lc_world::refit::rounds::{Change, Phase, Plan, Round};

/// Of a step, how long each point spends in each phase. The truss's is how far the sweep's front
/// leads the plating.
const TRUSS: f64 = 0.30;
const PLATING: f64 = 0.15;
const FITTING: f64 = 0.15;
const SCAFFOLD: f64 = 0.10;
/// The rest of the step is the front's travel from the joint to the far edge.
const TRAVEL: f64 = 1.0 - (TRUSS + PLATING + FITTING + SCAFFOLD);

/// How high a moved subtree arcs off the straight line, in lengths of that line.
const ARC: f64 = 0.25;
/// Points [`Working::mean`] averages over.
const MEAN_SAMPLES: usize = 32;
/// Directions a sliver's far edge is sought along.
const SPAN_SAMPLES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Truss,
    Plating,
    FittingOut,
    ScaffoldDown,
}

/// How much of each layer stands at a point, each from 0 to 1. The scaffold is the outer truss,
/// up with the lattice and down last.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub truss: f64,
    pub plating: f64,
    pub fitted: f64,
    pub scaffold: f64,
}

impl Look {
    /// A build, `d` of the way from the joint to the far edge and `fraction` through the step.
    fn built(d: f64, fraction: f64) -> (Look, Option<Band>) {
        let u = fraction - d.clamp(0.0, 1.0) * TRAVEL;
        let ramp = |from: f64, width: f64| ((u - from) / width).clamp(0.0, 1.0);
        let plated_at = TRUSS;
        let fitted_at = plated_at + PLATING;
        let down_at = fitted_at + FITTING;
        // `TRAVEL` is one less the bands, which rounds, and the far edge must finish exactly.
        if u >= down_at + SCAFFOLD - 1e-12 {
            return (Look::FINISHED, None);
        }
        let truss = ramp(0.0, TRUSS);
        let look = Look {
            truss,
            plating: ramp(plated_at, PLATING),
            fitted: ramp(fitted_at, FITTING),
            scaffold: truss * (1.0 - ramp(down_at, SCAFFOLD)),
        };
        let band = match u {
            u if u <= 0.0 => None,
            u if u < plated_at => Some(Band::Truss),
            u if u < fitted_at => Some(Band::Plating),
            u if u < down_at => Some(Band::FittingOut),
            _ => Some(Band::ScaffoldDown),
        };
        (look, band)
    }

    pub const FINISHED: Look = Look { truss: 1.0, plating: 1.0, fitted: 1.0, scaffold: 0.0 };
}

/// [`Working::look`] for one copy in meters, for a shader: at `x` meters from the joint a band's
/// share is `(front - x) / width`, clamped to 0..1, with `x` no farther than the span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    pub joint: DVec3,
    pub span_m: f64,
    /// Truss, plating, fitting-out and scaffold down, in that order.
    pub fronts_m: [f64; 4],
    pub widths_m: [f64; 4],
}

impl Sweep {
    pub fn look(&self, x_m: f64) -> Look {
        let x = x_m.clamp(0.0, self.span_m);
        // As `Look::built`: the far edge finishes exactly with the step.
        if self.fronts_m[3] - x >= self.widths_m[3] * (1.0 - 1e-12 / SCAFFOLD) {
            return Look::FINISHED;
        }
        let ramp = |b: usize| ((self.fronts_m[b] - x) / self.widths_m[b]).clamp(0.0, 1.0);
        let truss = ramp(0);
        Look { truss, plating: ramp(1), fitted: ramp(2), scaffold: truss * (1.0 - ramp(3)) }
    }
}

/// The step under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Working {
    pub step: usize,
    pub part: PartId,
    pub change: Change,
    /// Through the step, 0 to 1. Falls after a cancel.
    pub fraction: f64,
    /// Each copy the step acts on at the larger of its two sizes, which bounds the sliver. For a
    /// move, each copy of the moved part where it is now.
    pub outer: Vec<Piece>,
    /// The same copies at the smaller size, in `outer`'s order, or empty when the step builds or
    /// takes apart whole parts.
    pub inner: Vec<Piece>,
    /// Where each copy meets its parent and how far its far edge is from there, meters.
    joints: Vec<(DVec3, f64)>,
    /// For a move, every copy carried along with the moved part, itself included.
    pub carried: Vec<(PartId, Side)>,
    /// For a resize, the copies hanging from the part, where they stand at its larger size. They
    /// ride out on it, so each frame's are in [`Frame::pieces`].
    pub riders: Vec<Piece>,
}

impl Working {
    /// A point `d` of the way across the sliver from the joint. A dismantling is the build played
    /// backward: scaffold up from the far edge in, plating off, truss down. What differs is the
    /// drones' traffic, not the look.
    pub fn look(&self, d: f64) -> Look {
        match self.change.phase() {
            Phase::Build => Look::built(d, self.fraction).0,
            Phase::Dismantle => Look::built(d, 1.0 - self.fraction).0,
            Phase::Move => Look::FINISHED,
        }
    }

    pub fn band(&self, d: f64) -> Option<Band> {
        match self.change.phase() {
            Phase::Build => Look::built(d, self.fraction).1,
            Phase::Dismantle => Look::built(d, 1.0 - self.fraction).1,
            Phase::Move => None,
        }
    }

    /// How far across the sliver of copy `copy` a point is, ship frame.
    pub fn across(&self, copy: usize, p: DVec3) -> f64 {
        let (joint, span) = self.joints[copy];
        (p.distance(joint) / span).clamp(0.0, 1.0)
    }

    /// Copy `copy`'s bands in meters. `None` for a move, which builds nothing.
    pub fn sweep(&self, copy: usize) -> Option<Sweep> {
        self.sweep_at(copy, self.fraction)
    }

    /// The same at `fraction` through the step: at 1, how the step leaves the copy.
    pub fn sweep_at(&self, copy: usize, fraction: f64) -> Option<Sweep> {
        let f = match self.change.phase() {
            Phase::Build => fraction,
            Phase::Dismantle => 1.0 - fraction,
            Phase::Move => return None,
        };
        let (joint, span_m) = self.joints[copy];
        // `d` of the way across is `span_m * d` meters, and the front covers the span in `TRAVEL`.
        let k = span_m / TRAVEL;
        let starts = [0.0, TRUSS, TRUSS + PLATING, TRUSS + PLATING + FITTING];
        let widths = [TRUSS, PLATING, FITTING, SCAFFOLD];
        Some(Sweep {
            joint,
            span_m,
            fronts_m: starts.map(|s| (f - s) * k),
            widths_m: widths.map(|w| w * k),
        })
    }

    /// Every copy that moves through the step as a whole: what a move carries, and what rides
    /// out on a resized part.
    pub fn moving(&self) -> impl Iterator<Item = (PartId, Side)> + '_ {
        self.carried.iter().copied().chain(self.riders.iter().map(|p| (p.part, p.side)))
    }

    /// Each layer averaged across the sliver: what a placeholder with one mesh per part draws.
    pub fn mean(&self) -> Look {
        let mut sum = Look::default();
        for i in 0..MEAN_SAMPLES {
            let l = self.look((i as f64 + 0.5) / MEAN_SAMPLES as f64);
            sum.truss += l.truss;
            sum.plating += l.plating;
            sum.fitted += l.fitted;
            sum.scaffold += l.scaffold;
        }
        let n = MEAN_SAMPLES as f64;
        Look { truss: sum.truss / n, plating: sum.plating / n, fitted: sum.fitted / n, scaffold: sum.scaffold / n }
    }
}

/// The ship at one moment of a round.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Every copy standing, at its volume then, a moved subtree partway along its path.
    pub pieces: Vec<Piece>,
    /// What stands finished for the whole of the step: the form, less the working part or with it
    /// at its smaller size, and less whatever a move carries. It may not place on its own, since
    /// a part can hang from one the round has not built yet.
    pub standing: Form,
    /// Steps finished.
    pub finished: usize,
    pub working: Option<Working>,
}

impl Frame {
    /// At coordinate seconds `now_s`.
    pub fn at(plan: &Plan, balance: &Balance, now_s: f64) -> Frame {
        let progress = plan.at(now_s);
        Frame::of(plan, balance, &progress.form, progress.finished, progress.current)
    }

    /// A round stopped at `at_s`, leaving the ship `left`, as [`Plan::cancel`] does. The step under
    /// way runs backward from where it had reached, at its own pace; a dismantling that could not
    /// be paid back finishes at once, as it does in the ledger.
    pub fn canceled(plan: &Plan, balance: &Balance, at_s: f64, left: &Form, now_s: f64) -> Frame {
        if now_s <= at_s {
            return Frame::at(plan, balance, now_s);
        }
        let progress = plan.at(at_s);
        match backward(plan, at_s, left, now_s) {
            Some(current) => Frame::of(plan, balance, &progress.form, progress.finished, Some(current)),
            None => Frame::of(plan, balance, left, progress.finished, None),
        }
    }

    fn of(plan: &Plan, balance: &Balance, before: &Form, finished: usize, current: Option<(usize, f64)>) -> Frame {
        let next = current.map_or(finished, |(i, _)| i);
        let stand = Stand {
            ends: match plan.steps().get(next).map(|s| s.change.phase()) {
                Some(Phase::Dismantle) => [&plan.round().from, plan.target()],
                _ => [plan.target(), &plan.round().from],
            },
            min_part_m3: balance.min_part_m3,
        };
        let Some((step, fraction)) = current else {
            return Frame { pieces: stand.place(before), standing: before.clone(), finished, working: None };
        };
        let s = &plan.steps()[step];
        let after = applied(before, s.part, s.after);
        let placed_before = stand.place(before);
        let placed_after = stand.place(&after);
        let keys = |pieces: &[Piece]| pieces.iter().map(|p| (p.part, p.side)).collect::<BTreeSet<_>>();

        let mut carried = Vec::new();
        let mut riders = Vec::new();
        let (pieces, outer, inner) = match s.change {
            Change::Grow | Change::Shrink => {
                let from = part(before, s.part).expect("a resize has a part to resize");
                let to = s.after.expect("a resize leaves the part");
                let volume_m3 = from.volume_m3 + fraction * (to.volume_m3 - from.volume_m3);
                let pieces = stand.place(&applied(before, s.part, Some(Part { volume_m3, ..to })));
                let of = |placed: &[Piece]| placed.iter().filter(|p| p.part == s.part).copied().collect::<Vec<_>>();
                let (big, small) = if s.change == Change::Grow { (&placed_after, &placed_before) } else { (&placed_before, &placed_after) };
                let below = subtree(before, s.part);
                riders = big.iter().filter(|p| p.part != s.part && below.contains(&p.part)).copied().collect();
                (pieces, of(big), of(small))
            }
            Change::Add => {
                let old = keys(&placed_before);
                let new: Vec<Piece> = placed_after.iter().filter(|p| !old.contains(&(p.part, p.side))).copied().collect();
                let pieces = placed_after
                    .iter()
                    .filter_map(|p| if old.contains(&(p.part, p.side)) { Some(*p) } else { sized(p, &after, fraction) })
                    .collect();
                (pieces, new, Vec::new())
            }
            Change::Remove => {
                let kept = keys(&placed_after);
                let lost: Vec<Piece> = placed_before.iter().filter(|p| !kept.contains(&(p.part, p.side))).copied().collect();
                let pieces = placed_before
                    .iter()
                    .filter_map(|p| if kept.contains(&(p.part, p.side)) { Some(*p) } else { sized(p, before, 1.0 - fraction) })
                    .collect();
                (pieces, lost, Vec::new())
            }
            Change::Move => {
                let pieces = slid(before, &placed_before, &placed_after, s.part, fraction);
                let moved = pieces.iter().filter(|p| p.part == s.part).copied().collect();
                let subtree = subtree(before, s.part);
                carried = pieces.iter().filter(|p| subtree.contains(&p.part)).map(|p| (p.part, p.side)).collect();
                (pieces, moved, Vec::new())
            }
        };
        let joints = outer.iter().map(|p| joint(p, &after, before)).collect();
        let gone: BTreeSet<PartId> = carried.iter().map(|&(id, _)| id).chain(riders.iter().map(|p| p.part)).collect();
        let end = match s.change {
            Change::Grow | Change::Add | Change::Move => before,
            Change::Shrink | Change::Remove => &after,
        };
        let standing = Form { parts: end.parts.iter().filter(|p| !gone.contains(&p.id)).copied().collect() };
        Frame {
            pieces,
            standing,
            finished,
            working: Some(Working { step, part: s.part, change: s.change, fraction, outer, inner, joints, carried, riders }),
        }
    }

    /// [`Frame::standing`]'s copies, as placed this frame.
    pub fn standing_pieces(&self) -> Vec<Piece> {
        let Some(w) = &self.working else { return self.pieces.clone() };
        let busy = |p: &Piece| p.part == w.part || w.moving().any(|k| k == (p.part, p.side));
        self.pieces.iter().filter(|p| !busy(p)).chain(&w.inner).copied().collect()
    }

    pub fn piece(&self, part: PartId, side: Side) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.part == part && p.side == side)
    }

    pub fn volume_m3(&self, part: PartId, side: Side) -> Option<f64> {
        self.piece(part, side).map(|p| p.shape.volume())
    }
}

/// The step a cancel at `at_s` is running backward at `now_s`, and how far through it is. `None`
/// once it is back, and at once when the cancel finished the step rather than undoing it.
fn backward(plan: &Plan, at_s: f64, left: &Form, now_s: f64) -> Option<(usize, f64)> {
    let progress = plan.at(at_s);
    let (i, fraction) = progress.current.filter(|_| *left == progress.form)?;
    let back = fraction - (now_s - at_s).max(0.0) / plan.steps()[i].duration_s;
    (back > 0.0).then_some((i, back))
}

/// Places a form partway through a round, standing in any parent it lacks.
struct Stand<'a> {
    /// Where a missing parent is looked for, in order.
    ends: [&'a Form; 2],
    min_part_m3: f64,
}

impl Stand<'_> {
    fn place(&self, form: &Form) -> Vec<Piece> {
        let mut parts = form.parts.clone();
        let own: BTreeSet<PartId> = parts.iter().map(|p| p.id).collect();
        // Each pass adds at least one part or stops, and there are at most two ends' worth.
        for _ in 0..=self.ends.iter().map(|f| f.parts.len()).sum::<usize>() {
            let have: BTreeSet<PartId> = parts.iter().map(|p| p.id).collect();
            let missing: BTreeSet<PartId> =
                parts.iter().filter_map(|p| p.placement.map(|pl| pl.parent)).filter(|id| !have.contains(id)).collect();
            if missing.is_empty() {
                break;
            }
            for id in missing {
                if let Some(p) = self.ends.iter().find_map(|f| part(f, id)) {
                    parts.push(p);
                }
            }
        }
        let mut form = Form { parts };
        // A mix of both ends can make a cycle, which only an edit no player would make reaches;
        // the nearer end is then drawn rather than nothing.
        let poses = match form.place(self.min_part_m3) {
            Ok(poses) => poses,
            Err(_) => {
                form = self.ends[0].clone();
                form.place(self.min_part_m3).unwrap_or_default()
            }
        };
        let by_id: BTreeMap<PartId, &Part> = form.parts.iter().map(|p| (p.id, p)).collect();
        poses
            .iter()
            .filter(|(id, _, _)| own.contains(id))
            .map(|(id, side, pose)| {
                let part = by_id[&id];
                Piece { part: id, side, kind: part.kind, shape: part.shape(self.min_part_m3), pose: *pose }
            })
            .collect()
    }
}

fn part(form: &Form, id: PartId) -> Option<Part> {
    form.parts.iter().find(|p| p.id == id).copied()
}

fn applied(form: &Form, id: PartId, after: Option<Part>) -> Form {
    let mut parts: Vec<Part> = form.parts.iter().filter(|p| p.id != id).copied().collect();
    parts.extend(after);
    Form { parts }
}

fn mount(form: &Form, id: PartId) -> Option<Mount> {
    part(form, id).and_then(|p| p.placement).map(|p| p.mount)
}

/// The copy at `share` of its volume, still standing on its foot. `None` at nothing.
fn sized(piece: &Piece, form: &Form, share: f64) -> Option<Piece> {
    let p = part(form, piece.part)?;
    let volume_m3 = share * p.volume_m3;
    if volume_m3 <= 0.0 {
        return None;
    }
    let shape = p.primitive.at(p.primitive.scale(volume_m3));
    let pose = match mount(form, piece.part) {
        Some(Mount::Attached { .. }) => {
            let foot = piece.pose.position - piece.pose.axis() * piece.shape.reach();
            Pose { position: foot + piece.pose.axis() * shape.reach(), ..piece.pose }
        }
        _ => piece.pose,
    };
    Some(Piece { shape, pose, ..*piece })
}

/// Where a copy meets its parent, and how far its far edge is from there.
fn joint(piece: &Piece, after: &Form, before: &Form) -> (DVec3, f64) {
    let attached = matches!(mount(after, piece.part).or_else(|| mount(before, piece.part)), Some(Mount::Attached { .. }));
    let joint = if attached { piece.pose.position - piece.pose.axis() * piece.shape.reach() } else { piece.pose.position };
    let span = (0..SPAN_SAMPLES)
        .map(|i| piece.pose.to_outer(piece.shape.exit(fibonacci(i, SPAN_SAMPLES)).point).distance(joint))
        .fold(0.0, f64::max);
    (joint, span.max(f64::MIN_POSITIVE))
}

/// The `i`th of `n` directions spread evenly over the sphere.
fn fibonacci(i: usize, n: usize) -> DVec3 {
    let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
    let r = (1.0 - z * z).sqrt();
    let (s, c) = (golden * i as f64).sin_cos();
    DVec3::new(r * c, r * s, z)
}

fn inverse(p: &Pose) -> Pose {
    let r = p.rotation.transpose();
    Pose { position: -(r * p.position), rotation: r }
}

/// `root` and every part hanging from it, however deep.
fn subtree(form: &Form, root: PartId) -> BTreeSet<PartId> {
    let parent: BTreeMap<PartId, PartId> = form.parts.iter().filter_map(|p| Some((p.id, p.placement?.parent))).collect();
    let under = |mut id: PartId| {
        for _ in 0..=parent.len() {
            if id == root {
                return true;
            }
            let Some(&up) = parent.get(&id) else { return false };
            id = up;
        }
        false
    };
    form.parts.iter().map(|p| p.id).filter(|&id| under(id)).collect()
}

/// The moved part and everything hanging from it, carried rigidly along an arc from its old pose
/// to its new one. Eased, so it leaves and arrives at rest.
fn slid(before: &Form, placed_before: &[Piece], placed_after: &[Piece], moved: PartId, fraction: f64) -> Vec<Piece> {
    let subtree = subtree(before, moved);
    let carried = |id: PartId| subtree.contains(&id);
    let find = |pieces: &[Piece], side: Side| pieces.iter().find(|p| p.part == moved && p.side == side).map(|p| p.pose);
    let s = fraction * fraction * (3.0 - 2.0 * fraction);
    let path = |side: Side| -> Option<(Pose, Pose)> {
        let (p0, p1) = (find(placed_before, side)?, find(placed_after, side)?);
        let chord = p1.position - p0.position;
        let out = ((p0.position + p1.position) / 2.0).normalize_or_zero();
        let lift = out * (ARC * chord.length() * (std::f64::consts::PI * s).sin());
        let rotation = DQuat::from_mat3(&p0.rotation).slerp(DQuat::from_mat3(&p1.rotation), s);
        let now = Pose { position: p0.position + chord * s + lift, rotation: DMat3::from_quat(rotation) };
        Some((now, inverse(&p0)))
    };
    let paths = [path(Side::Original), path(Side::Mirror)];
    placed_before
        .iter()
        .map(|piece| {
            if !carried(piece.part) {
                return *piece;
            }
            // A mirror that starts below the moved part hangs from its original.
            let root = match (piece.side, &paths[1]) {
                (Side::Mirror, Some(_)) => &paths[1],
                _ => &paths[0],
            };
            match root {
                Some((now, undo)) => Piece { pose: now.then(&undo.then(&piece.pose)), ..*piece },
                None => *piece,
            }
        })
        .collect()
}

/// What `--demo refit` builds on the starting form, and the pace its clock runs at.
///
/// Not the Cluster preset, which the planner refuses from the starting form: its ids put an engine
/// where the only drone was, and reshaping the storage core would vent everything the build phase
/// needs. This stages one of each step instead: the data core taken apart, the deck moved aft,
/// the hull grown by half, which is under way at the round's midpoint, and a mirrored pair of pods
/// on spars built together.
///
/// `scale` makes every part but the Mind that many times larger, as `--form default*k` does, so a
/// GSV can be photographed mid-refit.
pub fn demo_round(balance: &Balance, scale: f64) -> Round {
    let mut from = Form::starting();
    let k3 = scale.powi(3);
    for p in from.parts.iter_mut().filter(|p| p.placement.is_some()) {
        p.volume_m3 *= k3;
    }
    let mut target = from.clone();
    target.parts.retain(|p| p.id != PartId(5));
    for p in &mut target.parts {
        match p.id.0 {
            1 => p.volume_m3 *= 1.5,
            4 => {
                if let Some(Placement { mount: Mount::Attached { anchor, .. }, .. }) = &mut p.placement {
                    *anchor = DVec3::new(-0.7, 0.0, 1.0);
                }
            }
            _ => {}
        }
    }
    let placement = |parent: u16, anchor: DVec3, standoff: f64, mirror: bool| Placement {
        parent: PartId(parent),
        mount: Mount::Attached { anchor, standoff },
        twist: 0.0,
        tilt: DVec2::ZERO,
        blend: 0.0,
        mirror,
    };
    target.parts.push(Part {
        id: PartId(6),
        kind: Kind::Spar(SparMode::Saddle),
        primitive: Primitive::Cylinder { length: 10.0 },
        volume_m3: 3.0e4 * k3,
        placement: Some(placement(1, DVec3::new(0.3, 1.0, 0.0), -0.3, true)),
    });
    target.parts.push(Part {
        id: PartId(7),
        kind: Kind::Storage,
        primitive: Primitive::Ellipsoid { axes: DVec3::new(1.4, 1.0, 1.0) },
        volume_m3: 3.0e5 * k3,
        placement: Some(placement(6, DVec3::X, -0.1, false)),
    });
    let stored_j = Capacities::of(&from, balance).storage_j;
    Round { from, target, stored_j, start_s: 0.0 }
}

/// Wall seconds `--demo refit` takes over its round when no `--rate` is given. At the design rate
/// it would take three minutes, most of them spent watching one part.
pub const DEMO_WALL_S: f64 = 40.0;

/// Where the round's clock is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clock {
    /// A fraction of the round, whatever the game's clock does.
    Frozen(f64),
    /// The game's coordinate time, round and round, starting this fraction in: a burst across a
    /// boundary starts just before it.
    Looping(f64),
    /// The game's coordinate time, once through: a round the shard is running.
    Coordinate,
}

/// The player's own ship mid-refit.
#[derive(Resource, Clone, Debug)]
pub struct Refit {
    pub plan: Plan,
    pub balance: Balance,
    pub clock: Clock,
    /// When the round was canceled, coordinate seconds, and the form the cancel left.
    pub canceled: Option<(f64, Form)>,
}

impl Refit {
    /// Coordinate seconds on the round's clock.
    pub fn now_s(&self, coordinate_s: f64) -> f64 {
        let (start, duration) = (self.plan.round().start_s, self.plan.duration_s());
        match self.clock {
            Clock::Frozen(f) => start + f.clamp(0.0, 1.0) * duration,
            Clock::Looping(from) if duration > 0.0 => start + (coordinate_s - start + from * duration).rem_euclid(duration),
            Clock::Looping(_) => start,
            Clock::Coordinate => coordinate_s,
        }
    }

    pub fn frame(&self, coordinate_s: f64) -> Frame {
        let now_s = self.now_s(coordinate_s);
        match &self.canceled {
            Some((at_s, left)) => Frame::canceled(&self.plan, &self.balance, *at_s, left, now_s),
            None => Frame::at(&self.plan, &self.balance, now_s),
        }
    }

    /// Whether nothing is left to draw at `coordinate_s`: the round done, or a cancel run back.
    /// A fixture's round is never over.
    pub fn is_over(&self, coordinate_s: f64) -> bool {
        if self.clock != Clock::Coordinate {
            return false;
        }
        match &self.canceled {
            Some((at_s, left)) => coordinate_s > *at_s && backward(&self.plan, *at_s, left, coordinate_s).is_none(),
            None => self.plan.is_done(coordinate_s),
        }
    }
}

/// Take one statement of a round into `building`: `running` is the round stated, if any, `left`
/// the form stated with it, and `stated_s` when. A new recipe starts a new [`Refit`]; a round no
/// longer stated before it is done was canceled then. Stating the same thing twice changes
/// nothing, and whether anything changed is returned.
pub fn follow(building: &mut Option<Refit>, running: Option<&Plan>, left: &Form, stated_s: f64, balance: &Balance) -> bool {
    match (running, building.as_mut()) {
        (Some(plan), Some(b)) if b.canceled.is_none() && b.plan.round() == plan.round() => false,
        (Some(plan), _) => {
            *building = Some(Refit { plan: plan.clone(), balance: *balance, clock: Clock::Coordinate, canceled: None });
            true
        }
        (None, Some(b)) if b.canceled.is_none() && !b.plan.is_done(stated_s) => {
            b.canceled = Some((stated_s, left.clone()));
            true
        }
        (None, _) => false,
    }
}

/// Follow the player's round from `Fitted`, on the game's clock, and stand it down once it is
/// over. `--demo refit` holds its own.
pub fn adopt_round(
    mut commands: Commands,
    dev: Res<crate::dev::DevEntry>,
    game: Res<crate::app::Game>,
    refit: Option<Res<Refit>>,
    mut building: Local<Option<Refit>>,
) {
    if dev.refit.is_some() {
        return;
    }
    let session = &game.0;
    let changed = match session.ship.fitting() {
        Some(f) => follow(&mut building, f.refit(), f.form(), f.since_s(), f.balance()),
        None => building.take().is_some(),
    };
    match building.as_ref().filter(|b| !b.is_over(session.coordinate_time_s())) {
        Some(b) if changed || refit.is_none() => commands.insert_resource(b.clone()),
        Some(_) => {}
        None if refit.is_some() => commands.remove_resource::<Refit>(),
        None => {}
    }
}

/// Other craft's rounds, each followed from its `Presence` and drawn as its light left it.
#[derive(Resource, Default)]
pub struct Sighted(std::collections::HashMap<lc_proto::ShipId, Watched>);

#[derive(Default)]
struct Watched {
    stated: Option<lc_proto::Round>,
    building: Option<Refit>,
}

impl Sighted {
    /// Take this frame's contacts. A round is solved only when its recipe changes, and under
    /// `balance`, the shard's: a client knows no other.
    pub fn watch(&mut self, contacts: &[crate::uplink::Contact], balance: &Balance) {
        self.0.retain(|id, _| contacts.iter().any(|c| c.ship_id == *id));
        for contact in contacts {
            let watched = self.0.entry(contact.ship_id).or_default();
            if watched.stated == contact.refit {
                continue;
            }
            watched.stated = contact.refit.clone();
            let round = contact.refit.as_ref().map(Round::from);
            let reused = watched.building.as_ref().filter(|b| Some(b.plan.round()) == round.as_ref()).map(|b| b.plan.clone());
            let plan = reused.or_else(|| round.and_then(|r| r.solve(balance).ok()));
            let left = Form::from(&contact.form);
            follow(&mut watched.building, plan.as_ref(), &left, contact.emitted_s, balance);
        }
    }

    /// `contact` mid-round as its light left it, at [`crate::uplink::Contact::emitted_s`] and never
    /// at now. `None` when it is not refitting then.
    pub fn frame(&self, contact: &crate::uplink::Contact) -> Option<Frame> {
        let building = self.0.get(&contact.ship_id)?.building.as_ref()?;
        (!building.is_over(contact.emitted_s)).then(|| building.frame(contact.emitted_s))
    }
}

/// Follow every contact's round. Nothing draws another craft's form until R10, so this is kept
/// for it, and for R15 to draw.
pub fn watch_contacts(uplink: Res<crate::uplink::Uplink>, mut sighted: ResMut<Sighted>) {
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    sighted.watch(&uplink.contacts, &balance);
}

/// Stage `--demo refit` on the player's ship, with no server involved.
pub fn adopt_demo(
    mut commands: Commands,
    mut dev: ResMut<crate::dev::DevEntry>,
    mut ui: ResMut<crate::app::Ui>,
    mut own: ResMut<crate::parts::OwnForm>,
) {
    let Some(clock) = dev.refit else { return };
    let balance = Balance::DEFAULT;
    let scale = dev.form.as_deref().and_then(crate::parts::fixture_scale).unwrap_or(1.0);
    let round = demo_round(&balance, scale);
    let plan = match round.solve(&balance) {
        Ok(plan) => plan,
        Err(e) => {
            ui.notify(format!("the demo refit is refused: {e:?}"), 0.0);
            return;
        }
    };
    match crate::parts::OwnForm::spanning(&[&round.from, &round.target], &balance) {
        Ok(formed) => *own = formed,
        Err(e) => {
            ui.notify(format!("the demo refit does not place: {e}"), 0.0);
            return;
        }
    }
    // Where each step falls, for `--refit-at`.
    for (i, s) in plan.steps().iter().enumerate() {
        let of = |t: f64| t / plan.duration_s();
        info!("refit step {i}: {:?} of {:?}, {:.3} to {:.3} of the round", s.change, s.part, of(s.begins_s), of(s.ends_s()));
    }
    if matches!(clock, Clock::Looping(_)) && !dev.rate_given {
        dev.actions.push(crate::action::Action::SetTimeRate(plan.duration_s() / (DEMO_WALL_S * crate::session::TIME_RATE)));
    }
    commands.insert_resource(Refit { plan, balance, clock, canceled: None });
    // A photograph waits for the meshes from the start, not from when they are first asked for.
    commands.insert_resource(crate::refit_hull::Unready(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn plan() -> Plan {
        demo_round(&B, 1.0).solve(&B).expect("the demo round solves")
    }

    fn start(plan: &Plan) -> f64 {
        plan.round().start_s
    }

    /// Steps of every kind, so a test over all of them covers each branch.
    #[test]
    fn the_demo_round_has_one_of_each_step() {
        let plan = plan();
        let changes: BTreeSet<_> = plan.steps().iter().map(|s| format!("{:?}", s.change)).collect();
        for want in ["Remove", "Move", "Grow", "Add"] {
            assert!(changes.contains(want), "no {want} in {changes:?}");
        }
        assert!(plan.steps().iter().all(|s| s.duration_s > 0.0), "{:?}", plan.steps());
    }

    /// At every boundary the frame is the plan's finished form, piece for piece and to the bit.
    #[test]
    fn at_step_boundaries_the_frame_is_what_the_plan_has_finished() {
        let plan = plan();
        let t0 = start(&plan);
        let mut boundaries = vec![0.0];
        boundaries.extend(plan.steps().iter().map(|s| s.ends_s()));
        for since in boundaries {
            let frame = Frame::at(&plan, &B, t0 + since);
            let progress = plan.at(t0 + since);
            assert!(frame.working.is_none() && progress.current.is_none(), "at {since}");
            let placed = progress.form.place(B.min_part_m3).expect("a finished form places here");
            assert_eq!(frame.pieces.len(), placed.len(), "at {since}");
            for (id, side, pose) in placed.iter() {
                let piece = frame.piece(id, side).unwrap_or_else(|| panic!("{id:?} {side:?} missing at {since}"));
                let part = progress.form.parts.iter().find(|p| p.id == id).unwrap();
                assert_eq!(piece.shape, part.shape(B.min_part_m3), "{id:?} at {since}");
                assert_eq!(piece.pose, *pose, "{id:?} at {since}");
            }
        }
    }

    /// Volumes approach the boundary's from inside the step, from either end, so nothing jumps.
    #[test]
    fn volumes_meet_the_plans_at_both_ends_of_each_step() {
        let plan = plan();
        let t0 = start(&plan);
        for s in plan.steps() {
            let eps = s.duration_s * 1e-9;
            for (inside, edge) in [(s.begins_s + eps, s.begins_s), (s.ends_s() - eps, s.ends_s())] {
                let (a, b) = (Frame::at(&plan, &B, t0 + inside), Frame::at(&plan, &B, t0 + edge));
                for piece in &b.pieces {
                    let (va, vb) = (a.volume_m3(piece.part, piece.side).unwrap_or(0.0), piece.shape.volume());
                    assert!((va - vb).abs() <= 1e-6 * vb.max(1.0), "{:?} {:?}: {va} vs {vb}", s.change, piece.part);
                }
                for piece in &a.pieces {
                    if b.piece(piece.part, piece.side).is_none() {
                        assert!(piece.shape.volume() <= 1e-6 * s.gross_j.max(1.0), "{:?} lingers", piece.part);
                    }
                }
            }
        }
    }

    fn worked(frame: &Frame, part: PartId) -> f64 {
        frame.pieces.iter().filter(|p| p.part == part).map(|p| p.shape.volume()).sum()
    }

    /// Within a build step the part only grows, and within a dismantling it only shrinks.
    #[test]
    fn volumes_are_monotone_within_a_step() {
        let plan = plan();
        let t0 = start(&plan);
        for s in plan.steps() {
            let sign = match s.change.phase() {
                Phase::Build => 1.0,
                Phase::Dismantle => -1.0,
                Phase::Move => continue,
            };
            let mut last = None;
            for i in 1..64 {
                let v = worked(&Frame::at(&plan, &B, t0 + s.begins_s + s.duration_s * i as f64 / 64.0), s.part);
                if let Some(last) = last {
                    assert!(sign * (v - last) > 0.0, "{:?} of {:?} went {last} to {v}", s.change, s.part);
                }
                last = Some(v);
            }
        }
    }

    /// Both copies of a mirrored part are built at once, at the same size.
    #[test]
    fn mirrored_copies_build_together() {
        let plan = plan();
        let s = plan.steps().iter().find(|s| s.part == PartId(6)).unwrap();
        let frame = Frame::at(&plan, &B, start(&plan) + s.begins_s + 0.4 * s.duration_s);
        let (a, b) = (frame.volume_m3(PartId(6), Side::Original).unwrap(), frame.volume_m3(PartId(6), Side::Mirror).unwrap());
        assert!(a > 0.0 && (a - b).abs() < 1e-9 * a, "{a} and {b}");
        assert_eq!(frame.working.as_ref().unwrap().outer.len(), 2);
    }

    /// What `--refit-at 0.5` photographs, so a change of balance cannot move it onto another step
    /// without saying so.
    #[test]
    fn the_demos_midpoint_is_the_hull_half_grown() {
        let refit = Refit { plan: plan(), balance: B, clock: Clock::Frozen(0.5), canceled: None };
        let working = refit.frame(0.0).working.expect("a step is under way");
        assert_eq!((working.change, working.part), (Change::Grow, PartId(1)));
        assert!((0.2..0.8).contains(&working.fraction), "{}", working.fraction);
    }

    #[test]
    fn a_frozen_clock_draws_the_same_frame_twice() {
        let refit = Refit { plan: plan(), balance: B, clock: Clock::Frozen(0.5), canceled: None };
        assert_eq!(refit.frame(10.0), refit.frame(1.0e6));
        let looping = Refit { clock: Clock::Looping(0.3), ..refit };
        assert_eq!(looping.frame(123.0), looping.frame(123.0));
    }

    /// The front sweeps out from the joint: nearer points are further on, and the far edge is
    /// done exactly as the step ends.
    #[test]
    fn the_bands_sweep_from_the_joint_and_finish_with_the_step() {
        let near = Look::built(0.0, 0.5).0;
        let far = Look::built(1.0, 0.5).0;
        assert!(near.plating > far.plating && near.truss >= far.truss, "{near:?} {far:?}");
        assert_eq!(Look::built(1.0, 1.0).0, Look::FINISHED);
        assert_eq!(Look::built(0.0, 0.0).0, Look::default());
        assert!(Look::built(1.0, 1.0 - 1e-6).1 == Some(Band::ScaffoldDown));
        let order: Vec<_> = (0..100).filter_map(|i| Look::built(0.0, i as f64 / 100.0).1).collect();
        let mut seen = order.clone();
        seen.dedup();
        assert_eq!(seen, [Band::Truss, Band::Plating, Band::FittingOut, Band::ScaffoldDown]);
    }

    /// A dismantling puts the scaffold up at the far edge first and takes the truss down last.
    #[test]
    fn a_dismantling_runs_the_phases_in_reverse() {
        let plan = plan();
        let s = plan.steps().iter().position(|s| s.change == Change::Remove).unwrap();
        let step = plan.steps()[s];
        let at = |f: f64| Frame::at(&plan, &B, start(&plan) + step.begins_s + f * step.duration_s).working.unwrap();
        let early = at(0.1);
        assert!(early.look(1.0).scaffold > early.look(0.0).scaffold, "{:?}", early.look(1.0));
        assert_eq!(early.look(0.0), Look::FINISHED);
        let late = at(0.99);
        assert!(late.look(0.0).truss < 1.0 && late.look(1.0).truss == 0.0, "{:?}", late.look(0.0));
    }

    /// A moved subtree leaves from where it was and arrives where it goes, rigidly.
    #[test]
    fn a_move_slides_from_its_old_place_to_its_new() {
        let plan = plan();
        let t0 = start(&plan);
        let s = plan.steps().iter().find(|s| s.change == Change::Move).copied().unwrap();
        let pose = |since: f64| Frame::at(&plan, &B, t0 + since).piece(s.part, Side::Original).unwrap().pose;
        let (a, b) = (pose(s.begins_s), pose(s.ends_s()));
        let (a1, b1) = (pose(s.begins_s + 1e-9 * s.duration_s), pose(s.ends_s() - 1e-9 * s.duration_s));
        assert!(a.position.distance(a1.position) < 1e-3 && b.position.distance(b1.position) < 1e-3);
        let mid = pose(s.begins_s + 0.5 * s.duration_s);
        let chord = b.position - a.position;
        assert!(chord.length() > 1.0);
        let off = (mid.position - (a.position + b.position) / 2.0).length();
        assert!(off > 0.0 && off < chord.length(), "{off}");
    }

    /// A canceled build runs backward from where it was and ends as the ledger left it.
    #[test]
    fn a_cancel_runs_the_step_backward() {
        let plan = plan();
        let t0 = start(&plan);
        let s = plan.steps().iter().find(|s| s.change == Change::Grow).copied().unwrap();
        let at_s = t0 + s.begins_s + 0.6 * s.duration_s;
        let canceled = plan.cancel(at_s, plan.at(at_s).stored_j);
        let frame = |dt: f64| Frame::canceled(&plan, &B, at_s, &canceled.form, at_s + dt * s.duration_s);
        let v = |dt: f64| worked(&frame(dt), s.part);
        assert!(v(0.1) < v(0.0) && v(0.3) < v(0.1), "{} {} {}", v(0.0), v(0.1), v(0.3));
        let back = frame(0.1).working.unwrap().fraction;
        assert!((back - 0.5).abs() < 1e-9, "{back}");
        let done = frame(0.7);
        assert!(done.working.is_none());
        let form = canceled.form.place(B.min_part_m3).unwrap();
        assert_eq!(done.pieces.len(), form.len());
    }

    fn midway(plan: &Plan, i: usize) -> f64 {
        let s = plan.steps()[i];
        start(plan) + s.begins_s + 0.5 * s.duration_s
    }

    /// The player's round is read from its fitting, and is the plan the ledger reads: the same
    /// step, the same fraction, at every moment.
    #[test]
    fn the_players_round_is_the_ledgers_plan() {
        let plan = plan();
        let mut fitting = lc_world::fitting::Fitting::full(plan.round().from.clone(), B, start(&plan));
        fitting.begin_refit(plan.clone());
        let mut building = None;
        assert!(follow(&mut building, fitting.refit(), fitting.form(), fitting.since_s(), fitting.balance()));
        assert!(!follow(&mut building, fitting.refit(), fitting.form(), fitting.since_s(), fitting.balance()), "stated twice");
        let refit = building.expect("a round is under way");
        assert_eq!(refit.clock, Clock::Coordinate);
        for i in 0..plan.steps().len() {
            let now = midway(&plan, i);
            let working = refit.frame(now).working.expect("a step is under way");
            let standing = crate::ledger::standing(fitting.refit().unwrap(), now);
            let (_, n, fraction) = standing.step.expect("the ledger has it under way");
            assert_eq!((working.step + 1, working.fraction), (n, fraction), "step {i}");
        }
        assert!(!refit.is_over(midway(&plan, 0)));
        assert!(refit.is_over(start(&plan) + plan.duration_s()), "done, nothing left over");
    }

    /// A round stated no longer, before it is done, was canceled then: the step runs backward
    /// from where it was, and is over once it is back.
    #[test]
    fn a_round_that_stops_being_stated_was_canceled() {
        let plan = plan();
        let i = plan.steps().iter().position(|s| s.change == Change::Grow).unwrap();
        let step = plan.steps()[i];
        let at_s = midway(&plan, i);
        let left = plan.cancel(at_s, plan.at(at_s).stored_j).form;
        let mut building = None;
        follow(&mut building, Some(&plan), plan.target(), start(&plan), &B);
        assert!(follow(&mut building, None, &left, at_s, &B));
        assert!(!follow(&mut building, None, &left, at_s + 1.0, &B), "a later statement moves nothing");
        let refit = building.clone().unwrap();
        let fraction = |dt: f64| refit.frame(at_s + dt * step.duration_s).working.map(|w| w.fraction);
        assert!((fraction(0.0).unwrap() - 0.5).abs() < 1e-9, "{:?}", fraction(0.0));
        assert!((fraction(0.2).unwrap() - 0.3).abs() < 1e-9, "{:?}", fraction(0.2));
        assert!(!refit.is_over(at_s + 0.4 * step.duration_s));
        assert!(refit.is_over(at_s + 0.6 * step.duration_s));
        assert_eq!(refit.frame(at_s + step.duration_s).working, None);

        // Once done, a round stated no longer simply finished.
        let mut done = None;
        follow(&mut done, Some(&plan), plan.target(), start(&plan), &B);
        assert!(!follow(&mut done, None, plan.target(), start(&plan) + plan.duration_s() + 1.0, &B));
        assert!(done.unwrap().canceled.is_none());
    }

    /// A new recipe after a cancel is a new round.
    #[test]
    fn a_new_recipe_starts_a_new_round() {
        let plan = plan();
        let mut building = None;
        follow(&mut building, Some(&plan), plan.target(), start(&plan), &B);
        follow(&mut building, None, &plan.round().from, midway(&plan, 0), &B);
        let again = Round { start_s: midway(&plan, 1), ..plan.round().clone() }.solve(&B).unwrap();
        assert!(follow(&mut building, Some(&again), &plan.round().from, again.round().start_s, &B));
        let refit = building.unwrap();
        assert!(refit.canceled.is_none() && refit.plan.round() == again.round());
    }

    fn contact(round: Option<&Round>, form: &Form, emitted_s: f64) -> crate::uplink::Contact {
        let presence = lc_proto::Presence {
            ship_id: lc_proto::ShipId(9),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            jet_power_w: 0.0,
            emitted_t: (emitted_s * 1.0e6) as i64,
            arrive_t: (emitted_s * 1.0e6) as i64 + 3_600_000_000,
            form: form.into(),
            refit: round.map(Into::into),
            glow: None,
            glare: None,
        };
        crate::uplink::Contact::seen(presence, None)
    }

    /// Another craft is drawn as its light left it: mid-step for light that left mid-step,
    /// whatever the time is here.
    #[test]
    fn another_craft_is_drawn_when_its_light_left() {
        let plan = plan();
        let i = plan.steps().iter().position(|s| s.change == Change::Grow).unwrap();
        let emitted_s = midway(&plan, i);
        let seen = contact(Some(plan.round()), &plan.at(emitted_s).form, emitted_s);
        let mut sighted = Sighted::default();
        sighted.watch(std::slice::from_ref(&seen), &B);
        let frame = sighted.frame(&seen).expect("under way when its light left");
        assert_eq!(frame, Frame::at(&plan, &B, seen.emitted_s));
        let working = frame.working.expect("a step is under way");
        assert!(working.step == i && (working.fraction - 0.5).abs() < 1e-6, "{} {}", working.step, working.fraction);

        let later = contact(Some(plan.round()), &plan.at(emitted_s).form, start(&plan) + plan.duration_s() + 1.0);
        sighted.watch(std::slice::from_ref(&later), &B);
        assert_eq!(sighted.frame(&later), None, "its light left after the round was done");
        sighted.watch(&[], &B);
        assert_eq!(sighted.frame(&seen), None, "out of sight, forgotten");
    }

    /// Another craft's cancel is seen when a statement without the round arrives, and runs back
    /// from there in its own light.
    #[test]
    fn another_crafts_cancel_runs_backward_in_its_light() {
        let plan = plan();
        let i = plan.steps().iter().position(|s| s.change == Change::Grow).unwrap();
        let step = plan.steps()[i];
        let at_s = midway(&plan, i);
        let left = plan.cancel(at_s, plan.at(at_s).stored_j).form;
        let mut sighted = Sighted::default();
        sighted.watch(&[contact(Some(plan.round()), &plan.at(at_s).form, at_s - 1.0)], &B);
        sighted.watch(&[contact(None, &left, at_s)], &B);
        let fraction = |dt: f64| sighted.frame(&contact(None, &left, at_s + dt * step.duration_s)).and_then(|f| f.working).map(|w| w.fraction);
        assert!((fraction(0.1).unwrap() - 0.4).abs() < 1e-9, "{:?}", fraction(0.1));
        assert_eq!(fraction(0.6), None, "back, and nothing left over");
    }

    /// A part hanging from one not yet built still draws, from where the target will have it.
    #[test]
    fn a_part_whose_parent_is_not_built_yet_still_places() {
        let mut round = demo_round(&B, 1.0);
        // The deck moves onto the new spar, which is built after it moves.
        let deck = round.target.parts.iter_mut().find(|p| p.id == PartId(4)).unwrap();
        deck.placement.as_mut().unwrap().parent = PartId(6);
        let plan = round.solve(&B).expect("solves");
        let spar = plan.steps().iter().find(|s| s.part == PartId(6)).unwrap();
        let frame = Frame::at(&plan, &B, spar.begins_s - 1e-6);
        assert!(frame.piece(PartId(4), Side::Original).is_some());
        assert!(frame.piece(PartId(6), Side::Original).is_none(), "a stand-in was drawn");
    }
}
