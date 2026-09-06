//! The rest of the mission: every leg of the chain except the one being flown.
//!
//! The ordinary trajectory renderer draws the arc in force, about the primary in force.
//! That is the right thing for a body on one orbit forever, but a craft on a patched chain
//! has a past and a future in other frames — round a moon, out into the star's, back again
//! — and none of it is visible from the current arc alone.
//!
//! Each leg is drawn about **its own** primary, positioned where that primary is now. Legs
//! therefore do not join end to end on screen: a hyperbola round the Moon and an ellipse
//! round Earth are pictures in two different frames, and no single view holds both without
//! distorting one. Drawing each in its own frame keeps every leg readable at the cost of
//! the seams, which is the trade the map view of every patched-conics game makes.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use em_foundations::time::Instant;
use em_sim::id::BodyIndex;
use em_sim::system::System;
use em_sim::trajectory;

use crate::body::universe::save::ViewSettings;
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::planetarium::FocusedBodyState;
use crate::sim::world::SimSystem;

use super::render_space::ToRender;
use super::trajectory::calculate_tube_radius;
use super::trajectory_material::TrajectoryMaterial;

/// Legs drawn at once. The solver's own patch budget bounds a chain to about this many, and
/// a pool costs nothing when it is not in use.
const MAX_LEGS: usize = 8;

/// Samples per leg. A leg is a fraction of a conic, not a whole one, so it needs fewer
/// points than a full orbit to read smoothly.
const LEG_RESOLUTION: usize = 96;

/// Sides of the tube's cross-section, matching the trajectory tubes.
const TUBE_SIDES: usize = 3;

/// How much dimmer a leg is than the arc being flown. Enough to read as context rather
/// than as the thing you are on.
const LEG_BRIGHTNESS: f32 = 0.28;

/// Below this apparent radius a leg is a few pixels of clutter.
const MIN_APPARENT_RADIUS: f64 = 0.002;

/// How far outside a leg the camera must be for the leg to be worth drawing.
///
/// Legs live in different frames and differ wildly in size. From inside the Earth system a
/// heliocentric leg is an ellipse an astronomical unit across, and the camera sits *on* it
/// — so it is not a shape on screen at all, it is a band across the sky that says nothing
/// about where the craft goes. An angular size alone does not catch that: such a leg
/// subtends less than a radian while filling the view. Being outside it is the test that
/// distinguishes a curve you can see the shape of from one you are inside.
const OUTSIDE_MARGIN: f64 = 1.05;

/// One leg of the chain, and what it was built from.
#[derive(Component)]
pub struct ChainLeg {
    /// Slot in the pool, not an index into the timeline.
    pub slot: usize,
    /// The primary this leg is measured about, if it is showing one.
    primary: Option<BodyIndex>,
    /// Arena generation and view scale the geometry was baked at.
    built: Option<(u32, f64)>,
    /// Which leg is baked here, by the event that starts it.
    ///
    /// Keyed on the leg, not on the slot: the list shifts as the clock crosses joins, since
    /// the arc being flown is left out of it, so a slot holds different legs at different
    /// times and keying on the slot leaves stale geometry behind.
    baked_start: Option<Instant>,
    /// Radius of the baked geometry, in render units, for the screen-size gate.
    extent: f64,
}

/// Give the pool its entities, hidden until a chain needs them.
pub fn spawn_chain_legs(
    mut commands: Commands,
    existing: Query<&ChainLeg>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    if !existing.is_empty() {
        return;
    }
    for slot in 0..MAX_LEGS {
        commands.spawn((
            Mesh3d(meshes.add(empty_leg_mesh())),
            MeshMaterial3d(materials.add(TrajectoryMaterial {
                distance_dim: 1.0,
                front: LEG_BRIGHTNESS,
                back: LEG_BRIGHTNESS,
                // The shader displaces each vertex along its normal by
                // `target - base`, so the baked radius and this must agree. The tube here
                // is baked flat, so the base is zero and the target is the whole thickness.
                base_tube_radius: 0.0,
                dynamic_thickness: 0.0,
                ..Default::default()
            })),
            Transform::default(),
            Visibility::Hidden,
            // Baked in a primary-relative frame and moved by transform, so its bounds say
            // nothing useful about where it draws.
            NoFrustumCulling,
            ChainLeg { slot, primary: None, built: None, baked_start: None, extent: 0.0 },
        ));
    }
}

/// Draw every leg of the focused body's chain except the one it is flying.
pub fn update_chain_legs(
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
    focused: Res<FocusedBodyState>,
    camera: Query<&Freecam, With<PlanetariumCamera>>,
    mut legs: Query<(&mut ChainLeg, &mut Transform, &mut Visibility, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Ok(freecam) = camera.single() else { return };
    let scale = view_settings.distance_factor();
    let now = system.0.time();

    let traveller = focused
        .current_body_id
        .as_deref()
        .and_then(|id| system.0.by_name(id));

    // Every event starts a leg. The one in force is already drawn by the ordinary
    // trajectory renderer, in full, so it is skipped here rather than drawn twice.
    let wanted: Vec<Instant> = match traveller {
        Some(t) => {
            let current = system.0.motive(t).active_segment_range(now).0;
            system
                .0
                .motive(t)
                .iter_events()
                .map(|(time, _, _)| time)
                .filter(|time| Some(*time) != current)
                // The first leg has no event before it, so `active_segment_range` reports
                // its start as `None`; drop it when that is the leg being flown.
                .filter(|time| !(current.is_none() && is_first_event(&system.0, t, *time)))
                .collect()
        }
        None => Vec::new(),
    };

    for (mut leg, mut transform, mut visibility, mesh) in legs.iter_mut() {
        let Some(&start) = wanted.get(leg.slot) else {
            *visibility = Visibility::Hidden;
            leg.primary = None;
            leg.built = None;
            leg.baked_start = None;
            leg.extent = 0.0;
            continue;
        };
        let Some(traveller) = traveller else { continue };

        let Some(primary) = system
            .0
            .motive(traveller)
            .motive_at(start)
            .1
            .primary_id()
            .and_then(|name| system.0.by_name(name))
        else {
            *visibility = Visibility::Hidden;
            continue;
        };

        // Rebake only when the plan, the view scale, or which leg sits in this slot
        // changes. The primary's motion is a transform, not new geometry.
        let key = (system.0.generation(), scale);
        if leg.built != Some(key) || leg.baked_start != Some(start) {
            let Some(path) = trajectory::sample_segment(&system.0, traveller, start, LEG_RESOLUTION)
            else {
                *visibility = Visibility::Hidden;
                continue;
            };
            let points: Vec<Vec3> = path
                .points
                .iter()
                .map(|(_, displacement)| displacement.to_render_scaled_f32(scale))
                .collect();

            leg.extent = points.iter().map(|p| p.length()).fold(0.0f32, f32::max) as f64;

            let Some(mesh) = meshes.get_mut(&mesh.0) else { continue };
            write_tube(mesh, &points);
            leg.built = Some(key);
            leg.baked_start = Some(start);
        }

        leg.primary = Some(primary);
        transform.translation = system.0.position(primary).to_render_relative(scale, freecam.bevy_pos);

        // A leg is drawn in its own primary's frame, so how big it looks has nothing to do
        // with how big the arc being flown looks.
        let distance = transform.translation.length() as f64;
        let apparent = (leg.extent / distance.max(f64::EPSILON)).atan();
        let outside = distance > leg.extent * OUTSIDE_MARGIN;
        *visibility = if outside && apparent > MIN_APPARENT_RADIUS {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Keep the legs' tubes at the same screen weight as the trajectory they continue.
pub fn update_chain_leg_thickness(
    legs: Query<(&Transform, &MeshMaterial3d<TrajectoryMaterial>), With<ChainLeg>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    for (transform, handle) in legs.iter() {
        let wanted = calculate_tube_radius(transform.translation.length());
        let Some(current) = materials.get(handle.id()) else { continue };
        if (current.target_tube_radius - wanted).abs() > 1.0e-9 {
            if let Some(material) = materials.get_mut(handle.id()) {
                material.target_tube_radius = wanted;
            }
        }
    }
}

fn is_first_event(system: &System, traveller: BodyIndex, time: Instant) -> bool {
    system
        .motive(traveller)
        .iter_events()
        .next()
        .is_some_and(|(first, _, _)| first == time)
}

/// A mesh with the layout `trajectory.wgsl` expects, and nothing in it.
fn empty_leg_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(Vec::new()));
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// Replace a mesh with a tube following `points`.
///
/// The shader pushes each vertex along its normal to set thickness, so the normals are what
/// carry the cross-section and the baked radius is nominal.
fn write_tube(mesh: &mut Mesh, points: &[Vec3]) {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    if points.len() >= 2 {
        const NOMINAL_RADIUS: f32 = 0.0;
        for (ring, centre) in points.iter().enumerate() {
            let tangent = if ring == 0 {
                points[1] - points[0]
            } else if ring == points.len() - 1 {
                points[ring] - points[ring - 1]
            } else {
                points[ring + 1] - points[ring - 1]
            }
            .normalize_or_zero();

            let (perp1, perp2) = perpendicular(tangent);
            // A leg is flown once, so brightness runs evenly along it rather than fading
            // round a cycle: amplitude in red, phase in alpha.
            let along = ring as f32 / (points.len() - 1) as f32;

            for side in 0..TUBE_SIDES {
                let angle = std::f32::consts::TAU * side as f32 / TUBE_SIDES as f32;
                let (sin, cos) = angle.sin_cos();
                let offset = perp1 * cos + perp2 * sin;
                let position = *centre + offset * NOMINAL_RADIUS;
                positions.push(position.into());
                normals.push(offset.normalize_or_zero().into());
                colors.push([1.0, 1.0, 1.0, along]);
            }

            if ring + 1 < points.len() {
                let base = (ring * TUBE_SIDES) as u32;
                let next = ((ring + 1) * TUBE_SIDES) as u32;
                for side in 0..TUBE_SIDES as u32 {
                    let side_next = (side + 1) % TUBE_SIDES as u32;
                    indices.extend_from_slice(&[
                        base + side, next + side, base + side_next,
                        base + side_next, next + side, next + side_next,
                    ]);
                }
            }
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));
}

/// Two unit vectors spanning the plane across `dir`.
fn perpendicular(dir: Vec3) -> (Vec3, Vec3) {
    let not_parallel = if dir.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    (perp1, dir.cross(perp1).normalize_or_zero())
}

