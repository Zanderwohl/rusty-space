//! The cones [`crate::plume`] draws, on the map as lines in the hazard color: the generators, the
//! rim at the courtesy radius, and a ring where a Black receiver would cook. This ship's beams are
//! drawn the same way, and a beam landing on it as a line back along its bearing: see
//! [`crate::emit_panel::on_map`].

use std::collections::HashMap;
use std::f32::consts::TAU;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_render::body_material::BASE_TUBE_RADIUS;
use em_render::render_space::sim_to_render;
use em_render::wire_mesh;
use glam::{DQuat, DVec3};
use lc_proto::ShipId;

use crate::emit_panel::OnMap;
use crate::map::{MAP_LAYER, Map};
use crate::map_line::MapLineMaterial;
use crate::map_scene::{LINE_COLOR_SCALE, LINE_PX, LINE_TUBE_FRACTION, line_material};
use crate::plume::{Drawn, Exhausts};
use crate::system::M_PER_LY;

const GENERATORS: usize = 8;
const RIM_SEGMENTS: usize = 64;
/// Under a line's width at any length, and a rim this small is a ring of coincident points in f32.
const AXIS_RAD: f64 = 1.0e-4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Exhaust(Option<ShipId>),
    Beam(OnMap),
}

#[derive(Component)]
pub struct MapCone(Key);

/// One mesh per shape, which a balance fixes.
#[derive(Default)]
pub(crate) struct Held {
    material: Option<Handle<MapLineMaterial>>,
    meshes: HashMap<(u64, u64), Handle<Mesh>>,
}

/// A cone of `half_angle_rad` one unit long down `+Y` from its apex at the origin, as polylines:
/// the generators, the rim, and a ring `cooking` of the way along when that is inside it. A cone
/// narrower than [`AXIS_RAD`] is its axis.
pub(crate) fn cone_curves(half_angle_rad: f64, cooking: f64) -> Vec<Vec<Vec3>> {
    if half_angle_rad < AXIS_RAD {
        return vec![vec![Vec3::ZERO, Vec3::Y]];
    }
    let tan = half_angle_rad.tan() as f32;
    let at = |along: f32, turn: f32| Vec3::new(tan * along * turn.cos(), along, tan * along * turn.sin());
    let ring = |along: f32| -> Vec<Vec3> {
        (0..=RIM_SEGMENTS).map(|i| at(along, TAU * i as f32 / RIM_SEGMENTS as f32)).collect()
    };
    let mut curves: Vec<Vec<Vec3>> =
        (0..GENERATORS).map(|i| vec![Vec3::ZERO, at(1.0, TAU * i as f32 / GENERATORS as f32)]).collect();
    curves.push(ring(1.0));
    if cooking > 0.0 && cooking < 1.0 {
        curves.push(ring(cooking as f32));
    }
    curves
}

/// The unit cone's apex put at the cone's, about the map's eye, turned down the exhaust and grown
/// to its length, render units.
pub(crate) fn cone_transform(cone: &Drawn, eye_ly: DVec3, meters_per_unit: f64) -> Transform {
    let apex = sim_to_render((cone.apex_ly - eye_ly) * M_PER_LY / meters_per_unit);
    Transform {
        translation: apex.as_vec3(),
        rotation: DQuat::from_rotation_arc(DVec3::Y, sim_to_render(cone.aft)).as_quat(),
        scale: Vec3::splat((cone.length_m / meters_per_unit) as f32),
    }
}

/// Bring the map's cones to [`Exhausts`] and this ship's beams: after the rest of the layer, from
/// the same frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lay_cones(
    mut commands: Commands,
    map: Res<Map>,
    exhausts: Res<Exhausts>,
    game: Res<crate::app::Game>,
    uplink: Res<crate::uplink::Uplink>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MapLineMaterial>>,
    mut held: Local<Held>,
    mut drawn: Query<(Entity, &MapCone, &mut Transform, &mut Mesh3d)>,
) {
    if !map.shown {
        for (entity, ..) in &drawn {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Some(frame) = map.frame.as_ref().filter(|_| map.due) else { return };
    let held = &mut *held;
    let material = held
        .material
        .get_or_insert_with(|| {
            materials.add(line_material(crate::ui::HAZARD, LINE_TUBE_FRACTION, LINE_PX, LINE_COLOR_SCALE))
        })
        .clone();
    let mut mesh_of = |cone: &Drawn| {
        // The same ratio at any power, but from two roots rounded apart, so held to six places.
        let cooking = (cone.cooking_m / cone.length_m * 1.0e6).round() / 1.0e6;
        held.meshes
            .entry((cone.half_angle_rad.to_bits(), cooking.to_bits()))
            .or_insert_with(|| {
                meshes.add(wire_mesh::tube_curves(&cone_curves(cone.half_angle_rad, cooking), BASE_TUBE_RADIUS, 4, 1.0))
            })
            .clone()
    };

    let here_ly = game.0.ship.motion.position_ly;
    // Past the view on any zoom, so a line back along a bearing leaves the picture.
    let reach_m = 4.0 * frame.eye_ly.distance(here_ly) * M_PER_LY;
    let beams = crate::emit_panel::on_map(&uplink.beams, here_ly, game.0.coordinate_time_s(), reach_m);
    let cones: Vec<(Key, &Drawn)> = exhausts
        .cones
        .iter()
        .map(|c| (Key::Exhaust(c.craft), c))
        .chain(beams.iter().map(|(key, c)| (Key::Beam(*key), c)))
        .collect();

    let mut kept = Vec::with_capacity(cones.len());
    for (entity, of, mut transform, mut mesh) in drawn.iter_mut() {
        let Some((_, cone)) = cones.iter().find(|(key, _)| *key == of.0) else {
            commands.entity(entity).despawn();
            continue;
        };
        kept.push(of.0);
        *transform = cone_transform(cone, frame.eye_ly, frame.meters_per_unit);
        let wanted = mesh_of(cone);
        if mesh.0 != wanted {
            mesh.0 = wanted;
        }
    }
    for (key, cone) in cones.iter().filter(|(key, _)| !kept.contains(key)) {
        commands.spawn((
            Mesh3d(mesh_of(cone)),
            MeshMaterial3d(material.clone()),
            cone_transform(cone, frame.eye_ly, frame.meters_per_unit),
            NoFrustumCulling,
            RenderLayers::layer(MAP_LAYER),
            MapCone(*key),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rim is the courtesy radius: every generator ends on it, `tan θ` out at unit length.
    #[test]
    fn the_lines_are_the_cone() {
        let half = 5f64.to_radians();
        let curves = cone_curves(half, 0.3);
        assert_eq!(curves.len(), GENERATORS + 2);
        for generator in &curves[..GENERATORS] {
            assert_eq!(generator[0], Vec3::ZERO);
            let end = generator[1];
            assert!((end.y - 1.0).abs() < 1.0e-6 && (Vec2::new(end.x, end.z).length() - half.tan() as f32).abs() < 1.0e-6);
        }
        let cooking = &curves[GENERATORS + 1];
        assert!(cooking.iter().all(|p| (p.y - 0.3).abs() < 1.0e-6));
        assert_eq!(cone_curves(half, 1.5).len(), GENERATORS + 1, "a cooking distance past the rim is not drawn");
    }

    /// Placed from the map's eye at its scale, pointing down the exhaust, as long as the cone.
    #[test]
    fn a_cone_is_placed_where_its_burn_is() {
        let eye_ly = DVec3::new(1.0e-5, 0.0, 0.0);
        let cone = Drawn {
            craft: None,
            apex_ly: eye_ly + DVec3::new(0.0, 3.0e4, 0.0) / M_PER_LY,
            aft: DVec3::NEG_Z,
            length_m: 2.1e4,
            half_angle_rad: 0.1,
            cooking_m: 700.0,
        };
        let t = cone_transform(&cone, eye_ly, 1.0e3);
        assert!((t.translation - sim_to_render(DVec3::new(0.0, 30.0, 0.0)).as_vec3()).length() < 1.0e-3, "{t:?}");
        assert!((t.scale.x - 21.0).abs() < 1.0e-4);
        assert!((t.rotation * Vec3::Y - sim_to_render(DVec3::NEG_Z).as_vec3()).length() < 1.0e-6);
    }
}
