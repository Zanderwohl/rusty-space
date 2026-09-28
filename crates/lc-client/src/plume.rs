//! Light leaving a craft's open faces: each lit face glowing at the flux leaving it, whatever lit
//! it, and a burn's cone out to the courtesy radius. 32 §The exhaust cone.
//!
//! The glow is light, drawn on every lit face of every craft with a form, from
//! [`crate::lit_faces`], under its hull's root so it moves with the hull. The cone is the main
//! drive's alone: an emit's spread is its own, and reaches an observer inside it as `Glare`. It is
//! an indicator, drawn for the player's own burn, for a burn whose
//! courtesy radius the player is inside, and for a selected ship; [`Exhausts`] hands the same cones
//! to the map. Another craft is drawn as its light shows it, from what its `Presence` stated.
//!
//! Each proxy is told where the eye is in its own space, worked out in `f64`: a render unit is an
//! AU, and a cone is thousands of hull lengths.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::exhaust_cone_material::{
    ApertureGlowMaterial, ApertureGlowUniform, ExhaustConeMaterial, ExhaustConeUniform,
};
use em_render::render_space::{render_to_sim, sim_to_render};
use glam::{DQuat, DVec3};
use lc_proto::ShipId;
use lc_world::courtesy::{cooking_distance_m, cooking_flux_w_m2, drive_courtesy_radius_m};
use lc_world::fitting::Balance;

use crate::hull::Eye;
use crate::lit_faces::{LitFaces, radiance};
use crate::session::Session;
use crate::ship_hull::ShipHull;
use crate::system::{M_PER_LY, UNIT_M};

/// The cone's brightness in the hazard color where it would cook, and at the courtesy radius.
pub const HOT_GAIN: f32 = 0.35;
pub const FAINT_GAIN: f32 = 0.02;

/// How far the near-field glow runs aft of the face and how wide it is, in aperture radii.
pub const GLOW_REACH: f32 = 6.0;
pub const GLOW_WIDTH: f32 = 1.0;
/// How far over the exposure's reference the glow sits side-on through its middle. A display
/// choice: the face is decades over and carries the physics.
pub const GLOW_STOPS: f64 = 4.0;
/// What a stop past the top of the exposure's window is worth as HDR value, so the face blooms.
pub const OVERFLOW_GAIN: f32 = 0.5;

/// How far aft of the hull's face its glow's disk sits, in aperture radii. On the face itself the
/// two fight for depth, and the mesh's cap is only as flat as its cells.
const FACE_STANDOFF: f64 = 0.05;

const LUMA: DVec3 = DVec3::new(0.2126, 0.7152, 0.0722);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lit {
    /// `None` for the player's own.
    pub craft: Option<ShipId>,
    /// `F c`, watts.
    pub power_w: f64,
    pub at_ly: DVec3,
    /// Simulation axes, unit.
    pub facing: DVec3,
    pub length_m: f64,
}

/// The cone's length, the courtesy radius, if it is drawn for an observer at `here_ly`.
pub fn cone_m(lit: &Lit, here_ly: DVec3, selected: Option<ShipId>, balance: &Balance) -> Option<f64> {
    if lit.power_w <= 0.0 {
        return None;
    }
    let radius_m = drive_courtesy_radius_m(balance, lit.power_w);
    let inside = lit.at_ly.distance(here_ly) * M_PER_LY <= radius_m;
    let chosen = lit.craft.is_some() && lit.craft == selected;
    (lit.craft.is_none() || inside || chosen).then_some(radius_m)
}

/// `eye_local` is the caller's to set.
pub fn cone_uniform(power_w: f64, balance: &Balance, length_m: f64) -> ExhaustConeUniform {
    let hazard = LinearRgba::from(crate::ui::HAZARD).to_vec3();
    ExhaustConeUniform {
        emission: Vec4::new(
            power_w as f32,
            balance.drive_spread_rad as f32,
            length_m as f32,
            cooking_flux_w_m2(balance) as f32,
        ),
        faint: (hazard * FAINT_GAIN).extend(0.0),
        hot: (hazard * HOT_GAIN).extend(0.0),
        eye_local: Vec4::ZERO,
    }
}

/// `face` is band-mapped linear RGB on the exposure's scale. `eye_local` is the caller's to set.
pub fn aperture_uniform(face: DVec3, reference: f64, stops: f32) -> ApertureGlowUniform {
    let luminance = face.dot(LUMA);
    let glow = if luminance > 0.0 { face * (reference * GLOW_STOPS.exp2() / luminance) } else { DVec3::ZERO };
    ApertureGlowUniform {
        face: face.as_vec3().extend(0.0),
        glow: glow.as_vec3().extend(0.0),
        shape: Vec4::new(GLOW_REACH, GLOW_WIDTH, 0.0, 0.0),
        eye_local: Vec4::ZERO,
        exposure: Vec4::new(reference as f32, stops, OVERFLOW_GAIN, 0.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub craft: Option<ShipId>,
    pub apex_ly: DVec3,
    /// Along the exhaust, simulation axes, unit.
    pub aft: DVec3,
    pub length_m: f64,
    pub half_angle_rad: f64,
    /// From the apex to where a Black receiver cooks, meters.
    pub cooking_m: f64,
}

#[derive(Resource, Default)]
pub struct Exhausts {
    pub cones: Vec<Drawn>,
    /// By the half-angle's bits.
    cone_proxy: Option<(u64, Handle<Mesh>)>,
    glow_proxy: Option<Handle<Mesh>>,
}

#[derive(Component)]
pub struct Cone(pub Option<ShipId>);

/// One lit face's glow, under its craft's [`ShipHull`], by its place in its [`crate::lit_faces::Lit`].
#[derive(Component)]
pub struct Glow {
    craft: Option<ShipId>,
    face: usize,
}

/// A hull root's transform in `f64`: the ship's frame in meters to render units about the eye.
#[derive(Clone, Copy, Debug)]
struct Root {
    translation: DVec3,
    rotation: DQuat,
    scale: f64,
}

impl Root {
    fn of(transform: &Transform) -> Self {
        Self {
            translation: transform.translation.as_dvec3(),
            rotation: transform.rotation.as_dquat(),
            scale: transform.scale.x as f64,
        }
    }

    fn to_render(&self, ship_m: DVec3) -> DVec3 {
        self.translation + self.rotation * (ship_m * self.scale)
    }

    /// The eye, which is the render origin, in the ship's frame.
    fn eye(&self) -> DVec3 {
        self.rotation.inverse() * -self.translation / self.scale
    }

    /// Along the exhaust, render axes: out of the stern.
    fn aft(&self) -> DVec3 {
        (self.rotation * DVec3::NEG_X).normalize()
    }
}

/// A proxy with `+y` along `axis` at `origin`, `scale` per local unit, all about the eye; and
/// the eye in its local space.
fn about_eye(origin: DVec3, axis: DVec3, scale: f64) -> (Transform, DVec3) {
    let rotation = DQuat::from_rotation_arc(DVec3::Y, axis);
    let transform = Transform {
        translation: origin.as_vec3(),
        rotation: rotation.as_quat(),
        scale: Vec3::splat(scale as f32),
    };
    (transform, rotation.inverse() * -origin / scale)
}

fn lits(session: &Session, uplink: &crate::uplink::Uplink, balance: &Balance) -> Vec<Lit> {
    let now = session.coordinate_time_s();
    let ship = &session.ship;
    let own = Lit {
        craft: None,
        power_w: lc_world::emit::drive_w(ship, balance, now),
        at_ly: ship.motion.position_ly,
        facing: ship.facing_at(now).unwrap_or(DVec3::X),
        length_m: ship.length_m,
    };
    let contacts = uplink.contacts.iter().map(|c| Lit {
        craft: Some(c.ship_id),
        power_w: c.drive_w,
        at_ly: c.position_ly,
        facing: c.facing,
        length_m: c.length_m,
    });
    std::iter::once(own)
        .chain(contacts)
        .map(|lit| Lit { facing: lit.facing.normalize_or_zero(), ..lit })
        .filter(|lit| lit.power_w > 0.0 && lit.facing != DVec3::ZERO)
        .collect()
}

/// After the hulls, whose roots the glows hang from.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn draw_exhaust(
    mut commands: Commands,
    (game, ui, uplink, eye): (Res<crate::app::Game>, Res<crate::app::Ui>, Res<crate::uplink::Uplink>, Res<Eye>),
    faces: Res<LitFaces>,
    mut exhausts: ResMut<Exhausts>,
    mut meshes: ResMut<Assets<Mesh>>,
    (mut cone_materials, mut glow_materials): (ResMut<Assets<ExhaustConeMaterial>>, ResMut<Assets<ApertureGlowMaterial>>),
    roots: Query<(Entity, &ShipHull, &Transform)>,
    mut glows: Query<(Entity, &Glow, &mut Transform, &MeshMaterial3d<ApertureGlowMaterial>), Without<ShipHull>>,
    mut cones: Query<
        (Entity, &Cone, &mut Transform, &MeshMaterial3d<ExhaustConeMaterial>),
        (Without<ShipHull>, Without<Glow>),
    >,
) {
    let session = &game.0;
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let look = ui.look.forward();
    let here = session.ship.motion.position_ly;
    let lit = lits(session, &uplink, &balance);
    let tone = &session.tone;
    let exhausts = &mut *exhausts;
    exhausts.cones.clear();
    let root_of = |craft: Option<ShipId>| roots.iter().find(|(_, hull, _)| hull.craft() == craft).map(|(e, _, t)| (e, Root::of(t)));

    let mut want_glows = Vec::new();
    for (craft, lit) in faces.iter() {
        let Some((entity, root)) = root_of(craft) else { continue };
        let eye_ship = root.eye();
        for (index, face) in lit.faces.iter().enumerate().filter(|(_, f)| f.power_w > 0.0) {
            let face_at = face.aperture;
            let rotation = DQuat::from_rotation_arc(DVec3::Y, face_at.out);
            let center = face_at.center + face_at.out * face_at.radius_m * FACE_STANDOFF;
            let transform = Transform {
                translation: center.as_vec3(),
                rotation: rotation.as_quat(),
                scale: Vec3::splat(face_at.radius_m as f32),
            };
            let eye_local = rotation.inverse() * (eye_ship - center) / face_at.radius_m;
            let mut uniforms =
                aperture_uniform(radiance(session, face.temperature_k), tone.surface_reference as f64, tone.surface_stops);
            uniforms.eye_local = eye_local.as_vec3().extend(0.0);
            want_glows.push((craft, index, entity, transform, uniforms));
        }
    }

    let mut want_cones = Vec::new();
    for burn in &lit {
        let root = root_of(burn.craft);
        let Some(length_m) = cone_m(burn, here, ui.selected_craft, &balance) else { continue };
        let aft_faces: Vec<_> =
            faces.of(burn.craft).map(|lit| lit.faces.iter().map(|f| f.aperture).filter(|a| a.aft()).collect()).unwrap_or_default();
        // From the aft faces' power-weighted middle, or the stern of a craft drawn with none.
        let (apex, aft, apex_from_center_m) = match root {
            Some((_, root)) if !aft_faces.is_empty() => {
                let middle: DVec3 = aft_faces.iter().map(|a| a.center * a.share).sum();
                (root.to_render(middle), root.aft(), render_to_sim(root.rotation * middle))
            }
            _ => {
                let stern = -burn.facing * burn.length_m * 0.5;
                let at = eye.offset_m(burn.at_ly, burn.craft, look) + stern;
                (sim_to_render(at / UNIT_M), sim_to_render(-burn.facing), stern)
            }
        };
        let (transform, eye_local) = about_eye(apex, aft, length_m / UNIT_M);
        let mut uniforms = cone_uniform(burn.power_w, &balance, length_m);
        uniforms.eye_local = eye_local.as_vec3().extend(0.0);
        want_cones.push((burn.craft, transform, uniforms));
        exhausts.cones.push(Drawn {
            craft: burn.craft,
            apex_ly: burn.at_ly + apex_from_center_m / M_PER_LY,
            aft: render_to_sim(aft),
            length_m,
            half_angle_rad: balance.drive_spread_rad,
            cooking_m: cooking_distance_m(&balance, burn.power_w, balance.drive_spread_rad, 1.0),
        });
    }

    let mut kept = Vec::with_capacity(want_glows.len());
    for (entity, glow, mut transform, material) in glows.iter_mut() {
        let Some((_, _, _, placed, uniforms)) =
            want_glows.iter().find(|(craft, face, ..)| *craft == glow.craft && *face == glow.face)
        else {
            commands.entity(entity).despawn();
            continue;
        };
        kept.push((glow.craft, glow.face));
        *transform = *placed;
        if let Some(mut asset) = glow_materials.get_mut(&material.0)
            && asset.uniforms != *uniforms
        {
            asset.uniforms = uniforms.clone();
        }
    }
    for (craft, face, root, transform, uniforms) in want_glows {
        if kept.contains(&(craft, face)) {
            continue;
        }
        let proxy = exhausts
            .glow_proxy
            .get_or_insert_with(|| meshes.add(ApertureGlowMaterial::proxy(GLOW_REACH, GLOW_WIDTH)))
            .clone();
        commands.spawn((
            Mesh3d(proxy),
            MeshMaterial3d(glow_materials.add(ApertureGlowMaterial { uniforms })),
            transform,
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            Glow { craft, face },
            ChildOf(root),
        ));
    }

    let half_angle = balance.drive_spread_rad;
    if exhausts.cone_proxy.as_ref().is_none_or(|(bits, _)| *bits != half_angle.to_bits()) {
        exhausts.cone_proxy = Some((half_angle.to_bits(), meshes.add(ExhaustConeMaterial::proxy(half_angle as f32))));
    }
    let proxy = exhausts.cone_proxy.as_ref().map(|(_, mesh)| mesh.clone()).expect("just made");
    let mut kept = Vec::with_capacity(want_cones.len());
    for (entity, cone, mut transform, material) in cones.iter_mut() {
        let Some((_, placed, uniforms)) = want_cones.iter().find(|(craft, ..)| *craft == cone.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        kept.push(cone.0);
        *transform = *placed;
        if let Some(mut asset) = cone_materials.get_mut(&material.0)
            && asset.uniforms != *uniforms
        {
            asset.uniforms = uniforms.clone();
        }
    }
    for (craft, transform, uniforms) in want_cones {
        if kept.contains(&craft) {
            continue;
        }
        commands.spawn((
            Mesh3d(proxy.clone()),
            MeshMaterial3d(cone_materials.add(ExhaustConeMaterial { uniforms })),
            transform,
            NoFrustumCulling,
            RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            Cone(craft),
        ));
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use lc_world::emit::{Ends, aperture_temperature_k};
    use lc_world::form::Form;
    use lc_world::form::capacity::apertures;
    use lc_world::form::presets::{Builtin, turned_fore};

    use super::*;
    use crate::ship_hull::RealHulls;

    const B: Balance = Balance::DEFAULT;

    fn lit(craft: Option<ShipId>, power_w: f64, at_m: DVec3) -> Lit {
        Lit { craft, power_w, at_ly: at_m / M_PER_LY, facing: DVec3::X, length_m: 500.0 }
    }

    /// Ship 1 in `form`, a thousand kilometers off `here_ly` along +x, stating `drive_w` and `emit`.
    fn presence(here_ly: DVec3, form: &Form, drive_w: f64, emit: Ends) -> lc_proto::Presence {
        lc_proto::Presence {
            ship_id: ShipId(1),
            name: "ship 1".into(),
            length_m: 500.0,
            at_ly: (here_ly + DVec3::X * 1.0e6 / M_PER_LY).to_array(),
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            drive_w,
            emit_fore_w: emit.fore_w,
            emit_aft_w: emit.aft_w,
            emitted_t: 0,
            arrive_t: 3_600_000_000,
            form: form.into(),
            building: None,
            glow: None,
            glare: None,
        }
    }

    /// Own, inside the radius and selected get a cone; nothing else, and nothing coasting.
    #[test]
    fn which_burns_have_a_cone() {
        let power = 1.1e20;
        let radius = drive_courtesy_radius_m(&B, power);
        let here = DVec3::ZERO;
        let (near, far) = (DVec3::Y * radius * 0.9, DVec3::Y * radius * 1.1);
        let other = Some(ShipId(4));
        let cone = |l: Lit, selected| cone_m(&l, here, selected, &B);

        assert_eq!(cone(lit(None, power, far), None), Some(radius), "your own, wherever it is");
        assert_eq!(cone(lit(other, power, near), None), Some(radius), "inside its radius");
        assert_eq!(cone(lit(other, power, far), None), None, "outside and unselected");
        assert_eq!(cone(lit(other, power, far), other), Some(radius), "selected");
        assert_eq!(cone(lit(other, power, far), Some(ShipId(5))), None, "another selected");
        assert_eq!(cone(lit(None, 0.0, here), None), None, "coasting");
        assert_eq!(cone(lit(other, 0.0, here), other), None, "selected and coasting");
    }

    /// The cone runs out at the courtesy radius, which grows as the root of the power.
    #[test]
    fn the_cone_is_as_long_as_the_courtesy_radius() {
        let at = |power| cone_m(&lit(None, power, DVec3::ZERO), DVec3::ZERO, None, &B).unwrap();
        assert_eq!(at(1.1e20), drive_courtesy_radius_m(&B, 1.1e20));
        assert!((at(4.4e20) / at(1.1e20) - 2.0).abs() < 1.0e-9);
    }

    /// The exhaust leaves the stern: a cone drawn forward is a ship pushing itself backwards.
    #[test]
    fn the_cone_points_away_from_the_nose() {
        for facing in [DVec3::X, DVec3::Y, DVec3::new(1.0, -2.0, 0.5).normalize()] {
            let rotation = crate::hull::frame(facing, Some(DVec3::Z), 0.3);
            let root = Root::of(&Transform::from_rotation(rotation));
            let want = sim_to_render(-facing);
            assert!((root.aft() - want).length() < 1.0e-6, "{facing} sent the exhaust to {}", root.aft());
        }
    }

    /// The eye each proxy is handed is where the camera really is in the proxy's space.
    #[test]
    fn a_proxy_knows_where_the_eye_is() {
        let apex = DVec3::new(3.0e-7, -1.0e-7, 2.0e-7);
        let (transform, eye) = about_eye(apex, DVec3::new(0.2, -1.0, 0.1).normalize(), 1.4e-7);
        let back = transform.transform_point(eye.as_vec3());
        assert!(back.length() < 1.0e-12, "the eye lands {back} from the origin");

        let root = Root { translation: apex, rotation: DQuat::from_rotation_z(0.7), scale: 1.0 / UNIT_M };
        assert!(root.to_render(root.eye()).length() < 1.0e-18);
    }

    /// A world with ship 1 as another ship in `form`, stating `drive_w` and `emit`, and its hull's
    /// root.
    fn scene_of(form: &Form, drive_w: f64, emit: Ends) -> (World, Entity) {
        let session = Session::new(&lc_world::sky::AuthoredStars::sample(), 3);
        let here = session.ship.motion.position_ly;
        let craft = Some(ShipId(1));
        let mut uplink = crate::uplink::Uplink::default();
        uplink.contacts = vec![crate::uplink::Contact::seen(presence(here, form, drive_w, emit), None)];
        let mut real = RealHulls::default();
        real.set(craft, 1, 1);

        let mut world = World::new();
        world.insert_resource(crate::app::Game(session));
        world.insert_resource(crate::app::Ui(Default::default()));
        world.insert_resource(uplink);
        world.insert_resource(Eye::default());
        world.insert_resource(crate::parts::OwnForm::default());
        world.insert_resource(real);
        world.init_resource::<Exhausts>();
        world.init_resource::<LitFaces>();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<ExhaustConeMaterial>>();
        world.init_resource::<Assets<ApertureGlowMaterial>>();
        let mesh = world.spawn_empty().id();
        let root = world
            .spawn((ShipHull::bare(craft, mesh), Transform::from_scale(Vec3::splat((1.0 / UNIT_M) as f32))))
            .id();
        (world, root)
    }

    /// A plate burning at `drive_w`.
    fn scene(drive_w: f64) -> (World, Entity) {
        scene_of(&Builtin::Plate.form(), drive_w, Ends::default())
    }

    fn draw(world: &mut World) {
        world.run_system_once(crate::lit_faces::light_faces).unwrap();
        world.run_system_once(draw_exhaust).unwrap();
    }

    /// Each glow drawn, by its face, with the face color it was handed.
    fn glows(world: &mut World) -> Vec<(usize, Vec4)> {
        let handles: Vec<(usize, Handle<ApertureGlowMaterial>)> =
            world.query::<(&Glow, &MeshMaterial3d<ApertureGlowMaterial>)>().iter(world).map(|(g, m)| (g.face, m.0.clone())).collect();
        let materials = world.resource::<Assets<ApertureGlowMaterial>>();
        let mut drawn: Vec<(usize, Vec4)> = handles.iter().map(|(face, m)| (*face, materials.get(m).unwrap().uniforms.face)).collect();
        drawn.sort_by_key(|(face, _)| *face);
        drawn
    }

    /// What a face at `kelvin` is drawn as.
    fn face_color(world: &World, kelvin: f64) -> Vec4 {
        let session = &world.resource::<crate::app::Game>().0;
        aperture_uniform(radiance(session, kelvin), session.tone.surface_reference as f64, session.tone.surface_stops).face
    }

    /// Another ship's cone and faces are drawn from the power it stated and nothing else.
    #[test]
    fn another_ship_is_drawn_from_the_power_it_stated() {
        let faces = apertures(&Builtin::Plate.form(), &B).unwrap();
        for stated in [3.0e17, 1.1e20, 4.4e21] {
            let (mut world, _) = scene(stated);
            world.resource_mut::<crate::app::Ui>().0.selected_craft = Some(ShipId(1));
            draw(&mut world);

            let [cone] = world.resource::<Exhausts>().cones[..] else { panic!("one cone") };
            assert_eq!(cone.length_m, drive_courtesy_radius_m(&B, stated));
            assert_eq!(cone.cooking_m, cooking_distance_m(&B, stated, B.drive_spread_rad, 1.0));

            let drawn = glows(&mut world);
            assert_eq!(drawn.len(), faces.len(), "both of the plate's faces fire aft");
            for (face, uniform) in drawn {
                let a = faces[face];
                let want = face_color(&world, aperture_temperature_k(stated * a.share, a.area_m2()));
                assert_eq!(uniform, want, "face {face} at {stated} W");
            }
        }
    }

    /// Another craft's emit lights the faces it leaves through at the temperature its `Presence`
    /// stated of each end, and draws no cone however it is selected.
    #[test]
    fn another_ships_faces_are_as_hot_as_its_stated_emit() {
        let form = turned_fore(Builtin::Plate.form(), 1);
        let faces = apertures(&form, &B).unwrap();
        let fore = faces.iter().position(|a| a.fore()).unwrap();
        let aft = faces.iter().position(|a| a.aft()).unwrap();
        for (emit, lit) in [
            (Ends { fore_w: 0.0, aft_w: 2.0e19 }, vec![aft]),
            (Ends { fore_w: 5.0e18, aft_w: 0.0 }, vec![fore]),
            (Ends { fore_w: 3.0e19, aft_w: 3.0e19 }, vec![fore, aft]),
        ] {
            let (mut world, _) = scene_of(&form, 0.0, emit);
            world.resource_mut::<crate::app::Ui>().0.selected_craft = Some(ShipId(1));
            draw(&mut world);

            let temperature = |a: &lc_world::form::capacity::Aperture| {
                let end_w = if a.fore() { emit.fore_w } else { emit.aft_w };
                aperture_temperature_k(end_w * a.share, a.area_m2())
            };
            let stated = world.resource::<LitFaces>().of(Some(ShipId(1))).unwrap().clone();
            for (face, a) in stated.faces.iter().zip(&faces) {
                assert_eq!(face.temperature_k, temperature(a), "{emit:?}");
            }
            let drawn = glows(&mut world);
            assert_eq!(drawn.iter().map(|(face, _)| *face).collect::<Vec<_>>(), lit, "{emit:?} lit the wrong ends");
            for (face, uniform) in drawn {
                assert_eq!(uniform, face_color(&world, temperature(&faces[face])));
            }
            assert!(world.resource::<Exhausts>().cones.is_empty(), "an emit has no cone");
        }
    }

    /// Glows under the hull, a cone only once selected, and neither once the drive is out.
    #[test]
    fn a_burn_is_glowed_and_coned_under_its_own_hull() {
        let craft = Some(ShipId(1));
        let (mut world, root) = scene(1.0e17);
        let run = |world: &mut World| {
            draw(world);
            let glows: Vec<Entity> = world.query::<(&Glow, &ChildOf)>().iter(world).map(|(_, c)| c.parent()).collect();
            let cones = world.query::<&Cone>().iter(world).count();
            (glows, cones, world.resource::<Exhausts>().cones.len())
        };

        let (glows, cones, drawn) = run(&mut world);
        assert_eq!(glows, [root, root], "one glow per aft face, under the hull");
        assert_eq!((cones, drawn), (0, 0), "outside its radius and unselected");

        world.resource_mut::<crate::app::Ui>().0.selected_craft = craft;
        let (glows, cones, drawn) = run(&mut world);
        assert_eq!((glows.len(), cones, drawn), (2, 1, 1), "selected");

        world.resource_mut::<crate::uplink::Uplink>().contacts[0].drive_w = 0.0;
        let (glows, cones, drawn) = run(&mut world);
        assert_eq!((glows.len(), cones, drawn), (0, 0, 0), "the drive went out");
    }

    /// The reaction drive's plume is retired, with its material, shader and churn.
    #[test]
    fn no_plume_material_remains() {
        let root = env!("CARGO_MANIFEST_DIR");
        for gone in ["assets/shaders/plume.wgsl", "assets/textures/plume.tgraph", "../em-render/src/plume_material.rs"] {
            assert!(!std::path::Path::new(root).join(gone).exists(), "{gone} is back");
        }
        assert!(!include_str!("../../em-render/src/lib.rs").contains("pub mod plume_material"));
    }
}
