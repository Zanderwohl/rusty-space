//! A photon drive's burn: each aft engine's open face glowing at the flux leaving it, and the
//! exhaust's cone out to the courtesy radius. 32 §The exhaust cone.
//!
//! The glow is light, drawn on every burning craft with a form, under its hull's root so it moves
//! with the hull. The cone is an indicator, drawn for the player's own burn, for a burn whose
//! courtesy radius the player is inside, and for a selected ship; [`Exhausts`] hands the same cones
//! to the map. Another craft is drawn as its light shows it, from what its `Presence` stated.
//!
//! Each proxy is told where the eye is in its own space, worked out in `f64`: a render unit is an
//! AU, and a cone is thousands of hull lengths.

use std::collections::HashMap;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::exhaust_cone_material::{
    ApertureGlowMaterial, ApertureGlowUniform, ExhaustConeMaterial, ExhaustConeUniform,
};
use em_render::render_space::{render_to_sim, sim_to_render};
use glam::{DQuat, DVec3};
use lc_proto::ShipId;
use lc_world::courtesy::{cooking_distance_m, cooking_flux_w_m2, drive_courtesy_radius_m};
use lc_world::emit::aperture_temperature_k;
use lc_world::fitting::Balance;
use lc_world::flight::{C_M_S, Drive};
use lc_world::form::Form;
use lc_world::form::capacity::{Aperture, aft_apertures};

use crate::hull::Eye;
use crate::session::Session;
use crate::ship_hull::{RealHulls, ShipHull};
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

const LUMA: DVec3 = DVec3::new(0.2126, 0.7152, 0.0722);

/// `F c`, watts: what a photon drive of the thrust a reaction drive states as `½ F v` sends aft.
///
/// `Drive` and the wire still state `½ F v`; courtesy and emission are photon drives.
pub fn exhaust_w(jet_power_w: f64, exhaust_v_m_s: f64) -> f64 {
    if jet_power_w <= 0.0 || exhaust_v_m_s <= 0.0 {
        return 0.0;
    }
    2.0 * jet_power_w * C_M_S / exhaust_v_m_s
}

/// A craft with its drive lit.
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

/// The cone's length, which is the drive's courtesy radius, when `lit`'s cone is drawn for an
/// observer at `here_ly` with `selected` picked out.
pub fn cone_m(lit: &Lit, here_ly: DVec3, selected: Option<ShipId>, balance: &Balance) -> Option<f64> {
    if lit.power_w <= 0.0 {
        return None;
    }
    let radius_m = drive_courtesy_radius_m(balance, lit.power_w);
    let inside = lit.at_ly.distance(here_ly) * M_PER_LY <= radius_m;
    let chosen = lit.craft.is_some() && lit.craft == selected;
    (lit.craft.is_none() || inside || chosen).then_some(radius_m)
}

/// An open face's temperature, kelvin: its share of `power_w` through its area.
pub fn face_k(power_w: f64, aperture: &Aperture) -> f64 {
    aperture_temperature_k(power_w * aperture.share, std::f64::consts::PI * aperture.radius_m.powi(2))
}

/// A cone for a drive of `power_w`, `length_m` long, in the hazard color. The eye is the caller's.
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

/// A face radiating `face`, band-mapped linear RGB on the exposure's scale, under a tone map of
/// `reference` and `stops`. The eye is the caller's.
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

/// One cone drawn this frame.
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

/// The cones drawn this frame, and what drawing them keeps.
#[derive(Resource, Default)]
pub struct Exhausts {
    pub cones: Vec<Drawn>,
    /// Each craft's faces, by the hash of the form they were worked out from.
    apertures: HashMap<Option<ShipId>, (u64, Vec<Aperture>)>,
    /// By the half-angle's bits.
    cone_proxy: Option<(u64, Handle<Mesh>)>,
    glow_proxy: Option<Handle<Mesh>>,
}

/// A craft's exhaust cone.
#[derive(Component)]
pub struct Cone(pub Option<ShipId>);

/// One aft face's glow, under its craft's [`ShipHull`], by its place in [`aft_apertures`].
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

fn lits(session: &Session, uplink: &crate::uplink::Uplink) -> Vec<Lit> {
    let now = session.coordinate_time_s();
    let ship = &session.ship;
    let own = Lit {
        craft: None,
        power_w: exhaust_w(ship.jet_power_w(now), ship.motion.drive.exhaust_v_m_s),
        at_ly: ship.motion.position_ly,
        facing: ship.facing_at(now).unwrap_or(DVec3::X),
        length_m: ship.length_m,
    };
    // A contact's exhaust speed is not stated, and every craft with a drive worth drawing is a ship.
    let contacts = uplink.contacts.iter().map(|c| Lit {
        craft: Some(c.ship_id),
        power_w: exhaust_w(c.jet_power_w, Drive::DEFAULT.exhaust_v_m_s),
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

/// `craft`'s aft faces, from the form it is stated in, worked out again only when that changes.
fn faces<'a>(
    cache: &'a mut HashMap<Option<ShipId>, (u64, Vec<Aperture>)>,
    craft: Option<ShipId>,
    stated: Option<u64>,
    form: impl FnOnce() -> Option<(Form, Balance)>,
) -> Option<&'a [Aperture]> {
    let hash = stated?;
    if cache.get(&craft).is_none_or(|(held, _)| *held != hash) {
        let (form, balance) = form()?;
        cache.insert(craft, (hash, aft_apertures(&form, &balance).unwrap_or_default()));
    }
    cache.get(&craft).map(|(_, faces)| faces.as_slice())
}

/// What a blackbody at `kelvin` looks like through this observer's bands, on the exposure's scale.
fn shine(session: &Session, kelvin: f64) -> DVec3 {
    Vec3::from_array(session.mapping.apply(&crate::session::spectrum_at(kelvin))).as_dvec3()
}

/// Glow every burning craft's aft faces, and draw the cones [`cone_m`] asks for. After the hulls,
/// whose roots the glows hang from.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn draw_exhaust(
    mut commands: Commands,
    (game, ui, uplink, eye): (Res<crate::app::Game>, Res<crate::app::Ui>, Res<crate::uplink::Uplink>, Res<Eye>),
    (own, real): (Res<crate::parts::OwnForm>, Res<RealHulls>),
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
    let lit = lits(session, &uplink);
    let tone = &session.tone;
    let exhausts = &mut *exhausts;
    exhausts.cones.clear();
    exhausts.apertures.retain(|craft, _| real.stated(*craft).is_some());

    let mut want_glows = Vec::new();
    let mut want_cones = Vec::new();
    for burn in &lit {
        let root = roots.iter().find(|(_, hull, _)| hull.craft() == burn.craft).map(|(e, _, t)| (e, Root::of(t)));
        let form = || match burn.craft {
            None => own.form().map(|f| (f.clone(), own.balance())),
            Some(id) => uplink
                .contacts
                .iter()
                .find(|c| c.ship_id == id)
                .filter(|c| !c.form.parts.is_empty())
                .map(|c| (Form::from(&c.form), balance)),
        };
        let apertures = root
            .and_then(|_| faces(&mut exhausts.apertures, burn.craft, real.stated(burn.craft), form))
            .unwrap_or_default();

        if let Some((entity, root)) = root {
            let eye_ship = root.eye();
            for (index, face) in apertures.iter().enumerate() {
                let rotation = DQuat::from_rotation_arc(DVec3::Y, face.out);
                let transform = Transform {
                    translation: face.center.as_vec3(),
                    rotation: rotation.as_quat(),
                    scale: Vec3::splat(face.radius_m as f32),
                };
                let eye_local = rotation.inverse() * (eye_ship - face.center) / face.radius_m;
                let mut uniforms = aperture_uniform(
                    shine(session, face_k(burn.power_w, face)),
                    tone.surface_reference as f64,
                    tone.surface_stops,
                );
                uniforms.eye_local = eye_local.as_vec3().extend(0.0);
                want_glows.push((burn.craft, index, entity, transform, uniforms));
            }
        }

        let Some(length_m) = cone_m(burn, here, ui.selected_craft, &balance) else { continue };
        // From the faces' power-weighted middle, or the stern of a craft drawn with none.
        let (apex, aft, apex_from_center_m) = match root {
            Some((_, root)) if !apertures.is_empty() => {
                let middle: DVec3 = apertures.iter().map(|a| a.center * a.share).sum();
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
    use lc_world::form::capacity::aft_aperture_w;
    use lc_world::form::presets::Builtin;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn lit(craft: Option<ShipId>, power_w: f64, at_m: DVec3) -> Lit {
        Lit { craft, power_w, at_ly: at_m / M_PER_LY, facing: DVec3::X, length_m: 500.0 }
    }

    /// Your own burn, a burn whose courtesy radius you are in, and a selected ship's; nobody
    /// else's, and nothing that is not burning.
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

    /// Each face radiates its share of the drive's power through its own area.
    #[test]
    fn a_face_glows_at_its_share_of_the_drive() {
        let power = 3.0e19;
        let start = aft_apertures(&Form::starting(), &B).unwrap();
        let face = start[0];
        let area = std::f64::consts::PI * face.radius_m * face.radius_m;
        assert_eq!(face_k(power, &face), aperture_temperature_k(power, area));

        let plate = aft_apertures(&Builtin::Plate.form(), &B).unwrap();
        assert_eq!(plate.len(), 2);
        for face in &plate {
            let area = std::f64::consts::PI * face.radius_m * face.radius_m;
            assert_eq!(face_k(power, face), aperture_temperature_k(power / 2.0, area));
        }
    }

    /// 32's figure: the starting drive at its rating, through its bell's 88 m face.
    #[test]
    fn the_starting_face_is_five_hundred_thousand_kelvin() {
        let start = Form::starting();
        let face = aft_apertures(&start, &B).unwrap()[0];
        let k = face_k(aft_aperture_w(&start, &B).unwrap(), &face);
        assert!((k / 5.3e5 - 1.0).abs() < 0.01, "{k} K");
    }

    /// What pushes a photon drive is `F c`, from the reaction drive's `½ F v` of the same thrust.
    #[test]
    fn the_exhaust_carries_thrust_times_c() {
        let drive = Drive::DEFAULT;
        let (mass, g) = (2.0e9, 5.0);
        let photon = lc_world::emit::thrust_power_w(mass, g * lc_world::flight::G0);
        let from = exhaust_w(drive.jet_power_w(mass, g), drive.exhaust_v_m_s);
        assert!((from / photon - 1.0).abs() < 1.0e-12, "{from} against {photon}");
        assert_eq!(exhaust_w(0.0, drive.exhaust_v_m_s), 0.0);
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

    /// The reaction drive's plume is retired, with its material, shader and churn.
    #[test]
    fn no_plume_material_remains() {
        let root = env!("CARGO_MANIFEST_DIR");
        for gone in ["assets/shaders/plume.wgsl", "assets/textures/plume.tgraph", "../em-render/src/plume_material.rs"] {
            assert!(!std::path::Path::new(root).join(gone).exists(), "{gone} is back");
        }
        assert!(!include_str!("../../em-render/src/lib.rs").contains("plume"));
    }
}
