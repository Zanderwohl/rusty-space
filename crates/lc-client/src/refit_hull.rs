//! A refit drawn over the real hull (R10) with the truss (R8): the construction overlay.
//!
//! Plating is a mask on R3's material, so while a craft has a round the whole craft is meshed here
//! as the step leaves it, and [`crate::ship_hull`] stands its hull aside once the first step's
//! meshes are shown. The player's round is its [`Refit`], `--demo refit`'s or the one in
//! `Fitted`; another craft's is the step its light shows, from [`crate::construction::sighted`].
//! When the round is over the last step's meshes stay up until that hull is the form the round
//! left, so the craft never flashes back to an earlier shape or to placeholders.
//!
//! A step is meshed once, as it starts: the craft it leaves alone, each copy it works on at the
//! larger of its two sizes, and each copy's truss. Within the step only uniforms move, each read
//! from [`Working::sweep`], so a paused clock draws the same frame twice. A step's meshes are shown
//! when all of them have landed, and the last step's stay up until then. They hang under the
//! craft's [`ShipHull`] root, which places them and takes them down with it.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::hull_material::{ALL_PLATED, HullMaterial, HullUniform};
use glam::DVec3;
use lc_proto::ShipId;
use lc_world::fitting::Balance;
use lc_world::form::place::{Pose, Side};
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Form, PartId};

use crate::construction::{Frame, Refit, Sweep};
use lc_world::refit::rounds::Phase;
use crate::hull::lighting;
use crate::hull_mesh::{Finish, HullForm, HullMeshState, HullSource, Union, form_hash};
use crate::session::Session;
use crate::ship_hull::{Palette, RealHulls, ShipHull};
use crate::surface_nets::Field;
use crate::truss::{self, GIRDER_RADIUS_M, PITCH_M, TrussBuffers};

/// Girders are painted safety yellow and lit by their own work lights, so a frontier too far off
/// to show a girder still reads as construction by its color.
const GIRDER_ALBEDO: Vec3 = Vec3::new(0.8, 0.52, 0.1);
/// The girders' work lights, lux. Only their luminance is used; the albedo carries the color.
const WORK_LUX: f64 = 2000.0;
const WORK_K: f64 = 4000.0;
/// Plating before it is fitted out.
const BARE: f32 = 0.45;
/// How long a round's last meshes wait for the real hull to be the form the round left, seconds of
/// wall time. Past it the hull is shown as it is, which is a `Fitted` that never agreed.
const HAND_BACK_S: f32 = 5.0;

pub struct RefitHullPlugin;

impl Plugin for RefitHullPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Unready>().init_resource::<Showing>();
    }
}

/// One step's meshes of one craft, under one root in its frame.
#[derive(Component)]
pub struct Generation {
    craft: Option<ShipId>,
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

/// The craft whose refit meshes are drawn, which their real hulls and, for the player's,
/// [`crate::parts`] stand aside for.
#[derive(Resource, Default)]
pub struct Showing(HashSet<Option<ShipId>>);

impl Showing {
    pub fn drawing(&self, craft: Option<ShipId>) -> bool {
        self.0.contains(&craft)
    }
}

/// The form a round leaves a craft in, and since when it has been waiting for the real hull.
#[derive(Default)]
pub struct Ending {
    form: Option<u64>,
    waited_s: f32,
}

/// What a round's last meshes do once the round is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandBack {
    /// There are none to take down.
    Nothing,
    /// Stay up: the real hull is not yet the form the round left.
    Keep,
    Handed,
    /// Waited [`HAND_BACK_S`] for a hull that never agreed.
    GaveUp,
}

/// `ending` is the [`form_hash`] the round left, `current` the one the real hull is current in,
/// and `waited_s` how long since the round was over.
fn hand_back(ending: Option<u64>, current: Option<u64>, waited_s: f32, meshes_up: bool) -> HandBack {
    if !meshes_up {
        HandBack::Nothing
    } else if ending.is_none_or(|form| current == Some(form)) {
        HandBack::Handed
    } else if waited_s > HAND_BACK_S {
        HandBack::GaveUp
    } else {
        HandBack::Keep
    }
}

pub(crate) struct Building {
    pub craft: Option<ShipId>,
    pub frame: Frame,
    pub balance: Balance,
    pub at_ly: DVec3,
}

/// Contacts are solved with the player's `balance`, as [`crate::ship_hull`] solves their forms.
pub(crate) fn buildings(
    session: &Session,
    refit: Option<&Refit>,
    contacts: &[crate::uplink::Contact],
    balance: &Balance,
) -> Vec<Building> {
    let own = refit.map(|r| Building {
        craft: None,
        frame: r.frame(session.coordinate_time_s()),
        balance: r.balance,
        at_ly: session.ship.motion.position_ly,
    });
    let others = contacts.iter().filter(|c| !c.form.parts.is_empty()).filter_map(|c| {
        Some(Building { craft: Some(c.ship_id), frame: crate::construction::sighted(c, balance)?, balance: *balance, at_ly: c.position_ly })
    });
    own.into_iter().chain(others).collect()
}

/// A finished hull at `at_ly` with the girders' work lights.
fn uniforms(session: &Session, star: Option<(DVec3, f64, f64)>, at_ly: DVec3, glow: lc_proto::Glow) -> HullUniform {
    let work = crate::hull::lamp(session, WORK_LUX / std::f64::consts::PI, WORK_K);
    HullUniform {
        girder: GIRDER_ALBEDO.extend(work.dot(crate::tonemap::LUMA)),
        ..crate::ship_hull::finished(session, star, at_ly, glow)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_refit(
    mut commands: Commands,
    (game, uplink, own, refit): (Res<crate::app::Game>, Res<crate::uplink::Uplink>, Res<crate::parts::OwnForm>, Option<Res<Refit>>),
    (palette, real, time): (Res<Palette>, Res<RealHulls>, Res<Time<Real>>),
    mut endings: Local<HashMap<Option<ShipId>, Ending>>,
    mut unready: ResMut<Unready>,
    mut showing: ResMut<Showing>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    hulls: Query<(Entity, &ShipHull)>,
    mut generations: Query<(Entity, &mut Generation, &mut Visibility)>,
    mut drawn: Query<(&Drawn, &ChildOf, Option<&HullMeshState>, &mut Transform), Without<Generation>>,
) {
    let session = &game.0;
    let star = lighting(session);
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let refit_changed = refit.as_ref().is_some_and(|r| r.is_changed());
    let refit = refit.as_deref().filter(|_| own.is_formed());
    let now = buildings(session, refit, &uplink.contacts, &balance);
    unready.0 = false;
    showing.0.clear();

    // Craft whose round is over, until their real hull is the form it left.
    let idle: HashSet<Option<ShipId>> =
        generations.iter().map(|(_, g, _)| g.craft).filter(|c| now.iter().all(|b| b.craft != *c)).collect();
    endings.retain(|craft, _| idle.contains(craft) || now.iter().any(|b| b.craft == *craft));
    for craft in idle {
        let ending = endings.entry(craft).or_default();
        ending.waited_s += time.delta_secs();
        // Another craft's round has left it in the form it is stated in.
        let left = if craft.is_none() { ending.form } else { real.stated(craft) };
        let decided = hand_back(left, real.current(craft), ending.waited_s, true);
        if decided != HandBack::Keep {
            match decided {
                HandBack::Handed => info!("refit_hull: {craft:?} handed back to the real hull after {:.2} s", ending.waited_s),
                HandBack::GaveUp => warn!("refit_hull: {craft:?}'s real hull never became the form the round left"),
                HandBack::Nothing | HandBack::Keep => {}
            }
            for (root, generation, _) in &generations {
                if generation.craft == craft {
                    commands.entity(root).despawn();
                }
            }
            endings.remove(&craft);
            continue;
        }
        let at_ly = uplink.contacts.iter().find(|c| Some(c.ship_id) == craft).map_or(session.ship.motion.position_ly, |c| c.position_ly);
        let finished = uniforms(session, star, at_ly, crate::hull::glow_of(session, &uplink, craft));
        for (_, generation, _) in generations.iter().filter(|(_, g, _)| g.craft == craft) {
            for state in &generation.copies {
                if let Some(mut asset) = materials.get_mut(&state.material) {
                    asset.uniforms = building(&finished, state.end, state.meshed != Some(true));
                }
            }
            if generation.shown {
                showing.0.insert(craft);
            }
        }
    }

    for b in &now {
        let ending = endings.entry(b.craft).or_default();
        ending.waited_s = 0.0;
        if b.craft.is_none()
            && let Some(refit) = refit
            && (refit_changed || ending.form.is_none())
        {
            let left = refit.canceled.as_ref().map_or(refit.plan.target(), |(_, left)| left);
            ending.form = Some(form_hash(left, &refit.balance));
        }
        let (Some((albedo, light_tiles)), Some((hull, _))) = (palette.ready(), hulls.iter().find(|(_, h)| h.craft() == b.craft)) else {
            unready.0 = true;
            continue;
        };
        let finished = uniforms(session, star, b.at_ly, crate::hull::glow_of(session, &uplink, b.craft));
        let key = (b.frame.finished, b.frame.working.as_ref().map(|w| w.step));
        let ours = |g: &Generation| g.craft == b.craft;

        if !generations.iter().any(|(_, g, _)| ours(g) && g.key == key) {
            let root = spawn(&mut commands, b, key, &finished, &albedo, &light_tiles, &mut materials);
            commands.entity(root).insert(ChildOf(hull));
        }

        let mut current_shown = false;
        for (root, mut generation, mut visibility) in &mut generations {
            if !ours(&generation) {
                continue;
            }
            let current = generation.key == key;
            let working = b.frame.working.as_ref().filter(|_| current);
            for (copy, state) in generation.copies.iter_mut().enumerate() {
                if let Some(task) = state.truss.as_mut()
                    && let Some(done) = check_ready(task)
                {
                    state.truss = None;
                    state.meshed = Some(done.is_some());
                    if let Some(buffers) = done {
                        info!("refit_hull: {:?}'s copy {copy}'s truss is {} girders", b.craft, buffers.count);
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
            if !current {
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
            current_shown = generation.shown;
        }
        unready.0 |= !current_shown;
        if generations.iter().any(|(_, g, _)| ours(g) && g.shown) {
            showing.0.insert(b.craft);
        }
        if current_shown {
            for (root, generation, _) in &generations {
                if ours(generation) && generation.key != key {
                    commands.entity(root).despawn();
                }
            }
        }
    }

    let frames: HashMap<Entity, &Frame> = generations
        .iter()
        .filter_map(|(root, g, _)| Some((root, &now.iter().find(|b| b.craft == g.craft)?.frame)))
        .collect();
    for (drawn, parent, _, mut transform) in &mut drawn {
        if let Drawn::Carried(part, side) = *drawn
            && let Some(piece) = frames.get(&parent.parent()).and_then(|f| f.piece(part, side))
        {
            *transform = posed(&piece.pose);
        }
    }
}

/// A step's meshes, spawned hidden under a new root.
#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    b: &Building,
    key: (usize, Option<usize>),
    finished: &HullUniform,
    albedo: &Handle<Image>,
    light_tiles: &Handle<Image>,
    materials: &mut Assets<HullMaterial>,
) -> Entity {
    let mut material = |uniforms: HullUniform| {
        materials.add(HullMaterial { uniforms, albedo: albedo.clone(), lights: light_tiles.clone() })
    };
    let (frame, balance) = (&b.frame, &b.balance);
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

    let root = commands.spawn((Transform::IDENTITY, Visibility::Hidden)).id();
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
    commands.entity(root).insert(Generation { craft: b.craft, key, shown: false, copies });
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

    /// A round's last meshes stay up until the real hull is the form the round left, and no
    /// longer than [`HAND_BACK_S`] if it never is.
    #[test]
    fn the_last_meshes_wait_for_the_real_hull() {
        let (left, earlier) = (Some(7), Some(3));
        assert_eq!(hand_back(left, None, 0.1, true), HandBack::Keep, "not meshed yet");
        assert_eq!(hand_back(left, earlier, 0.1, true), HandBack::Keep, "meshed in the form before the round");
        assert_eq!(hand_back(left, left, 0.1, true), HandBack::Handed);
        assert_eq!(hand_back(left, earlier, HAND_BACK_S + 0.1, true), HandBack::GaveUp);
        assert_eq!(hand_back(left, earlier, 0.1, false), HandBack::Nothing);
        assert_eq!(hand_back(None, None, 0.0, true), HandBack::Handed, "no round was seen to end");
    }

    /// Another craft's round is meshed under that craft's own root, at the step its light shows
    /// rather than the one it is on now, and nothing of it is shown before its meshes land.
    #[test]
    fn another_crafts_round_is_meshed_for_it_as_its_light_left() {
        use lc_world::sky::AuthoredStars;
        let plan = demo_round(&B, 1.0).solve(&B).unwrap();
        let i = plan.steps().iter().position(|s| s.change == lc_world::refit::rounds::Change::Grow).unwrap();
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
            ship_id: ShipId(9),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            jet_power_w: 0.0,
            emitted_t: (stated_s * 1.0e6) as i64,
            arrive_t: (stated_s * 1.0e6) as i64 + 3_600_000_000,
            form: (&plan.at(stated_s).form).into(),
            building: Some(under.into()),
            glow: None,
            glare: None,
        };
        let mut uplink = crate::uplink::Uplink::default();
        uplink.contacts = vec![crate::uplink::Contact::seen(presence, None)];
        uplink.contacts[0].emitted_s = stated_s + 0.25 * step.duration_s;

        let mut session = crate::session::Session::new(&AuthoredStars::sample(), 3);
        // Long after the round is over where the craft is now.
        session.set_coordinate_time_us(((stated_s + 10.0 * plan.duration_s()) * 1.0e6) as i64);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::app::Game(session))
            .insert_resource(uplink)
            .init_resource::<crate::parts::OwnForm>()
            .insert_resource(Palette::baked())
            .init_resource::<RealHulls>()
            .init_resource::<Unready>()
            .init_resource::<Showing>()
            .init_resource::<Assets<HullMaterial>>()
            .init_resource::<Assets<Mesh>>()
            .add_systems(Update, draw_refit);
        let world = app.world_mut();
        let mesh = world.spawn_empty().id();
        let root = world.spawn(ShipHull::bare(Some(ShipId(9)), mesh)).id();
        app.update();
        app.update();

        let world = app.world_mut();
        let generations: Vec<_> = world.query::<(Entity, &Generation, &ChildOf)>().iter(world).map(|(e, g, c)| (e, g.craft, g.key, c.parent())).collect();
        assert_eq!(generations.len(), 1, "{generations:?}");
        let (generation, craft, key, parent) = generations[0];
        assert_eq!((craft, parent), (Some(ShipId(9)), root));
        assert_eq!(key, (i, Some(i)), "drawn at the step its light shows, not the one it is on now");
        let hulls = world.query::<(&HullForm, &ChildOf)>().iter(world).filter(|(_, c)| c.parent() == generation).count();
        assert!(hulls >= 2, "the standing craft and its working copy: {hulls}");
        assert!(!world.resource::<Showing>().drawing(Some(ShipId(9))), "shown before its meshes landed");
        assert!(world.resource::<Unready>().0);

        // Its round over: the last meshes wait for its hull to be the form it is stated in.
        world.resource_mut::<crate::uplink::Uplink>().contacts[0].building = None;
        world.resource_mut::<RealHulls>().set(Some(ShipId(9)), 7, 3);
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&Generation>().iter(world).count(), 1, "handed back to a hull in an earlier form");
        world.resource_mut::<RealHulls>().set(Some(ShipId(9)), 7, 7);
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&Generation>().iter(world).count(), 0, "kept once its hull caught up");
        assert!(!world.resource::<Showing>().drawing(Some(ShipId(9))));
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
