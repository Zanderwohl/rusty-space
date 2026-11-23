//! A slab or trapezoid bay as built geometry rather than a distance field: a shell open at its +x
//! face, and inside it a shadow box of flat layers. Truss ribs ring the hold every
//! [`RIB_PITCH_M`], following its walls; decks run front to back out of the two side walls every
//! [`LEVEL_M`], leaving a hall up the middle for a craft to come in by, rise or sink to a level and
//! turn aside into a berth. Each level's decks reach a share of the hold's width at that height,
//! so in a trapezoid the hall steps wider or narrower level by level, a terrace. Each berth,
//! between two ribs on one deck, holds a pad and maybe a cutout card of a primitive's outline.
//! See `lightcone/docs/32-ship-rendering.md` §The bay.
//!
//! Everything inside is the bay's own graph, an industrial steel, but for the lamps and the painted
//! occupants.
//!
//! Laid out in the bay's own frame: x out of the mouth, z its up, y across. A slab's up is whichever
//! of its y and z runs most along the ship's z; a trapezoid's is its own z, its section's axis,
//! turned over with the part. The hold's section is a trapezoid either way, a slab's with equal
//! sides. The shell's outside wears the storage's aztec plate in the part's own charts; everything
//! inside is charted on its own faces. Detail is in meters and does not change with the mesher's
//! resolution.
//!
//! Lit from inside by the hull material, which finds a fragment's berth from [`HullBay`] by the
//! same arithmetic as [`Hold`]: runway strips down the shaft's floor and ceiling, and two floods in
//! each occupied berth aimed at what is in it. The constants the shader repeats are checked against
//! `hull.wgsl` by a test.
//!
//! Square cornered: a slab's rounded `corner` is not drawn, so the shell stands proud of the
//! distance field at its edges by at most `corner · (1 − 1/√3)`.

use glam::{DMat3, DVec2, DVec3};
use lc_world::form::Kind;
use lc_world::form::place::Side;
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::Piece;
use std::collections::HashMap;

use bevy::prelude::{AssetId, Assets, Entity, Local, Mesh, Mesh3d, MeshMaterial3d, Query, ResMut};
use em_render::hull_material::{BAYS, HullBay, HullMaterial, face_attribute, region_weights};
use glam::Vec4;

use crate::charts::{ChartKind, Oriented, chart_at, chart_of, oriented};
use crate::hull_mesh::{HullBuffers, HullForm, HullSource, region};

const WALL_M: f64 = 1.5;
const RIB_PITCH_M: f64 = 12.0;
/// Along x.
const RIB_DEPTH_M: f64 = 1.2;
/// From the walls in to the rib's inner chord.
const RIB_WIDTH_M: f64 = 2.5;
const CHORD_M: f64 = 0.7;
const WEB_M: f64 = 0.35;
const LEVEL_M: f64 = 9.0;
const DECK_M: f64 = 0.8;
const RAIL_M: f64 = 0.4;
/// Each buttress stands in a rib's plane from deck to deck, its back upright and standing off the
/// wall by this share of the deck's reach: this share of the reach wide at its foot and this at its
/// head, and this thick along x, with a flange this wide down the slope it turns to the shaft. It
/// is pierced through the middle, leaving a frame this share of its foot's width all round.
const BUTTRESS_STANDOFF: f64 = 1.0 / 3.0;
const BUTTRESS_FRAME: f64 = 0.22;
const BUTTRESS_FOOT: f64 = 1.0 / 6.0;
const BUTTRESS_HEAD: f64 = 1.0 / 3.0;
const BUTTRESS_M: f64 = 0.6;
const FLANGE_M: f64 = 0.35;
const PAD_M: f64 = 0.25;
/// Each side's decks reach this share of the hold's width at their own height, so the hall is the
/// rest.
const DECK_SHARE: f64 = 0.28;
/// Past these the pitch stretches, so a bay kilometers long is not tens of thousands of ribs.
const MAX_SPANS: usize = 48;
const MAX_LEVELS: usize = 16;
const MAX_PANELS: usize = 24;
/// The share of berths left empty.
const VACANT: f32 = 0.3;
/// Each berth's two floods hang under its ceiling in from the rib on its mouth's side, either side
/// of its middle by this share of the deck's reach, and light the face its card turns to the mouth
/// from about 45 degrees.
const FLOOD_INSET_M: f64 = 0.6;
const FLOOD_SPREAD: f64 = 0.3;
const FLOOD_DROP_M: f64 = 0.3;
const FLOOD_HEAD_M: f64 = 0.5;
/// The runway strips lie down the shaft at this share of the way from its middle to the decks.
const RUNWAY_SHARE: f64 = 0.5;
const STRIP_M: f64 = 0.3;
const STRIP_RISE_M: f64 = 0.08;

/// What a vertex is, to the shader: [`HullBay`]'s fourth byte. An occupant is painted, a hue a
/// berth; a bracket's steel is a shade darker than the hold's and a rib's girders darker again, so
/// the layers stand apart.
#[derive(Clone, Copy)]
enum Lamp {
    None = 0,
    Runway = 1,
    Flood = 2,
    Occupant = 3,
    Bracket = 4,
    Girder = 5,
}
const DISC_SIDES: usize = 20;

/// Whether `piece` is meshed here rather than by the distance field: a bay that is a slab, or a
/// trapezoid wide enough at both ends to hold one.
pub fn takes(piece: &Piece) -> bool {
    piece.kind == Kind::Bay && layout(piece).is_some()
}

/// The bay's frame in the part's, and its hold.
fn layout(piece: &Piece) -> Option<(DMat3, Hold)> {
    let up = piece.pose.rotation.transpose() * DVec3::Z;
    let sign = |x: f64| if x < 0.0 { -1.0 } else { 1.0 };
    // Proper, so a bay-frame triangle keeps its winding in the part's.
    let frame = |u: DVec3| DMat3::from_cols(DVec3::X, u.cross(DVec3::X), u);
    match piece.shape {
        Shape::Slab { edges, .. } => {
            let f = frame(if up.y.abs() > up.z.abs() { DVec3::Y * sign(up.y) } else { DVec3::Z * sign(up.z) });
            let dims = DVec3::from_array([0, 1, 2].map(|k| edges.dot(f.col(k).abs())));
            Some((f, Hold::new(dims.x, dims.y, dims.y, dims.z)?))
        }
        Shape::Trapezoid { lower, upper, height, length } => {
            let turned = up.z < 0.0;
            let (bottom, top) = if turned { (upper, lower) } else { (lower, upper) };
            Some((frame(DVec3::Z * sign(up.z)), Hold::new(length, bottom, top, height)?))
        }
        _ => None,
    }
}

/// Appends `piece`, which [`takes`], to `out`. `slot` is its place in [`HullBay`]s, if it has one,
/// and lights it.
pub fn mesh(piece: &Piece, slot: Option<usize>, out: &mut HullBuffers) {
    let Some((frame, hold)) = layout(piece) else { return };
    let up = piece.pose.rotation.transpose() * DVec3::Z;
    let mut b = Builder {
        out,
        piece,
        frame,
        hull: oriented(&piece.shape, up),
        h: 1.0e-4 * piece.shape.least_dimension().max(1.0),
        bay: slot.filter(|&s| s < BAYS).map_or(0, |s| s as u8 + 1),
        lamp: Lamp::None,
    };
    b.shell(&hold);
    b.lamp = Lamp::Girder;
    b.ribs(&hold);
    b.lamp = Lamp::None;
    b.decks(&hold);
    b.runway(&hold);
    b.lamp = Lamp::Bracket;
    b.buttresses(&hold);
    b.lamp = Lamp::None;
    b.berths(&hold, seed(piece));
}

/// `piece`'s hold as the hull material reads it.
pub fn hull_bay(piece: &Piece) -> Option<HullBay> {
    let (frame, hold) = layout(piece)?;
    // The bay's axes in the ship's frame.
    let axes = piece.pose.rotation * frame;
    let row = |i: usize| {
        let a = axes.col(i);
        Vec4::new(a.x as f32, a.y as f32, a.z as f32, -a.dot(piece.pose.position) as f32)
    };
    Some(HullBay {
        to_bay: [row(0), row(1), row(2)],
        hold: Vec4::new(hold.half.x as f32, hold.back as f32, hold.low as f32, hold.tall as f32),
        grid: Vec4::new(hold.spans as f32, hold.levels as f32, hold.high as f32, seed(piece) as f32),
    })
}

/// The first [`BAYS`] bays among `pieces`, in the order [`crate::hull_mesh::mesh_pieces`] gives
/// them their slots.
pub fn hull_bays(pieces: &[Piece]) -> [HullBay; BAYS] {
    let mut out = [HullBay::default(); BAYS];
    for (slot, bay) in pieces.iter().filter(|p| takes(p)).filter_map(hull_bay).take(BAYS).enumerate() {
        out[slot] = bay;
    }
    out
}

/// Hands each hull material its bays once the mesh they were built into lands. By the mesh's id:
/// `Mesh3d` is touched every frame, and a form's pieces are worth placing only when it changes.
pub fn fit_bays(
    hulls: Query<(Entity, &Mesh3d, &HullForm, &MeshMaterial3d<HullMaterial>)>,
    materials: Option<ResMut<Assets<HullMaterial>>>,
    mut seen: Local<HashMap<Entity, AssetId<Mesh>>>,
) {
    // Absent where meshes are made without the hull material, as the mesher's own tests do.
    let Some(mut materials) = materials else { return };
    let mut now = HashMap::with_capacity(seen.len());
    for (entity, mesh, form, material) in &hulls {
        now.insert(entity, mesh.id());
        if seen.get(&entity) == Some(&mesh.id()) {
            continue;
        }
        let bays = match &form.source {
            HullSource::Form(form, balance) => {
                lc_world::form::sdf::Sdf::new(form, balance).map(|sdf| hull_bays(sdf.pieces())).unwrap_or_default()
            }
            HullSource::Pieces(pieces) => hull_bays(pieces),
        };
        if materials.get(&material.0).is_some_and(|m| m.uniforms.bays != bays)
            && let Some(mut asset) = materials.get_mut(&material.0)
        {
            asset.uniforms.bays = bays;
        }
    }
    *seen = now;
}

/// Whether berth `index` of a bay seeded `seed` holds something. `hull.wgsl`'s `bay_occupied`.
pub fn occupied(seed: u32, index: u32) -> bool {
    unit(mix_hash(seed, index)) >= VACANT
}

/// Counting up the levels and then the spans, the -y side before the +y.
fn berth_index(hold: &Hold, span: usize, level: usize, side: f64) -> u32 {
    ((span * hold.levels + level) * 2 + usize::from(side > 0.0)) as u32
}

/// `hash.wgsl`'s, bit for bit.
fn pcg(x: u32) -> u32 {
    let state = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn mix_hash(a: u32, b: u32) -> u32 {
    pcg(a ^ b.wrapping_add(0x9e37_79b9).wrapping_add(a << 6).wrapping_add(a >> 2))
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

/// The cavity and how it is divided. The shell's section is `outer`, counterclockwise in (y, z);
/// the hold runs x from `back` to the mouth at `half.x` and z within `±tall`, its walls `low` from
/// the middle at its floor and `high` at its ceiling, straight between.
struct Hold {
    half: DVec3,
    outer: [DVec2; 4],
    back: f64,
    low: f64,
    high: f64,
    tall: f64,
    spans: usize,
    levels: usize,
}

/// Narrower at either end than this, a hold has no room for a hall and decks either side.
const NARROWEST_M: f64 = 4.0;

impl Hold {
    /// A shell `length` along x and `height` tall, `bottom` wide at its floor and `top` at its roof.
    fn new(length: f64, bottom: f64, top: f64, height: f64) -> Option<Self> {
        let half = DVec3::new(length, bottom.max(top), height) / 2.0;
        let outer = [
            DVec2::new(-bottom / 2.0, -half.z),
            DVec2::new(bottom / 2.0, -half.z),
            DVec2::new(top / 2.0, half.z),
            DVec2::new(-top / 2.0, half.z),
        ];
        let wall = WALL_M.min(0.1 * length.min(height).min(bottom.min(top)));
        let inner = inset(&outer, wall)?;
        let (low, high, tall) = (inner[1].x, inner[2].x, inner[2].y);
        if 2.0 * low.min(high) < NARROWEST_M || 2.0 * tall < NARROWEST_M {
            return None;
        }
        let back = -half.x + wall;
        let run = half.x - back;
        Some(Hold {
            half,
            outer,
            back,
            low,
            high,
            tall,
            spans: ((run / RIB_PITCH_M).round() as usize).clamp(1, MAX_SPANS),
            levels: ((2.0 * tall / LEVEL_M).floor() as usize).clamp(1, MAX_LEVELS),
        })
    }

    /// The hold's own section, inside the walls, counterclockwise from its floor's -y corner.
    fn section(&self) -> [DVec2; 4] {
        let (l, h, t) = (self.low, self.high, self.tall);
        [DVec2::new(-l, -t), DVec2::new(l, -t), DVec2::new(h, t), DVec2::new(-h, t)]
    }

    /// How far each wall stands from the middle at height `z`.
    fn wall(&self, z: f64) -> f64 {
        self.low + (self.high - self.low) * (z + self.tall) / (2.0 * self.tall)
    }

    /// Half the hall's width where a deck at height `z` would leave it: [`DECK_SHARE`] of the hold's
    /// width there, from each wall.
    fn hall_at(&self, z: f64) -> f64 {
        self.wall(z) * (1.0 - 2.0 * DECK_SHARE)
    }

    fn hall(&self, k: usize) -> f64 {
        self.hall_at(self.floor(k))
    }

    /// A berth's outer edge: its wall at the floor or the ceiling, whichever leans further in, so
    /// what stands in it clears the wall.
    fn edge(&self, k: usize) -> f64 {
        self.wall(self.floor(k)).min(self.wall(self.ceiling(k)))
    }

    /// How much of a berth's width, from its edge to the hall, is room for what stands in it.
    fn room(&self, k: usize) -> f64 {
        self.edge(k) - self.hall(k)
    }

    fn middle(&self, k: usize) -> f64 {
        (self.edge(k) + self.hall(k)) / 2.0
    }

    /// Where rib `i` stands, from 0 at the back wall to `spans` at the mouth.
    fn rib(&self, i: usize) -> f64 {
        self.back + (self.half.x - self.back) * i as f64 / self.spans as f64
    }

    /// The top of the floor level `k` stands on: the hold's floor for 0.
    fn floor(&self, k: usize) -> f64 {
        -self.tall + 2.0 * self.tall * k as f64 / self.levels as f64
    }

    fn ceiling(&self, k: usize) -> f64 {
        if k + 1 == self.levels { self.tall } else { self.floor(k + 1) - DECK_M }
    }
}

enum Chart {
    /// The part's own, as the distance field's mesh would have it.
    Hull,
    /// The quad's own plane, u along its first edge.
    Own,
}

struct Builder<'a> {
    out: &'a mut HullBuffers,
    piece: &'a Piece,
    /// The bay's frame to the part's.
    frame: DMat3,
    hull: Oriented,
    h: f64,
    /// One past the bay's slot, 0 for none.
    bay: u8,
    lamp: Lamp,
}

impl Builder<'_> {
    /// `corners` counterclockwise about the face's normal, bay frame.
    fn quad(&mut self, corners: [DVec3; 4], region: u32, chart: Chart) {
        let normal = (corners[1] - corners[0]).cross(corners[3] - corners[0]).normalize_or_zero();
        if normal == DVec3::ZERO {
            return;
        }
        let u = (corners[1] - corners[0]).normalize();
        let first = self.out.positions.len() as u32;
        match chart {
            Chart::Hull => {
                let local = corners.map(|c| self.hull.turn * self.frame * c);
                let middle = (local[0] + local[2]) / 2.0;
                let index = chart_of(&self.hull.shape, middle);
                // One tangent for the quad, square to its own face. The distance field's normal
                // would not do: these corners are square and the slab's are rounded, so out at a
                // corner it leans, and a look that seeds its layout from the tangent re-seeds
                // along every line where the leaning tangent rounds differently.
                let flat = self.hull.turn * self.frame * normal;
                let du = DVec3::from_array([DVec3::X, DVec3::Y, DVec3::Z].map(|a| {
                    let at = |p: DVec3| chart_at(&self.hull.shape, index, p, self.hull.lying).uv.x;
                    at(middle + a * self.h) - at(middle - a * self.h)
                }));
                let along_turned = (du - flat * flat.dot(du)).try_normalize().unwrap_or_else(|| flat.any_orthonormal_vector());
                let along = self.piece.pose.rotation * self.hull.turn.transpose() * along_turned;
                for (c, l) in corners.into_iter().zip(local) {
                    let here = chart_at(&self.hull.shape, index, l, self.hull.lying);
                    let flag = if here.end { 1.0 } else if here.lying { 2.0 } else { 0.0 };
                    let at = [here.uv.x, here.uv.y, here.kind as u8 as f64, flag];
                    self.vertex(c, normal, region, at, here.bounds, along);
                }
            }
            Chart::Own => {
                let v = normal.cross(u);
                let uv = corners.map(|c| DVec2::new(c.dot(u), c.dot(v)));
                let (lo, hi) = uv.iter().fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                let along = self.piece.pose.rotation * self.frame * u;
                for (c, p) in corners.into_iter().zip(uv) {
                    self.vertex(c, normal, region, [p.x, p.y, ChartKind::Rect as u8 as f64, 0.0], [lo.x, hi.x, lo.y, hi.y], along);
                }
            }
        }
        self.out.indices.extend([0, 1, 2, 0, 2, 3].map(|k| first + k));
    }

    fn vertex(&mut self, at: DVec3, normal: DVec3, region: u32, chart: [f64; 4], bounds: [f64; 4], along: DVec3) {
        let pose = self.piece.pose;
        self.out.positions.push(pose.to_outer(self.frame * at).as_vec3().to_array());
        self.out.normals.push((pose.rotation * self.frame * normal).as_vec3().to_array());
        self.out.regions.push(region_weights(region, region, 0.0));
        let mut face = face_attribute(0.0, None);
        face[2] = self.bay;
        face[3] = self.lamp as u8;
        self.out.faces.push(face);
        self.out.charts.push(chart.map(|x| x as f32));
        self.out.chart_bounds.push(bounds.map(|x| x as f32));
        self.out.tangents.push(along.as_vec3().extend(1.0).to_array());
    }

    /// A rectangle about `center` spanning `±a` and `±b`, facing `a × b`.
    fn rect(&mut self, center: DVec3, a: DVec3, b: DVec3, region: u32, chart: Chart) {
        self.quad([center - a - b, center + a - b, center + a + b, center - a + b], region, chart);
    }

    /// A box about `center`, `half` along each of `axes`, which are right-handed and orthonormal.
    fn cuboid(&mut self, center: DVec3, axes: [DVec3; 3], half: DVec3, region: u32) {
        for i in 0..3 {
            let (j, k) = ((i + 1) % 3, (i + 2) % 3);
            let (a, b) = (axes[j] * half[j], axes[k] * half[k]);
            let out = axes[i] * half[i];
            self.rect(center + out, a, b, region, Chart::Own);
            self.rect(center - out, b, a, region, Chart::Own);
        }
    }

    /// A square beam `width` across from `a` to `b`, two of its faces square to `hint`.
    fn beam(&mut self, a: DVec3, b: DVec3, width: f64, hint: DVec3, region: u32) {
        let along = b - a;
        let length = along.length();
        if length < 1e-6 {
            return;
        }
        let x = along / length;
        let y = (hint - x * hint.dot(x)).try_normalize().unwrap_or_else(|| x.any_orthonormal_vector());
        self.cuboid((a + b) / 2.0, [x, y, x.cross(y)], DVec3::new(length / 2.0, width / 2.0, width / 2.0), region);
    }

    /// The outline's sides and back outside, a rim at the mouth between it and the hold, and the
    /// hold's walls and back wall inside.
    fn shell(&mut self, hold: &Hold) {
        let (outside, inside) = (region(Kind::Storage), region(Kind::Bay));
        let (back, mouth) = (hold.back, hold.half.x);
        let at = |p: DVec2, x: f64| DVec3::new(x, p.x, p.y);
        let (o, i) = (hold.outer, hold.section());
        self.quad([at(o[0], -mouth), at(o[3], -mouth), at(o[2], -mouth), at(o[1], -mouth)], outside, Chart::Hull);
        self.quad(i.map(|p| at(p, back)), inside, Chart::Own);
        for k in 0..4 {
            let (a, b) = (o[k], o[(k + 1) % 4]);
            let (c, d) = (i[k], i[(k + 1) % 4]);
            self.quad([at(a, -mouth), at(b, -mouth), at(b, mouth), at(a, mouth)], outside, Chart::Hull);
            self.quad([at(a, mouth), at(b, mouth), at(d, mouth), at(c, mouth)], outside, Chart::Hull);
            self.quad([at(d, back), at(c, back), at(c, mouth), at(d, mouth)], inside, Chart::Own);
        }
    }

    /// Each rib a ring of truss in its own plane following the walls: a chord along them, an inner
    /// chord inset from them and chamfered at the corners, and struts between, upright to the walls
    /// and diagonal in turn.
    fn ribs(&mut self, hold: &Hold) {
        let steel = region(Kind::Bay);
        let section = hold.section();
        let width = RIB_WIDTH_M.min(0.25 * hold.low.min(hold.high).min(hold.tall));
        let (Some(along), Some(ends), Some(inner)) = (inset(&section, CHORD_M / 2.0), inset(&section, CHORD_M), inset(&section, width)) else {
            return;
        };
        let shortest = (0..4).map(|k| inner[k].distance(inner[(k + 1) % 4])).fold(f64::INFINITY, f64::min);
        let chamfer = (2.0 * width).min(0.3 * shortest);
        // Each inner corner cut back along both its edges: where its chamfer starts and ends.
        let cut: Vec<(DVec2, DVec2)> = (0..4)
            .map(|k| {
                let (p, prev, next) = (inner[k], inner[(k + 3) % 4], inner[(k + 1) % 4]);
                (p + (prev - p).normalize() * chamfer, p + (next - p).normalize() * chamfer)
            })
            .collect();
        let ring: Vec<DVec2> = cut.iter().flat_map(|&(a, b)| [a, b]).collect();
        for i in 1..=hold.spans {
            let x = hold.rib(i).min(hold.half.x - RIB_DEPTH_M / 2.0);
            let at = |p: DVec2| DVec3::new(x, p.x, p.y);
            for k in 0..8 {
                self.chord(at(ring[k]), at(ring[(k + 1) % 8]));
            }
            for k in 0..4 {
                let next = (k + 1) % 4;
                self.chord(at(along[k]), at(along[next]));
                // Side k runs between corner k's chamfer and the next's, square to wall k.
                let (a, b) = (cut[k].1, cut[next].0);
                let inward = (inner[next] - inner[k]).normalize().perp();
                let wall = |p: DVec2| p - inward * (width - CHORD_M);
                let panels = ((a.distance(b) / (1.5 * width)).ceil() as usize).clamp(1, MAX_PANELS);
                for j in 0..=panels {
                    let p = a.lerp(b, j as f64 / panels as f64);
                    self.beam(at(p), at(wall(p)), WEB_M, DVec3::X, steel);
                    if j < panels {
                        let q = a.lerp(b, (j + 1) as f64 / panels as f64);
                        let (from, to) = if j % 2 == 0 { (p, wall(q)) } else { (q, wall(p)) };
                        self.beam(at(from), at(to), WEB_M, DVec3::X, steel);
                    }
                }
                let bend = (cut[next].0 + cut[next].1) / 2.0;
                self.beam(at(bend), at(ends[next]), WEB_M, DVec3::X, steel);
            }
        }
    }

    fn chord(&mut self, a: DVec3, b: DVec3) {
        let spar = region(Kind::Bay);
        // Deep along x and narrower across, so a rib reads as a plane edge on from the mouth.
        let along = b - a;
        let length = along.length();
        if length < 1e-6 {
            return;
        }
        let d = along / length;
        let across = d.cross(DVec3::X);
        let half = DVec3::new(length / 2.0 + CHORD_M / 2.0, RIB_DEPTH_M / 2.0, CHORD_M / 2.0);
        self.cuboid((a + b) / 2.0, [d, DVec3::X, across], half, spar);
    }

    /// Each level's decks from the walls in to its hall, their outer edges on the walls' slope, and
    /// a rail along every hall edge, the floor's included.
    fn decks(&mut self, hold: &Hold) {
        let steel = region(Kind::Bay);
        let length = hold.half.x - hold.back;
        let middle = hold.back + length / 2.0;
        for k in 1..hold.levels {
            let (top, hall) = (hold.floor(k), hold.hall(k));
            let under = top - DECK_M;
            for side in [-1.0, 1.0] {
                let o = |y: f64, z: f64| DVec2::new(side * y, z);
                let outline = [o(hall, top), o(hold.wall(top), top), o(hold.wall(under), under), o(hall, under)];
                self.prism(middle, length / 2.0, &outline, steel);
            }
        }
        for k in 0..hold.levels {
            let top = hold.floor(k);
            for side in [-1.0, 1.0] {
                let edge = side * (hold.hall(k) + RAIL_M / 2.0);
                let a = DVec3::new(hold.back, edge, top + RAIL_M / 2.0);
                self.beam(a, a + DVec3::X * length, RAIL_M, DVec3::Z, steel);
            }
        }
    }

    /// Two strips of lamps down the shaft on its floor and two on its ceiling, mouth to back.
    fn runway(&mut self, hold: &Hold) {
        let length = hold.half.x - hold.back;
        let axes = [DVec3::X, DVec3::Y, DVec3::Z];
        self.lamp = Lamp::Runway;
        for (z, side) in [(-hold.tall, 1.0), (hold.tall, -1.0)] {
            let y = RUNWAY_SHARE * hold.hall_at(z);
            for across in [-y, y] {
                let center = DVec3::new(hold.back + length / 2.0, across, z + side * STRIP_RISE_M / 2.0);
                self.cuboid(center, axes, DVec3::new(length / 2.0, STRIP_M / 2.0, STRIP_RISE_M / 2.0), region(Kind::Bay));
            }
        }
        self.lamp = Lamp::None;
    }

    /// A pad on every berth's floor, on most a cutout card standing on it facing the mouth, and two
    /// flood heads under its ceiling, lit only when it is occupied.
    fn berths(&mut self, hold: &Hold, seed: u32) {
        let (pad, card) = (region(Kind::Bay), region(Kind::Storage));
        let axes = [DVec3::X, DVec3::Y, DVec3::Z];
        for i in 0..hold.spans {
            let (x0, x1) = (hold.rib(i) + RIB_DEPTH_M / 2.0, hold.rib(i + 1) - RIB_DEPTH_M / 2.0);
            let x = (x0 + x1) / 2.0;
            for k in 0..hold.levels {
                let (floor, ceiling) = (hold.floor(k), hold.ceiling(k));
                for side in [-1.0, 1.0] {
                    let index = berth_index(hold, i, k, side);
                    let mut roll = Roll(splitmix(u64::from(seed) << 32 | u64::from(index)));
                    let (y, room) = (side * hold.middle(k), hold.room(k));
                    let size = 0.7 * (x1 - x0).min(room);
                    if size <= 0.0 {
                        continue;
                    }
                    self.cuboid(DVec3::new(x, y, floor + PAD_M / 2.0), axes, DVec3::new(size / 2.0, size / 2.0, PAD_M / 2.0), pad);
                    self.lamp = Lamp::Flood;
                    for spread in [-FLOOD_SPREAD, FLOOD_SPREAD] {
                        let head = DVec3::new(x1 - FLOOD_INSET_M, y + spread * room, ceiling - FLOOD_DROP_M);
                        self.cuboid(head, axes, DVec3::splat(FLOOD_HEAD_M / 2.0), pad);
                    }
                    self.lamp = Lamp::None;
                    if !occupied(seed, index) {
                        continue;
                    }
                    let headroom = (ceiling - floor - PAD_M).max(0.0);
                    let width = 0.8 * room * (0.6 + 0.4 * roll.next());
                    let height = 0.8 * headroom * (0.5 + 0.5 * roll.next());
                    let outline = Outline::ALL[(roll.next() * Outline::ALL.len() as f64) as usize % Outline::ALL.len()];
                    let points = outline.points(width, height, &mut roll);
                    let base = DVec2::new(y, floor + PAD_M + height / 2.0);
                    self.lamp = Lamp::Occupant;
                    self.card(x, &points.iter().map(|p| *p + base).collect::<Vec<_>>(), card);
                    self.lamp = Lamp::None;
                }
            }
        }
    }

    /// In every rib's plane between two berths, on every level and either side, a bracket from deck
    /// to deck, its back upright and well off the wall, widening toward the shaft as it rises, with a
    /// flange down its slope.
    fn buttresses(&mut self, hold: &Hold) {
        let spar = region(Kind::Bay);
        for i in 1..hold.spans {
            let x = hold.rib(i);
            for k in 0..hold.levels {
                let (floor, ceiling) = (hold.floor(k), hold.ceiling(k));
                for side in [-1.0, 1.0] {
                    let room = hold.room(k);
                    let back = side * (hold.edge(k) - BUTTRESS_STANDOFF * room);
                    let foot = back - side * BUTTRESS_FOOT * room;
                    let head = back - side * BUTTRESS_HEAD * room;
                    let outline = [DVec2::new(back, floor), DVec2::new(foot, floor), DVec2::new(head, ceiling), DVec2::new(back, ceiling)];
                    let frame = BUTTRESS_FRAME * BUTTRESS_FOOT * room;
                    self.pierced(x, BUTTRESS_M / 2.0, &outline, frame, spar);
                    let slope = |p: DVec2| DVec3::new(x, p.x, p.y);
                    let inward = DVec2::new(-side, 0.0) * (FLANGE_M / 2.0);
                    self.beam(slope(DVec2::new(foot, floor) + inward), slope(DVec2::new(head, ceiling) + inward), FLANGE_M, DVec3::X, spar);
                }
            }
        }
    }

    /// [`Builder::prism`] with a hole through it along x: the outline less itself inset by `frame`
    /// meters, the hole's walls facing into it.
    fn pierced(&mut self, x: f64, half: f64, outline: &[DVec2], frame: f64, region: u32) {
        let outer = counterclockwise(outline);
        let Some(inner) = inset(&outer, frame) else {
            self.prism(x, half, &outer, region);
            return;
        };
        let at = |p: DVec2, dx: f64| DVec3::new(x + dx, p.x, p.y);
        let n = outer.len();
        for k in 0..n {
            let (a, b) = (outer[k], outer[(k + 1) % n]);
            let (c, d) = (inner[k], inner[(k + 1) % n]);
            // The outline's sides face out and the hole's in; each cap is a ring of quads between.
            self.quad([at(a, -half), at(b, -half), at(b, half), at(a, half)], region, Chart::Own);
            self.quad([at(d, -half), at(c, -half), at(c, half), at(d, half)], region, Chart::Own);
            self.quad([at(a, half), at(b, half), at(d, half), at(c, half)], region, Chart::Own);
            self.quad([at(a, -half), at(c, -half), at(d, -half), at(b, -half)], region, Chart::Own);
        }
    }

    /// A convex outline in (y, z), in either winding, stood `half` meters either side of the plane
    /// at `x`: two caps and a side per edge.
    fn prism(&mut self, x: f64, half: f64, outline: &[DVec2], region: u32) {
        let ccw = counterclockwise(outline);
        let at = |p: DVec2, dx: f64| DVec3::new(x + dx, p.x, p.y);
        for k in 0..ccw.len() {
            let (a, b) = (ccw[k], ccw[(k + 1) % ccw.len()]);
            // Counterclockwise about the outward normal, which is the edge turned clockwise in (y, z).
            self.quad([at(a, -half), at(b, -half), at(b, half), at(a, half)], region, Chart::Own);
        }
        for facing in [1.0, -1.0] {
            let n = ccw.len() as u32;
            let first = self.out.positions.len() as u32;
            let (lo, hi) = ccw.iter().fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            let along = self.piece.pose.rotation * self.frame * (DVec3::Y * facing);
            let bounds = if facing > 0.0 { [lo.x, hi.x, lo.y, hi.y] } else { [-hi.x, -lo.x, lo.y, hi.y] };
            for p in &ccw {
                self.vertex(at(*p, facing * half), DVec3::X * facing, region, [facing * p.x, p.y, ChartKind::Rect as u8 as f64, 0.0], bounds, along);
            }
            for k in 1..n - 1 {
                let (b, c) = if facing > 0.0 { (k, k + 1) } else { (k + 1, k) };
                self.out.indices.extend([first, first + b, first + c]);
            }
        }
    }

    /// A convex outline in the plane at `x`, counterclockwise in (y, z), as two faces back to back.
    fn card(&mut self, x: f64, outline: &[DVec2], region: u32) {
        if outline.len() < 3 {
            return;
        }
        let (lo, hi) = outline.iter().fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        for facing in [1.0, -1.0] {
            let normal = DVec3::X * facing;
            // u cross v is the normal, v up.
            let u = DVec3::Y * facing;
            let along = self.piece.pose.rotation * self.frame * u;
            let bounds = if facing > 0.0 { [lo.x, hi.x, lo.y, hi.y] } else { [-hi.x, -lo.x, lo.y, hi.y] };
            let first = self.out.positions.len() as u32;
            for p in outline {
                let at = DVec3::new(x + facing * 0.01, p.x, p.y);
                self.vertex(at, normal, region, [facing * p.x, p.y, ChartKind::Rect as u8 as f64, 0.0], bounds, along);
            }
            for k in 1..outline.len() as u32 - 1 {
                let (b, c) = if facing > 0.0 { (k, k + 1) } else { (k + 1, k) };
                self.out.indices.extend([first, first + b, first + c]);
            }
        }
    }
}

fn counterclockwise(outline: &[DVec2]) -> Vec<DVec2> {
    let area: f64 = (0..outline.len()).map(|k| outline[k].perp_dot(outline[(k + 1) % outline.len()])).sum();
    if area >= 0.0 { outline.to_vec() } else { outline.iter().rev().copied().collect() }
}

/// A counterclockwise convex outline with every edge moved `by` meters inward, corner `k` where
/// edges `k - 1` and `k` meet. `None` where it would turn inside out.
fn inset(outline: &[DVec2], by: f64) -> Option<Vec<DVec2>> {
    let n = outline.len();
    // Each edge's inward normal and its offset along it, from `a` to the next corner.
    let lines: Vec<(DVec2, f64)> = (0..n)
        .map(|k| {
            let (a, b) = (outline[k], outline[(k + 1) % n]);
            let normal = (b - a).perp().normalize();
            (normal, normal.dot(a) + by)
        })
        .collect();
    let corners: Vec<DVec2> = (0..n)
        .map(|k| {
            let ((n0, d0), (n1, d1)) = (lines[(k + n - 1) % n], lines[k]);
            let det = n0.perp_dot(n1);
            DVec2::new(d0 * n1.y - d1 * n0.y, n0.x * d1 - n1.x * d0) / det
        })
        .collect();
    // Inside out, an edge runs backward, though the winding may survive it.
    let forward = (0..n).all(|k| (corners[(k + 1) % n] - corners[k]).dot(outline[(k + 1) % n] - outline[k]) > 0.0);
    (corners.iter().all(|c| c.is_finite()) && forward).then_some(corners)
}

/// What a berth holds, drawn as a primitive seen from the side.
#[derive(Clone, Copy)]
enum Outline {
    Disc,
    Block,
    Capsule,
    Wedge,
    Hexagon,
}

impl Outline {
    const ALL: [Outline; 5] = [Outline::Disc, Outline::Block, Outline::Capsule, Outline::Wedge, Outline::Hexagon];

    /// Counterclockwise about the origin, within `width` by `height`.
    fn points(self, width: f64, height: f64, roll: &mut Roll) -> Vec<DVec2> {
        let (a, b) = (width / 2.0, height / 2.0);
        let around = |n: usize, rx: f64, ry: f64, phase: f64| -> Vec<DVec2> {
            (0..n).map(|k| {
                let t = phase + std::f64::consts::TAU * k as f64 / n as f64;
                DVec2::new(rx * t.cos(), ry * t.sin())
            }).collect()
        };
        match self {
            Outline::Disc => {
                let r = a.min(b);
                around(DISC_SIDES, r, r, 0.0)
            }
            Outline::Block => {
                let (x, y) = (a * (0.5 + 0.5 * roll.next()), b * (0.5 + 0.5 * roll.next()));
                vec![DVec2::new(-x, -y), DVec2::new(x, -y), DVec2::new(x, y), DVec2::new(-x, y)]
            }
            Outline::Capsule => {
                let r = b.min(a) * 0.6;
                let reach = (a - r).max(0.0);
                let half = DISC_SIDES / 2;
                let arc = |center: f64, from: f64| {
                    (0..=half).map(move |k| {
                        let t = from + std::f64::consts::PI * k as f64 / half as f64;
                        DVec2::new(center + r * t.cos(), r * t.sin())
                    })
                };
                arc(reach, -std::f64::consts::FRAC_PI_2).chain(arc(-reach, std::f64::consts::FRAC_PI_2)).collect()
            }
            Outline::Wedge => {
                // A cone side on, nose toward the mouth's left or right.
                let nose = if roll.next() < 0.5 { -a } else { a };
                let base = -nose;
                let mut points = vec![DVec2::new(base, -b * 0.7), DVec2::new(nose, 0.0), DVec2::new(base, b * 0.7)];
                if nose < 0.0 {
                    points.reverse();
                }
                points
            }
            Outline::Hexagon => {
                let r = a.min(b);
                around(6, r, r, std::f64::consts::FRAC_PI_6)
            }
        }
    }
}

/// Small enough to ride in an `f32` exactly.
fn seed(piece: &Piece) -> u32 {
    u32::from(piece.part.0) * 2 + u32::from(piece.side == Side::Mirror)
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Uniform in `[0, 1)`.
struct Roll(u64);

impl Roll {
    fn next(&mut self) -> f64 {
        self.0 = splitmix(self.0);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::form::PartId;
    use lc_world::form::place::Pose;

    fn bay(edges: DVec3, rotation: DMat3) -> (Piece, HullBuffers) {
        let piece = Piece {
            part: PartId(3),
            side: Side::Original,
            kind: Kind::Bay,
            shape: Shape::Slab { edges, corner: 0.0 },
            pose: Pose { position: DVec3::new(5.0, -2.0, 1.0), rotation },
        };
        let mut out = HullBuffers::default();
        mesh(&piece, Some(0), &mut out);
        (piece, out)
    }

    /// Möller–Trumbore, for `t` in `(0, 1)` along `from` to `to`.
    fn crossings(out: &HullBuffers, from: DVec3, to: DVec3) -> usize {
        let p = |i: u32| DVec3::from_array(out.positions[i as usize].map(f64::from));
        let d = to - from;
        out.indices.chunks(3).filter(|t| {
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let (e1, e2) = (b - a, c - a);
            let q = d.cross(e2);
            let det = e1.dot(q);
            if det.abs() < 1e-12 {
                return false;
            }
            let s = from - a;
            let u = s.dot(q) / det;
            let r = s.cross(e1);
            let v = d.dot(r) / det;
            let t = e2.dot(r) / det;
            u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 && t < 1.0
        }).count()
    }

    #[test]
    fn the_mouth_opens_down_a_clear_shaft_to_the_back_wall() {
        let (piece, out) = bay(DVec3::new(100.0, 70.0, 50.0), DMat3::from_rotation_z(0.4));
        assert!(!out.indices.is_empty());
        let pose = piece.pose;
        let off = DVec3::new(0.0, 0.37, 0.21);
        // In from outside the mouth to just short of the back wall: nothing.
        let shaft = crossings(&out, pose.to_outer(DVec3::new(60.0, 0.0, 0.0) + off), pose.to_outer(DVec3::new(-48.0, 0.0, 0.0) + off));
        assert_eq!(shaft, 0);
        // On through the back wall and the shell behind it.
        let through = crossings(&out, pose.to_outer(DVec3::new(60.0, 0.0, 0.0) + off), pose.to_outer(DVec3::new(-60.0, 0.0, 0.0) + off));
        assert_eq!(through, 2);
        // Across the hold from side to side, under the first deck: the berths' cards and pads,
        // ribs and rails may stand in the way, but the shell is crossed twice at least.
        let across = crossings(&out, pose.to_outer(DVec3::new(3.1, -40.0, -20.0)), pose.to_outer(DVec3::new(3.1, 40.0, -20.0)));
        assert!(across >= 4, "{across}");
    }

    #[test]
    fn every_vertex_is_inside_the_slab_and_every_buffer_agrees() {
        let edges = DVec3::new(60.0, 30.0, 40.0);
        let (piece, out) = bay(edges, DMat3::from_rotation_x(1.2) * DMat3::from_rotation_z(-0.7));
        let n = out.positions.len();
        for len in [out.normals.len(), out.regions.len(), out.faces.len(), out.charts.len(), out.chart_bounds.len(), out.tangents.len()] {
            assert_eq!(len, n);
        }
        assert!(out.indices.iter().all(|&i| (i as usize) < n));
        for p in &out.positions {
            let local = piece.pose.to_local(DVec3::from_array(p.map(f64::from)));
            assert!(local.abs().cmple(edges / 2.0 + 1e-3).all(), "{local}");
        }
        for v in &out.normals {
            assert!((glam::Vec3::from_array(*v).length() - 1.0).abs() < 1e-4);
        }
    }

    /// Up is the part's axis most along the ship's z, so turned on its side the decks still lie
    /// level: nothing between two levels' floors is crossed going straight up the shaft.
    #[test]
    fn decks_stack_up_the_ships_z_whichever_way_the_part_is_turned() {
        let turned = DMat3::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let (piece, out) = bay(DVec3::new(80.0, 40.0, 60.0), turned);
        let up = piece.pose.rotation.transpose() * DVec3::Z;
        assert!(up.y.abs() > 0.99);
        // Up the shaft from floor to ceiling: only the shell, twice, and the shaft's own ends.
        let at = |h: f64| piece.pose.to_outer(DVec3::new(1.3, 0.0, 0.0) + up * h + DVec3::Z * 0.17);
        assert_eq!(crossings(&out, at(-30.0), at(30.0)), 4);
    }

    /// The hull material finds berths and lamps by the same numbers.
    #[test]
    fn hull_wgsl_repeats_the_holds_constants() {
        let wgsl = include_str!("../assets/shaders/hull.wgsl");
        let constants = [
            ("BAY_RIB_DEPTH_M", RIB_DEPTH_M),
            ("BAY_DECK_M", DECK_M),
            ("BAY_PAD_M", PAD_M),
            ("BAY_DECK_SHARE", DECK_SHARE),
            ("BAY_FLOOD_INSET_M", FLOOD_INSET_M),
            ("BAY_FLOOD_SPREAD", FLOOD_SPREAD),
            ("BAY_FLOOD_DROP_M", FLOOD_DROP_M),
            ("BAY_RUNWAY_SHARE", RUNWAY_SHARE),
            ("BAY_STRIP_RISE_M", STRIP_RISE_M),
            ("BAY_VACANT", VACANT as f64),
        ];
        assert!(wgsl.contains(&format!("const BAY_REGION: u32 = {}u;", region(Kind::Bay))));
        for (name, value) in constants {
            let line = wgsl.lines().find(|l| l.starts_with(&format!("const {name}: f32 = "))).unwrap_or_else(|| panic!("{name} missing"));
            let text = line.split('=').nth(1).unwrap().trim().trim_end_matches(';');
            assert_eq!(text.parse::<f32>().unwrap(), value as f32, "{name}");
        }
        assert!(wgsl.contains(&format!("const BAYS: u32 = {}u;", BAYS)));
        let roles = [
            ("BAY_RUNWAY_LAMP", Lamp::Runway),
            ("BAY_FLOOD_LAMP", Lamp::Flood),
            ("BAY_OCCUPANT", Lamp::Occupant),
            ("BAY_BRACKET", Lamp::Bracket),
            ("BAY_GIRDER", Lamp::Girder),
        ];
        let sodium = crate::looks::sprue::SODIUM;
        assert!(wgsl.contains(&format!("const BAY_SODIUM: vec3<f32> = vec3<f32>({:?}, {:?}, {:?});", sodium.x, sodium.y, sodium.z)));
        for (name, lamp) in roles {
            assert!(wgsl.contains(&format!("const {name}: u32 = {}u;", lamp as u8)), "{name}");
        }
    }

    /// The formula in `hash.wgsl`, worked out apart from this port, so a slip in either shows.
    #[test]
    fn the_occupancy_hash_is_pcg() {
        assert_eq!(pcg(0), 129_708_002);
        assert_eq!(pcg(1), 2_831_084_092);
    }

    #[test]
    fn a_bays_rows_carry_the_ship_frame_into_its_own() {
        let rotation = DMat3::from_rotation_z(0.7) * DMat3::from_rotation_x(0.3);
        let (piece, _) = bay(DVec3::new(90.0, 40.0, 60.0), rotation);
        let hull = hull_bay(&piece).unwrap();
        let (frame, hold) = layout(&piece).unwrap();
        let inside = DVec3::new(10.0, -5.0, 3.0);
        let ship = piece.pose.to_outer(frame * inside);
        let row = |r: glam::Vec4| r.truncate().as_dvec3().dot(ship) + r.w as f64;
        let got = DVec3::new(row(hull.to_bay[0]), row(hull.to_bay[1]), row(hull.to_bay[2]));
        assert!(got.abs_diff_eq(inside, 1e-3), "{got}");
        assert_eq!(hull.grid.x as usize, hold.spans);
        assert_eq!(hull.grid.w as u32, seed(&piece));
    }

    /// A tangent leaning off its face is what re-seeded the aztec plate in stripes across the
    /// shell, where square corners stand outside the slab's rounded ones.
    #[test]
    fn every_tangent_lies_in_its_face() {
        let (mut piece, _) = bay(DVec3::new(100.0, 70.0, 50.0), DMat3::from_rotation_z(0.4) * DMat3::from_rotation_y(0.2));
        piece.shape = Shape::Slab { edges: DVec3::new(100.0, 70.0, 50.0), corner: 6.0 };
        let mut out = HullBuffers::default();
        mesh(&piece, Some(0), &mut out);
        for (n, t) in out.normals.iter().zip(&out.tangents) {
            let (n, t) = (glam::Vec3::from_array(*n), glam::Vec4::from_array(*t).truncate());
            assert!(n.dot(t).abs() < 1e-4, "{n} against {t}");
        }
    }

    /// Every triangle of a prism, either winding in, faces out from its middle, as its normals say.
    #[test]
    fn a_prism_faces_out_whichever_way_its_outline_runs() {
        let (piece, _) = bay(DVec3::new(40.0, 30.0, 20.0), DMat3::IDENTITY);
        let (frame, _) = layout(&piece).unwrap();
        let outline = [DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(1.0, 3.0), DVec2::new(0.0, 3.0)];
        for points in [outline.to_vec(), outline.iter().rev().copied().collect()] {
            let mut out = HullBuffers::default();
            let mut b = Builder { out: &mut out, piece: &piece, frame, hull: oriented(&piece.shape, DVec3::Z), h: 1e-3, bay: 0, lamp: Lamp::None };
            b.prism(2.0, 0.5, &points, 0);
            let p = |i: u32| DVec3::from_array(out.positions[i as usize].map(f64::from));
            let middle = out.positions.iter().map(|q| DVec3::from_array(q.map(f64::from))).sum::<DVec3>() / out.positions.len() as f64;
            for t in out.indices.chunks(3) {
                let wound = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).normalize();
                let stored = DVec3::from_array(out.normals[t[0] as usize].map(f64::from));
                assert!(wound.dot(stored) > 0.99, "{wound} against {stored}");
                assert!(wound.dot((p(t[0]) + p(t[1]) + p(t[2])) / 3.0 - middle) > 0.0);
            }
        }
    }

    /// A pierced plate's enclosed volume, summed over its triangles as wound, is the outline's area
    /// less the hole's times its thickness: every face, the hole's walls among them, wound outward.
    #[test]
    fn a_pierced_plate_is_wound_closed_around_its_hole() {
        let (piece, _) = bay(DVec3::new(40.0, 30.0, 20.0), DMat3::IDENTITY);
        let (frame, _) = layout(&piece).unwrap();
        let outline = [DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(6.0, 8.0), DVec2::new(0.0, 8.0)];
        let mut out = HullBuffers::default();
        let mut b = Builder { out: &mut out, piece: &piece, frame, hull: oriented(&piece.shape, DVec3::Z), h: 1e-3, bay: 0, lamp: Lamp::None };
        b.pierced(2.0, 0.5, &outline, 1.0, 0);
        let p = |i: u32| DVec3::from_array(out.positions[i as usize].map(f64::from));
        let volume: f64 = out.indices.chunks(3).map(|t| p(t[0]).dot(p(t[1]).cross(p(t[2]))) / 6.0).sum();
        let area = |o: &[DVec2]| (0..o.len()).map(|k| o[k].perp_dot(o[(k + 1) % o.len()])).sum::<f64>() / 2.0;
        let hole = inset(&outline, 1.0).unwrap();
        let want = (area(&outline) - area(&hole)) * 1.0;
        assert!((volume - want).abs() < 1e-3 * want, "{volume} against {want}");
        assert!(area(&hole) > 0.0);
    }

    #[test]
    fn inset_moves_each_edge_in_by_its_distance() {
        let square = [DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(4.0, 4.0), DVec2::new(0.0, 4.0)];
        let inner = inset(&square, 1.0).unwrap();
        assert!(inner[0].abs_diff_eq(DVec2::new(1.0, 1.0), 1e-9), "{:?}", inner);
        assert!(inner[2].abs_diff_eq(DVec2::new(3.0, 3.0), 1e-9));
        assert!(inset(&square, 2.5).is_none());
    }

    fn trapezoid(lower: f64, upper: f64, rotation: DMat3) -> (Piece, HullBuffers) {
        let piece = Piece {
            part: PartId(4),
            side: Side::Original,
            kind: Kind::Bay,
            shape: Shape::Trapezoid { lower, upper, height: 40.0, length: 80.0 },
            pose: Pose { position: DVec3::new(-3.0, 7.0, 2.0), rotation },
        };
        let mut out = HullBuffers::default();
        mesh(&piece, Some(0), &mut out);
        (piece, out)
    }

    /// Wide at the top and wide at the bottom, and turned over: everything inside the trapezoid,
    /// and the hall clear from the mouth to the back wall.
    #[test]
    fn a_trapezoid_bay_stays_inside_its_shape_with_its_hall_clear() {
        for (lower, upper) in [(60.0, 30.0), (30.0, 60.0)] {
            for rotation in [DMat3::IDENTITY, DMat3::from_rotation_x(std::f64::consts::PI)] {
                let (piece, out) = trapezoid(lower, upper, rotation);
                assert!(takes(&piece) && !out.indices.is_empty());
                for p in &out.positions {
                    let local = piece.pose.to_local(DVec3::from_array(p.map(f64::from)));
                    assert!(piece.shape.distance(local) < 1e-3, "{lower}/{upper}: {local} outside");
                }
                let (frame, hold) = layout(&piece).unwrap();
                let into = |b: DVec3| piece.pose.to_outer(frame * b);
                let off = DVec3::new(0.0, 0.37, 0.21);
                let shaft = crossings(&out, into(DVec3::new(50.0, 0.0, 0.0) + off), into(DVec3::new(hold.back + 0.5, 0.0, 0.0) + off));
                assert_eq!(shaft, 0, "{lower}/{upper}");
            }
        }
    }

    /// Each level's hall is its share of the hold's width at that height, so it steps out level by
    /// level under a wide roof and in under a narrow one.
    #[test]
    fn a_trapezoid_holds_halls_are_terraced_the_way_its_walls_lean() {
        for (lower, upper, widening) in [(30.0, 60.0, true), (60.0, 30.0, false)] {
            let (piece, _) = trapezoid(lower, upper, DMat3::IDENTITY);
            let (_, hold) = layout(&piece).unwrap();
            assert!(hold.levels > 2);
            for k in 1..hold.levels {
                assert_eq!(hold.hall(k) > hold.hall(k - 1), widening, "{lower}/{upper} level {k}");
                // What stands in a berth clears both its walls.
                assert!(hold.edge(k) <= hold.wall(hold.floor(k)) && hold.edge(k) <= hold.wall(hold.ceiling(k)));
                assert!(hold.room(k) > 0.0);
            }
        }
    }

    #[test]
    fn a_wedge_is_too_narrow_at_its_point_to_hold_a_bay() {
        let (piece, out) = trapezoid(60.0, 0.0, DMat3::IDENTITY);
        assert!(!takes(&piece));
        assert!(out.indices.is_empty());
    }

    #[test]
    fn only_a_slab_or_trapezoid_bay_is_taken() {
        let (mut piece, _) = bay(DVec3::new(10.0, 10.0, 10.0), DMat3::IDENTITY);
        assert!(takes(&piece));
        piece.kind = Kind::Storage;
        assert!(!takes(&piece));
        piece.kind = Kind::Bay;
        piece.shape = Shape::Cylinder { radius: 3.0, length: 9.0 };
        assert!(!takes(&piece));
        piece.shape = Shape::Trapezoid { lower: 20.0, upper: 10.0, height: 12.0, length: 30.0 };
        assert!(takes(&piece));
    }
}
