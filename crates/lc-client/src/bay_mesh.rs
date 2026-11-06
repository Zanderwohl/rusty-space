//! A slab bay as built geometry rather than a distance field: a shell open at its +x face, and
//! inside it a shadow box of flat layers. Truss ribs ring the hold every [`RIB_PITCH_M`]; decks
//! run front to back out of the two side walls every [`LEVEL_M`], leaving a shaft up the middle
//! for a craft to come in by, rise or sink to a level and turn aside into a berth. Each berth,
//! between two ribs on one deck, holds a pad and maybe a cutout card of a primitive's outline.
//! See `lightcone/docs/32-ship-rendering.md` §The bay.
//!
//! Laid out in the bay's own frame: x out of the mouth, z its up, y across. Up is whichever of the
//! part's y and z runs most along the ship's z. The shell's outside wears the storage's aztec
//! plate in the part's own charts, as any slab would; everything inside is charted on its own
//! faces. Detail is in meters and does not change with the mesher's resolution.
//!
//! Square cornered: a slab's rounded `corner` is not drawn, so the shell stands proud of the
//! distance field at its edges by at most `corner · (1 − 1/√3)`.

use glam::{DMat3, DVec2, DVec3};
use lc_world::form::{Kind, SparMode};
use lc_world::form::place::Side;
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::Piece;
use em_render::hull_material::{face_attribute, region_weights};

use crate::charts::{ChartKind, Oriented, chart_at, chart_of, oriented, tangent};
use crate::hull_mesh::{HullBuffers, region};

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
const PAD_M: f64 = 0.25;
/// Each side's decks reach this share of the hold's width in, so the shaft is the rest.
const DECK_SHARE: f64 = 0.28;
/// Past these the pitch stretches, so a bay kilometers long is not tens of thousands of ribs.
const MAX_SPANS: usize = 48;
const MAX_LEVELS: usize = 16;
const MAX_PANELS: usize = 24;
/// The share of berths left empty.
const VACANT: f64 = 0.3;
const DISC_SIDES: usize = 20;

/// Whether `piece` is meshed here rather than by the distance field.
pub fn takes(piece: &Piece) -> bool {
    piece.kind == Kind::Bay && matches!(piece.shape, Shape::Slab { .. })
}

/// Appends `piece`, which [`takes`], to `out`.
pub fn mesh(piece: &Piece, out: &mut HullBuffers) {
    let Shape::Slab { edges, .. } = piece.shape else { return };
    let up = piece.pose.rotation.transpose() * DVec3::Z;
    let u = if up.y.abs() > up.z.abs() { DVec3::Y * up.y.signum() } else { DVec3::Z * if up.z < 0.0 { -1.0 } else { 1.0 } };
    // Proper, so a bay-frame triangle keeps its winding in the part's.
    let frame = DMat3::from_cols(DVec3::X, u.cross(DVec3::X), u);
    let dims = DVec3::from_array([0, 1, 2].map(|k| edges.dot(frame.col(k).abs())));
    let hold = Hold::new(dims);
    let mut b = Builder {
        out,
        piece,
        frame,
        hull: oriented(&piece.shape, up),
        h: 1.0e-4 * piece.shape.least_dimension().max(1.0),
    };
    b.shell(&hold);
    b.ribs(&hold);
    b.decks(&hold);
    b.berths(&hold, seed(piece));
}

/// The cavity and how it is divided. `half` is the shell's half extent; the hold runs x from
/// `back` to the mouth at `half.x`, y within `±wide` and z within `±tall`.
struct Hold {
    half: DVec3,
    back: f64,
    wide: f64,
    tall: f64,
    spans: usize,
    levels: usize,
    /// How far each side's decks reach in from its wall.
    reach: f64,
}

impl Hold {
    fn new(dims: DVec3) -> Self {
        let half = dims / 2.0;
        let wall = WALL_M.min(0.1 * dims.min_element());
        let (back, wide, tall) = (-half.x + wall, half.y - wall, half.z - wall);
        let length = half.x - back;
        Hold {
            half,
            back,
            wide,
            tall,
            spans: ((length / RIB_PITCH_M).round() as usize).clamp(1, MAX_SPANS),
            levels: ((2.0 * tall / LEVEL_M).floor() as usize).clamp(1, MAX_LEVELS),
            reach: DECK_SHARE * 2.0 * wide,
        }
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
                let index = chart_of(&self.hull.shape, (local[0] + local[2]) / 2.0);
                for (c, l) in corners.into_iter().zip(local) {
                    let here = chart_at(&self.hull.shape, index, l, self.hull.lying);
                    let along = self.hull.turn.transpose() * tangent(&self.hull.shape, index, l, self.h, self.hull.lying);
                    let flag = if here.end { 1.0 } else if here.lying { 2.0 } else { 0.0 };
                    let at = [here.uv.x, here.uv.y, here.kind as u8 as f64, flag];
                    self.vertex(c, normal, region, at, here.bounds, self.piece.pose.rotation * along);
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
        self.out.faces.push(face_attribute(0.0, None));
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

    fn shell(&mut self, hold: &Hold) {
        let outside = region(Kind::Storage);
        let inside = region(Kind::Bay);
        let DVec3 { x: ex, y: ey, z: ez } = hold.half;
        let (x, y, z) = (DVec3::X, DVec3::Y, DVec3::Z);
        self.rect(-x * ex, z * ez, y * ey, outside, Chart::Hull);
        self.rect(y * ey, z * ez, x * ex, outside, Chart::Hull);
        self.rect(-y * ey, x * ex, z * ez, outside, Chart::Hull);
        self.rect(z * ez, x * ex, y * ey, outside, Chart::Hull);
        self.rect(-z * ez, y * ey, x * ex, outside, Chart::Hull);

        // The mouth's rim, a frame between the shell's edge and the hold's.
        let (w, t) = (hold.wide, hold.tall);
        let mouth = x * ex;
        let band_z = (ez - t) / 2.0;
        let band_y = (ey - w) / 2.0;
        self.rect(mouth + z * (t + band_z), y * ey, z * band_z, outside, Chart::Hull);
        self.rect(mouth - z * (t + band_z), y * ey, z * band_z, outside, Chart::Hull);
        self.rect(mouth + y * (w + band_y), y * band_y, z * t, outside, Chart::Hull);
        self.rect(mouth - y * (w + band_y), y * band_y, z * t, outside, Chart::Hull);

        let mid = (hold.back + ex) / 2.0;
        let run = (ex - hold.back) / 2.0;
        self.rect(x * hold.back, y * w, z * t, inside, Chart::Own);
        self.rect(DVec3::new(mid, 0.0, -t), x * run, y * w, inside, Chart::Own);
        self.rect(DVec3::new(mid, 0.0, t), y * w, x * run, inside, Chart::Own);
        self.rect(DVec3::new(mid, -w, 0.0), z * t, x * run, inside, Chart::Own);
        self.rect(DVec3::new(mid, w, 0.0), x * run, z * t, inside, Chart::Own);
    }

    /// Each rib a ring of truss in its own plane: a chord along the walls, an inner chord
    /// chamfered at the corners, and struts between them, upright and diagonal in turn.
    fn ribs(&mut self, hold: &Hold) {
        let spar = region(Kind::Spar(SparMode::Saddle));
        let (w, t) = (hold.wide, hold.tall);
        let width = RIB_WIDTH_M.min(0.25 * w.min(t));
        let (iw, it) = (w - width, t - width);
        let chamfer = (2.0 * width).min(0.3 * iw.min(it));
        let (ow, ot) = (w - CHORD_M / 2.0, t - CHORD_M / 2.0);
        // Counterclockwise in (y, z) from the +y wall's foot; each straight side with the wall it
        // faces, as the point on that wall square to a point on the side.
        let inner = [
            DVec2::new(iw, -it + chamfer),
            DVec2::new(iw, it - chamfer),
            DVec2::new(iw - chamfer, it),
            DVec2::new(-iw + chamfer, it),
            DVec2::new(-iw, it - chamfer),
            DVec2::new(-iw, -it + chamfer),
            DVec2::new(-iw + chamfer, -it),
            DVec2::new(iw - chamfer, -it),
        ];
        let walls: [fn(DVec2, DVec2) -> DVec2; 4] = [
            |p, o| DVec2::new(o.x, p.y),
            |p, o| DVec2::new(p.x, o.y),
            |p, o| DVec2::new(-o.x, p.y),
            |p, o| DVec2::new(p.x, -o.y),
        ];
        let corners = [DVec2::new(ow, ot), DVec2::new(-ow, ot), DVec2::new(-ow, -ot), DVec2::new(ow, -ot)];
        let o = DVec2::new(ow - CHORD_M / 2.0, ot - CHORD_M / 2.0);
        for i in 1..=hold.spans {
            let x = hold.rib(i).min(hold.half.x - RIB_DEPTH_M / 2.0);
            let at = |p: DVec2| DVec3::new(x, p.x, p.y);
            for k in 0..8 {
                self.chord(at(inner[k]), at(inner[(k + 1) % 8]));
            }
            for k in 0..4 {
                self.chord(at(corners[k]), at(corners[(k + 1) % 4]));
                // Side k runs from inner[2k] to inner[2k + 1]; the chamfer after it meets corner k.
                let (a, b) = (inner[2 * k], inner[2 * k + 1]);
                let panels = ((a.distance(b) / (1.5 * width)).ceil() as usize).clamp(1, MAX_PANELS);
                for j in 0..=panels {
                    let p = a.lerp(b, j as f64 / panels as f64);
                    self.beam(at(p), at(walls[k](p, o)), WEB_M, DVec3::X, spar);
                    if j < panels {
                        let q = a.lerp(b, (j + 1) as f64 / panels as f64);
                        let (from, to) = if j % 2 == 0 { (p, walls[k](q, o)) } else { (q, walls[k](p, o)) };
                        self.beam(at(from), at(to), WEB_M, DVec3::X, spar);
                    }
                }
                let bend = (b + inner[(2 * k + 2) % 8]) / 2.0;
                self.beam(at(bend), at(corners[k] - corners[k].signum() * CHORD_M / 2.0), WEB_M, DVec3::X, spar);
            }
        }
    }

    fn chord(&mut self, a: DVec3, b: DVec3) {
        let spar = region(Kind::Spar(SparMode::Saddle));
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

    fn decks(&mut self, hold: &Hold) {
        let (deck, spar) = (region(Kind::Bay), region(Kind::Spar(SparMode::Saddle)));
        let length = hold.half.x - hold.back;
        let axes = [DVec3::X, DVec3::Y, DVec3::Z];
        for k in 1..hold.levels {
            let top = hold.floor(k);
            for side in [-1.0, 1.0] {
                let y = side * (hold.wide - hold.reach / 2.0);
                let center = DVec3::new(hold.back + length / 2.0, y, top - DECK_M / 2.0);
                self.cuboid(center, axes, DVec3::new(length / 2.0, hold.reach / 2.0, DECK_M / 2.0), deck);
            }
        }
        for k in 0..hold.levels {
            let top = hold.floor(k);
            for side in [-1.0, 1.0] {
                let edge = side * (hold.wide - hold.reach + RAIL_M / 2.0);
                let a = DVec3::new(hold.back, edge, top + RAIL_M / 2.0);
                self.beam(a, a + DVec3::X * length, RAIL_M, DVec3::Z, spar);
            }
        }
    }

    /// A pad on every berth's floor, and on most a cutout card standing on it facing the mouth.
    fn berths(&mut self, hold: &Hold, seed: u64) {
        let (pad, card) = (region(Kind::Spar(SparMode::Saddle)), region(Kind::Storage));
        let axes = [DVec3::X, DVec3::Y, DVec3::Z];
        let mut berth = 0u64;
        for i in 0..hold.spans {
            let (x0, x1) = (hold.rib(i) + RIB_DEPTH_M / 2.0, hold.rib(i + 1) - RIB_DEPTH_M / 2.0);
            let x = (x0 + x1) / 2.0;
            for k in 0..hold.levels {
                let (floor, ceiling) = (hold.floor(k), hold.ceiling(k));
                for side in [-1.0, 1.0] {
                    berth += 1;
                    let mut roll = Roll(splitmix(seed ^ berth.wrapping_mul(0x9e37_79b9_7f4a_7c15)));
                    let y = side * (hold.wide - hold.reach / 2.0);
                    let size = 0.7 * (x1 - x0).min(hold.reach);
                    if size <= 0.0 {
                        continue;
                    }
                    self.cuboid(DVec3::new(x, y, floor + PAD_M / 2.0), axes, DVec3::new(size / 2.0, size / 2.0, PAD_M / 2.0), pad);
                    if roll.next() < VACANT {
                        continue;
                    }
                    let room = (ceiling - floor - PAD_M).max(0.0);
                    let width = 0.8 * hold.reach * (0.6 + 0.4 * roll.next());
                    let height = 0.8 * room * (0.5 + 0.5 * roll.next());
                    let outline = Outline::ALL[(roll.next() * Outline::ALL.len() as f64) as usize % Outline::ALL.len()];
                    let points = outline.points(width, height, &mut roll);
                    let base = DVec2::new(y, floor + PAD_M + height / 2.0);
                    self.card(x, &points.iter().map(|p| *p + base).collect::<Vec<_>>(), card);
                }
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

fn seed(piece: &Piece) -> u64 {
    splitmix(piece.part.0 as u64 * 2 + u64::from(piece.side == Side::Mirror))
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
        mesh(&piece, &mut out);
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

    #[test]
    fn only_a_slab_bay_is_taken() {
        let (mut piece, _) = bay(DVec3::new(10.0, 10.0, 10.0), DMat3::IDENTITY);
        assert!(takes(&piece));
        piece.kind = Kind::Storage;
        assert!(!takes(&piece));
        piece.kind = Kind::Bay;
        piece.shape = Shape::Cylinder { radius: 3.0, length: 9.0 };
        assert!(!takes(&piece));
    }
}
