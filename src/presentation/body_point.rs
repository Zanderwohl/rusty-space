//! Distant body point rendering system.
//!
//! Renders small bodies as illuminated point-dots when they're too small to see
//! the wireframe detail. Uses phase-angle brightness computed from all stars.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use crate::body::appearance::Appearance;
use crate::body::motive::info::BodyInfo;
use crate::camera::PlanetariumCamera;

use super::body_point_material::BodyPointMaterial;
use super::{BodyWireframeLink, OccluderLink, OccluderMesh};

/// Angular size threshold (in pixels) below which the point is at full brightness
const POINT_FULL_THRESHOLD_PX: f32 = 10.0;

/// Angular size threshold (in pixels) above which the point is hidden
const POINT_FADE_THRESHOLD_PX: f32 = 20.0;

/// Desired angular size of the point dot in pixels
const POINT_SIZE_PX: f32 = 6.0;

/// Emission strength for the point material
const POINT_EMISSION_STRENGTH: f32 = 8.0;

/// Reference distance for brightness falloff (in scaled Bevy units).
/// At this distance from a star, brightness is at 50% due to distance alone.
/// ~150 units ≈ 1 AU at default 1e-9 scale.
const DISTANCE_FALLOFF_REFERENCE: f32 = 150.0;

/// Minimum brightness floor - objects never go completely dark
const MIN_POINT_BRIGHTNESS: f32 = 0.05;

/// Marker component for body point mesh entities.
#[derive(Component)]
pub struct BodyPointMesh {
    pub body_entity: Entity,
}

/// Component linking a body to its point mesh entity.
#[derive(Component)]
pub struct BodyPointLink(pub Entity);

/// System to spawn body point meshes for DebugBall bodies.
pub fn spawn_body_point_meshes(
    mut commands: Commands,
    bodies: Query<(Entity, &Appearance), (With<BodyInfo>, Without<BodyPointLink>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodyPointMaterial>>,
) {
    for (body_entity, appearance) in bodies.iter() {
        if let Appearance::DebugBall(debug_ball) = appearance {
            // Create low-poly icosphere for the point
            let mesh = Sphere::new(1.0f32).mesh().ico(2).unwrap();
            let mesh_handle = meshes.add(mesh);

            // Base color from body's albedo
            let color = LinearRgba::new(
                debug_ball.color.r as f32 / 255.0,
                debug_ball.color.g as f32 / 255.0,
                debug_ball.color.b as f32 / 255.0,
                1.0,
            );

            let material = BodyPointMaterial {
                base_color: color,
                brightness: 0.0,
                emission_strength: POINT_EMISSION_STRENGTH,
                alpha_mode: AlphaMode::Blend,
            };
            let material_handle = materials.add(material);

            // Spawn as separate entity (not child) for independent visibility control
            let point_entity = commands
                .spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    Visibility::Hidden,
                    NoFrustumCulling,
                    BodyPointMesh { body_entity },
                ))
                .id();

            commands.entity(body_entity).insert(BodyPointLink(point_entity));
        }
    }
}

/// System to update body point visibility, scale, and brightness.
pub fn update_body_points(
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PlanetariumCamera>>,
    bodies: Query<(Entity, &Transform, &BodyPointLink, Option<&BodyWireframeLink>, Option<&OccluderLink>, &crate::body::motive::info::BodyInfo), Without<BodyPointMesh>>,
    stars: Query<(&Transform, &Appearance), (Without<BodyPointLink>, Without<BodyPointMesh>)>,
    mut points: Query<(&BodyPointMesh, &mut Transform, &mut Visibility, &MeshMaterial3d<BodyPointMaterial>), Without<BodyPointLink>>,
    mut wireframes: Query<&mut Visibility, (With<super::BodyWireframeMesh>, Without<BodyPointMesh>, Without<OccluderMesh>)>,
    mut occluders: Query<&mut Visibility, (With<OccluderMesh>, Without<BodyPointMesh>, Without<super::BodyWireframeMesh>)>,
    mut materials: ResMut<Assets<BodyPointMaterial>>,
) {
    // Get camera info
    let Ok((camera, camera_global, projection)) = cameras.single() else {
        return;
    };

    let Some(viewport_size) = camera.logical_viewport_size() else {
        return;
    };

    let fov_y = match projection {
        Projection::Perspective(persp) => persp.fov,
        _ => 1.0,
    };

    // Collect star data for brightness calculation
    let star_data: Vec<(Vec3, f32)> = stars
        .iter()
        .filter_map(|(transform, appearance)| {
            if let Appearance::Star(star_ball) = appearance {
                Some((transform.translation, star_ball.intensity()))
            } else {
                None
            }
        })
        .collect();

    // Find max intensity for normalization
    let max_intensity = star_data
        .iter()
        .map(|(_, intensity)| *intensity)
        .fold(0.0f32, f32::max);

    let camera_pos = camera_global.translation();

    for (_body_entity, body_transform, point_link, wireframe_link, occluder_link, _body_info) in bodies.iter() {
        let Ok((_point_mesh, mut point_transform, mut point_visibility, point_material_handle)) =
            points.get_mut(point_link.0)
        else {
            continue;
        };

        // Calculate distance and angular size
        let body_center = body_transform.translation;
        let distance = (body_center - camera_pos).length();

        if distance <= 0.0 {
            continue;
        }

        let radius = body_transform.scale.x;
        let angular_radius = (radius / distance).min(1.0);
        let screen_radius = angular_radius / (fov_y * 0.5) * viewport_size.y * 0.5;

        // Calculate fade factor: 0 at >= 20px, 1 at <= 10px
        let fade_factor = if screen_radius >= POINT_FADE_THRESHOLD_PX {
            0.0
        } else if screen_radius <= POINT_FULL_THRESHOLD_PX {
            1.0
        } else {
            (POINT_FADE_THRESHOLD_PX - screen_radius)
                / (POINT_FADE_THRESHOLD_PX - POINT_FULL_THRESHOLD_PX)
        };

        // Update visibility
        if fade_factor > 0.0 {
            // Show point
            *point_visibility = Visibility::Visible;

            // Update point position (copy from body)
            point_transform.translation = body_center;

            // Calculate point scale for fixed screen size
            // desired_angular_px / viewport_height * fov_y = desired_radius / distance
            // desired_radius = (desired_angular_px / viewport_height) * fov_y * distance * 0.5
            let desired_radius = (POINT_SIZE_PX / viewport_size.y) * (fov_y * 0.5) * distance;
            point_transform.scale = Vec3::splat(desired_radius);

            // Calculate phase brightness from all stars with distance falloff
            let mut total_brightness = 0.0f32;

            if max_intensity > 0.0 {
                for (star_pos, star_intensity) in &star_data {
                    // Direction from body to camera (camera is at origin in world space)
                    let to_camera = (-body_center).normalize();
                    // Direction from body to star
                    let to_star = (*star_pos - body_center).normalize();

                    // Phase angle brightness: (1 + cos(phase)) / 2
                    let cos_phase = to_camera.dot(to_star);
                    let phase_brightness = (1.0 + cos_phase) / 2.0;

                    // Distance falloff: soft inverse-square
                    // At reference distance, factor = 0.5; approaches 0 at infinity
                    let body_to_star_dist = (*star_pos - body_center).length();
                    let dist_sq = body_to_star_dist * body_to_star_dist;
                    let ref_sq = DISTANCE_FALLOFF_REFERENCE * DISTANCE_FALLOFF_REFERENCE;
                    let distance_factor = ref_sq / (dist_sq + ref_sq);

                    // Weight by relative intensity
                    let relative_intensity = star_intensity / max_intensity;
                    total_brightness += phase_brightness * distance_factor * relative_intensity;
                }
            }

            // Apply minimum floor and clamp
            total_brightness = total_brightness.max(MIN_POINT_BRIGHTNESS).min(1.0);

            // Update material brightness (phase brightness * fade factor)
            // Only update if value changed to avoid spurious asset change detection
            let new_brightness = total_brightness * fade_factor;
            if let Some(material) = materials.get(point_material_handle.id()) {
                if (material.brightness - new_brightness).abs() > 0.001 {
                    if let Some(material) = materials.get_mut(point_material_handle.id()) {
                        material.brightness = new_brightness;
                    }
                }
            }

            // Hide wireframe only when body is sub-pixel
            if screen_radius <= 1.0 {
                if let Some(wireframe_link) = wireframe_link {
                    if let Ok(mut wf_vis) = wireframes.get_mut(wireframe_link.0) {
                        *wf_vis = Visibility::Hidden;
                    }
                }
            } else {
                if let Some(wireframe_link) = wireframe_link {
                    if let Ok(mut wf_vis) = wireframes.get_mut(wireframe_link.0) {
                        *wf_vis = Visibility::Visible;
                    }
                }
            }

            // Hide occluder when point is visible (point would be inside occluder otherwise)
            if let Some(occluder_link) = occluder_link {
                if let Ok(mut occ_vis) = occluders.get_mut(occluder_link.0) {
                    *occ_vis = Visibility::Hidden;
                }
            }
        } else {
            // Hide point, ensure wireframe and occluder visible
            *point_visibility = Visibility::Hidden;

            if let Some(wireframe_link) = wireframe_link {
                if let Ok(mut wf_vis) = wireframes.get_mut(wireframe_link.0) {
                    *wf_vis = Visibility::Visible;
                }
            }

            // Show occluder when point is hidden
            if let Some(occluder_link) = occluder_link {
                if let Ok(mut occ_vis) = occluders.get_mut(occluder_link.0) {
                    *occ_vis = Visibility::Visible;
                }
            }
        }
    }
}

/// Cleanup orphaned body point meshes when bodies are despawned.
/// System to clean up orphaned body point entities.
/// Uses RemovedComponents to only run when bodies are actually removed.
pub fn cleanup_orphaned_body_points(
    mut commands: Commands,
    mut removed_bodies: RemovedComponents<BodyInfo>,
    points: Query<(Entity, &BodyPointMesh)>,
) {
    // Early exit if no bodies were removed
    if removed_bodies.is_empty() {
        return;
    }
    
    // Collect removed body entities
    let removed: std::collections::HashSet<Entity> = removed_bodies.read().collect();
    
    for (point_entity, point_mesh) in points.iter() {
        if removed.contains(&point_mesh.body_entity) {
            commands.entity(point_entity).despawn();
        }
    }
}
