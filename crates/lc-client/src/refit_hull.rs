//! A refit drawn on the hull meshes (R2) in the hull material (R3), with the truss (R8).
//!
//! The game keeps its placeholders until R10, but plating is a mask on R3's material and means
//! nothing on a Bevy primitive. So while the player's ship has a [`Refit`], `--demo refit`'s or a
//! round in the game, the whole ship is meshed, and [`crate::parts`] stands aside once the first
//! step's meshes are shown. When the round is over they go, and the placeholders come back.
//!
//! A step is meshed once, as it starts: the ship it leaves alone, each copy it works on at the
//! larger of its two sizes, and each copy's truss. Within the step only uniforms move, each read
//! from [`Working::sweep`], so a paused clock draws the same frame twice. A step's meshes are shown
//! when all of them have landed, and the last step's stay up until then.

use std::sync::Arc;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::hull_material::{
    ALL_PLATED, HullMaterial, HullMaterialPlugin, HullUniform, REGIONS, Tile, tile_array,
};
use glam::DVec3;
use lc_world::fitting::Balance;
use lc_world::form::place::{Pose, Side};
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Form, Kind, PartId, SparMode};

use crate::construction::{Frame, Refit, Sweep};
use lc_world::refit::rounds::Phase;
use crate::hull::{ALBEDO, Eye, lighting, lit};
use crate::hull_mesh::{Finish, HullForm, HullMeshPlugin, HullMeshState, HullSource, REGION_GRAPHS, Union, region};
use crate::procedural::{Bakes, Shape, Target, placeholder};
use crate::surface_nets::Field;
use crate::truss::{self, GIRDER_RADIUS_M, PITCH_M, TrussBuffers};

/// As `examples/hull_void.rs`: a graph's unit square is this many meters, in this many texels.
const TILE_M: f32 = 64.0;
const TILE_TEXELS: u32 = 512;

/// Lights by kind, as shares of the exposure's reference. A stand-in until R15 gives them real
/// powers: bright enough to read on a night side, lost on a lit one.
fn lights(kind: &str) -> Vec3 {
    match kind {
        "drone" => 0.02 * Vec3::new(0.85, 0.92, 1.0),
        "living" => 0.03 * Vec3::new(1.0, 0.8, 0.55),
        "engine" => 0.08 * Vec3::new(0.75, 0.85, 1.0),
        "mind" => 0.006 * Vec3::new(0.6, 0.9, 1.0),
        "bay" => 0.02 * Vec3::new(1.0, 0.92, 0.8),
        _ => Vec3::ZERO,
    }
}

/// Girders are painted safety yellow and lit by their own work lights, so a frontier too far off
/// to show a girder still reads as construction by its color.
const GIRDER_ALBEDO: Vec3 = Vec3::new(0.8, 0.52, 0.1);
const WORK_LIGHTS: f32 = 1.0;
/// Plating before it is fitted out.
const BARE: f32 = 0.45;

pub struct RefitHullPlugin;

impl Plugin for RefitHullPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((HullMaterialPlugin, HullMeshPlugin))
            .init_resource::<Palette>()
            .init_resource::<Unready>()
            .init_resource::<Showing>()
            .add_systems(Update, request_palette)
            .add_systems(Last, take_texels);
    }
}

/// The hull graphs baked into one palette, on demand.
#[derive(Resource, Default)]
pub struct Palette {
    requested: bool,
    albedo: Vec<Handle<Image>>,
    lights: Vec<Handle<Image>>,
    albedo_texels: Vec<Option<Vec<u8>>>,
    light_texels: Vec<Option<Vec<u8>>>,
    ready: Option<(Handle<Image>, Handle<Image>)>,
}

fn request_palette(
    refit: Option<Res<Refit>>,
    mut palette: ResMut<Palette>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut bakes: ResMut<Bakes>,
) {
    if refit.is_none() || palette.requested {
        return;
    }
    let plane = Target::new(Shape::Plane(TILE_TEXELS));
    let palette = &mut *palette;
    palette.requested = true;
    palette.albedo_texels = vec![None; REGION_GRAPHS.len()];
    palette.light_texels = vec![None; REGION_GRAPHS.len()];
    for kind in REGION_GRAPHS {
        let graph = assets.load(format!("textures/hull/{kind}.tgraph"));
        for (target, into) in [(plane.color(), &mut palette.albedo), (plane.layer("lights"), &mut palette.lights)] {
            let image = images.add(placeholder(target));
            bakes.request(graph.clone(), 1, target, image.clone());
            into.push(image);
        }
    }
}

/// In `Last`, after a bake lands and before extraction takes its bytes away.
fn take_texels(mut palette: ResMut<Palette>, mut images: ResMut<Assets<Image>>) {
    let palette = &mut *palette;
    if !palette.requested || palette.ready.is_some() {
        return;
    }
    for (handles, texels) in [(&palette.albedo, &mut palette.albedo_texels), (&palette.lights, &mut palette.light_texels)] {
        for (handle, slot) in handles.iter().zip(texels.iter_mut()) {
            let Some(image) = images.get(handle) else { continue };
            if slot.is_none() && image.width() == TILE_TEXELS {
                *slot = image.data.clone();
            }
        }
    }
    let (Some(albedo), Some(lights)) = (
        palette.albedo_texels.iter().cloned().collect::<Option<Vec<_>>>(),
        palette.light_texels.iter().cloned().collect::<Option<Vec<_>>>(),
    ) else {
        return;
    };
    palette.ready = Some((
        images.add(tile_array(&albedo, TILE_TEXELS, Tile::Albedo)),
        images.add(tile_array(&lights, TILE_TEXELS, Tile::Lights)),
    ));
}

/// One step's meshes, under one root in the ship's frame.
#[derive(Component)]
pub struct Generation {
    key: (usize, Option<usize>),
    shown: bool,
    copies: Vec<Sliver>,
}

struct Sliver {
    material: Handle<HullMaterial>,
    truss: Option<Task<Option<TrussBuffers>>>,
    /// Whether a girder mesh stands in for the lattice; `None` until its task lands.
    meshed: Option<bool>,
    /// How the step leaves the copy, drawn once the clock is past it and until the next step's
    /// meshes land: a part taken apart stays gone.
    end: Option<Sweep>,
}

/// What a mesh under a [`Generation`] is.
#[derive(Component, Clone, Copy)]
pub enum Drawn {
    Standing,
    /// A copy the step works on, or its truss.
    Working,
    /// A copy a move carries, meshed in its own frame and posed each frame.
    Carried(PartId, Side),
}

/// Whether a photograph should wait: something is under way that has not been drawn yet.
#[derive(Resource, Default)]
pub struct Unready(pub bool);

/// Whether a step's meshes are drawing the refit, which [`crate::parts`] stands aside for.
#[derive(Resource, Default)]
pub struct Showing(pub bool);

/// Draw the staged refit, replacing [`crate::parts::update_parts`]'s placeholders.
#[allow(clippy::too_many_arguments)]
pub fn draw_refit(
    mut commands: Commands,
    game: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    eye: Res<Eye>,
    own: Res<crate::parts::OwnForm>,
    refit: Option<Res<Refit>>,
    palette: Res<Palette>,
    mut unready: ResMut<Unready>,
    mut showing: ResMut<Showing>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut generations: Query<(Entity, &mut Generation, &mut Transform, &mut Visibility)>,
    mut drawn: Query<(&Drawn, &ChildOf, Option<&HullMeshState>, &mut Transform), Without<Generation>>,
) {
    let Some(refit) = refit.filter(|_| own.is_formed()) else {
        for (root, ..) in &generations {
            commands.entity(root).despawn();
        }
        (unready.0, showing.0) = (false, false);
        return;
    };
    unready.0 = true;
    let Some((albedo, light_tiles)) = palette.ready.clone() else { return };
    let session = &game.0;
    let frame = refit.frame(session.coordinate_time_s());
    let key = (frame.finished, frame.working.as_ref().map(|w| w.step));
    let placed = crate::parts::ship_frame(session, &eye, &ui);

    let star = lighting(session);
    let base = lit(session, star, session.ship.motion.position_ly, Vec4::ONE);
    let reference = base.exposure.x;
    let mut emitted = [Vec4::ZERO; REGIONS];
    for (slot, kind) in emitted.iter_mut().zip(REGION_GRAPHS) {
        *slot = (reference * lights(kind)).extend(0.0);
    }
    let finished = HullUniform {
        to_star: base.to_star,
        reflected: base.reflected / ALBEDO as f32,
        exposure: base.exposure,
        detail: Vec4::new(TILE_M, 0.0, 0.0, 0.0),
        bolted: 1 << region(Kind::Spar(SparMode::Saddle)),
        emitted,
        girder: GIRDER_ALBEDO.extend(WORK_LIGHTS * reference / GIRDER_ALBEDO.length()),
        ..default()
    };

    if !generations.iter().any(|(_, g, ..)| g.key == key) {
        let root = spawn(&mut commands, &frame, &refit.balance, key, &finished, &albedo, &light_tiles, &mut materials);
        commands.entity(root).insert(placed);
    }

    for (root, mut generation, mut transform, mut visibility) in &mut generations {
        *transform = placed;
        let current = generation.key == key;
        let working = frame.working.as_ref().filter(|_| current);
        for (copy, state) in generation.copies.iter_mut().enumerate() {
            if let Some(task) = state.truss.as_mut()
                && let Some(done) = check_ready(task)
            {
                state.truss = None;
                state.meshed = Some(done.is_some());
                if let Some(buffers) = done {
                    info!("refit_hull: copy {copy}'s truss is {} girders", buffers.count);
                    commands.spawn((
                        Mesh3d(meshes.add(buffers.into_mesh())),
                        MeshMaterial3d(state.material.clone()),
                        Transform::IDENTITY,
                        NoFrustumCulling,
                        RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
                        Drawn::Working,
                        ChildOf(root),
                    ));
                }
            }
            let Some(mut asset) = materials.get_mut(&state.material) else { continue };
            let sweep = if current { working.and_then(|w| w.sweep(copy)) } else { state.end };
            let next = building(&finished, sweep, state.meshed != Some(true));
            if asset.uniforms != next {
                asset.uniforms = next;
            }
        }
        if generation.key != key {
            continue;
        }
        let meshed = drawn
            .iter()
            .filter(|(_, parent, ..)| parent.parent() == root)
            .all(|(_, _, state, _)| state.is_none_or(HullMeshState::current));
        let trussed = generation.copies.iter().all(|c| c.meshed.is_some());
        if !generation.shown && meshed && trussed {
            generation.shown = true;
            *visibility = Visibility::Inherited;
        }
        unready.0 = !generation.shown;
    }
    showing.0 = generations.iter().any(|(_, g, ..)| g.shown);
    if generations.iter().any(|(_, g, ..)| g.key == key && g.shown) {
        for (root, generation, ..) in &generations {
            if generation.key != key {
                commands.entity(root).despawn();
            }
        }
    }
    for (drawn, _, _, mut transform) in &mut drawn {
        if let Drawn::Carried(part, side) = *drawn
            && let Some(piece) = frame.piece(part, side)
        {
            *transform = posed(&piece.pose);
        }
    }
}

/// A step's meshes, spawned hidden under a new root.
#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    frame: &Frame,
    balance: &Balance,
    key: (usize, Option<usize>),
    finished: &HullUniform,
    albedo: &Handle<Image>,
    light_tiles: &Handle<Image>,
    materials: &mut Assets<HullMaterial>,
) -> Entity {
    let mut material = |uniforms: HullUniform| {
        materials.add(HullMaterial { uniforms, albedo: albedo.clone(), lights: light_tiles.clone() })
    };
    let standing = standing(frame, balance);
    let finish = material(finished.clone());
    let working = frame.working.as_ref();
    let builds = working.filter(|w| w.change.phase() != Phase::Move);
    let riders: Arc<[Piece]> = working.map_or(Arc::from([]), |w| w.riders.clone().into());
    let copies: Vec<Sliver> = builds
        .map_or(&[][..], |w| &w.outer[..])
        .iter()
        .enumerate()
        .map(|(n, piece)| {
            let (sweep, end) = builds.map_or((None, None), |w| (w.sweep(n), w.sweep_at(n, 1.0)));
            let (piece, standing, riders) = (*piece, standing.clone(), riders.clone());
            let truss = sweep.map(|s| {
                AsyncComputeTaskPool::get().spawn(async move {
                    // Nor where what rides on the part will stand when it is done.
                    let riding = Union::new(&riders);
                    let scratch = std::cell::RefCell::new(Vec::new());
                    let field = |p: DVec3| {
                        let scratch = &mut *scratch.borrow_mut();
                        let on = if riders.is_empty() { f64::INFINITY } else { riding.distance(p, scratch) };
                        standing.distance(p, scratch).min(on)
                    };
                    let girders = truss::girders(&piece, &field)?;
                    Some(truss::mesh(&girders, s.joint, s.span_m))
                })
            });
            let meshed = sweep.is_none().then_some(false);
            Sliver { material: material(building(finished, sweep, true)), truss, meshed, end }
        })
        .collect();

    let root = commands.spawn(Visibility::Hidden).id();
    let mut hull = |source: HullSource, drawn: Drawn, material: Handle<HullMaterial>, at: Transform| {
        commands.spawn((
            HullForm { source, finish: Finish::Smooth, cells: None },
            MeshMaterial3d(material),
            at,
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            drawn,
            ChildOf(root),
        ));
    };
    hull(standing.source(), Drawn::Standing, finish.clone(), Transform::IDENTITY);
    if let Some(w) = working {
        if w.change.phase() != Phase::Move {
            for (n, piece) in w.outer.iter().enumerate() {
                hull(HullSource::Pieces(Arc::from([*piece])), Drawn::Working, copies[n].material.clone(), Transform::IDENTITY);
            }
        }
        for (part, side) in w.moving() {
            let Some(piece) = frame.piece(part, side) else { continue };
            let alone = Piece { pose: Pose { position: DVec3::ZERO, rotation: glam::DMat3::IDENTITY }, ..*piece };
            hull(HullSource::Pieces(Arc::from([alone])), Drawn::Carried(part, side), finish.clone(), posed(&piece.pose));
        }
    }
    commands.entity(root).insert(Generation { key, shown: false, copies });
    root
}

/// What stands finished through a step, as a form where it places on its own and as its bare
/// copies where it hangs from a part the round has not built.
#[derive(Clone)]
enum Standing {
    Form(Arc<Sdf>, Arc<Form>, Balance),
    Pieces(Arc<[Piece]>),
}

fn standing(frame: &Frame, balance: &Balance) -> Standing {
    match Sdf::new(&frame.standing, balance) {
        Ok(sdf) => Standing::Form(Arc::new(sdf), Arc::new(frame.standing.clone()), *balance),
        Err(_) => Standing::Pieces(frame.standing_pieces().into()),
    }
}

impl Standing {
    fn source(&self) -> HullSource {
        match self {
            Standing::Form(_, form, balance) => HullSource::Form(form.clone(), *balance),
            Standing::Pieces(pieces) => HullSource::Pieces(pieces.clone()),
        }
    }

    fn distance(&self, p: DVec3, scratch: &mut Vec<f64>) -> f64 {
        match self {
            Standing::Form(sdf, ..) => sdf.distance_with(p, scratch),
            Standing::Pieces(pieces) if pieces.is_empty() => f64::INFINITY,
            Standing::Pieces(pieces) => Union::new(pieces).distance(p, scratch),
        }
    }
}

/// The material for a copy at one moment of its step. `on_surface` draws the lattice on the copy
/// itself, for a sliver too large to mesh the truss of.
fn building(finished: &HullUniform, sweep: Option<Sweep>, on_surface: bool) -> HullUniform {
    let Some(s) = sweep else { return finished.clone() };
    let [truss, plating, fitting, down] = s.fronts_m.map(|x| x as f32);
    let [wt, wp, wf, wd] = s.widths_m.map(|x| x as f32);
    HullUniform {
        reveal: s.joint.as_vec3().extend(plating.min(ALL_PLATED * 0.5)),
        reveal_panel: Vec4::new(PITCH_M as f32, wp, 0.0, 0.0),
        build_fronts: Vec4::new(truss, fitting, down, s.span_m as f32),
        build_widths: Vec4::new(wt, wf, wd, truss::LAYERS as f32),
        lattice: Vec4::new(PITCH_M as f32, GIRDER_RADIUS_M as f32, if on_surface { 1.0 } else { 0.0 }, BARE),
        ..finished.clone()
    }
}

/// A piece's pose as a transform in the ship's frame, for a mesh built in the piece's own.
fn posed(pose: &Pose) -> Transform {
    Transform {
        translation: pose.position.as_vec3(),
        rotation: Quat::from_mat3(&pose.rotation.as_mat3()),
        scale: Vec3::ONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::{Clock, Working, demo_round};

    const B: Balance = Balance::DEFAULT;

    /// The material agrees with [`Working::look`] everywhere across the sliver, through every step
    /// that builds or takes apart: a band's share in the shader's terms is the look's.
    #[test]
    fn the_uniforms_carry_the_look() {
        let plan = demo_round(&B, 1.0).solve(&B).unwrap();
        let share = |front: f32, width: f32, x: f32| ((front - x) / width).clamp(0.0, 1.0);
        let mut checked = 0;
        for step in plan.steps() {
            for i in 1..20 {
                let frame = Frame::at(&plan, &B, step.begins_s + step.duration_s * i as f64 / 20.0);
                let working: &Working = frame.working.as_ref().unwrap();
                let Some(sweep) = working.sweep(0) else { continue };
                let u = building(&HullUniform::default(), Some(sweep), false);
                for j in 0..=20 {
                    let d = j as f64 / 20.0;
                    let x = (d * sweep.span_m) as f32;
                    let look = working.look(d);
                    let truss = share(u.build_fronts.x, u.build_widths.x, x);
                    let plating = share(u.reveal.w, u.reveal_panel.y, x);
                    let fitted = share(u.build_fronts.y, u.build_widths.y, x);
                    let scaffold = truss * (1.0 - share(u.build_fronts.z, u.build_widths.z, x));
                    for (a, b) in [(truss, look.truss), (plating, look.plating), (fitted, look.fitted), (scaffold, look.scaffold)] {
                        assert!((a as f64 - b).abs() < 1e-4, "{:?} at {i}/20, {d} across: {a} against {b}", step.change);
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    /// Past its end, a step's copies are drawn as it left them: a dismantled part discards every
    /// point, girders included, and a built one is finished.
    #[test]
    fn a_step_passed_is_drawn_as_it_ended() {
        let plan = demo_round(&B, 1.0).solve(&B).unwrap();
        let share = |front: f32, width: f32, x: f32| ((front - x) / width).clamp(0.0, 1.0);
        let mut dismantles = 0;
        for step in plan.steps() {
            let frame = Frame::at(&plan, &B, step.begins_s + 0.5 * step.duration_s);
            let working = frame.working.as_ref().unwrap();
            let Some(end) = working.sweep_at(0, 1.0) else { continue };
            let u = building(&HullUniform::default(), Some(end), false);
            for j in 0..=20 {
                let x = end.span_m as f32 * j as f32 / 20.0;
                let truss = share(u.build_fronts.x, u.build_widths.x, x);
                let plating = share(u.reveal.w, u.reveal_panel.y, x);
                match step.change.phase() {
                    Phase::Dismantle => assert!(truss == 0.0 && plating == 0.0, "{x}: {truss} {plating}"),
                    _ => assert_eq!(end.look(x as f64), crate::construction::Look::FINISHED),
                }
            }
            dismantles += usize::from(step.change.phase() == Phase::Dismantle);
        }
        assert!(dismantles > 0);
    }

    /// What hangs from the growing hull rides out on it: it is out of what stands, posed from the
    /// frame, and arrives where the grown hull's form places it.
    #[test]
    fn what_hangs_from_a_growing_part_rides_out_on_it() {
        let plan = demo_round(&B, 1.0).solve(&B).unwrap();
        let step = plan.steps().iter().find(|s| s.change == lc_world::refit::rounds::Change::Grow).unwrap();
        let at = |f: f64| Frame::at(&plan, &B, step.begins_s + f * step.duration_s);
        let (early, late) = (at(0.1), at(1.0 - 1e-9));
        let riders = &early.working.as_ref().unwrap().riders;
        assert!(riders.len() >= 3, "{riders:?}");
        for rider in riders {
            assert!(early.standing.parts.iter().all(|p| p.id != rider.part), "{:?} stands still", rider.part);
            assert!(early.standing_pieces().iter().all(|p| p.part != rider.part));
            let (a, b) = (early.piece(rider.part, rider.side).unwrap().pose, late.piece(rider.part, rider.side).unwrap().pose);
            assert!(a.position.distance(b.position) > 1.0, "{:?} did not ride", rider.part);
            assert!(b.position.distance(rider.pose.position) < 1e-3, "{:?} ends apart from the grown form", rider.part);
        }
    }

    /// The truss the demo's grown hull puts up is meshed, and within the budget by a margin.
    #[test]
    fn the_starting_hulls_truss_is_meshed() {
        let plan = demo_round(&B, 1.0).solve(&B).unwrap();
        let refit = Refit { plan, balance: B, clock: Clock::Frozen(0.5), canceled: None };
        let frame = refit.frame(0.0);
        let working = frame.working.as_ref().unwrap();
        let standing = standing(&frame, &B);
        assert!(matches!(standing, Standing::Form(..)));
        let field = |p: DVec3| standing.distance(p, &mut Vec::new());
        let girders = truss::girders(&working.outer[0], &field).expect("a starting hull's truss meshes");
        assert!((1_000..truss::MAX_GIRDERS / 2).contains(&girders.len()), "{}", girders.len());
    }
}
