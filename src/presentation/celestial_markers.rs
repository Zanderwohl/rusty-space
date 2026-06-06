//! Celestial reference markers rendered as glowing tube-mesh symbols.
//!
//! Currently renders the Point of Aries (♈) at 1 LY along the vernal equinox
//! direction (+X ecliptic J2000), using the same material pipeline as trajectories.

use std::f32::consts::PI;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::render::view::ColorGrading;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

use crate::body::universe::save::ViewSettings;
use crate::camera::{Freecam, PlanetariumCamera};
use crate::util::bevystuff::GlamVec;

use super::trajectory_material::TrajectoryMaterial;

const LIGHT_YEAR_M: f64 = 9.460_730_472_580_8e15;

const TUBE_SIDES: u32 = 6;
const MARKER_BRIGHTNESS: f32 = 2.0;
const CAMERA_MOVE_THRESHOLD: f64 = 0.005;

/// Angular size of the symbol (radians). ~1.7 degrees on screen.
const SYMBOL_ANGULAR_SIZE: f32 = 0.03;
/// Tube wall thickness as a fraction of overall symbol size.
const TUBE_THICKNESS_RATIO: f32 = 0.06;
/// Sample points per horn arc.
const HORN_SAMPLES: u32 = 24;

/// Horn circle-center X offset in symbol-local coordinates.
const HORN_CX: f32 = 0.35;
/// Horn circle-center Y offset in symbol-local coordinates (slightly above cusp for taller horns).
const HORN_CY: f32 = 0.05;
/// Arc sweep per horn (degrees, clockwise). 210° gives horns that rise, curve out, and curl down.
const HORN_SWEEP_DEG: f32 = 210.0;
/// Length of the vertical stem below the horn meeting point.
const STEM_LENGTH: f32 = 0.35;

/// Build an empty mesh that still declares the vertex layout required by
/// `trajectory.wgsl` (`position`, `normal`, `color`).
fn empty_marker_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(Vec::new()),
    );
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

#[derive(Component)]
pub struct CelestialMarker {
    /// Position in simulation space (meters, Z-up ecliptic J2000).
    pub sim_position: DVec3,
}

#[derive(Component, Default)]
pub struct CelestialMarkerCache {
    last_camera_pos: Option<DVec3>,
}

pub fn spawn_celestial_markers(
    mut commands: Commands,
    existing: Query<&CelestialMarker>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    if !existing.is_empty() {
        return;
    }

    // Point of Aries: vernal equinox direction = +X in ecliptic J2000
    let aries_pos = DVec3::new(LIGHT_YEAR_M, 0.0, 0.0);

    let mesh = empty_marker_mesh();
    let mesh_handle = meshes.add(mesh);

    let material = TrajectoryMaterial {
        base_color: LinearRgba::new(0.85, 0.55, 0.15, 1.0),
        brightness_threshold: 0.3,
        emission_strength: 4.0,
        alpha_mode: AlphaMode::Blend,
    };
    let material_handle = materials.add(material);

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::default(),
        Visibility::Visible,
        NoFrustumCulling,
        CelestialMarker { sim_position: aries_pos },
        CelestialMarkerCache::default(),
    ));
}

pub fn update_celestial_markers(
    mut markers: Query<(&CelestialMarker, &mut CelestialMarkerCache, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
    view_settings: Res<ViewSettings>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    color_grading: Single<&ColorGrading>,
) {
    let distance_scale = view_settings.distance_factor();
    let camera_pos = fcam.bevy_pos;
    let exposure = color_grading.global.exposure;
    let brightness = MARKER_BRIGHTNESS * 2f32.powf(-exposure);

    for (marker, mut cache, mesh3d) in markers.iter_mut() {
        let camera_moved = cache
            .last_camera_pos
            .map(|last| (last - camera_pos).length() > CAMERA_MOVE_THRESHOLD)
            .unwrap_or(true);

        if !camera_moved {
            continue;
        }

        // Marker center in camera-relative Bevy space
        let bevy_center =
            marker.sim_position.as_bevy_scaled_cheated(distance_scale, camera_pos);
        let distance = bevy_center.length();

        if distance < 0.001 {
            continue;
        }

        // Billboard: symbol plane faces the camera (at origin in cheated space)
        let forward = (-bevy_center).normalize();
        let world_up = Vec3::Y;
        let right_candidate = world_up.cross(forward);
        let right = if right_candidate.length_squared() > 1e-6 {
            right_candidate.normalize()
        } else {
            Vec3::Z.cross(forward).normalize()
        };
        let up = forward.cross(right).normalize();

        let symbol_size = distance * SYMBOL_ANGULAR_SIZE;
        let tube_radius = symbol_size * TUBE_THICKNESS_RATIO;

        let mesh = generate_aries_mesh(
            bevy_center,
            right,
            up,
            symbol_size,
            tube_radius,
            brightness,
        );

        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = mesh;
        }

        cache.last_camera_pos = Some(camera_pos);
    }
}

pub fn cleanup_celestial_markers(
    mut commands: Commands,
    markers: Query<Entity, With<CelestialMarker>>,
) {
    for entity in markers.iter() {
        commands.entity(entity).despawn();
    }
}

// ---------------------------------------------------------------------------
// Mesh generation
// ---------------------------------------------------------------------------

/// Build the ♈ Aries symbol as tube geometry.
///
/// The symbol is placed at `center` in a billboard plane defined by `right`
/// and `up`, scaled to `size` bevy-units across.
fn generate_aries_mesh(
    center: Vec3,
    right: Vec3,
    up: Vec3,
    size: f32,
    tube_radius: f32,
    brightness: f32,
) -> Mesh {
    let horn_radius = (HORN_CX * HORN_CX + HORN_CY * HORN_CY).sqrt();
    let sweep = HORN_SWEEP_DEG.to_radians();

    // Right horn: clockwise arc from cusp (0,0), rising upward then curving right and down
    let right_start = f32::atan2(-HORN_CY, -HORN_CX);
    let right_horn: Vec<Vec3> = (0..=HORN_SAMPLES)
        .map(|i| {
            let t = i as f32 / HORN_SAMPLES as f32;
            let angle = right_start - sweep * t;
            let x = HORN_CX + horn_radius * angle.cos();
            let y = HORN_CY + horn_radius * angle.sin();
            center + right * x * size + up * y * size
        })
        .collect();

    // Left horn: mirror of the right horn in x
    let left_horn: Vec<Vec3> = (0..=HORN_SAMPLES)
        .map(|i| {
            let t = i as f32 / HORN_SAMPLES as f32;
            let angle = right_start - sweep * t;
            let x = HORN_CX + horn_radius * angle.cos();
            let y = HORN_CY + horn_radius * angle.sin();
            center - right * x * size + up * y * size
        })
        .collect();

    // Vertical stem below the cusp
    let stem_points = 4u32;
    let stem: Vec<Vec3> = (0..=stem_points)
        .map(|i| {
            let t = i as f32 / stem_points as f32;
            let y = -STEM_LENGTH * (1.0 - t);
            center + up * y * size
        })
        .collect();

    let strokes: &[&[Vec3]] = &[&right_horn, &left_horn, &stem];
    build_combined_tube_mesh(strokes, brightness, tube_radius, TUBE_SIDES)
}

/// Assemble multiple open polyline strokes into a single tube mesh.
fn build_combined_tube_mesh(
    strokes: &[&[Vec3]],
    brightness: f32,
    tube_radius: f32,
    sides: u32,
) -> Mesh {
    let mut all_positions: Vec<[f32; 3]> = Vec::new();
    let mut all_normals: Vec<[f32; 3]> = Vec::new();
    let mut all_colors: Vec<[f32; 4]> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();

    for stroke in strokes {
        if stroke.len() < 2 {
            continue;
        }

        let index_offset = all_positions.len() as u32;
        let ring_count = stroke.len();
        let verts_per_ring = sides as usize;

        for (ring_idx, center) in stroke.iter().enumerate() {
            let tangent = if ring_idx == 0 {
                (stroke[1] - *center).normalize_or_zero()
            } else if ring_idx == ring_count - 1 {
                (*center - stroke[ring_idx - 1]).normalize_or_zero()
            } else {
                (stroke[ring_idx + 1] - stroke[ring_idx - 1]).normalize_or_zero()
            };

            let (perp1, perp2) = perpendicular_vectors(tangent);

            for i in 0..sides {
                let angle = (i as f32 / sides as f32) * 2.0 * PI;
                let (sin_a, cos_a) = angle.sin_cos();

                let offset = perp1 * cos_a * tube_radius + perp2 * sin_a * tube_radius;
                let pos = *center + offset;
                all_positions.push([pos.x, pos.y, pos.z]);

                let normal = offset.normalize_or_zero();
                all_normals.push([normal.x, normal.y, normal.z]);

                all_colors.push([1.0, 1.0, 1.0, brightness]);
            }

            if ring_idx < ring_count - 1 {
                let base = index_offset + (ring_idx * verts_per_ring) as u32;
                let next_base = index_offset + ((ring_idx + 1) * verts_per_ring) as u32;

                for i in 0..sides {
                    let i_next = (i + 1) % sides;

                    all_indices.push(base + i);
                    all_indices.push(next_base + i);
                    all_indices.push(base + i_next);

                    all_indices.push(base + i_next);
                    all_indices.push(next_base + i);
                    all_indices.push(next_base + i_next);
                }
            }
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, all_positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, all_normals);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(all_colors),
    );
    mesh.insert_indices(Indices::U32(all_indices));

    mesh
}

fn perpendicular_vectors(dir: Vec3) -> (Vec3, Vec3) {
    let not_parallel = if dir.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    let perp2 = dir.cross(perp1).normalize_or_zero();
    (perp1, perp2)
}
