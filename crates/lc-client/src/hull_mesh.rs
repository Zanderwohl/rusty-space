//! A form's hull as a mesh: each piece on its own grid, by [`crate::surface_nets`] with vertices
//! placed by dual contouring, and the pieces' meshes laid end to end. Nothing is filleted or
//! fused: a piece sunk into another keeps its whole surface, and where they cross the depth buffer
//! draws the join. In the ship's frame in meters, carrying the region weights and open faces
//! `em_render::hull_material` reads. See `lightcone/docs/32-ship-rendering.md` §From distance
//! field to mesh.
//!
//! [`mesh_form`] is the pure core, form and resolution in and buffers out. [`HullMeshPlugin`] runs
//! it on the async compute pool, caches the meshes by [`form_hash`], and swaps each into its
//! entity when it lands, so a remesh never holds up a frame.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use em_render::hull_material::{ATTRIBUTE_HULL_CHART, ATTRIBUTE_HULL_CHART_BOUNDS, ATTRIBUTE_HULL_FACE, face_attribute, insert_region_weights, region_weights};
use glam::DVec3;
use lc_world::fitting::Balance;
use lc_world::form::capacity::{Aperture, apertures};
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Form, FormError, Kind, Mount, Part, PartId, Placement, Primitive, SparMode};

use crate::charts::{lay_out, tangent};
use crate::surface_nets::{AXES, Field, Grid, Surface, gradients, nets};

/// The palette's order: layer `i` is drawn with `textures/hull/{REGION_GRAPHS[i]}.tgraph`. Each
/// kind's region is its own layer, and the engine is drawn with [`ENGINE_FLANK`] off its open
/// faces.
pub const REGION_GRAPHS: [&str; 9] = ["storage", "drone", "living", "engine", "data", "mind", "spar", "bay", "engine_flank"];

pub const ENGINE_FLANK: u32 = 8;

pub fn region(kind: Kind) -> u32 {
    match kind {
        Kind::Storage => 0,
        Kind::Drone => 1,
        Kind::Living => 2,
        Kind::Engine => 3,
        Kind::Data => 4,
        Kind::Mind => 5,
        Kind::Spar(_) => 6,
        Kind::Bay => 7,
    }
}

pub const MIN_CELLS: u32 = 16;
pub const MAX_CELLS: u32 = 256;

const PIXELS_PER_CELL: f32 = 4.0;

/// One cell per four pixels along the ship's longest side, as a power of two from 16 to 256.
/// `current` is kept until the ideal is more than three quarters of a doubling from it, so a
/// ship held near a boundary does not remesh back and forth.
pub fn cells_for(pixels: f32, current: Option<u32>) -> u32 {
    let ideal = (pixels / PIXELS_PER_CELL).max(1.0).log2();
    let (lo, hi) = (MIN_CELLS.ilog2() as f32, MAX_CELLS.ilog2() as f32);
    if let Some(c) = current
        && (ideal.clamp(lo, hi) - (c as f32).log2()).abs() <= 0.75
    {
        return c;
    }
    1 << ideal.round().clamp(lo, hi) as u32
}

/// Pixels spanned by something `extent_m` long at `distance_m` through a perspective camera
/// `fov_y` radians tall on a viewport `height_px` tall. Never more than it would span from
/// half its own length, where the camera would be inside it.
pub fn pixels_across(extent_m: f32, distance_m: f32, fov_y: f32, height_px: f32) -> f32 {
    let distance = distance_m.max(0.5 * extent_m).max(f32::MIN_POSITIVE);
    extent_m / (2.0 * distance * (0.5 * fov_y).tan()) * height_px
}

/// A triangle corner whose field normal is farther than about 37 degrees from the triangle's own
/// takes the triangle's: the corner is on a crease.
const CREASE_COS: f64 = 0.8;

/// Cells across a piece's thinnest dimension, at the least: a piece thinner than a cell would
/// vanish whole.
const THINNEST_CELLS: f64 = 3.0;

/// Mesh buffers in the ship's frame, meters.
#[derive(Clone, Debug, Default)]
pub struct HullBuffers {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub regions: Vec<[[u8; 4]; 4]>,
    /// [`ATTRIBUTE_HULL_FACE`]'s, indexing [`apertures`].
    pub faces: Vec<[u8; 4]>,
    /// [`ATTRIBUTE_HULL_CHART`]'s and [`ATTRIBUTE_HULL_CHART_BOUNDS`]'s; see [`crate::charts`].
    pub charts: Vec<[f32; 4]>,
    pub chart_bounds: Vec<[f32; 4]>,
    /// The direction the chart's u grows, in the frame the positions are in; `w` is 1.
    pub tangents: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl HullBuffers {
    pub fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        insert_region_weights(&mut mesh, &self.regions);
        mesh.insert_attribute(ATTRIBUTE_HULL_FACE, VertexAttributeValues::Unorm8x4(self.faces));
        mesh.insert_attribute(ATTRIBUTE_HULL_CHART, self.charts);
        mesh.insert_attribute(ATTRIBUTE_HULL_CHART_BOUNDS, self.chart_bounds);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, self.tangents);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// [`mesh_pieces`] over the form's pieces, painting its engines' open faces.
pub fn mesh_form(form: &Form, balance: &Balance, cells: u32) -> Result<HullBuffers, FormError> {
    let sdf = Sdf::new(form, balance)?;
    let faces = apertures(form, balance).unwrap_or_default();
    if faces.len() > em_render::hull_material::FACES {
        bevy::log::warn!("{} engine faces: the grid is drawn on the first {}", faces.len(), em_render::hull_material::FACES);
    }
    Ok(mesh_pieces(sdf.pieces(), cells, &faces))
}

/// Each piece on its own grid, at the step of `cells` along the longest side of all of them, or
/// finer where a piece is thin. Every piece paints every face in `faces`: an engine sunk into a
/// neighbor leaves its face on the neighbor's surface, which must draw it. Empty for no pieces.
pub fn mesh_pieces(pieces: &[Piece], cells: u32, faces: &[Aperture]) -> HullBuffers {
    let mut out = HullBuffers::default();
    if pieces.is_empty() {
        return out;
    }
    let step = extent(Union::new(pieces).bounds) / cells.max(1) as f64;
    let (mut scratch, mut aside) = (Vec::new(), Vec::new());
    for piece in pieces {
        let alone = Union::new(std::slice::from_ref(piece));
        let grid = Grid::sample(&alone, piece_cells(piece, extent(alone.bounds), step));
        let surface = nets(&grid, &alone);
        let normals = gradients(&alone, &surface, grid.step);
        let painted: Vec<[u8; 4]> = surface
            .vertices
            .iter()
            .map(|&p| {
                let distance = alone.distance(p, &mut scratch);
                open_face(
                    p,
                    faces.iter().enumerate().map(|(face, aperture)| {
                        let near = face_near(aperture, p, distance, grid.step);
                        let mut own = || gradient(&mut |q| alone.distance(q, &mut aside), p, grid.step);
                        (face, aperture, if near > 0.0 { near * turned(own(), aperture) } else { 0.0 })
                    }),
                )
            })
            .collect();
        let r = region(piece.kind);
        append(&mut out, piece, &surface, &normals, region_weights(r, r, 0.0), &painted);
    }
    out
}

fn extent((min, max): (DVec3, DVec3)) -> f64 {
    (max - min).max_element()
}

fn piece_cells(piece: &Piece, extent_m: f64, step: f64) -> u32 {
    let thin = THINNEST_CELLS * extent_m / piece.shape.least_dimension().max(f64::MIN_POSITIVE);
    (extent_m / step).max(thin).ceil().clamp(MIN_CELLS as f64, MAX_CELLS as f64) as u32
}

/// FNV-1a over a canonical encoding of the form, each part's solved shape, and what of
/// `balance` shapes it. Parts in id order, and `-0.0` as `0.0`, so two forms that mesh alike
/// hash alike.
pub fn form_hash(form: &Form, balance: &Balance) -> u64 {
    let mut h = Fnv::default();
    for x in [balance.min_part_m3, balance.spar_gap, balance.spar_thickness] {
        h.f64(x);
    }
    let mut parts: Vec<&Part> = form.parts.iter().collect();
    parts.sort_by_key(|p| p.id);
    for part in parts {
        h.u64(part.id.0 as u64);
        h.kind(part.kind);
        h.f64(part.volume_m3);
        // The solved shape covers the primitive's proportions and its scale.
        h.shape(&part.shape(balance.min_part_m3));
        match part.placement {
            None => h.u64(0),
            Some(Placement { parent, mount, twist, tilt, blend, mirror }) => {
                h.u64(1 + parent.0 as u64);
                match mount {
                    Mount::Enclosing => h.u64(0),
                    Mount::Attached { anchor, standoff } => {
                        h.u64(1);
                        for x in anchor.to_array() {
                            h.f64(x);
                        }
                        h.f64(standoff);
                    }
                }
                for x in [twist, tilt.x, tilt.y, blend] {
                    h.f64(x);
                }
                h.u64(mirror as u64);
            }
        }
    }
    h.0
}

/// [`form_hash`] for bare copies, which are their poses and shapes and nothing else. Salted, so
/// no set of pieces hashes as a form does.
pub fn pieces_hash(pieces: &[Piece]) -> u64 {
    let mut h = Fnv::default();
    h.u64(0x5049_4543_4553);
    for piece in pieces {
        h.u64(piece.part.0 as u64);
        h.u64(piece.side as u64);
        h.kind(piece.kind);
        h.shape(&piece.shape);
        for x in piece.pose.position.to_array().into_iter().chain(piece.pose.rotation.to_cols_array()) {
            h.f64(x);
        }
    }
    h.0
}

fn mesh_key(form: u64, cells: u32) -> u64 {
    let mut h = Fnv(form);
    h.u64(cells as u64);
    h.0
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn u64(&mut self, x: u64) {
        self.bytes(&x.to_le_bytes());
    }

    /// `x + 0.0` is `0.0` for either zero and `x` otherwise.
    fn f64(&mut self, x: f64) {
        self.u64((x + 0.0).to_bits());
    }

    fn kind(&mut self, kind: Kind) {
        self.u64(match kind {
            Kind::Mind => 0,
            Kind::Storage => 1,
            Kind::Drone => 2,
            Kind::Engine => 3,
            Kind::Living => 4,
            Kind::Data => 5,
            Kind::Bay => 6,
            Kind::Spar(SparMode::Saddle) => 7,
            Kind::Spar(SparMode::Strap) => 8,
        });
    }

    fn shape(&mut self, shape: &Shape) {
        let dimensions: Vec<f64> = match *shape {
            Shape::Ellipsoid { semi_axes } => semi_axes.to_array().to_vec(),
            Shape::Capsule { radius, length } | Shape::Cylinder { radius, length } => vec![radius, length],
            Shape::Slab { edges, corner } => vec![edges.x, edges.y, edges.z, corner],
            Shape::Torus { major, minor } => vec![major, minor],
            Shape::Frustum { length, start, end } => vec![length, start, end],
        };
        for x in dimensions {
            self.f64(x);
        }
    }
}

/// The open face `p` lies most on, and how much, from each face's index, aperture and share at
/// `p`. Off every face, the nearest face's center names one at no share.
fn open_face<'a>(p: DVec3, faces: impl Iterator<Item = (usize, &'a Aperture, f64)>) -> [u8; 4] {
    let mut on: Option<(f64, usize)> = None;
    let mut nearest: Option<(f64, usize)> = None;
    for (face, aperture, share) in faces {
        let center_m = p.distance(aperture.center);
        if nearest.is_none_or(|(d, _)| center_m < d) {
            nearest = Some((center_m, face));
        }
        if share > 0.0 && on.is_none_or(|(s, _)| share > s) {
            on = Some((share, face));
        }
    }
    match (on, nearest) {
        (Some((share, face)), _) => face_attribute(share as f32, Some(face)),
        (None, nearest) => face_attribute(0.0, nearest.map(|(_, face)| face)),
    }
}

/// On a piece's surface `distance` meters off, in the face's plane and inside its rim, each eased
/// over a cell.
fn face_near(aperture: &Aperture, p: DVec3, distance: f64, step: f64) -> f64 {
    let off = p - aperture.center;
    let along = off.dot(aperture.out);
    let radial = (off - aperture.out * along).length();
    let on = 1.0 - smoothstep(step, 2.0 * step, distance.abs());
    let plane = 1.0 - smoothstep(step, 2.0 * step, along.abs());
    // Inside the rim, whose vertex is the corner and belongs to the flank as much as the face.
    let rim = 1.0 - smoothstep(aperture.radius_m - 1.5 * step, aperture.radius_m - 0.5 * step, radial);
    on * plane * rim
}

/// Turned along the exhaust, so a piece's side passing through the face's plane is not drawn as it.
fn turned(gradient: DVec3, aperture: &Aperture) -> f64 {
    smoothstep(0.5, 0.9, gradient.dot(aperture.out))
}

fn gradient(f: &mut dyn FnMut(DVec3) -> f64, p: DVec3, step: f64) -> DVec3 {
    let h = 1e-3 * step;
    DVec3::from_array([0, 1, 2].map(|a| f(p + AXES[a] * h) - f(p - AXES[a] * h))).normalize_or_zero()
}

fn smoothstep(lo: f64, hi: f64, x: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Copies drawn as the union of their bare shapes: no blend, and a spar uncut.
pub struct Union<'a> {
    pieces: &'a [Piece],
    inverse: Vec<glam::DMat3>,
    bounds: (DVec3, DVec3),
}

impl<'a> Union<'a> {
    pub fn new(pieces: &'a [Piece]) -> Self {
        let mut bounds = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for piece in pieces {
            let half = piece.shape.extent(piece.pose.rotation, 0.0);
            bounds = (bounds.0.min(piece.pose.position - half), bounds.1.max(piece.pose.position + half));
        }
        Union { pieces, inverse: pieces.iter().map(|p| p.pose.rotation.transpose()).collect(), bounds }
    }

    fn each(&self, p: DVec3, into: &mut Vec<f64>) {
        into.clear();
        into.extend(self.pieces.iter().zip(&self.inverse).map(|(piece, inv)| piece.shape.distance(*inv * (p - piece.pose.position))));
    }
}

impl Field for Union<'_> {
    fn distance(&self, p: DVec3, scratch: &mut Vec<f64>) -> f64 {
        self.each(p, scratch);
        scratch.iter().copied().fold(f64::INFINITY, f64::min)
    }

    fn bounds(&self) -> (DVec3, DVec3) {
        self.bounds
    }
}

/// Appends `piece`'s surface with its vertices shared, but for a corner on a crease, which gets a
/// vertex of its own with its triangle's flat normal. Each triangle is laid out whole in the chart
/// its middle is in, its corners unwrapped to within half a period of its first across a band's or
/// ring's seam, so a pattern never runs backwards across it; a corner shared between charts, or
/// between sides of a seam, is a vertex in each.
fn append(out: &mut HullBuffers, piece: &Piece, surface: &Surface, normals: &[DVec3], regions: [[u8; 4]; 4], faces: &[[u8; 4]]) {
    let mut shared: HashMap<(u32, u8, i32, i32), u32> = HashMap::new();
    let pose = piece.pose;
    // Fine against any part: a millimeter in a part a few meters across.
    let h = 1.0e-4 * piece.shape.least_dimension().max(1.0);
    for t in &surface.triangles {
        let [a, b, c] = t.map(|i| surface.vertices[i as usize]);
        let flat = (b - a).cross(c - a).normalize_or_zero();
        let (chart, laid) = lay_out(&piece.shape, [a, b, c].map(|p| pose.to_local(p)));
        for (k, &v) in t.iter().enumerate() {
            let (here, turns) = laid[k];
            let uv = here.uv;
            let key = (v, chart, turns[0], turns[1]);
            let smooth = normals[v as usize];
            let creased = flat != DVec3::ZERO && smooth.dot(flat) < CREASE_COS;
            let mut push = |normal: DVec3| {
                let p = surface.vertices[v as usize];
                let along = pose.rotation * tangent(&piece.shape, chart, pose.to_local(p), h);
                out.positions.push(p.as_vec3().to_array());
                out.normals.push(normal.as_vec3().to_array());
                out.regions.push(regions);
                out.faces.push(faces[v as usize]);
                out.charts.push([uv.x as f32, uv.y as f32, here.kind as u8 as f32, if here.end { 1.0 } else { 0.0 }]);
                out.chart_bounds.push(here.bounds.map(|x| x as f32));
                out.tangents.push(along.as_vec3().extend(1.0).to_array());
                out.positions.len() as u32 - 1
            };
            let index = if creased { push(flat) } else { *shared.entry(key).or_insert_with(|| push(smooth)) };
            out.indices.push(index);
        }
    }
}

/// A storage sphere with one spar. [`SparMode::Saddle`]: a boom off it with an engine ball on
/// its end, embedded at both joints. [`SparMode::Strap`]: a band around its equator, a torus
/// whose tube straddles the surface. For tests and the void.
pub fn spar_fixture(mode: SparMode) -> Form {
    let balance = Balance::DEFAULT;
    let hang = |id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount| {
        let placement = Placement { parent: PartId(parent), mount, twist: 0.0, tilt: glam::DVec2::ZERO, blend: 0.0, mirror: false };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    };
    let sphere = Primitive::Capsule { length: 0.0 };
    let tank = hang(1, Kind::Storage, sphere, 5.0e5, 0, Mount::Enclosing);
    let spar = match mode {
        SparMode::Saddle => {
            let anchor = Mount::Attached { anchor: DVec3::new(0.3, 1.0, 0.2), standoff: -0.3 };
            hang(2, Kind::Spar(mode), Primitive::Cylinder { length: 8.0 }, 2.0e4, 1, anchor)
        }
        SparMode::Strap => {
            let r = tank.shape(balance.min_part_m3).reach();
            let band = Primitive::Torus { major: 7.0 };
            hang(2, Kind::Spar(mode), band, band.volume(r / 7.0), 1, Mount::Enclosing)
        }
    };
    let mut parts = vec![Part::mind(PartId(0), balance.min_part_m3), tank, spar];
    if mode == SparMode::Saddle {
        parts.push(hang(3, Kind::Engine, sphere, 1.0e4, 2, Mount::Attached { anchor: DVec3::X, standoff: -0.5 }));
    }
    Form { parts }
}

/// What an entity draws as its hull. A mesh arrives as its `Mesh3d` once meshed; give it a
/// `MeshMaterial3d<HullMaterial>` yourself. Its grid is sized by [`measure`].
#[derive(Component, Clone)]
#[require(HullPixels, HullMeshState)]
pub struct HullForm {
    pub source: HullSource,
    /// A fixed resolution instead of one from [`HullPixels`].
    pub cells: Option<u32>,
}

impl HullForm {
    pub fn form(form: Form, balance: Balance) -> Self {
        HullForm { source: HullSource::Form(Arc::new(form), balance), cells: None }
    }
}

#[derive(Clone, Debug)]
pub enum HullSource {
    Form(Arc<Form>, Balance),
    /// Through [`mesh_pieces`], with no open faces.
    Pieces(Arc<[Piece]>),
}

impl HullSource {
    fn hash(&self) -> u64 {
        match self {
            HullSource::Form(form, balance) => form_hash(form, balance),
            HullSource::Pieces(pieces) => pieces_hash(pieces),
        }
    }

    fn extent_m(&self) -> f64 {
        match self {
            HullSource::Form(form, balance) => Sdf::new(form, balance).map_or(0.0, |sdf| extent(Union::new(sdf.pieces()).bounds)),
            HullSource::Pieces(pieces) if pieces.is_empty() => 0.0,
            HullSource::Pieces(pieces) => extent(Union::new(pieces).bounds),
        }
    }

    fn mesh(&self, cells: u32) -> Result<HullBuffers, FormError> {
        match self {
            HullSource::Form(form, balance) => mesh_form(form, balance, cells),
            HullSource::Pieces(pieces) => Ok(mesh_pieces(pieces, cells, &[])),
        }
    }
}

/// Pixels the hull spans on screen along its longest side, written by [`measure`].
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct HullPixels(pub f32);

#[derive(Component, Debug, Default)]
pub struct HullMeshState {
    form: u64,
    extent_m: f64,
    cells: Option<u32>,
    wanted: Option<u64>,
    shown: Option<u64>,
}

impl HullMeshState {
    /// The longest side of the form's bounds, for [`pixels_across`].
    pub fn extent_m(&self) -> f64 {
        self.extent_m
    }

    /// The [`form_hash`] or [`pieces_hash`] of the source wanted.
    pub fn form(&self) -> u64 {
        self.form
    }

    pub fn cells(&self) -> Option<u32> {
        self.cells
    }

    /// Whether what is drawn is what is wanted.
    pub fn current(&self) -> bool {
        self.wanted.is_some() && self.wanted == self.shown
    }
}

/// Meshes kept after nothing wants them, so zooming back is free.
const CACHED: usize = 32;

#[derive(Resource, Default)]
pub struct HullMeshes {
    ready: HashMap<u64, Option<Handle<Mesh>>>,
    order: VecDeque<u64>,
    pending: HashMap<u64, Task<Option<Mesh>>>,
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HullMeshSystems;

pub struct HullMeshPlugin;

impl Plugin for HullMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HullMeshes>().add_systems(Update, (measure, want, land).chain().in_set(HullMeshSystems));
    }
}

/// Each hull's pixels on screen, the most it spans through any active perspective camera, so it
/// is meshed for its nearest view. A frame behind, as the transforms are.
///
/// Not the editor's camera, which stands in the ship's frame in meters rather than the sky's
/// render space: its distance to a hull out there means nothing.
pub fn measure(
    cameras: Query<(&Camera, &Projection, &GlobalTransform), Without<crate::form_view::FormCamera>>,
    mut hulls: Query<(&GlobalTransform, &HullMeshState, &mut HullPixels)>,
) {
    for (at, state, mut pixels) in &mut hulls {
        pixels.0 = cameras
            .iter()
            .filter(|(camera, ..)| camera.is_active)
            .filter_map(|(camera, projection, eye)| {
                let Projection::Perspective(perspective) = projection else { return None };
                let height = camera.physical_viewport_size()?.y as f32;
                let distance = eye.translation().distance(at.translation());
                // The ship's frame is meters under a root that scales it into render units.
                let extent = state.extent_m as f32 * at.scale().max_element();
                Some(pixels_across(extent, distance, perspective.fov, height))
            })
            .fold(0.0, f32::max);
    }
}

fn want(mut meshes: ResMut<HullMeshes>, mut hulls: Query<(Ref<HullForm>, &HullPixels, &mut HullMeshState)>) {
    for (hull, pixels, mut state) in &mut hulls {
        if hull.is_changed() {
            state.form = hull.source.hash();
            state.extent_m = hull.source.extent_m();
        }
        let cells = hull.cells.unwrap_or_else(|| cells_for(pixels.0, state.cells));
        let key = mesh_key(state.form, cells);
        if state.wanted == Some(key) {
            continue;
        }
        state.cells = Some(cells);
        state.wanted = Some(key);
        if meshes.ready.contains_key(&key) || meshes.pending.contains_key(&key) {
            continue;
        }
        let source = hull.source.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let started = bevy::platform::time::Instant::now();
            match source.mesh(cells) {
                Ok(buffers) => {
                    let (vertices, ms) = (buffers.positions.len(), started.elapsed().as_secs_f64() * 1e3);
                    debug!("hull_mesh: {cells} cells, {vertices} vertices in {ms:.0} ms");
                    Some(buffers.into_mesh())
                }
                Err(error) => {
                    warn!("hull_mesh: {error}");
                    None
                }
            }
        });
        meshes.pending.insert(key, task);
    }
}

fn land(
    mut commands: Commands,
    mut meshes: ResMut<HullMeshes>,
    mut assets: ResMut<Assets<Mesh>>,
    mut hulls: Query<(Entity, &mut HullMeshState, Option<&mut Mesh3d>)>,
) {
    let meshes = &mut *meshes;
    let mut landed = Vec::new();
    meshes.pending.retain(|&key, task| match check_ready(task) {
        Some(mesh) => {
            landed.push((key, mesh));
            false
        }
        None => true,
    });
    for (key, mesh) in landed {
        meshes.ready.insert(key, mesh.map(|m| assets.add(m)));
        meshes.order.push_back(key);
    }
    for (entity, mut state, mesh) in &mut hulls {
        let Some(wanted) = state.wanted else { continue };
        if state.shown == Some(wanted) {
            continue;
        }
        let Some(ready) = meshes.ready.get(&wanted) else { continue };
        state.shown = Some(wanted);
        let Some(handle) = ready.clone() else { continue };
        match mesh {
            Some(mut mesh) => mesh.0 = handle,
            None => {
                commands.entity(entity).insert(Mesh3d(handle));
            }
        }
    }
    // After the handing out, and never one still wanted: `want` asks for a key only once.
    let wanted: HashSet<u64> = hulls.iter().filter_map(|(_, state, _)| state.wanted).collect();
    let mut excess = meshes.order.len().saturating_sub(CACHED);
    let ready = &mut meshes.ready;
    meshes.order.retain(|key| {
        if excess == 0 || wanted.contains(key) {
            return true;
        }
        ready.remove(key);
        excess -= 1;
        false
    });
}

#[cfg(test)]
mod tests {
    use lc_world::form::presets::Builtin;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn pieces(form: &Form) -> Vec<Piece> {
        Sdf::new(form, &B).unwrap().pieces().to_vec()
    }

    /// One piece's surface, as [`mesh_pieces`] extracts it, and the grid's step.
    fn surface(piece: &Piece, cells: u32) -> (Surface, f64) {
        let alone = Union::new(std::slice::from_ref(piece));
        let grid = Grid::sample(&alone, cells);
        (nets(&grid, &alone), grid.step)
    }

    /// Closed, manifold and consistently wound: every edge is used once in each direction.
    /// Returns the Euler characteristic and the number of connected pieces.
    fn check_closed(s: &Surface, what: &str) -> (i64, i64) {
        assert!(!s.triangles.is_empty(), "{what}: empty");
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
        for t in &s.triangles {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2], "{what}: degenerate {t:?}");
            for k in 0..3 {
                *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &directed {
            assert_eq!(n, 1, "{what}: edge {a}->{b} used {n} times");
            assert_eq!(directed.get(&(b, a)), Some(&1), "{what}: edge {a}-{b} has one face");
        }
        // Around each vertex, its triangles form one fan: two cones meeting at a point do not.
        let mut fans: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
        for t in &s.triangles {
            for k in 0..3 {
                fans.entry(t[k]).or_default().insert(t[(k + 1) % 3], t[(k + 2) % 3]);
            }
        }
        for (v, fan) in &fans {
            let start = *fan.keys().next().unwrap();
            let (mut at, mut steps) = (start, 0);
            loop {
                at = fan[&at];
                steps += 1;
                if at == start {
                    break;
                }
            }
            assert_eq!(steps, fan.len(), "{what}: vertex {v} is where {} fans meet", fan.len() as f64 / steps as f64);
        }
        let used: HashSet<u32> = s.triangles.iter().flatten().copied().collect();
        let (v, e, f) = (used.len() as i64, directed.len() as i64 / 2, s.triangles.len() as i64);

        let mut parent: HashMap<u32, u32> = used.iter().map(|&i| (i, i)).collect();
        fn root(parent: &mut HashMap<u32, u32>, mut i: u32) -> u32 {
            while parent[&i] != i {
                let up = parent[&parent[&i]];
                parent.insert(i, up);
                i = up;
            }
            i
        }
        for t in &s.triangles {
            for k in 1..3 {
                let (a, b) = (root(&mut parent, t[0]), root(&mut parent, t[k]));
                parent.insert(a, b);
            }
        }
        let pieces = used.iter().filter(|&&i| root(&mut parent, i) == i).count() as i64;
        (v - e + f, pieces)
    }

    fn volume(s: &Surface) -> f64 {
        s.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| s.vertices[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    fn mind() -> Part {
        Part::mind(PartId(0), B.min_part_m3)
    }

    /// `primitive` around the Mind, and the piece it makes.
    fn alone(primitive: Primitive, volume_m3: f64) -> (Form, Piece) {
        let placement =
            Placement { parent: PartId(0), mount: Mount::Enclosing, twist: 0.0, tilt: glam::DVec2::ZERO, blend: 0.0, mirror: false };
        let part = Part { id: PartId(1), kind: Kind::Storage, primitive, volume_m3, placement: Some(placement) };
        let form = Form { parts: vec![mind(), part] };
        let piece = *pieces(&form).iter().find(|p| p.part == PartId(1)).unwrap();
        (form, piece)
    }

    /// Each primitive, and how many handles it has.
    fn primitives() -> Vec<(Primitive, i64)> {
        vec![
            (Primitive::Ellipsoid { axes: DVec3::new(3.0, 1.0, 2.0) }, 0),
            (Primitive::Capsule { length: 2.0 }, 0),
            (Primitive::Capsule { length: 0.0 }, 0),
            (Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.2 }, 0),
            (Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.0 }, 0),
            (Primitive::Cylinder { length: 3.0 }, 0),
            (Primitive::Torus { major: 3.0 }, 1),
            (Primitive::Frustum { length: 2.0, taper: 0.5 }, 0),
            (Primitive::Frustum { length: 2.0, taper: 0.0 }, 0),
            (Primitive::Frustum { length: 2.0, taper: 2.5 }, 0),
        ]
    }

    const RESOLUTIONS: [u32; 3] = [16, 40, 96];

    #[test]
    fn every_primitive_is_closed_at_three_resolutions() {
        for (primitive, handles) in primitives() {
            let (_, piece) = alone(primitive, 5.0e5);
            let field = Union::new(std::slice::from_ref(&piece));
            let truth = piece.shape.volume();
            for cells in RESOLUTIONS {
                let what = format!("{primitive:?} at {cells}");
                let (s, step) = surface(&piece, cells);
                let (chi, pieces) = check_closed(&s, &what);
                assert_eq!(chi, 2 * pieces - 2 * handles, "{what}: χ {chi} over {pieces} pieces");
                let v = volume(&s);
                assert!(v > 0.0, "{what}: wound inward, {v}");
                if cells == 96 {
                    assert!((v / truth - 1.0).abs() < 0.03, "{what}: {v} against {truth}");
                }
                let bound = 2.0 * 3f64.sqrt() * step;
                for &p in &s.vertices {
                    let d = field.distance(p, &mut Vec::new());
                    assert!(d.abs() <= bound, "{what}: {p} is {d} off");
                }
            }
        }
    }

    #[test]
    fn every_piece_of_the_spar_fixtures_and_presets_is_closed() {
        let forms = [spar_fixture(SparMode::Saddle), spar_fixture(SparMode::Strap), Form::reference(), Form::starting()]
            .into_iter()
            .chain(Builtin::ALL.map(Builtin::form));
        for (k, form) in forms.enumerate() {
            for piece in pieces(&form) {
                for cells in [16, 64] {
                    let (s, _) = surface(&piece, cells);
                    check_closed(&s, &format!("form {k}, {:?} at {cells}", piece.kind));
                }
            }
        }
    }

    /// A strap about two meters deep on a grid of about seven: kept where a sample lands in it
    /// and nowhere else, and closed either way. [`mesh_pieces`] meshes it finer, so it stays whole.
    #[test]
    fn a_strap_thinner_than_a_cell_is_holed_not_broken() {
        let form = spar_fixture(SparMode::Strap);
        let strap = *pieces(&form).iter().find(|p| matches!(p.kind, Kind::Spar(_))).unwrap();
        let r = form.parts[1].shape(B.min_part_m3).reach();
        let depth = B.spar_thickness * 2.0 * r;
        let (s, step) = surface(&strap, 16);
        assert!(step > 3.0 * depth, "{step} against {depth}");
        check_closed(&s, "coarse strap");
        let alone = Union::new(std::slice::from_ref(&strap));
        assert!(piece_cells(&strap, extent(alone.bounds), extent(alone.bounds) / 16.0) > 16, "meshed at the step it vanishes at");
    }

    #[test]
    fn a_thin_piece_is_meshed_fine_enough_to_stay() {
        // A disk of an engine, thinner than a cell of 16 across its width.
        let mut form = Form::starting();
        let engine = form.parts.iter_mut().find(|p| p.kind == Kind::Engine).unwrap();
        engine.primitive = Primitive::Cylinder { length: 0.1 };
        for piece in pieces(&form) {
            assert!(!mesh_pieces(&[piece], MIN_CELLS, &[]).positions.is_empty(), "a {:?} vanished", piece.kind);
        }
    }

    /// The bell's wide end is lit whole, and nothing turned away from it is. Half the radius in, as
    /// the rim eases over a cell and the starting ship's faces are four cells across at 96.
    #[test]
    fn the_open_face_is_the_engines_wide_end_and_nothing_else() {
        for form in [Form::reference(), Form::starting(), Builtin::Plate.form()] {
            let b = mesh_form(&form, &B, 96).unwrap();
            let faces = apertures(&form, &B).unwrap();
            let (mut on, mut away) = (0, 0);
            for ((p, n), face) in b.positions.iter().zip(&b.normals).zip(&b.faces) {
                let (p, n) = (Vec3::from_array(*p).as_dvec3(), Vec3::from_array(*n).as_dvec3());
                let a = faces[face[1] as usize];
                let off = p - a.center;
                let along = off.dot(a.out);
                if along.abs() < 1e-3 * a.radius_m && (off - a.out * along).length() < 0.5 * a.radius_m && n.dot(a.out) > 0.99 {
                    assert_eq!(face[0], 255, "{p} is on the face and not lit whole");
                    on += 1;
                }
                if n.dot(a.out) < 0.3 {
                    assert_eq!(face[0], 0, "{p} faces {n} and is lit");
                    away += 1;
                }
            }
            assert!(on > 20 && away > 200, "{on} on the face, {away} turned away");
        }
    }

    #[test]
    fn buffers_carry_every_attribute_and_face_their_normals() {
        let b = mesh_form(&Builtin::Cluster.form(), &B, 32).unwrap();
        let n = b.positions.len();
        assert!(n > 0 && b.indices.len() % 3 == 0);
        assert_eq!((b.normals.len(), b.regions.len(), b.faces.len(), b.charts.len(), b.chart_bounds.len(), b.tangents.len()), (n, n, n, n, n, n));
        assert!(b.indices.iter().all(|&i| (i as usize) < n));
        let against = b
            .indices
            .chunks(3)
            .filter(|t| {
                let [p, q, r] = [0, 1, 2].map(|k| Vec3::from(b.positions[t[k] as usize]));
                let face = (q - p).cross(r - p);
                t.iter().any(|&i| face.dot(Vec3::from(b.normals[i as usize])) < -1e-3 * face.length())
            })
            .count();
        assert!(against <= b.indices.len() / 3 / 100, "{against} triangles face against their normals");
    }

    #[test]
    fn each_vertex_wears_one_region_and_every_piece_is_drawn() {
        let form = Form::starting();
        let b = mesh_form(&form, &B, 32).unwrap();
        let mut seen = HashSet::new();
        for weights in &b.regions {
            let on: Vec<usize> = (0..16).filter(|&r| weights[r / 4][r % 4] > 0).collect();
            assert_eq!(on.len(), 1, "{weights:?}");
            seen.insert(on[0] as u32);
        }
        let wanted: HashSet<u32> = pieces(&form).iter().map(|p| region(p.kind)).collect();
        assert_eq!(seen, wanted);
    }

    #[test]
    fn a_sphere_and_a_strap_are_closed_at_the_finest_grid() {
        let (_, sphere) = alone(Primitive::Capsule { length: 0.0 }, 5.0e5);
        let strap = *pieces(&spar_fixture(SparMode::Strap)).iter().find(|p| matches!(p.kind, Kind::Spar(_))).unwrap();
        for piece in [sphere, strap] {
            let (s, _) = surface(&piece, MAX_CELLS);
            check_closed(&s, "at MAX_CELLS");
        }
    }

    /// Fifty kilometers across, with positions in `f32`: a millimeter or two at the rim.
    #[test]
    fn a_fifty_kilometer_hull_is_closed_and_placed_to_the_centimeter() {
        let radius: f64 = 25_000.0;
        let (_, piece) = alone(Primitive::Capsule { length: 0.0 }, 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3));
        let field = Union::new(std::slice::from_ref(&piece));
        let (s, step) = surface(&piece, 64);
        let (chi, pieces) = check_closed(&s, "50 km");
        assert_eq!((chi, pieces), (2, 1));
        for &p in &s.vertices {
            let rounded = p.as_vec3().as_dvec3();
            assert!((rounded - p).length() < 0.01, "{p}");
            assert!(field.distance(rounded, &mut Vec::new()).abs() < step, "{p}");
        }
    }

    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<Mesh>().add_plugins(HullMeshPlugin);
        app
    }

    fn step_until(app: &mut App, what: &str, done: impl Fn(&mut World) -> bool) {
        for _ in 0..20_000 {
            app.update();
            if done(app.world_mut()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("never {what}");
    }

    fn hull(form: Form, cells: u32) -> HullForm {
        HullForm { cells: Some(cells), ..HullForm::form(form, B) }
    }

    fn shown(world: &World, e: Entity) -> Option<Handle<Mesh>> {
        world.get::<Mesh3d>(e).map(|m| m.0.clone())
    }

    /// The old mesh stays up through the frame that asks for a new one and every frame until it
    /// lands, and the new one is swapped in once it has.
    #[test]
    fn a_remesh_never_holds_up_a_frame() {
        let mut app = headless();
        let e = app.world_mut().spawn(hull(Builtin::Cluster.form(), 16)).id();
        step_until(&mut app, "meshed", |w| w.get::<HullMeshState>(e).unwrap().current());
        let old = shown(app.world(), e).expect("a mesh once current");

        app.world_mut().get_mut::<HullForm>(e).unwrap().cells = Some(128);
        app.update();
        assert_eq!(shown(app.world(), e), Some(old.clone()), "the frame that asked still draws the old mesh");
        assert!(!app.world().get::<HullMeshState>(e).unwrap().current());
        assert_eq!(app.world().resource::<HullMeshes>().pending.len(), 1, "meshing elsewhere");

        let mut frames = 0;
        loop {
            let pending = !app.world().resource::<HullMeshes>().pending.is_empty();
            app.update();
            frames += 1;
            let now = shown(app.world(), e);
            if now != Some(old.clone()) {
                assert!(pending || frames == 1, "swapped before the task finished");
                assert!(app.world().resource::<HullMeshes>().pending.is_empty());
                assert!(app.world().get::<HullMeshState>(e).unwrap().current());
                break;
            }
            assert!(frames < 20_000, "the new mesh never landed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(frames > 1, "landed on the first frame after asking: not meshed off the frame");
    }

    /// More meshes landing in one frame than the cache keeps: every hull still gets its own.
    #[test]
    fn a_crowd_landing_at_once_is_all_drawn() {
        let mut app = headless();
        let crowd: Vec<Entity> = (0..CACHED + 1)
            .map(|k| {
                let (form, _) = alone(Primitive::Capsule { length: 0.0 }, 5.0e5 * (1.0 + k as f64 / 100.0));
                app.world_mut().spawn(hull(form, 16)).id()
            })
            .collect();
        app.update();
        let tasks = |w: &mut World| w.resource::<HullMeshes>().pending.values().all(|t| t.is_finished());
        while !tasks(app.world_mut()) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.update();
        app.update();
        for e in crowd {
            assert!(shown(app.world(), e).is_some(), "a hull left without its mesh");
            assert!(app.world().get::<HullMeshState>(e).unwrap().current());
        }
    }

    #[test]
    fn a_hash_names_the_shape() {
        let a = Form::reference();
        let mut b = a.clone();
        b.parts.reverse();
        assert_eq!(form_hash(&a, &B), form_hash(&b, &B), "the order parts are listed in");
        let mut c = a.clone();
        c.parts[2].volume_m3 *= 1.0 + 1e-12;
        assert_ne!(form_hash(&a, &B), form_hash(&c, &B), "a part's size");
        let mut d = a.clone();
        d.parts[4].placement.as_mut().unwrap().twist = -0.0;
        assert_eq!(form_hash(&a, &B), form_hash(&d, &B), "the sign of a zero");
        let gap = Balance { spar_gap: 1.0, ..B };
        assert_ne!(form_hash(&a, &B), form_hash(&a, &gap));
    }

    #[test]
    fn cells_track_pixels_in_doublings_and_hold_near_a_boundary() {
        assert_eq!(cells_for(10.0, None), MIN_CELLS);
        assert_eq!(cells_for(10_000.0, None), MAX_CELLS);
        assert_eq!(cells_for(256.0, None), 64);
        assert_eq!(cells_for(256.0 * 1.6, Some(64)), 64);
        assert_eq!(cells_for(256.0 * 1.8, Some(64)), 128);
        assert_eq!(cells_for(10_000.0, Some(MAX_CELLS)), MAX_CELLS);
        assert!(pixels_across(500.0, 1000.0, 1.0, 720.0) > pixels_across(500.0, 2000.0, 1.0, 720.0));
    }

    /// Zero everywhere: no gradient for a plane, so each vertex is its loop's mean.
    struct Level;

    impl Field for Level {
        fn distance(&self, _: DVec3, _: &mut Vec<f64>) -> f64 {
            0.0
        }

        fn bounds(&self) -> (DVec3, DVec3) {
            (DVec3::ZERO, DVec3::splat(3.0))
        }
    }

    /// Break the cleaning on purpose: without it, alternating squares survive and the mesh has
    /// edges with four faces.
    #[test]
    fn without_cleaning_a_checkerboard_is_not_manifold() {
        let mut grid = Grid { origin: DVec3::ZERO, step: 1.0, n: [4, 4, 4], values: vec![1.0; 64], near: vec![[0, 0, 0]] };
        for i in [[1, 1, 1], [2, 2, 1], [1, 1, 2], [2, 2, 2]] {
            let k = grid.index(i);
            grid.values[k] = -1.0;
        }
        let dirty = std::panic::catch_unwind(|| {
            let s = nets(&grid, &Level);
            check_closed(&s, "dirty");
        });
        assert!(dirty.is_err(), "an uncleaned checkerboard came out closed");
        grid.clean();
        let s = nets(&grid, &Level);
        assert_eq!(check_closed(&s, "clean"), (2, 1));
    }
}
