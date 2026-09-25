//! A craft's form drawn as placeholder parts: one Bevy primitive per copy of each part, at its
//! solved size and pose, flat colored by kind.
//!
//! 32 §Temporary assets. No blends, spars uncut and a slab's corners square; the mesher replaces
//! all of it. The pieces sit in the ship's frame (x nose, y port, z up) in meters under one root,
//! which is the only thing placed each frame. Only the player's own ship has a form so far, and
//! only from `--form`.

use std::borrow::Cow;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::body_material::BodyWireframeMaterial;
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::render_space::sim_to_render;
use em_render::wire_mesh;
use glam::DVec3;
use lc_world::fitting::Balance;
use lc_world::form::presets::Builtin;
use lc_world::form::place::Side;
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Form, FormError, Kind, PartId};

use crate::construction::{Frame, Refit};
use crate::hull::{Eye, frame, lighting, lit};
use crate::system::UNIT_M;

/// Bevy's primitives lie along their Y and a part's axis is its local x: a quarter turn about z,
/// taking the mesh's Y to x and its X to −y.
const AXIS: Quat = Quat::from_xyzw(0.0, 0.0, -std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);

const SPHERE_SECTORS: u32 = 32;
const SPHERE_STACKS: u32 = 16;

/// The player's own ship's form, solved once.
#[derive(Resource, Default)]
pub struct OwnForm(Option<Formed>);

struct Formed {
    /// For the editor to draw and to cast the pointer into.
    sdf: Sdf,
    /// The form standing still, which is what is drawn when no refit is under way.
    still: Frame,
    /// From the Mind to the farthest corner of the form's bounds, meters.
    reach_m: f64,
}

impl OwnForm {
    pub fn new(form: &Form, balance: &Balance) -> Result<OwnForm, FormError> {
        let sdf = Sdf::new(form, balance)?;
        let (min, max) = sdf.bounds();
        let reach_m = min.abs().max(max.abs()).length();
        let still = Frame { pieces: sdf.pieces().to_vec(), standing: form.clone(), finished: 0, working: None };
        Ok(OwnForm(Some(Formed { sdf, still, reach_m })))
    }

    /// The solved form, for the editor to draw and to cast the pointer into.
    pub fn sdf(&self) -> Option<&Sdf> {
        self.0.as_ref().map(|f| &f.sdf)
    }

    /// Framed to hold every one of `forms`, drawn as the first: a refit's two ends, so the camera
    /// does not breathe as the round runs.
    pub fn spanning(forms: &[&Form], balance: &Balance) -> Result<OwnForm, FormError> {
        let mut own = OwnForm::new(forms[0], balance)?;
        for form in &forms[1..] {
            let (min, max) = Sdf::new(form, balance)?.bounds();
            if let Some(f) = own.0.as_mut() {
                f.reach_m = f.reach_m.max(min.abs().max(max.abs()).length());
            }
        }
        Ok(own)
    }

    pub fn is_formed(&self) -> bool {
        self.0.is_some()
    }

    /// The length the camera's boom is counted in: a sphere about the Mind holding the whole form,
    /// framed as an ovoid of that length would be. The bounds' corner rather than the form's own
    /// extent, which F6 adds and which replaces this.
    pub fn length_m(&self) -> Option<f64> {
        self.0.as_ref().map(|f| 2.0 * f.reach_m)
    }
}

/// A preset by `--form`'s spelling: a builtin's name, or `default` for the starting form. A
/// suffix `*k` makes every part but the Mind `k` times larger at the same proportions, which is
/// how a hull tens of kilometers long is photographed before anything can build one.
pub fn fixture(spec: &str) -> Option<Form> {
    let name = spec.split_once('*').map_or(spec, |(name, _)| name);
    let scale = fixture_scale(spec)?;
    let mut form = match name.eq_ignore_ascii_case("default") {
        true => Form::starting(),
        false => Builtin::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(name)).map(Builtin::form)?,
    };
    for part in form.parts.iter_mut().filter(|p| p.placement.is_some()) {
        part.volume_m3 *= scale.powi(3);
    }
    Some(form)
}

/// The `k` of a `--form` spelling's `*k`, one without it, and `None` for a `k` that is no scale.
pub fn fixture_scale(spec: &str) -> Option<f64> {
    match spec.split_once('*') {
        Some((_, k)) => k.parse::<f64>().ok().filter(|k| k.is_finite() && *k > 0.0),
        None => Some(1.0),
    }
}

/// Give the player's ship the form `--form` named, with no server involved.
pub fn adopt_fixture(dev: Res<crate::dev::DevEntry>, mut ui: ResMut<crate::app::Ui>, mut own: ResMut<OwnForm>) {
    let Some(name) = dev.form.as_deref() else { return };
    let Some(form) = fixture(name) else {
        ui.notify(format!("no such form: {name}"), 0.0);
        return;
    };
    match OwnForm::new(&form, &Balance::DEFAULT) {
        Ok(formed) => *own = formed,
        Err(e) => ui.notify(format!("form {name} does not place: {e}"), 0.0),
    }
}

/// Flat albedo by kind, linear.
pub(crate) fn paint(kind: Kind) -> Vec4 {
    let [r, g, b] = match kind {
        Kind::Mind => [0.60, 0.45, 0.10],
        Kind::Storage => [0.30, 0.31, 0.33],
        Kind::Drone => [0.55, 0.25, 0.06],
        Kind::Engine => [0.45, 0.10, 0.08],
        Kind::Living => [0.14, 0.45, 0.16],
        Kind::Data => [0.10, 0.24, 0.55],
        Kind::Bay => [0.45, 0.40, 0.28],
        Kind::Spar(_) => [0.12, 0.12, 0.13],
    };
    Vec4::new(r, g, b, 1.0)
}

/// `size` along the part's x, y and z, as the mesh's X, Y and Z before [`AXIS`] turns it.
fn in_mesh_axes(size: DVec3) -> Vec3 {
    (AXIS.inverse() * size.as_vec3()).abs()
}

/// The mesh for `shape` and the scale it is drawn at, both in the mesh's own axes, meters.
pub(crate) fn solid(shape: &Shape) -> (Mesh, Vec3) {
    let f = |x: f64| x as f32;
    match *shape {
        Shape::Ellipsoid { semi_axes } => {
            (Sphere::new(1.0).mesh().uv(SPHERE_SECTORS, SPHERE_STACKS), in_mesh_axes(semi_axes))
        }
        Shape::Capsule { radius, length } => (Capsule3d::new(f(radius), f(length)).into(), Vec3::ONE),
        Shape::Slab { edges, .. } => (Cuboid::from_size(in_mesh_axes(edges)).into(), Vec3::ONE),
        Shape::Cylinder { radius, length } => (Cylinder::new(f(radius), f(length)).into(), Vec3::ONE),
        Shape::Torus { major, minor } => (Torus::new(f(major - minor), f(major + minor)).into(), Vec3::ONE),
        // The mesh's +Y end is its top, and [`AXIS`] takes it to the part's +x.
        Shape::Frustum { length, start, end } => {
            let frustum = ConicalFrustum { radius_top: f(end), radius_bottom: f(start), height: f(length) };
            (frustum.into(), Vec3::ONE)
        }
    }
}

/// A piece's transform in the ship's frame, meters.
pub(crate) fn local(piece: &Piece, scale: Vec3) -> Transform {
    Transform {
        translation: piece.pose.position.as_vec3(),
        rotation: Quat::from_mat3(&piece.pose.rotation.as_mat3()) * AXIS,
        scale,
    }
}

/// The ship's frame, placed and turned. Scaled from meters to render units. Holds which moment
/// of a refit its pieces were spawned for, so they are respawned only when a step changes.
#[derive(Component)]
pub struct FormRoot(Option<(usize, Option<usize>)>);

/// A copy drawn solid. Its mesh is built at `reach_m` and scaled to the copy's size each frame.
#[derive(Component)]
pub struct Painted {
    color: Vec4,
    part: PartId,
    side: Side,
    mesh_scale: Vec3,
    reach_m: f64,
}

/// The cage around the sliver of the working step's `copy`th copy.
#[derive(Component)]
pub struct Cage {
    copy: usize,
    tube_m: f32,
}

/// Parallels and meridians of a copy's cage.
const CAGE_RINGS: usize = 7;
const CAGE_MERIDIANS: usize = 12;
const CAGE_SAMPLES: usize = 48;
/// Of the whole form's reach, so a small part's cage is as legible as a large one's.
const CAGE_TUBE: f64 = 0.005;
/// Linear, before the exposure; the cage is lit by its own work lights, not the star.
const CAGE_COLOR: LinearRgba = LinearRgba::new(0.9, 0.55, 0.18, 1.0);
const CAGE_EMISSION: f32 = 2.0;
/// Of a cage's full thickness, while any of it stands.
const CAGE_THINNEST: f32 = 0.3;

/// Lines over a copy's surface, part frame, meters: each point where a ray from the center leaves
/// it, so one grid of directions fits every primitive.
fn cage(shape: &Shape) -> Vec<Vec<Vec3>> {
    let on = |theta: f64, phi: f64| {
        let d = DVec3::new(theta.cos(), theta.sin() * phi.cos(), theta.sin() * phi.sin());
        shape.exit(d).point.as_vec3()
    };
    let tau = std::f64::consts::TAU;
    let pi = std::f64::consts::PI;
    let mut curves = Vec::new();
    for r in 1..=CAGE_RINGS {
        let theta = pi * r as f64 / (CAGE_RINGS + 1) as f64;
        curves.push((0..=CAGE_SAMPLES).map(|i| on(theta, tau * i as f64 / CAGE_SAMPLES as f64)).collect());
    }
    for m in 0..CAGE_MERIDIANS {
        let phi = tau * m as f64 / CAGE_MERIDIANS as f64;
        curves.push((0..=CAGE_SAMPLES).map(|i| on(pi * i as f64 / CAGE_SAMPLES as f64, phi)).collect());
    }
    curves
}

/// What is drawn this frame: a refit's moment, or the form standing still.
fn this_frame<'a>(own: &'a Formed, refit: Option<&Refit>, now_s: f64) -> (Cow<'a, Frame>, Option<(usize, Option<usize>)>) {
    match refit {
        Some(refit) => {
            let frame = refit.frame(now_s);
            let key = (frame.finished, frame.working.as_ref().map(|w| w.step));
            (Cow::Owned(frame), Some(key))
        }
        None => (Cow::Borrowed(&own.still), None),
    }
}

/// The ship's frame in meters, placed and turned, and scaled into render units.
pub(crate) fn ship_frame(session: &crate::session::Session, eye: &Eye, ui: &crate::app::Ui) -> Transform {
    let at_ly = session.ship.motion.position_ly;
    let star = lighting(session);
    let facing = session.ship.facing_at(session.coordinate_time_s()).unwrap_or(DVec3::X);
    Transform {
        translation: sim_to_render(eye.offset_m(at_ly, None, ui.look.forward()) / UNIT_M).as_vec3(),
        rotation: frame(facing, star.map(|(star_ly, _, _)| star_ly - at_ly)),
        scale: Vec3::splat((1.0 / UNIT_M) as f32),
    }
}

/// Draw the player's form, if it has one, where [`crate::hull::update_hulls`] would have put
/// its ovoid.
pub fn update_parts(
    mut commands: Commands,
    game: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    eye: Res<Eye>,
    own: Res<OwnForm>,
    refit: Option<Res<Refit>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    mut wires: ResMut<Assets<BodyWireframeMaterial>>,
    surfaces: Res<crate::surfaces::Surfaces>,
    mut roots: Query<(Entity, &mut Transform, &FormRoot), (Without<Painted>, Without<Cage>)>,
    mut pieces: Query<(&MeshMaterial3d<BodySurfaceMaterial>, &Painted, &mut Transform, &mut Visibility), Without<Cage>>,
    mut cages: Query<(&MeshMaterial3d<BodyWireframeMaterial>, &Cage, &mut Transform, &mut Visibility), Without<Painted>>,
) {
    if own.0.is_none() && !own.is_changed() {
        return;
    }
    let session = &game.0;
    let at_ly = session.ship.motion.position_ly;
    let star = lighting(session);
    let placed = ship_frame(session, &eye, &ui);
    // `crate::refit_hull` draws a staged refit on the hull meshes instead.
    let formed = own.0.as_ref().filter(|_| refit.is_none());
    let Some(formed) = formed else {
        for (root, _, _) in &roots {
            commands.entity(root).despawn();
        }
        return;
    };
    let (drawn, key) = this_frame(formed, refit.as_deref(), session.coordinate_time_s());

    let stale = own.is_changed() || roots.iter().all(|(_, _, root)| root.0 != key);
    if stale {
        for (root, _, _) in &roots {
            commands.entity(root).despawn();
        }
        let root = commands.spawn((placed, Visibility::default(), FormRoot(key))).id();
        let outer = drawn.working.as_ref().map_or(&[][..], |w| &w.outer[..]);
        for piece in &drawn.pieces {
            // Built at the sliver's outer size, so a part starting from nothing has a mesh to grow.
            let full = outer.iter().find(|o| o.part == piece.part && o.side == piece.side).unwrap_or(piece);
            let (mesh, mesh_scale) = solid(&full.shape);
            let color = paint(piece.kind);
            let painted = Painted { color, part: piece.part, side: piece.side, mesh_scale, reach_m: full.shape.reach() };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(surfaces.flat.material(lit(session, star, at_ly, color)))),
                sized(piece, &painted),
                // As for a hull: placed by hand at a scale where a mesh's bounds say nothing.
                NoFrustumCulling,
                RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
                painted,
                ChildOf(root),
            ));
        }
        for (copy, piece) in outer.iter().enumerate() {
            let tube_m = (CAGE_TUBE * formed.reach_m) as f32;
            let material = BodyWireframeMaterial {
                base_color: CAGE_COLOR,
                emission_strength: CAGE_EMISSION,
                base_tube_radius: tube_m,
                target_tube_radius: 0.0,
                ..default()
            };
            commands.spawn((
                Mesh3d(meshes.add(wire_mesh::tube_curves(&cage(&piece.shape), tube_m, 4, 1.0))),
                MeshMaterial3d(wires.add(material)),
                caged(piece),
                NoFrustumCulling,
                RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
                Cage { copy, tube_m },
                ChildOf(root),
            ));
        }
        return;
    }

    for (_, mut transform, _) in &mut roots {
        *transform = placed;
    }
    for (material, painted, mut transform, mut visibility) in &mut pieces {
        match drawn.piece(painted.part, painted.side) {
            Some(piece) => {
                *transform = sized(piece, painted);
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
        let Some(mut asset) = materials.get_mut(&material.0) else { continue };
        let next = lit(session, star, at_ly, painted.color);
        if asset.uniforms != next {
            asset.uniforms = next;
        }
    }
    let Some(working) = &drawn.working else { return };
    let scaffold = working.mean().scaffold as f32;
    for (material, cage, mut transform, mut visibility) in &mut cages {
        let Some(piece) = working.outer.get(cage.copy) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        *transform = caged(piece);
        *visibility = if scaffold > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
        let Some(mut asset) = wires.get_mut(&material.0) else { continue };
        // Thinner than this the tubes alias into dots, which read as nothing being built.
        let radius = cage.tube_m * (CAGE_THINNEST + (1.0 - CAGE_THINNEST) * scaffold);
        if asset.target_tube_radius != radius {
            asset.target_tube_radius = radius;
        }
    }
}

/// A painted copy at its size this frame: its mesh's scale times how much bigger it has grown.
fn sized(piece: &Piece, painted: &Painted) -> Transform {
    let grown = (piece.shape.reach() / painted.reach_m) as f32;
    local(piece, painted.mesh_scale * grown)
}

/// The cage is built in the part's own frame, so it takes the pose without the quarter turn.
fn caged(piece: &Piece) -> Transform {
    Transform {
        translation: piece.pose.position.as_vec3(),
        rotation: Quat::from_mat3(&piece.pose.rotation.as_mat3()),
        scale: Vec3::ONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    /// Every vertex of `shape`'s mesh, in the part's frame.
    fn vertices(shape: &Shape) -> Vec<Vec3> {
        let (mesh, scale) = solid(shape);
        let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("no positions");
        };
        positions.iter().map(|&p| AXIS * (Vec3::from_array(p) * scale)).collect()
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() < 1e-3 * b.abs().max_element().max(1.0)
    }

    #[test]
    fn the_quarter_turn_takes_the_meshs_axis_to_the_parts() {
        assert!(close(AXIS * Vec3::Y, Vec3::X));
        assert!(close(AXIS * Vec3::X, -Vec3::Y));
        assert!(close(AXIS * Vec3::Z, Vec3::Z));
    }

    /// Each primitive's extent along the part's x, y and z, stated from 29's definitions rather
    /// than from the mesh builders, with every dimension different so a swapped axis shows.
    #[test]
    fn each_primitive_spans_its_shape_along_the_parts_axes() {
        let cases = [
            (Shape::Ellipsoid { semi_axes: DVec3::new(5.0, 3.0, 1.0) }, Vec3::new(5.0, 3.0, 1.0)),
            (Shape::Capsule { radius: 2.0, length: 6.0 }, Vec3::new(5.0, 2.0, 2.0)),
            (Shape::Slab { edges: DVec3::new(9.0, 5.0, 0.5), corner: 0.1 }, Vec3::new(4.5, 2.5, 0.25)),
            (Shape::Cylinder { radius: 1.5, length: 10.0 }, Vec3::new(5.0, 1.5, 1.5)),
            (Shape::Torus { major: 4.0, minor: 1.0 }, Vec3::new(1.0, 5.0, 5.0)),
            (Shape::Frustum { length: 8.0, start: 3.0, end: 1.0 }, Vec3::new(4.0, 3.0, 3.0)),
        ];
        for (shape, half) in cases {
            let v = vertices(&shape);
            let max = v.iter().fold(Vec3::NEG_INFINITY, |m, p| m.max(*p));
            let min = v.iter().fold(Vec3::INFINITY, |m, p| m.min(*p));
            assert!(close(max, half) && close(min, -half), "{shape:?} spans {min} to {max}");
        }
    }

    /// A frustum's first end, the one at −x, has the `start` radius.
    #[test]
    fn a_frustum_starts_at_minus_x() {
        let (length, start, end) = (8.0, 3.0, 1.0);
        let v = vertices(&Shape::Frustum { length, start, end });
        let radius_at = |x: f32| {
            v.iter().filter(|p| (p.x - x).abs() < 1e-4).map(|p| p.yz().length()).fold(0.0, f32::max)
        };
        assert!((radius_at(-4.0) - start as f32).abs() < 1e-3, "{}", radius_at(-4.0));
        assert!((radius_at(4.0) - end as f32).abs() < 1e-3, "{}", radius_at(4.0));
    }

    fn render(v: DVec3) -> Vec3 {
        sim_to_render(v).as_vec3()
    }

    /// The form's nose goes along the facing, its dorsal face toward the star, and the frame
    /// stays right-handed through the change of axes.
    #[test]
    fn the_ships_frame_points_its_nose_ahead_and_its_back_at_the_star() {
        let fore = DVec3::Y;
        let q = frame(fore, Some(DVec3::X * 3.0));
        assert!(close(q * Vec3::X, render(fore)), "{}", q * Vec3::X);
        assert!(close(q * Vec3::Z, render(DVec3::X)), "{}", q * Vec3::Z);
        assert!(close((q * Vec3::X).cross(q * Vec3::Y), q * Vec3::Z), "handedness was lost");
    }

    #[test]
    fn every_fixture_name_is_a_form_that_places() {
        for name in ["default", "plate", "Spindle", "CLUSTER"] {
            let form = fixture(name).unwrap_or_else(|| panic!("{name} is no fixture"));
            let own = OwnForm::new(&form, &Balance::DEFAULT).expect("a preset places");
            assert!(own.length_m().is_some_and(|l| l > 0.0 && l.is_finite()));
        }
        assert!(fixture("ovoid").is_none());
        assert!(fixture("spindle*0").is_none() && fixture("spindle*x").is_none());
    }

    #[test]
    fn a_scaled_fixture_is_that_many_times_larger() {
        let length = |spec: &str| OwnForm::new(&fixture(spec).unwrap(), &Balance::DEFAULT).unwrap().length_m().unwrap();
        let ratio = length("spindle*100") / length("spindle");
        assert!((90.0..=101.0).contains(&ratio), "{ratio}: the Mind alone keeps its size");
    }

    /// A mirrored copy is drawn as the reflection of its original through y = 0, with no
    /// negative scale anywhere: every vertex of one, reflected, lands on a vertex of the other.
    #[test]
    fn a_mirrored_pair_is_symmetric_about_the_port_starboard_plane() {
        use lc_world::form::PartId;
        use lc_world::form::place::Side;
        let mut form = Builtin::Plate.form();
        form.parts.retain(|p| p.id != PartId(3));
        let engine = form.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap();
        engine.placement.as_mut().unwrap().mirror = true;
        let own = OwnForm::new(&form, &Balance::DEFAULT).unwrap();
        let drawn = |side: Side| {
            let piece = own.0.as_ref().unwrap().still.pieces.iter().find(|p| p.part == PartId(2) && p.side == side).unwrap();
            let (_, scale) = solid(&piece.shape);
            let t = local(piece, scale);
            assert!(scale.min_element() > 0.0);
            vertices(&piece.shape).into_iter().map(|v| t.translation + Quat::from_mat3(&piece.pose.rotation.as_mat3()) * v).collect::<Vec<_>>()
        };
        let (original, mirror) = (drawn(Side::Original), drawn(Side::Mirror));
        assert!(original.iter().map(|v| v.y).sum::<f32>().abs() > 1.0, "the original sits on the plane");
        let tolerance = 1e-4 * original.iter().map(|v| v.length()).fold(0.0, f32::max);
        for v in &original {
            let reflected = v * Vec3::new(1.0, -1.0, 1.0);
            let nearest = mirror.iter().map(|m| m.distance(reflected)).fold(f32::INFINITY, f32::min);
            assert!(nearest < tolerance, "{v} reflected is {nearest} m from the mirror");
        }
    }

    /// The camera's nearest stop on the smallest preset still clears the near plane.
    #[test]
    fn the_nearest_the_camera_comes_to_a_form_is_outside_the_near_plane() {
        let fov_x = crate::hull::fov_x(std::f32::consts::FRAC_PI_4, 16.0 / 9.0);
        // A 45° view 1024 pixels high.
        const RAD_PER_PX: f32 = std::f32::consts::FRAC_PI_4 / 1024.0;
        let (near, _) = crate::hull::boom_limits(RAD_PER_PX, fov_x);
        let shortest = ["default", "plate", "spindle", "cluster"]
            .map(|name| OwnForm::new(&fixture(name).unwrap(), &Balance::DEFAULT).unwrap().length_m().unwrap())
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        // The framed sphere's radius is half a length.
        const RADIUS_IN_LENGTHS: f64 = 0.5;
        let clearance = (near - RADIUS_IN_LENGTHS) * shortest / UNIT_M;
        assert!(clearance > crate::app::NEAR_PLANE as f64, "{clearance} units of clearance");
    }

    /// Every piece is inside the sphere the camera frames, which is what keeps the near stop
    /// from putting the eye inside the ship.
    #[test]
    fn the_framed_length_holds_every_piece() {
        for name in ["default", "plate", "spindle", "cluster"] {
            let form = fixture(name).unwrap();
            let own = OwnForm::new(&form, &Balance::DEFAULT).unwrap();
            let reach = own.length_m().unwrap() as f32 / 2.0;
            for piece in &own.0.as_ref().unwrap().still.pieces {
                for v in vertices(&piece.shape) {
                    let p = piece.pose.to_outer(v.as_dvec3()).as_vec3();
                    assert!(p.length() <= reach * 1.001, "{name}: {:?} reaches {}", piece.part, p.length());
                }
            }
        }
    }
}
