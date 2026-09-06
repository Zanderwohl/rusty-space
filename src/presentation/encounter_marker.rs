//! Markers at sphere-of-influence crossings.
//!
//! For the focused body, the search looks back and forward over a few of its revolutions
//! and marks the nearest crossing in each direction with a three-ringed target, square to
//! the track, as though pasted to the wall of the sphere where the craft goes through it.
//! The detection itself is [`em_sim::influence::crossings`]; this module draws the answer.
//!
//! Nothing here changes the trajectory. A crossing is a geometric fact about the path as
//! drawn, and the path keeps going straight through it.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy_mesh::{Indices, PrimitiveTopology};

use bevy::math::DVec3;
use em_foundations::time::Instant;
use em_sim::motive::Frontier;
use em_sim::{patch, propagate};

use crate::body::universe::save::ViewSettings;
use crate::camera::Freecam;
use crate::gui::planetarium::FocusedBodyState;
use crate::sim::world::SimSystem;

use super::encounter_marker_material::EncounterMarkerMaterial;
use super::render_space::ToRender;

/// Stations around each circle. The marker holds a constant angular size, so this is a
/// fixed on-screen smoothness rather than something that has to scale with anything.
const MARKER_STATIONS: usize = 96;

/// Sides of each tube's cross-section. Three is enough for a line a pixel or so wide.
const MARKER_TUBE_SIDES: usize = 4;

/// Concentric circles per marker.
const MARKER_RINGS: usize = 3;

/// Which of the two crossings a marker shows.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EncounterMarkerKind {
    Previous,
    Next,
}

/// A drawn crossing. Held on the marker entity so the panel and the picker can read what
/// it refers to without recomputing the search.
#[derive(Component)]
pub struct EncounterMarker {
    pub kind: EncounterMarkerKind,
    /// When the crossing this marker is showing happens, if any.
    pub time: Option<Instant>,
}

/// How much solving to do in one frame, in boundary evaluations.
///
/// The chain of a craft that ends up on a heliocentric arc can take seconds to work out in
/// full, and a frame has milliseconds. So the work is handed out a slice at a time: the
/// timeline fills in over the next few frames and the framerate never notices. Bigger
/// slices fill in sooner and cost more per frame.
///
/// At this size a lunar flyby's whole chain lands in a few dozen frames, at a few
/// milliseconds each in a dev build — where this workspace's own crates are unoptimised.
const SOLVE_SAMPLES_PER_FRAME: usize = 250;

/// Which arena the chains on the timelines belong to.
///
/// Only invalidation lives here — the progress itself is on each body's own timeline, in
/// [`Frontier`](em_sim::motive::Frontier), so an interrupted solve resumes rather than
/// restarts.
#[derive(Resource, Default)]
pub struct FlightPlans {
    generation: u32,
}

/// Work on the focused body's chain of arcs, a slice per frame.
///
/// Runs before the simulation advances so the arena rebuilds around any new events in the
/// same frame it gains them.
pub fn advance_flight_plan(
    mut system: ResMut<SimSystem>,
    focused: Res<FocusedBodyState>,
    mut plans: ResMut<FlightPlans>,
) {
    let Some(traveller) = focused
        .current_body_id
        .as_deref()
        .and_then(|id| system.0.by_name(id))
    else {
        return;
    };

    // A structural edit moves the bodies an arc was measured against, so the chain has to
    // be worked out again from the epoch.
    if plans.generation != system.0.generation() {
        system.0.motive_mut(traveller).set_frontier(Frontier::Unsolved);
    }

    patch::advance(&mut system.0, traveller, SOLVE_SAMPLES_PER_FRAME);

    // Advancing edits the motive, which bumps the generation; record where that left it, or
    // the next frame would read this solve as invalidating itself and start over forever.
    plans.generation = system.0.generation();
}

/// The shared marker mesh, built once.
#[derive(Resource)]
pub struct EncounterMarkerMesh(pub Handle<Mesh>);

impl FromWorld for EncounterMarkerMesh {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        Self(meshes.add(marker_mesh(MARKER_RINGS, MARKER_STATIONS, MARKER_TUBE_SIDES)))
    }
}

/// Concentric circles as closed tubes, stored as angles rather than positions.
///
/// `POSITION` is `(cos phi, sin phi, ring index)` and `NORMAL` is `(cos c, sin c, 0)`. The
/// vertex shader turns those into a circle of the right screen size at the right place, so
/// this mesh never needs rebuilding — not when the camera moves, not when the crossing
/// moves, not when the marker changes plane.
fn marker_mesh(rings: usize, stations: usize, sides: usize) -> Mesh {
    let mut positions = Vec::with_capacity(rings * stations * sides);
    let mut normals = Vec::with_capacity(rings * stations * sides);
    let mut indices = Vec::with_capacity(rings * stations * sides * 6);

    for ring in 0..rings {
        let ring_base = (ring * stations * sides) as u32;

        for station in 0..stations {
            let phi = std::f32::consts::TAU * station as f32 / stations as f32;
            let (sin_phi, cos_phi) = phi.sin_cos();

            for side in 0..sides {
                let c = std::f32::consts::TAU * side as f32 / sides as f32;
                let (sin_c, cos_c) = c.sin_cos();
                positions.push([cos_phi, sin_phi, ring as f32]);
                normals.push([cos_c, sin_c, 0.0]);
            }

            let base = ring_base + (station * sides) as u32;
            let next = ring_base + (((station + 1) % stations) * sides) as u32;
            for side in 0..sides {
                let s = side as u32;
                let s_next = ((side + 1) % sides) as u32;
                indices.extend_from_slice(&[
                    base + s, next + s, base + s_next,
                    base + s_next, next + s, next + s_next,
                ]);
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Create the two markers once, hidden until there is a crossing to put them on.
pub fn spawn_encounter_markers(
    mut commands: Commands,
    existing: Query<&EncounterMarker>,
    shared: Res<EncounterMarkerMesh>,
    mut materials: ResMut<Assets<EncounterMarkerMaterial>>,
) {
    if !existing.is_empty() {
        return;
    }
    for kind in [EncounterMarkerKind::Previous, EncounterMarkerKind::Next] {
        commands.spawn((
            Mesh3d(shared.0.clone()),
            MeshMaterial3d(materials.add(EncounterMarkerMaterial::default())),
            Transform::default(),
            Visibility::Hidden,
            // The mesh is a unit circle blown up in the vertex shader; its computed bounds
            // say nothing about where it draws.
            NoFrustumCulling,
            EncounterMarker { kind, time: None },
        ));
    }
}

/// Place the markers on the crossings either side of the clock.
///
/// Only the joins bounding the arc in force are shown: a target marks where *this* orbit
/// begins and ends, so it has no meaning while a different arc is being flown.
///
/// No search: the chain was solved when the plan changed, so this reads the traveller's own
/// event list and evaluates two instants.
pub fn update_encounter_markers(
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
    focused: Res<FocusedBodyState>,
    camera: Query<&Freecam, With<crate::camera::PlanetariumCamera>>,
    mut markers: Query<(
        &mut EncounterMarker,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<EncounterMarkerMaterial>,
    )>,
    mut materials: ResMut<Assets<EncounterMarkerMaterial>>,
) {
    let Ok(freecam) = camera.single() else { return };
    let scale = view_settings.distance_factor();
    let now = system.0.time();

    let traveller = focused
        .current_body_id
        .as_deref()
        .and_then(|id| system.0.by_name(id));

    // The arc in force now is the one the trajectory is drawn from, so it is the frame
    // every marker is anchored in — whichever arc the crossing itself belongs to.
    let anchor = traveller.and_then(|t| system.0.parent(t));

    // Only the two joins that bound the arc actually being flown. A join further along the
    // chain belongs to an arc that is not happening yet, and one further back to an arc
    // already left; drawing either puts a target on a trajectory that is not on screen.
    let (previous, next) = traveller
        .map(|t| system.0.motive(t).bounding_soi_changes(now))
        .unwrap_or((None, None));

    for (mut marker, mut transform, mut visibility, handle) in markers.iter_mut() {
        let time = match marker.kind {
            EncounterMarkerKind::Previous => previous,
            EncounterMarkerKind::Next => next,
        };
        marker.time = time;

        let placed = time
            .zip(traveller)
            .zip(anchor)
            .and_then(|((time, traveller), anchor)| place(&system.0, traveller, anchor, time));

        let Some((offset, plane_x, plane_y)) = placed else {
            *visibility = Visibility::Hidden;
            continue;
        };

        // Anchor where the trajectory is drawn, not where the crossing happens in inertial
        // space: the orbit is drawn about the primary's *current* position, so a crossing a
        // fortnight out would otherwise land where the primary will be by then — tens of
        // millions of kilometres off the drawn path.
        let drawn = system.0.position(anchor.expect("checked above")) + offset;
        transform.translation = drawn.to_render_relative(scale, freecam.bevy_pos);
        *visibility = Visibility::Visible;

        if let Some(current) = materials.get(handle.id()) {
            let moved = (current.uniform.plane_x - plane_x).length() > 1.0e-4
                || (current.uniform.plane_y - plane_y).length() > 1.0e-4;
            if moved {
                // `get_mut` flags a GPU re-upload, so only take it when something changed.
                if let Some(material) = materials.get_mut(handle.id()) {
                    material.uniform.plane_x = plane_x;
                    material.uniform.plane_y = plane_y;
                }
            }
        }
    }
}

/// Where a crossing sits relative to `anchor`, and how the target is turned there.
///
/// Both arcs meeting at a join describe the same state, so which one answers does not
/// matter — the state is evaluated in absolute terms and then measured from the anchor.
fn place(
    system: &em_sim::system::System,
    traveller: em_sim::id::BodyIndex,
    anchor: em_sim::id::BodyIndex,
    time: Instant,
) -> Option<(DVec3, Vec4, Vec4)> {
    let (position, velocity) = propagate::state_at(system, traveller, time)?;
    let (anchor_position, anchor_velocity) = propagate::state_at(system, anchor, time)?;

    let local_position = position - anchor_position;
    let local_velocity = velocity - anchor_velocity;
    let normal = local_position.cross(local_velocity);
    if normal.length_squared() == 0.0 || local_velocity.length_squared() == 0.0 {
        return None;
    }

    let (plane_x, plane_y) = marker_face(local_velocity, normal)?;
    Some((local_position, plane_x, plane_y))
}

/// The face of the target, in render space, as two perpendicular in-plane axes.
///
/// The marker faces along the velocity — the craft flies into it, like a target pasted to
/// the wall of the sphere — so its plane is at right angles to the orbital plane. The two
/// axes spanning that face are the orbital plane's normal and the in-plane direction
/// square to the track.
///
/// `None` for a degenerate orbit, which has no plane to square up against.
fn marker_face(velocity: DVec3, orbit_normal: DVec3) -> Option<(Vec4, Vec4)> {
    // Sim space is Z-up and the renderer Y-up; `ToRender` is the only conversion.
    let facing = velocity.to_render().normalize_or_zero();
    let orbit_normal = orbit_normal.to_render().normalize_or_zero();
    if facing == Vec3::ZERO || orbit_normal == Vec3::ZERO {
        return None;
    }

    // Orthogonalise: `r x v` is perpendicular to `v` in exact arithmetic, but both arrive
    // here through a narrowing to f32.
    let plane_x = (orbit_normal - facing * facing.dot(orbit_normal)).normalize_or_zero();
    if plane_x == Vec3::ZERO {
        return None;
    }
    let plane_y = facing.cross(plane_x);
    Some((plane_x.extend(0.0), plane_y.extend(0.0)))
}
