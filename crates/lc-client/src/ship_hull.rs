//! Every craft with a form drawn as its real hull: R2's mesh in R3's material. 32 §The hull.
//!
//! The player's own from [`crate::parts::OwnForm`], which follows `Fitted`, and everyone else's
//! from the form their `Presence` stated. A hull is meshed at the resolution its pixels on screen
//! ask for, on the async pool, and until its first mesh lands it is drawn as placeholders: the
//! player's by [`crate::parts`], anyone else's here. After that a new form or band keeps the old
//! mesh up until the new one lands.
//!
//! A refit is drawn over this by [`crate::refit_hull`], which stands the player's hull aside
//! while a step's meshes are shown and hands the ship back once this hull is the form the round
//! left.
//!
//! Meshes are shared by form, resolution and finish, so a sky of one design costs one mesh per
//! band. A hull is about 64 bytes a vertex with its indices; at 256 cells the starting form is
//! about 32 MB, at 64 cells about 2 MB, and a distant ship is 16 cells and a few hundred kB.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::hull_material::{HullMaterial, HullMaterialPlugin, HullUniform, REGIONS, Tile, tile_array};
use em_render::render_space::sim_to_render;
use glam::DVec3;
use lc_proto::ShipId;
use lc_world::fitting::Balance;
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Form, Kind, SparMode};

use crate::hull::{ALBEDO, Eye, frame, lighting, lit};
use crate::hull_mesh::{Finish, HullForm, HullMeshPlugin, HullMeshState, HullSource, REGION_GRAPHS, form_hash, region};
use crate::procedural::{Bakes, Shape, Target, placeholder};
use crate::session::Session;
use crate::system::UNIT_M;

/// As `examples/hull_void.rs`: a graph's unit square is this many meters, in this many texels.
pub(crate) const TILE_M: f32 = 64.0;
const TILE_TEXELS: u32 = 512;

/// Rolls kept for forms no craft is drawn in any more.
const ROLLS_KEPT: usize = 64;

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

pub struct ShipHullPlugin;

impl Plugin for ShipHullPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((HullMaterialPlugin, HullMeshPlugin))
            .init_resource::<Palette>()
            .init_resource::<Rolls>()
            .init_resource::<RealHulls>()
            .add_systems(Update, request_palette.run_if(in_state(crate::app::AppState::InGame)))
            .add_systems(Last, take_texels);
    }
}

/// The hull graphs baked into one palette, once the game starts.
#[derive(Resource, Default)]
pub struct Palette {
    requested: bool,
    albedo: Vec<Handle<Image>>,
    lights: Vec<Handle<Image>>,
    albedo_texels: Vec<Option<Vec<u8>>>,
    light_texels: Vec<Option<Vec<u8>>>,
    ready: Option<(Handle<Image>, Handle<Image>)>,
}

impl Palette {
    /// The albedo and lights tile arrays, once baked.
    pub fn ready(&self) -> Option<(Handle<Image>, Handle<Image>)> {
        self.ready.clone()
    }
}

fn request_palette(
    mut palette: ResMut<Palette>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut bakes: ResMut<Bakes>,
) {
    if palette.requested {
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

/// A finished hull at `at_ly`, lit by `star`.
pub(crate) fn finished(session: &Session, star: Option<(DVec3, f64, f64)>, at_ly: DVec3) -> HullUniform {
    let base = lit(session, star, at_ly, Vec4::ONE);
    let reference = base.exposure.x;
    let mut emitted = [Vec4::ZERO; REGIONS];
    for (slot, kind) in emitted.iter_mut().zip(REGION_GRAPHS) {
        *slot = (reference * lights(kind)).extend(0.0);
    }
    HullUniform {
        to_star: base.to_star,
        reflected: base.reflected / ALBEDO as f32,
        exposure: base.exposure,
        detail: Vec4::new(TILE_M, 0.0, 0.0, 0.0),
        bolted: 1 << region(Kind::Spar(SparMode::Saddle)),
        emitted,
        ..default()
    }
}

/// The roll each form presents its broadside at, from the same grid the server measures, worked
/// out on the async pool: a grid is too slow for a frame. Keyed by [`form_hash`].
#[derive(Resource, Default)]
pub struct Rolls {
    ready: HashMap<u64, f64>,
    pending: HashMap<u64, Task<f64>>,
}

impl Rolls {
    fn get(&mut self, hash: u64, form: &Arc<Form>, balance: Balance) -> Option<f64> {
        if let Some(&roll) = self.ready.get(&hash) {
            return Some(roll);
        }
        if !self.pending.contains_key(&hash) {
            let form = form.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                match lc_world::form::grid::FormGrid::new(&form, &balance) {
                    Ok(grid) => lc_world::solar::roll_rad(&grid.geometry(1.0)),
                    Err(_) => 0.0,
                }
            });
            self.pending.insert(hash, task);
        }
        None
    }

    fn land(&mut self, wanted: &HashSet<u64>) {
        let ready = &mut self.ready;
        self.pending.retain(|&hash, task| match check_ready(task) {
            Some(roll) => {
                ready.insert(hash, roll);
                false
            }
            None => true,
        });
        if ready.len() > ROLLS_KEPT {
            ready.retain(|hash, _| wanted.contains(hash));
        }
    }
}

/// Which craft's real hulls have a mesh up, and the form the player's is current in. Read by
/// [`crate::parts`], which draws placeholders only for a craft missing here, and by
/// [`crate::refit_hull`] to hand the ship back after a round.
#[derive(Resource, Default)]
pub struct RealHulls {
    drawn: HashSet<Option<ShipId>>,
    own_current: Option<u64>,
}

impl RealHulls {
    pub fn drawn(&self, craft: Option<ShipId>) -> bool {
        self.drawn.contains(&craft)
    }

    /// The [`form_hash`] the player's hull is drawn in, once its mesh is the one wanted.
    pub fn own_current(&self) -> Option<u64> {
        self.own_current
    }
}

/// One craft's hull: a root placed in its frame each frame, with the mesh under it and, until
/// that has landed, placeholders beside it.
#[derive(Component)]
pub struct ShipHull {
    craft: Option<ShipId>,
    /// The form handed to the mesher, and the roll it is drawn at once its mesh is up.
    form: Option<(u64, f64)>,
    /// The roll the mesh on screen was drawn at: a new form's is taken only as its mesh lands, so
    /// the old shape never turns to the new one's roll.
    shown_roll: f64,
    /// The last rotation, for a craft stated with no facing.
    rotation: Quat,
    mesh: Entity,
    placeholders: Option<(u64, Entity)>,
}

/// The mesh under a [`ShipHull`]. It has no [`HullForm`] until the form's roll is known.
#[derive(Component)]
pub struct HullMesh;

/// A placeholder part of another craft, in its paint, shown until the craft's mesh lands.
#[derive(Component)]
pub struct Placeholder(Vec4);

/// A form as decoded and hashed, kept while it is still the one stated.
#[derive(Clone)]
struct Decoded {
    form: Arc<Form>,
    hash: u64,
}

/// Each craft's last stated form, decoded, so an unchanged one is not decoded and hashed again
/// every frame.
#[derive(Default)]
pub struct Stated {
    own: Option<Decoded>,
    contacts: HashMap<ShipId, (lc_proto::Form, Decoded)>,
}

/// One craft with a form, this frame.
struct Wanted {
    craft: Option<ShipId>,
    form: Decoded,
    at_ly: DVec3,
    placed: Transform,
    facing: DVec3,
}

/// Draw every craft with a form as its real hull, and the player's own while no refit stands it
/// aside.
#[allow(clippy::too_many_arguments)]
pub fn draw_hulls(
    mut commands: Commands,
    (game, ui, uplink, eye): (Res<crate::app::Game>, Res<crate::app::Ui>, Res<crate::uplink::Uplink>, Res<Eye>),
    own: Res<crate::parts::OwnForm>,
    (palette, showing, surfaces): (Res<Palette>, Res<crate::refit_hull::Showing>, Res<crate::surfaces::Surfaces>),
    mut stated: Local<Stated>,
    mut rolls: ResMut<Rolls>,
    mut real: ResMut<RealHulls>,
    mut unready: ResMut<crate::refit_hull::Unready>,
    (mut materials, mut flat, mut meshes): (ResMut<Assets<HullMaterial>>, ResMut<Assets<BodySurfaceMaterial>>, ResMut<Assets<Mesh>>),
    mut hulls: Query<(Entity, &mut ShipHull, &mut Transform)>,
    mut drawn: Query<
        (Option<&mut HullForm>, Option<&HullMeshState>, Has<Mesh3d>, &MeshMaterial3d<HullMaterial>, &mut Visibility),
        With<HullMesh>,
    >,
    placeholders: Query<(&Placeholder, &MeshMaterial3d<BodySurfaceMaterial>)>,
    children: Query<&Children>,
) {
    real.drawn.clear();
    real.own_current = None;
    let Some((albedo, light_tiles)) = palette.ready() else {
        unready.0 |= own.is_formed() || uplink.contacts.iter().any(|c| !c.form.parts.is_empty());
        return;
    };
    let session = &game.0;
    let star = lighting(session);
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let wanted = wanted(&mut stated, session, &uplink, &eye, &ui, &own, balance);
    rolls.land(&wanted.iter().map(|w| w.form.hash).collect());

    for (entity, hull, _) in &hulls {
        if !wanted.iter().any(|w| w.craft == hull.craft) {
            commands.entity(entity).despawn();
        }
    }
    for want in &wanted {
        let uniforms = finished(session, star, want.at_ly);
        let Some((root, mut hull, mut transform)) = hulls.iter_mut().find(|(_, h, _)| h.craft == want.craft) else {
            let material = materials.add(HullMaterial { uniforms, albedo: albedo.clone(), lights: light_tiles.clone() });
            spawn(&mut commands, want, material);
            unready.0 = true;
            continue;
        };
        let Ok((form, state, has_mesh, material, mut visibility)) = drawn.get_mut(hull.mesh) else { continue };

        // Another craft's roll is worked out before its form is handed over, so the mesh and the
        // roll it is drawn at arrive together. The player's is the fitting's.
        if hull.form.is_none_or(|(hash, _)| hash != want.form.hash) {
            let roll = match want.craft {
                None => Some(0.0),
                Some(_) => rolls.get(want.form.hash, &want.form.form, balance),
            };
            if let Some(roll) = roll {
                hull.form = Some((want.form.hash, roll));
                let source = HullSource::Form(want.form.form.clone(), balance);
                match form {
                    Some(mut form) => form.source = source,
                    None => {
                        commands.entity(hull.mesh).insert(HullForm { source, finish: Finish::Smooth, cells: None });
                    }
                }
            }
        }
        let current = state.is_some_and(|s| s.current() && hull.form.is_some_and(|(hash, _)| hash == s.form()));
        if current && let Some((_, roll)) = hull.form {
            hull.shown_roll = roll;
        }

        *transform = match want.craft {
            None => want.placed,
            Some(_) => {
                let to_star = star.map(|(star_ly, _, _)| star_ly - want.at_ly);
                if want.facing != DVec3::ZERO {
                    hull.rotation = frame(want.facing, to_star, hull.shown_roll);
                }
                Transform { rotation: hull.rotation, ..want.placed }
            }
        };
        if let Some(mut asset) = materials.get_mut(&material.0)
            && asset.uniforms != uniforms
        {
            asset.uniforms = uniforms;
        }
        let aside = want.craft.is_none() && showing.0;
        *visibility = if has_mesh && !aside { Visibility::Inherited } else { Visibility::Hidden };
        if has_mesh {
            real.drawn.insert(want.craft);
        }
        if want.craft.is_none() && current {
            real.own_current = hull.form.map(|(hash, _)| hash);
        }
        // A mesh that failed is settled too: its placeholders stay.
        unready.0 |= !has_mesh && !state.is_some_and(HullMeshState::current);

        // The player's placeholders are `crate::parts`'s, which draw a refit's as well.
        let wants_placeholders = want.craft.is_some() && !has_mesh;
        match hull.placeholders {
            Some((hash, parts)) if wants_placeholders && hash == want.form.hash => {
                for child in children.get(parts).into_iter().flatten() {
                    let Ok((Placeholder(paint), material)) = placeholders.get(*child) else { continue };
                    let next = lit(session, star, want.at_ly, *paint);
                    if let Some(mut asset) = flat.get_mut(&material.0)
                        && asset.uniforms != next
                    {
                        asset.uniforms = next;
                    }
                }
            }
            _ if wants_placeholders => {
                if let Some((_, old)) = hull.placeholders.take() {
                    commands.entity(old).despawn();
                }
                let paint = |kind| lit(session, star, want.at_ly, crate::parts::paint(kind));
                let parts = spawn_placeholders(&mut commands, &want.form.form, balance, &mut meshes, &mut flat, &surfaces, paint);
                commands.entity(parts).insert(ChildOf(root));
                hull.placeholders = Some((want.form.hash, parts));
            }
            Some((_, old)) => {
                commands.entity(old).despawn();
                hull.placeholders = None;
            }
            None => {}
        }
    }
}

/// The player's ship and every contact stated with a form.
fn wanted(
    stated: &mut Stated,
    session: &Session,
    uplink: &crate::uplink::Uplink,
    eye: &Eye,
    ui: &crate::app::Ui,
    own: &Res<crate::parts::OwnForm>,
    balance: Balance,
) -> Vec<Wanted> {
    let look = ui.look.forward();
    let mut out = Vec::with_capacity(uplink.contacts.len() + 1);
    match own.form() {
        Some(form) => {
            if own.is_changed() || stated.own.is_none() {
                let hash = form_hash(form, &own.balance());
                stated.own = Some(Decoded { form: Arc::new(form.clone()), hash });
            }
            let decoded = stated.own.as_ref().expect("just decoded");
            out.push(Wanted {
                craft: None,
                form: decoded.clone(),
                at_ly: session.ship.motion.position_ly,
                placed: crate::parts::ship_frame(session, eye, ui),
                facing: DVec3::ZERO,
            });
        }
        None => stated.own = None,
    }
    stated.contacts.retain(|id, _| uplink.contacts.iter().any(|c| c.ship_id == *id && !c.form.parts.is_empty()));
    for contact in uplink.contacts.iter().filter(|c| !c.form.parts.is_empty()) {
        if stated.contacts.get(&contact.ship_id).is_none_or(|(form, _)| *form != contact.form) {
            let form = Form::from(&contact.form);
            let hash = form_hash(&form, &balance);
            stated.contacts.insert(contact.ship_id, (contact.form.clone(), Decoded { form: Arc::new(form), hash }));
        }
        let (_, decoded) = &stated.contacts[&contact.ship_id];
        out.push(Wanted {
            craft: Some(contact.ship_id),
            form: decoded.clone(),
            at_ly: contact.position_ly,
            placed: Transform {
                translation: sim_to_render(eye.offset_m(contact.position_ly, Some(contact.ship_id), look) / UNIT_M).as_vec3(),
                rotation: Quat::IDENTITY,
                scale: Vec3::splat((1.0 / UNIT_M) as f32),
            },
            facing: contact.facing,
        });
    }
    out
}

fn spawn(commands: &mut Commands, want: &Wanted, material: Handle<HullMaterial>) {
    let mesh = commands
        .spawn((
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            HullMesh,
        ))
        .id();
    let root = commands
        .spawn((
            want.placed,
            Visibility::default(),
            ShipHull { craft: want.craft, form: None, shown_roll: 0.0, rotation: Quat::IDENTITY, mesh, placeholders: None },
        ))
        .id();
    commands.entity(mesh).insert(ChildOf(root));
}

fn spawn_placeholders(
    commands: &mut Commands,
    form: &Form,
    balance: Balance,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BodySurfaceMaterial>,
    surfaces: &crate::surfaces::Surfaces,
    paint: impl Fn(Kind) -> em_render::body_surface_material::BodySurfaceUniform,
) -> Entity {
    let root = commands.spawn((Transform::IDENTITY, Visibility::default())).id();
    let pieces: Vec<Piece> = Sdf::new(form, &balance).map(|sdf| sdf.pieces().to_vec()).unwrap_or_default();
    for piece in &pieces {
        let (mesh, scale) = crate::parts::solid(&piece.shape);
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(surfaces.flat.material(paint(piece.kind)))),
            crate::parts::local(piece, scale),
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            Placeholder(crate::parts::paint(piece.kind)),
            ChildOf(root),
        ));
    }
    root
}
