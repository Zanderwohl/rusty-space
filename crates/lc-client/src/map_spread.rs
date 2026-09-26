//! Error bars on the map: straight, or along the orbit a phase error lies on, each capped at
//! its open ends.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_map::{ItemKey, Placement, Spread};
use em_render::body_material::BASE_TUBE_RADIUS;
use em_render::wire_mesh;

use crate::map_line::MapLineMaterial;
use crate::map_scene::{MapDrawn, SPREAD_CAP_PX, render, unit_bounds};

/// One piece of an item's spread, by its place in [`Placement::spread`], and which part of it.
#[derive(Component)]
pub struct MapSpreadOf {
    pub key: ItemKey,
    pub piece: usize,
    pub part: SpreadPart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadPart {
    Line,
    /// By its place in [`Spread::ends`].
    Cap(usize),
}

/// What an arc's mesh was built from: its points about the item, render axes and units.
#[derive(Component)]
pub struct ArcShape(Vec<Vec3>);

/// How far an arc may drift from the mesh it was built as before it is rebuilt, as a fraction
/// of its own reach. Well under a pixel for anything the map frames.
const ARC_REBUILD_FRACTION: f32 = 1.0e-3;

/// What was spawned for one item's spread.
#[derive(Default)]
pub(crate) struct Spawned {
    pub(crate) entities: Vec<Entity>,
    layout: Vec<Layout>,
}

/// What decides which entities a spread needs. Anything else is a transform or a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    arc: bool,
    ends: usize,
}

fn layout(spread: &[Spread<Vec3>]) -> Vec<Layout> {
    spread
        .iter()
        .map(|piece| Layout { arc: matches!(piece, Spread::Arc { .. }), ends: piece.ends().len() })
        .collect()
}

/// Spawn what `placement`'s spread needs, respawning only if its layout changed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync(
    commands: &mut Commands,
    spawned: &mut Spawned,
    placement: &Placement,
    material: impl FnOnce() -> Handle<MapLineMaterial>,
    bar: &Handle<Mesh>,
    meshes: &mut Assets<Mesh>,
    layer: &RenderLayers,
    rad_per_px: f32,
) {
    let wanted = layout(&placement.spread);
    if wanted == spawned.layout {
        return;
    }
    for entity in spawned.entities.drain(..) {
        commands.entity(entity).despawn();
    }
    spawned.layout = wanted;
    if placement.spread.is_empty() {
        return;
    }
    let material = material();
    for (piece, spread) in placement.spread.iter().enumerate() {
        let of = |part| MapSpreadOf { key: placement.key, piece, part };
        let line = match spread {
            Spread::Bar(near, far) => commands.spawn((
                // One dash: a solid line.
                Mesh3d(bar.clone()),
                MeshMaterial3d(material.clone()),
                segment_transform(*near, *far),
                unit_bounds(),
                layer.clone(),
                MapDrawn,
                of(SpreadPart::Line),
            )),
            Spread::Arc { points, .. } => {
                let shape = arc_shape(points, placement.at);
                commands.spawn((
                    Mesh3d(meshes.add(arc_mesh(&shape))),
                    MeshMaterial3d(material.clone()),
                    arc_transform(placement),
                    NoFrustumCulling,
                    layer.clone(),
                    MapDrawn,
                    ArcShape(shape),
                    of(SpreadPart::Line),
                ))
            }
        };
        spawned.entities.push(line.id());
        for (end, (at, beside)) in spread.ends().into_iter().enumerate() {
            let cap = commands.spawn((
                Mesh3d(bar.clone()),
                MeshMaterial3d(material.clone()),
                cap_transform(at, beside, rad_per_px),
                unit_bounds(),
                layer.clone(),
                MapDrawn,
                of(SpreadPart::Cap(end)),
            ));
            spawned.entities.push(cap.id());
        }
    }
}

/// Place one part where `placement` now says, rebuilding an arc's mesh if its shape moved.
pub(crate) fn lay(
    placement: &Placement,
    of: &MapSpreadOf,
    place: &mut Transform,
    mesh: &mut Mesh3d,
    shape: Option<Mut<ArcShape>>,
    meshes: &mut Assets<Mesh>,
    rad_per_px: f32,
) {
    let Some(spread) = placement.spread.get(of.piece) else { return };
    match (of.part, spread) {
        (SpreadPart::Line, Spread::Bar(near, far)) => *place = segment_transform(*near, *far),
        (SpreadPart::Line, Spread::Arc { points, .. }) => {
            *place = arc_transform(placement);
            let Some(mut built) = shape else { return };
            let now = arc_shape(points, placement.at);
            if moved(&built.0, &now) {
                mesh.0 = meshes.add(arc_mesh(&now));
                built.0 = now;
            }
        }
        (SpreadPart::Cap(end), _) => {
            if let Some((at, beside)) = spread.ends().get(end) {
                *place = cap_transform(*at, *beside, rad_per_px);
            }
        }
    }
}

/// The arc about the item it belongs to, so `f32` resolves it however far off the eye is.
fn arc_shape(points: &[Vec3], at: Vec3) -> Vec<Vec3> {
    points.iter().map(|p| render((*p - at).as_dvec3())).collect()
}

/// Built at its true size rather than a unit one, so the shader's width cap is the same as a
/// bar's and the two arms of a cross match.
fn arc_mesh(shape: &[Vec3]) -> Mesh {
    wire_mesh::tube_curves(&[shape.to_vec()], BASE_TUBE_RADIUS, 4, 0.8)
}

fn arc_transform(placement: &Placement) -> Transform {
    Transform::from_translation(render(placement.at.as_dvec3()))
}

fn moved(built: &[Vec3], now: &[Vec3]) -> bool {
    if built.len() != now.len() {
        return true;
    }
    let reach = now.iter().map(|p| p.length()).fold(0.0, f32::max);
    let tolerance = reach * ARC_REBUILD_FRACTION;
    built.iter().zip(now).any(|(a, b)| a.distance(*b) > tolerance)
}

/// The unit line along `+Y`, laid from `near` to `far`.
fn segment_transform(near: Vec3, far: Vec3) -> Transform {
    let (near, far) = (render(near.as_dvec3()), render(far.as_dvec3()));
    let span = far - near;
    let length = span.length();
    Transform {
        translation: near,
        rotation: match length > f32::EPSILON {
            true => Quat::from_rotation_arc(Vec3::Y, span / length),
            false => Quat::IDENTITY,
        },
        scale: Vec3::new(1.0, length, 1.0),
    }
}

/// Square to both the line it closes and the line of sight, so it looks square from any angle.
/// `beside` is the next point along, which for an arc is the direction it leaves the end in.
/// The eye is the render origin, so a point is its own line of sight.
pub(crate) fn cap_transform(end: Vec3, beside: Vec3, rad_per_px: f32) -> Transform {
    let (end, beside) = (render(end.as_dvec3()), render(beside.as_dvec3()));
    let along = (beside - end).normalize_or(Vec3::Y);
    let across = along.cross(end).try_normalize().unwrap_or_else(|| along.any_orthonormal_vector());
    let length = end.length() * rad_per_px * SPREAD_CAP_PX;
    Transform {
        translation: end - across * (0.5 * length),
        rotation: Quat::from_rotation_arc(Vec3::Y, across),
        scale: Vec3::new(1.0, length, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arc's cap closes the arc where it ends, square to the way the arc leaves it rather
    /// than to the chord between its ends.
    #[test]
    fn an_arcs_cap_is_square_to_the_arc_at_its_end() {
        let at = Vec3::new(0.0, 40.0, 0.0);
        let points: Vec<Vec3> =
            (0..=8).map(|i| at + Vec3::new((i as f32 * 0.1).sin(), 1.0 - (i as f32 * 0.1).cos(), 0.0)).collect();
        let arc = Spread::Arc { points: points.clone(), closed: false };
        let ends = arc.ends();
        let cap = cap_transform(ends[0].0, ends[0].1, 1.0e-3);
        let across = cap.rotation * Vec3::Y;
        let leaving = (render(points[1].as_dvec3()) - render(points[0].as_dvec3())).normalize();
        assert!(across.dot(leaving).abs() < 1.0e-4, "not square to the arc");
        let chord = (render(points[8].as_dvec3()) - render(points[0].as_dvec3())).normalize();
        assert!(across.dot(chord).abs() > 1.0e-3, "square to the chord instead: this proves nothing");
    }

    /// An arc is rebuilt when it moves along the orbit, not for every rounding of the eye.
    #[test]
    fn an_arc_is_rebuilt_only_when_its_shape_moves() {
        let shape = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0)];
        let jitter: Vec<Vec3> = shape.iter().map(|p| *p + Vec3::splat(1.0e-6)).collect();
        assert!(!moved(&shape, &jitter));
        let slid: Vec<Vec3> = shape.iter().map(|p| *p + Vec3::X * 0.01).collect();
        assert!(moved(&shape, &slid));
        assert!(moved(&shape, &shape[..2]));
    }
}
