//! Drones at work: what `em_render::drone_material` is told, from a refit's [`Frame`]. The player's
//! from its [`Refit`]; another craft's from the step its light shows, as [`crate::refit_hull`] draws
//! its hull, one swarm under each craft's root.
//!
//! Docks, targets, the count and the rest are pure functions of the frame, so a paused clock draws
//! the same frame twice. Only a working step sends drones out: to the sliver's frontier on a build
//! or a dismantle, carrying pieces home on the latter, and to the moved part's joint on a move.
//! Otherwise a few patrol and the rest stay docked. See 32 §Drones.
//!
//! The shader sends each drone to a new target every trip, so a target that jumped while drones
//! were on their way to it would make them jump too. Each target is therefore a continuous function
//! of the step's fraction: a fixed meridian of the part, crossed at a fixed share of the way
//! through the band ([`targets`]).

use std::collections::HashMap;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::drone_material::{
    DroneMaterial, DroneMaterialPlugin, DroneUniform, MAX_DOCKS, MAX_TARGETS, drone_quads,
};
use glam::DVec3;
use lc_proto::ShipId;
use lc_world::fitting::Balance;
use lc_world::form::Kind;
use lc_world::form::sdf::Piece;
use lc_world::refit::rounds::{Phase, Plan};
use lc_world::seen::Underway;

use crate::construction::{Clock, Look, Refit, Working};
use crate::parts::OwnForm;
use crate::session::TIME_RATE;

/// Drones per cubic meter of drone part: 785 on the starting ship, one to each ten-meter cube.
pub const PARTICLES_PER_M3: f64 = 1.0e-3;
/// Quads in the mesh, which every frame's vertex shader runs over whether drawn or docked. A GSV's
/// drone part would hold about 10⁹ at the density above; past the cap each mote stands for several
/// ([`mote`]), and from any distance that frames such a hull, single drones would be below a pixel
/// anyway.
pub const MAX_DRONES: u32 = 8192;
/// A mote's width, of the cube of drone part each drone stands for.
const MOTE_FILL: f64 = 0.4;

/// Wall seconds at the design rate for one trip out, dwell and back, and for a patroller to circle
/// the hull. Coordinate time is what the shader runs on, so `--rate` speeds both up.
const CYCLE_WALL_S: f64 = 12.0;
const PATROL_WALL_S: f64 = 90.0;

/// Of the drawn drones: working while a step builds or dismantles, swarming a moved joint, and
/// patrolling at any time.
const WORKING: f64 = 0.85;
const SWARMING: f64 = 0.6;
const PATROL: f64 = 0.04;
/// Of a trip, at the target.
const DWELL: f64 = 0.3;
const SWARM_DWELL: f64 = 0.5;
/// Traffic fades in over this much of a step's start and out over its end, at most a trip. The
/// targets change from one step to the next, and a drone on its way when they did would jump; this
/// way hardly any are out.
const RAMP: f64 = 0.1;

/// Directions a drone part is searched over for docks.
const DOCK_CANDIDATES: usize = 128;
/// Of a part's reach: how far a dock or target stands off its surface, and how far a drone wanders
/// while it dwells.
const STANDOFF: f64 = 0.02;
const HOVER: f64 = 0.01;
/// Samples along a meridian when finding where it crosses the band.
const MERIDIAN_SAMPLES: usize = 32;
/// Directions the patrol's bounds are sought along, per piece.
const BOUND_SAMPLES: usize = 48;

/// How many drones are drawn for `drone_m3` of drone part, and how many real ones each stands for.
pub fn population(drone_m3: f64) -> (u32, f64) {
    let real = drone_m3.max(0.0) * PARTICLES_PER_M3;
    let drawn = real.round().min(MAX_DRONES as f64);
    let each = if real > MAX_DRONES as f64 { real / drawn } else { 1.0 };
    (drawn as u32, each)
}

/// Meters across a mote standing for `each` drones, and how much brighter than one drone it is.
///
/// A fixed share of the width of the cube of drone part it stands for, with the rest of `each`
/// drones' light in its brightness, so a capped swarm is as bright as the whole one would be.
/// Widening it by `√each` instead, to carry all of the light in area, drew a GSV's swarm as a few
/// hundred blobs.
pub fn mote(each: f64) -> (f64, f64) {
    let width = each.cbrt();
    (MOTE_FILL * PARTICLES_PER_M3.powf(-1.0 / 3.0) * width, each / (width * width))
}

/// Points just off each drone part's open surface, where it is not inside another part, spread
/// evenly over all the drone parts. Up to [`MAX_DOCKS`], ship frame.
pub fn docks(pieces: &[Piece]) -> Vec<DVec3> {
    let drones: Vec<&Piece> = pieces.iter().filter(|p| p.kind == Kind::Drone).collect();
    let Some(share) = MAX_DOCKS.checked_div(drones.len()).map(|s| s.max(1)) else { return Vec::new() };
    let buried = |p: DVec3, own: &Piece| pieces.iter().any(|o| o != own && o.shape.distance(o.pose.to_local(p)) < 0.0);
    let mut docks = Vec::new();
    for drone in drones {
        let standoff = STANDOFF * drone.shape.reach();
        let all: Vec<DVec3> = (0..DOCK_CANDIDATES)
            .map(|i| {
                let dir = fibonacci(i, DOCK_CANDIDATES);
                drone.pose.to_outer(drone.shape.exit(dir).point + dir * standoff)
            })
            .collect();
        let open: Vec<DVec3> = all.iter().copied().filter(|p| !buried(*p, drone)).collect();
        let from = if open.is_empty() { &all } else { &open };
        let n = share.min(from.len());
        docks.extend((0..n).map(|i| from[i * from.len() / n]));
    }
    docks.truncate(MAX_DOCKS);
    docks
}

/// The stretch of the sliver the bands are crossing, as [`Working::across`] measures it: from the
/// first point not finished to the first not begun. `None` on a move, or before the front leaves
/// the joint.
pub fn frontier(working: &Working) -> Option<(f64, f64)> {
    if working.change.phase() == Phase::Move {
        return None;
    }
    let lo = first(|d| working.look(d) != Look::FINISHED);
    let hi = first(|d| working.look(d) == Look::default());
    (hi > lo).then_some((lo, hi))
}

/// The least `d` in `0..=1` where `past` holds, for a `past` that holds from some point onward.
fn first(past: impl Fn(f64) -> bool) -> f64 {
    if past(0.0) {
        return 0.0;
    }
    if !past(1.0) {
        return 1.0;
    }
    let (mut a, mut b) = (0.0, 1.0);
    for _ in 0..48 {
        let m = 0.5 * (a + b);
        if past(m) { b = m } else { a = m }
    }
    b
}

/// Where working drones go, ship frame, up to [`MAX_TARGETS`].
///
/// On a build or a dismantle, target `k` is on copy `k mod copies`, on a meridian of it at the
/// golden angle times `k`, where [`Working::across`] is a fixed share of the way through the
/// [`frontier`]: so it slides with the band and never jumps. On a move they ring each copy's rim
/// between its foot and its middle, and ride along with it.
pub fn targets(working: &Working) -> Vec<DVec3> {
    let copies = working.outer.len();
    if copies == 0 {
        return Vec::new();
    }
    if working.change.phase() == Phase::Move {
        return (0..MAX_TARGETS)
            .map(|k| {
                let piece = &working.outer[k % copies];
                let (s, c) = (GOLDEN * k as f64).sin_cos();
                let out = DVec3::new(0.0, c, s);
                let reach = piece.shape.reach();
                let rim = piece.shape.exit(out).point + out * STANDOFF * reach;
                piece.pose.to_outer(rim - DVec3::X * reach * unit(k as u32))
            })
            .collect();
    }
    let Some((lo, hi)) = frontier(working) else { return Vec::new() };
    let meridians: Vec<Meridian> = (0..copies).map(|c| Meridian::of(working, c)).collect();
    (0..MAX_TARGETS)
        .map(|k| {
            let c = k % copies;
            let share = (k as f64 + 0.5) / MAX_TARGETS as f64;
            meridians[c].crossing(working, k, lo + share * (hi - lo))
        })
        .collect()
}

/// Which half of a copy's meridians targets are sought along, from the point nearest the joint.
struct Meridian {
    /// Sample index nearest the joint, and whether to walk toward θ = 0 (+x), toward π, or both
    /// alternately.
    nearest: usize,
    halves: Halves,
}

#[derive(Clone, Copy)]
enum Halves {
    TowardNose,
    TowardTail,
    Both,
}

impl Meridian {
    /// Judged on the meridian at φ = 0, and fixed for the step, since the choice of half is
    /// discrete and must not flip as the band moves.
    fn of(working: &Working, copy: usize) -> Meridian {
        let g: Vec<f64> = (0..=MERIDIAN_SAMPLES).map(|i| across_at(working, copy, theta(i), 0.0)).collect();
        let nearest = (0..=MERIDIAN_SAMPLES).min_by(|&a, &b| g[a].total_cmp(&g[b])).unwrap_or(0);
        let rise = |to: usize| (g[to] - g[nearest]).max(0.0);
        let (nose, tail) = (rise(0), rise(MERIDIAN_SAMPLES));
        let halves = match (nose, tail) {
            (n, t) if t < 0.5 * n => Halves::TowardNose,
            (n, t) if n < 0.5 * t => Halves::TowardTail,
            _ => Halves::Both,
        };
        Meridian { nearest, halves }
    }

    /// The target `k` on this copy, where its meridian first reaches `across`, just off the
    /// surface. Walks the running maximum, so a meridian that dips is still inverted continuously.
    fn crossing(&self, working: &Working, k: usize, across: f64) -> DVec3 {
        let copy = k % working.outer.len();
        let phi = GOLDEN * k as f64;
        let toward_nose = match self.halves {
            Halves::TowardNose => true,
            Halves::TowardTail => false,
            Halves::Both => (k / working.outer.len()) % 2 == 0,
        };
        let path: Vec<usize> = if toward_nose {
            (0..=self.nearest).rev().collect()
        } else {
            (self.nearest..=MERIDIAN_SAMPLES).collect()
        };
        let piece = &working.outer[copy];
        let mut g0 = across_at(working, copy, theta(path[0]), phi);
        for pair in path.windows(2) {
            let (t0, t1) = (theta(pair[0]), theta(pair[1]));
            let g1 = g0.max(across_at(working, copy, t1, phi));
            if g1 >= across {
                let s = if g1 > g0 { ((across - g0) / (g1 - g0)).clamp(0.0, 1.0) } else { 0.0 };
                return on_surface(piece, t0 + s * (t1 - t0), phi);
            }
            g0 = g1;
        }
        on_surface(piece, theta(path[path.len() - 1]), phi)
    }
}

fn theta(i: usize) -> f64 {
    std::f64::consts::PI * i as f64 / MERIDIAN_SAMPLES as f64
}

fn meridian_dir(theta: f64, phi: f64) -> DVec3 {
    DVec3::new(theta.cos(), theta.sin() * phi.cos(), theta.sin() * phi.sin())
}

fn across_at(working: &Working, copy: usize, theta: f64, phi: f64) -> f64 {
    let piece = &working.outer[copy];
    working.across(copy, piece.pose.to_outer(piece.shape.exit(meridian_dir(theta, phi)).point))
}

/// Stood off along the ray rather than the normal, which turns at once over an edge such as a
/// cylinder's rim and would carry the target with it.
fn on_surface(piece: &Piece, theta: f64, phi: f64) -> DVec3 {
    let dir = meridian_dir(theta, phi);
    piece.pose.to_outer(piece.shape.exit(dir).point + dir * STANDOFF * piece.shape.reach())
}

/// The ellipsoid patrollers circle, center and semi-axes, ship frame: the pieces' bounds.
pub fn patrol(pieces: &[Piece]) -> (DVec3, DVec3) {
    let (mut min, mut max) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for piece in pieces {
        for i in 0..BOUND_SAMPLES {
            let p = piece.pose.to_outer(piece.shape.exit(fibonacci(i, BOUND_SAMPLES)).point);
            (min, max) = (min.min(p), max.max(p));
        }
    }
    if !min.is_finite() {
        return (DVec3::ZERO, DVec3::ONE);
    }
    (0.5 * (min + max), (0.5 * (max - min)).max(DVec3::splat(1.0)))
}

/// Everything the material is told but the clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    pub count: u32,
    pub mote_m: f64,
    /// Times one drone's light, per unit of a mote's area.
    pub gain: f64,
    pub haze_m: f64,
    pub docks: Vec<DVec3>,
    pub targets: Vec<DVec3>,
    pub working: f64,
    pub patrol: f64,
    pub carrying: f64,
    pub dwell: f64,
    pub hover_m: f64,
    pub patrol_center: DVec3,
    pub patrol_radii: DVec3,
}

/// The traffic for `pieces` as they stand, with `working` the step under way, if any, which lasts
/// `step_s` coordinate seconds.
pub fn traffic(pieces: &[Piece], working: Option<(&Working, f64)>) -> Traffic {
    let drone_m3 = pieces.iter().filter(|p| p.kind == Kind::Drone).map(|p| p.shape.volume()).sum();
    let (count, each) = population(drone_m3);
    let (mote, gain) = mote(each);
    let (patrol_center, patrol_radii) = patrol(pieces);
    let spacing = |area: f64, share: f64| (area / (share * count as f64).max(1.0)).sqrt().max(mote);
    let mean_radius = (patrol_radii.x * patrol_radii.y * patrol_radii.z).cbrt();
    let idle = Traffic {
        count,
        mote_m: mote,
        gain,
        haze_m: spacing(4.0 * std::f64::consts::PI * mean_radius * mean_radius, PATROL),
        docks: docks(pieces),
        targets: Vec::new(),
        working: 0.0,
        patrol: PATROL,
        carrying: 0.0,
        dwell: DWELL,
        hover_m: 0.0,
        patrol_center,
        patrol_radii,
    };
    let Some((working, step_s)) = working else { return idle };
    let targets = targets(working);
    if targets.is_empty() {
        return idle;
    }
    let reach = working.outer.iter().map(|p| p.shape.reach()).fold(0.0, f64::max);
    let ramp_s = (RAMP * step_s).min(CYCLE_WALL_S * TIME_RATE).max(f64::MIN_POSITIVE);
    let edge_s = step_s * working.fraction.min(1.0 - working.fraction);
    let ramp = smoothstep((edge_s / ramp_s).clamp(0.0, 1.0));
    let copies = working.outer.len() as f64;
    match working.change.phase() {
        Phase::Move => Traffic {
            working: SWARMING * ramp,
            dwell: SWARM_DWELL,
            hover_m: 2.0 * HOVER * reach,
            haze_m: spacing(2.0 * std::f64::consts::PI * reach * reach * copies, SWARMING),
            targets,
            ..idle
        },
        phase => {
            let band = frontier(working).map_or(0.0, |(lo, hi)| hi - lo);
            Traffic {
                working: WORKING * ramp,
                carrying: if phase == Phase::Dismantle { 1.0 } else { 0.0 },
                hover_m: HOVER * reach,
                haze_m: spacing(4.0 * reach * reach * band.max(0.05) * copies, WORKING),
                targets,
                ..idle
            }
        }
    }
}

impl Traffic {
    /// With `time_s` the coordinate seconds on the drones' own clock.
    pub fn uniform(&self, time_s: f64, seed: u32) -> DroneUniform {
        let v = |p: DVec3| p.as_vec3();
        let mut uniform = DroneUniform {
            patrol_center: v(self.patrol_center).extend(0.0),
            patrol_radii: v(self.patrol_radii * 1.05).extend(0.0),
            time: time_s as f32,
            seed,
            count: self.count,
            working: self.working as f32,
            patrol: self.patrol as f32,
            carrying: self.carrying as f32,
            cycle_s: (CYCLE_WALL_S * TIME_RATE) as f32,
            dwell: self.dwell as f32,
            hover_m: self.hover_m as f32,
            patrol_period_s: (PATROL_WALL_S * TIME_RATE) as f32,
            mote_m: self.mote_m as f32,
            haze_m: self.haze_m as f32,
            ..default()
        };
        uniform.color *= self.gain as f32;
        uniform.carry_color *= self.gain as f32;
        uniform.set_docks(&self.docks.iter().map(|&p| v(p)).collect::<Vec<_>>());
        uniform.set_targets(&self.targets.iter().map(|&p| v(p)).collect::<Vec<_>>());
        uniform
    }
}

fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

const GOLDEN: f64 = 2.399_963_229_728_653;

/// The `i`th of `n` directions spread evenly over the sphere.
fn fibonacci(i: usize, n: usize) -> DVec3 {
    let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
    let r = (1.0 - z * z).sqrt();
    let (s, c) = (GOLDEN * i as f64).sin_cos();
    DVec3::new(r * c, r * s, z)
}

/// `[0, 1)` from `k`, stable across runs.
fn unit(k: u32) -> f64 {
    let h = k.wrapping_mul(0x9e37_79b9).rotate_left(15).wrapping_mul(0x85eb_ca6b);
    (h >> 8) as f64 / (1u32 << 24) as f64
}

/// The player's drones. `origin_s` is the coordinate time the view was spawned, which is the
/// idle clock's zero.
#[derive(Component)]
pub struct Swarm {
    material: Handle<DroneMaterial>,
    origin_s: f64,
}

const SEED: u32 = 1;

/// The drones' clock, coordinate seconds: the round's own while it runs or loops, so traffic
/// restarts with it. `--refit-at` freezes the construction but not the traffic, so a burst of one
/// step moves; `--rate 0` freezes both.
fn clock_s(refit: Option<&Refit>, now_s: f64, origin_s: f64) -> f64 {
    match refit {
        Some(r) if matches!(r.clock, Clock::Looping(_) | Clock::Coordinate) => r.now_s(now_s) - r.plan.round().start_s,
        _ => now_s - origin_s,
    }
}

fn step_s(plan: &Plan, working: &Working) -> f64 {
    plan.steps().get(working.step).map_or(0.0, |s| s.duration_s)
}

/// Draw the player's drones over its form, in the ship's frame. Placed from
/// [`crate::parts::ship_frame`] rather than from a root, since the placeholders' root is gone
/// while [`crate::refit_hull`] draws the ship.
#[allow(clippy::too_many_arguments)]
pub fn update_drones(
    mut commands: Commands,
    game: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    eye: Res<crate::hull::Eye>,
    own: Res<OwnForm>,
    refit: Option<Res<Refit>>,
    mut swarms: Query<(Entity, &Swarm, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DroneMaterial>>,
) {
    let Some(sdf) = own.sdf() else {
        for (swarm, ..) in &swarms {
            commands.entity(swarm).despawn();
        }
        return;
    };
    let root = crate::parts::ship_frame(&game.0, &eye, &ui);
    let now_s = game.0.coordinate_time_s();
    let refit = refit.as_deref();
    let frame = refit.map(|r| r.frame(now_s));
    let traffic = match &frame {
        Some(frame) => {
            let working = frame.working.as_ref().zip(refit).map(|(w, r)| (w, step_s(&r.plan, w)));
            traffic(&frame.pieces, working)
        }
        None => traffic(sdf.pieces(), None),
    };
    let Some((_, swarm, mut transform)) = swarms.iter_mut().next() else {
        let uniforms = traffic.uniform(clock_s(refit, now_s, now_s), SEED);
        let material = materials.add(DroneMaterial { uniforms });
        commands.spawn((
            Mesh3d(meshes.add(drone_quads(MAX_DRONES))),
            MeshMaterial3d(material.clone()),
            root,
            // The mesh's positions are not where anything is drawn.
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            Swarm { material, origin_s: now_s },
        ));
        return;
    };
    *transform = root;
    let next = traffic.uniform(clock_s(refit, now_s, swarm.origin_s), SEED);
    if let Some(mut material) = materials.get_mut(&swarm.material)
        && material.uniforms != next
    {
        material.uniforms = next;
    }
}

/// Another craft's drones. `origin_s` is when the light left it as the view was spawned.
#[derive(Component)]
pub struct CraftSwarm {
    craft: ShipId,
    material: Handle<DroneMaterial>,
    quads: u32,
    origin_s: f64,
}

/// Pixels a craft spans before its drones are drawn. Below it their haze, never narrower than a few
/// pixels, would be wider than the craft.
const MIN_CRAFT_PX: f32 = 24.0;

/// So no two crafts' swarms move in step. Bijective on the id's low 32 bits; one id in 2³² shares
/// the player's [`SEED`].
fn seed(craft: ShipId) -> u32 {
    (craft.0 as u32).wrapping_mul(0x9e37_79b9) ^ 0x2545_f491
}

/// Quads for `count` drones, a power of two so a drone part growing through a round remeshes a
/// dozen times rather than every frame, and crafts of a size share one mesh.
fn quads(count: u32) -> u32 {
    count.max(1).next_power_of_two().min(MAX_DRONES)
}

/// Draw each other craft's drones while it has a round, under its
/// [`ShipHull`](crate::ship_hull::ShipHull) root, from [`crate::refit_hull::buildings`]. Their clock
/// is when its light left.
#[allow(clippy::too_many_arguments)]
pub fn draw_craft_drones(
    mut commands: Commands,
    (game, uplink): (Res<crate::app::Game>, Res<crate::uplink::Uplink>),
    cameras: Query<(&Camera, &Projection, &GlobalTransform), With<crate::app::SkyCamera>>,
    hulls: Query<(Entity, &crate::ship_hull::ShipHull, Option<&GlobalTransform>)>,
    mut swarms: Query<(Entity, &mut CraftSwarm, &mut Visibility, &mut Mesh3d)>,
    mut shared: Local<HashMap<u32, Handle<Mesh>>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DroneMaterial>>,
) {
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let buildings = crate::refit_hull::buildings(&game.0, None, &uplink.contacts, &balance);
    for (entity, swarm, ..) in &swarms {
        if buildings.iter().all(|b| b.craft != Some(swarm.craft)) {
            commands.entity(entity).despawn();
        }
    }
    for b in &buildings {
        let Some(craft) = b.craft else { continue };
        let Some(contact) = uplink.contacts.iter().find(|c| c.ship_id == craft) else { continue };
        let Some((hull, _, at)) = hulls.iter().find(|(_, h, _)| h.craft() == b.craft) else { continue };
        let views = cameras.iter().filter(|(camera, ..)| camera.is_active).filter_map(|(camera, projection, eye)| {
            let Projection::Perspective(perspective) = projection else { return None };
            Some((perspective.fov, camera.physical_viewport_size()?.y as f32, eye.translation()))
        });
        let px = at.map_or(f32::INFINITY, |at| craft_px(contact.length_m, at, views));
        let existing = swarms.iter_mut().find(|(_, s, ..)| s.craft == craft);
        if px < MIN_CRAFT_PX {
            if let Some((_, _, mut visibility, _)) = existing {
                *visibility = Visibility::Hidden;
            }
            continue;
        }
        let step_s = contact.building.as_ref().map_or(0.0, |(_, s)| Underway::from(s).duration_s);
        let traffic = traffic(&b.frame.pieces, b.frame.working.as_ref().map(|w| (w, step_s)));
        let n = quads(traffic.count);
        let mesh = shared.entry(n).or_insert_with(|| meshes.add(drone_quads(n))).clone();
        let Some((_, mut swarm, mut visibility, mut mesh3d)) = existing else {
            let uniforms = traffic.uniform(0.0, seed(craft));
            let material = materials.add(DroneMaterial { uniforms });
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                NoFrustumCulling,
                RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
                CraftSwarm { craft, material, quads: n, origin_s: contact.emitted_s },
                ChildOf(hull),
            ));
            continue;
        };
        visibility.set_if_neq(Visibility::Inherited);
        if swarm.quads != n {
            swarm.quads = n;
            mesh3d.0 = mesh;
        }
        let next = traffic.uniform(contact.emitted_s - swarm.origin_s, seed(craft));
        if let Some(mut material) = materials.get_mut(&swarm.material)
            && material.uniforms != next
        {
            material.uniforms = next;
        }
    }
}

/// The most pixels a craft `length_m` long under the root at `at` spans in any of `views`, each a
/// vertical field of view, a viewport height in pixels and an eye; unbounded with none to measure.
fn craft_px(length_m: f64, at: &GlobalTransform, views: impl Iterator<Item = (f32, f32, Vec3)>) -> f32 {
    let extent = length_m as f32 * at.scale().max_element();
    views
        .map(|(fov, height, eye)| crate::hull_mesh::pixels_across(extent, eye.distance(at.translation()), fov, height))
        .reduce(f32::max)
        .unwrap_or(f32::INFINITY)
}

pub struct DronesPlugin;

impl Plugin for DronesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DroneMaterialPlugin).add_systems(
            Update,
            (update_drones, draw_craft_drones)
                .after(crate::parts::update_parts)
                .in_set(crate::app::Stage::Scene)
                .run_if(in_state(crate::app::AppState::InGame)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::{Frame, demo_round};
    use crate::ship_hull::ShipHull;
    use lc_world::fitting::Balance;
    use lc_world::form::Form;
    use lc_world::refit::rounds::{Change, Step};

    const B: Balance = Balance::DEFAULT;

    fn plan() -> Plan {
        demo_round(&B, 1.0).solve(&B).expect("the demo round solves")
    }

    fn at(plan: &Plan, step: &Step, fraction: f64) -> Frame {
        Frame::at(plan, &B, plan.round().start_s + step.begins_s + fraction * step.duration_s)
    }

    fn step(plan: &Plan, change: Change) -> Step {
        *plan.steps().iter().find(|s| s.change == change).unwrap_or_else(|| panic!("no {change:?}"))
    }

    fn starting() -> Vec<Piece> {
        crate::parts::OwnForm::new(&Form::starting(), &B).unwrap().sdf().unwrap().pieces().to_vec()
    }

    #[test]
    fn the_count_follows_drone_volume_up_to_the_cap() {
        let drone_m3: f64 = starting().iter().filter(|p| p.kind == Kind::Drone).map(|p| p.shape.volume()).sum();
        let (count, each) = population(drone_m3);
        assert!((700..900).contains(&count) && each == 1.0, "{count} drones, {each} each");
        assert_eq!(population(0.0).0, 0);
        // A GSV, a hundred times the starting ship's length.
        let (count, each) = population(drone_m3 * 1.0e6);
        assert_eq!(count, MAX_DRONES);
        assert!((each * count as f64 - drone_m3 * 1.0e6 * PARTICLES_PER_M3).abs() < 1.0);
        assert!(mote(each).0 < 100.0 * mote(1.0).0, "{} m motes", mote(each).0);
    }

    /// What the material is told for a capped swarm: each mote's area carries `each` drones' light.
    #[test]
    fn a_capped_swarm_is_as_bright_as_the_whole() {
        let gsv = crate::parts::fixture("default*100").unwrap();
        let pieces = crate::parts::OwnForm::new(&gsv, &B).unwrap().sdf().unwrap().pieces().to_vec();
        let drone_m3: f64 = pieces.iter().filter(|p| p.kind == Kind::Drone).map(|p| p.shape.volume()).sum();
        let (count, each) = population(drone_m3);
        assert!(count == MAX_DRONES && each > 1.0e4, "{count} drawn, {each} each");
        let uniform = traffic(&pieces, None).uniform(0.0, SEED);
        let (width, gain) = mote(each);
        assert!(((width * width * gain / each) / mote(1.0).0.powi(2) - 1.0).abs() < 1e-9);
        let one = DroneUniform::default();
        assert_eq!(uniform.mote_m, width as f32);
        assert_eq!(uniform.color, one.color * gain as f32);
        assert_eq!(uniform.carry_color, one.carry_color * gain as f32);
    }

    #[test]
    fn docks_are_on_the_drone_parts_open_surface() {
        let pieces = starting();
        let found = docks(&pieces);
        assert!(!found.is_empty() && found.len() <= MAX_DOCKS, "{}", found.len());
        let drone = pieces.iter().find(|p| p.kind == Kind::Drone).unwrap();
        for d in &found {
            let off = drone.shape.distance(drone.pose.to_local(*d));
            assert!(off > 0.0 && off < 2.0 * STANDOFF * drone.shape.reach(), "{off} m off the drone part");
            for other in pieces.iter().filter(|p| *p != drone) {
                assert!(other.shape.distance(other.pose.to_local(*d)) >= 0.0, "a dock inside {:?}", other.part);
            }
        }
        assert!(docks(&pieces.iter().filter(|p| p.kind != Kind::Drone).copied().collect::<Vec<_>>()).is_empty());
    }

    /// The band leaves the joint on a build and comes in from the far edge on a dismantle.
    #[test]
    fn the_frontier_sweeps_the_way_the_bands_do() {
        let plan = plan();
        for (change, outward) in [(Change::Grow, true), (Change::Add, true), (Change::Remove, false)] {
            let s = step(&plan, change);
            let window = |f: f64| frontier(at(&plan, &s, f).working.as_ref().unwrap());
            let (early, late) = (window(0.1).unwrap(), window(0.9).unwrap());
            let mid = |(lo, hi): (f64, f64)| 0.5 * (lo + hi);
            assert_eq!(mid(late) > mid(early), outward, "{change:?}: {early:?} then {late:?}");
            let working = at(&plan, &s, 0.5).working.unwrap();
            let (lo, hi) = frontier(&working).unwrap();
            assert!(working.band(0.5 * (lo + hi)).is_some(), "{change:?}: no band inside {lo}..{hi}");
        }
        let s = step(&plan, Change::Move);
        assert!(frontier(at(&plan, &s, 0.5).working.as_ref().unwrap()).is_none());
    }

    /// Targets lie where the band is, on the sliver's outer surface.
    #[test]
    fn targets_are_on_the_frontier() {
        let plan = plan();
        for change in [Change::Grow, Change::Add, Change::Remove] {
            let s = step(&plan, change);
            for f in [0.2, 0.5, 0.8] {
                let working = at(&plan, &s, f).working.unwrap();
                let (lo, hi) = frontier(&working).unwrap();
                let targets = targets(&working);
                assert_eq!(targets.len(), MAX_TARGETS);
                // Some of a band can be off the surface, inside the part: those crowd its edge.
                let inside = targets
                    .iter()
                    .enumerate()
                    .filter(|(k, t)| {
                        let c = k % working.outer.len();
                        let a = working.across(c, **t);
                        a >= lo - 0.05 && a <= hi + 0.05
                    })
                    .count();
                assert!(inside * 4 >= targets.len() * 3, "{change:?} at {f}: {inside} of {} in {lo}..{hi}", targets.len());
                for (k, t) in targets.iter().enumerate() {
                    let piece = &working.outer[k % working.outer.len()];
                    let off = piece.shape.distance(piece.pose.to_local(*t));
                    assert!(off > 0.0 && off < 2.0 * STANDOFF * piece.shape.reach(), "{change:?}: {off} m off");
                }
            }
        }
    }

    /// The pitfall: a target that jumped would make every drone flying to it jump. Across a whole
    /// step, wherever a target moves more than a little in a 400th of it, an eighth of that
    /// interval moves it a good deal less, or hardly at all: it is fast there, not discontinuous.
    /// Where the band crosses a flat stretch, such as the hull's belt seen from its center, it is
    /// fast, since the band really does reach all of the belt at once.
    #[test]
    fn every_target_moves_continuously_through_a_step() {
        let plan = plan();
        for change in [Change::Grow, Change::Add, Change::Remove, Change::Move] {
            let s = step(&plan, change);
            let reach = at(&plan, &s, 0.5).working.unwrap().outer[0].shape.reach();
            let targets_at = |f: f64| targets(&at(&plan, &s, f).working.unwrap());
            const N: usize = 400;
            const FINE: usize = 8;
            for i in 1..N - 1 {
                let (f0, f1) = (i as f64 / N as f64, (i + 1) as f64 / N as f64);
                let (a, b) = (targets_at(f0), targets_at(f1));
                if a.len() != b.len() {
                    continue;
                }
                let jumps: Vec<f64> = a.iter().zip(&b).map(|(p, q)| p.distance(*q)).collect();
                if jumps.iter().all(|j| *j < 0.01 * reach) {
                    continue;
                }
                let fine: Vec<Vec<DVec3>> = (0..=FINE).map(|j| targets_at(f0 + (f1 - f0) * j as f64 / FINE as f64)).collect();
                for (k, jump) in jumps.into_iter().enumerate().filter(|(_, j)| *j >= 0.01 * reach) {
                    let finest = fine.windows(2).map(|w| w[0][k].distance(w[1][k])).fold(0.0, f64::max);
                    assert!(finest < (0.5 * jump).max(0.025 * reach), "{change:?} target {k} jumped {jump} m at {f0}, {finest} m of it at once");
                }
            }
        }
    }

    /// Around the moved part's rim, on the joint's half of it, and outside it.
    #[test]
    fn a_move_swarms_the_joint() {
        let plan = plan();
        let s = step(&plan, Change::Move);
        let working = at(&plan, &s, 0.5).working.unwrap();
        let piece = &working.outer[0];
        for t in targets(&working) {
            let local = piece.pose.to_local(t);
            let (x, reach) = (local.x, piece.shape.reach());
            assert!((-reach..=0.0).contains(&x) && piece.shape.distance(local) > 0.0, "{local} in the part's frame");
        }
        let traffic = traffic(&at(&plan, &s, 0.5).pieces, Some((&working, s.duration_s)));
        assert!(traffic.working > 0.0 && traffic.carrying == 0.0);
    }

    /// No work, no traffic: the idle ship sends nobody out, and a step does so only while it runs.
    #[test]
    fn drones_go_out_only_while_a_step_is_working() {
        let idle = traffic(&starting(), None);
        assert!(idle.working == 0.0 && idle.targets.is_empty() && idle.patrol > 0.0 && idle.patrol < 0.1);
        assert!(!idle.docks.is_empty() && idle.count > 0);
        let plan = plan();
        for s in plan.steps() {
            let traffic_at = |f: f64| {
                let frame = at(&plan, s, f);
                let working = frame.working.as_ref().map(|w| (w, s.duration_s));
                traffic(&frame.pieces, working)
            };
            assert_eq!(traffic_at(0.0).working, 0.0, "{:?} busy as it starts", s.change);
            assert!(traffic_at(0.5).working > 0.5, "{:?} idle halfway", s.change);
            assert!(traffic_at(1.0 - 1e-9).working < 1e-3, "{:?} busy as it ends", s.change);
            assert_eq!(traffic_at(0.5).carrying > 0.0, s.change.phase() == Phase::Dismantle, "{:?}", s.change);
        }
    }

    /// A contact stated a quarter through the demo round's grow step, or with no round, with the
    /// live clock long past the round.
    fn seen_building(plan: &Plan, id: i64, round: bool) -> (crate::uplink::Contact, Step, f64) {
        let i = plan.steps().iter().position(|s| s.change == Change::Grow).unwrap();
        let step = plan.steps()[i];
        let stated_s = plan.round().start_s + step.begins_s + 0.25 * step.duration_s;
        let under = lc_world::seen::Underway {
            step: i,
            part: step.part,
            change: step.change,
            after: step.after,
            fraction: 0.25,
            duration_s: step.duration_s,
            reversing: false,
        };
        let presence = lc_proto::Presence {
            ship_id: ShipId(id),
            name: format!("craft {id}"),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            drive_w: 0.0,
            emitted_t: (stated_s * 1.0e6) as i64,
            arrive_t: (stated_s * 1.0e6) as i64 + 3_600_000_000,
            form: (&plan.at(stated_s).form).into(),
            building: round.then(|| under.into()),
            glow: None,
            glare: None,
        };
        (crate::uplink::Contact::seen(presence, None), step, stated_s)
    }

    fn app(plan: &Plan, contacts: Vec<crate::uplink::Contact>) -> App {
        use lc_world::sky::AuthoredStars;
        let mut session = crate::session::Session::new(&AuthoredStars::sample(), 3);
        session.set_coordinate_time_us(((plan.round().start_s + 10.0 * plan.duration_s()) * 1.0e6) as i64);
        let mut uplink = crate::uplink::Uplink::default();
        uplink.contacts = contacts;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::app::Game(session))
            .insert_resource(crate::app::Ui(crate::ui::UiState::default()))
            .init_resource::<crate::hull::Eye>()
            .insert_resource(crate::parts::OwnForm::new(&Form::starting(), &B).unwrap())
            .insert_resource(uplink)
            .init_resource::<Assets<DroneMaterial>>()
            .init_resource::<Assets<Mesh>>()
            .add_systems(Update, (update_drones, draw_craft_drones));
        app
    }

    fn craft_swarms(app: &mut App) -> Vec<(ShipId, Entity, DroneUniform)> {
        let world = app.world_mut();
        let found: Vec<_> =
            world.query::<(&CraftSwarm, &ChildOf)>().iter(world).map(|(s, c)| (s.craft, c.parent(), s.material.clone())).collect();
        let materials = world.resource::<Assets<DroneMaterial>>();
        let mut found: Vec<_> = found.into_iter().map(|(c, p, m)| (c, p, materials.get(&m).unwrap().uniforms.clone())).collect();
        found.sort_by_key(|(c, ..)| c.0);
        found
    }

    /// Another craft with a round gets drones of its own, under its own root, working the frontier
    /// of the step its light shows; one with no round gets none; and the player's are untouched.
    #[test]
    fn another_crafts_drones_work_the_step_its_light_shows() {
        let plan = plan();
        let (mut nine, step, stated_s) = seen_building(&plan, 9, true);
        let (ten, ..) = seen_building(&plan, 10, true);
        let (eleven, ..) = seen_building(&plan, 11, false);
        nine.emitted_s = stated_s + 0.25 * step.duration_s;
        let mut app = app(&plan, vec![nine, ten, eleven]);
        let world = app.world_mut();
        let roots: Vec<Entity> = [9, 10, 11].map(|id| world.spawn(ShipHull::bare(Some(ShipId(id)), Entity::PLACEHOLDER)).id()).into();
        app.update();

        let swarms = craft_swarms(&mut app);
        let placed: Vec<_> = swarms.iter().map(|(c, p, _)| (*c, *p)).collect();
        assert_eq!(placed, vec![(ShipId(9), roots[0]), (ShipId(10), roots[1])]);
        let expected = |fraction: f64| {
            let frame = at(&plan, &step, fraction);
            traffic(&frame.pieces, frame.working.as_ref().map(|w| (w, step.duration_s)))
        };
        let close = |u: &DroneUniform, t: &Traffic| {
            assert!(t.working > 0.5 && (u.working - t.working as f32).abs() < 1e-3, "{} working, {} expected", u.working, t.working);
            assert_eq!(u.target_count as usize, t.targets.len());
            let off = t.targets.iter().zip(&u.targets).map(|(a, b)| a.as_vec3().distance(b.truncate())).fold(0.0, f32::max);
            assert!(off < 0.5, "targets {off} m from the step at its light's fraction");
        };
        close(&swarms[0].2, &expected(0.5));
        assert_ne!(swarms[0].2.seed, swarms[1].2.seed);
        assert!(swarms.iter().all(|(.., u)| u.seed != SEED));

        let world = app.world_mut();
        let own: Vec<_> = world.query::<(&Swarm, Option<&ChildOf>)>().iter(world).map(|(s, c)| (s.material.clone(), c.is_some())).collect();
        assert_eq!(own.len(), 1, "one player's swarm");
        assert!(!own[0].1, "the player's swarm is placed in the ship's frame, under no root");
        let uniforms = world.resource::<Assets<DroneMaterial>>().get(&own[0].0).unwrap().uniforms.clone();
        assert_eq!(uniforms, traffic(&starting(), None).uniform(0.0, SEED));

        // More of the step reaches us, and its drones move on with it.
        world.resource_mut::<crate::uplink::Uplink>().contacts[0].emitted_s = stated_s + 0.5 * step.duration_s;
        app.update();
        close(&craft_swarms(&mut app)[0].2, &expected(0.75));

        // Its round no longer stated: its drones go.
        app.world_mut().resource_mut::<crate::uplink::Uplink>().contacts[0].building = None;
        app.update();
        assert_eq!(craft_swarms(&mut app).iter().map(|(c, ..)| *c).collect::<Vec<_>>(), vec![ShipId(10)]);
    }

    #[test]
    fn crafts_have_seeds_of_their_own() {
        let seeds: std::collections::HashSet<u32> = (0..1000).map(|id| seed(ShipId(id))).collect();
        assert_eq!(seeds.len(), 1000);
        assert!(!seeds.contains(&SEED));
    }

    /// A craft too small on screen to show a drone draws none, and one close enough does.
    #[test]
    fn a_craft_too_small_on_screen_draws_no_drones() {
        let root = GlobalTransform::from(Transform::from_scale(Vec3::splat(1.0e-3)));
        let from = |km: f32| std::iter::once((1.0, 720.0, Vec3::new(km, 0.0, 0.0)));
        assert!(craft_px(500.0, &root, from(2.0)) > MIN_CRAFT_PX);
        assert!(craft_px(500.0, &root, from(20_000.0)) < MIN_CRAFT_PX);
        assert_eq!(craft_px(500.0, &root, std::iter::empty()), f32::INFINITY);
    }

    /// In the game: none spawned while the craft is too small, then shown, hidden and shown again
    /// as its root comes near, goes far and comes back.
    #[test]
    fn a_crafts_swarm_follows_its_size_on_screen() {
        use bevy::camera::Viewport;
        let plan = plan();
        let (contact, ..) = seen_building(&plan, 9, true);
        let mut app = app(&plan, vec![contact]);
        let world = app.world_mut();
        let viewport = Viewport { physical_size: UVec2::new(1280, 720), ..default() };
        world.spawn((
            Camera { viewport: Some(viewport), ..default() },
            Projection::Perspective(PerspectiveProjection { fov: 1.0, ..default() }),
            GlobalTransform::IDENTITY,
            crate::app::SkyCamera,
        ));
        // The root scales meters to kilometers, as the sky's unit would.
        let at = |km: f32| GlobalTransform::from(Transform::from_xyz(km, 0.0, 0.0).with_scale(Vec3::splat(1.0e-3)));
        let root = world.spawn((ShipHull::bare(Some(ShipId(9)), Entity::PLACEHOLDER), at(20_000.0))).id();
        let visibility = |app: &mut App| {
            let world = app.world_mut();
            world.query::<(&CraftSwarm, &Visibility)>().iter(world).map(|(_, v)| *v).collect::<Vec<_>>()
        };
        app.update();
        assert!(visibility(&mut app).is_empty(), "spawned for a craft too small on screen");
        for (km, seen) in [(2.0, Visibility::Inherited), (20_000.0, Visibility::Hidden), (2.0, Visibility::Inherited)] {
            *app.world_mut().get_mut::<GlobalTransform>(root).unwrap() = at(km);
            app.update();
            assert_eq!(visibility(&mut app), vec![seen], "at {km} km");
        }
    }

    #[test]
    fn a_swarms_mesh_is_its_population_to_a_power_of_two() {
        assert_eq!((quads(785), quads(0), quads(1_000_000)), (1024, 1, MAX_DRONES));
    }

    #[test]
    fn a_paused_clock_tells_the_material_the_same_thing_twice() {
        let plan = plan();
        let s = step(&plan, Change::Grow);
        let uniform = || {
            let frame = at(&plan, &s, 0.4);
            traffic(&frame.pieces, frame.working.as_ref().map(|w| (w, s.duration_s))).uniform(123.0, SEED)
        };
        assert_eq!(uniform(), uniform());
    }
}
