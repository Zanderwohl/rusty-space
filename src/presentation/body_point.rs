//! Distant body point rendering system.
//!
//! Renders small bodies as illuminated point-dots when they're too small to see
//! the wireframe detail. Uses phase-angle brightness computed from all stars.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;

use crate::body::appearance::Appearance;
use crate::body::motive::info::BodyInfo;
use crate::camera::PlanetariumCamera;
use crate::gui::settings::Settings;

use super::body_point_material::BodyPointMaterial;
use super::star_cache::StarLightingFrameCache;
use super::{BodyWireframeLink, BodyWireframeMaterial, OccluderLink, OccluderMaterial, OccluderMesh};

/// Emission strength for the point material
const POINT_EMISSION_STRENGTH: f32 = 8.0;
/// Base emission strength for DebugBall wireframe meshes.
const WIREFRAME_EMISSION_STRENGTH: f32 = 3.0;
/// Minimum on-screen radius for very small point bodies. Subpixel dots are
/// expanded to this footprint and brightness is reduced by area ratio.
const POINT_AA_MIN_RADIUS_PX: f32 = 3.0;

/// Physical radius (m) of the "small" reference object (1 m sphere), mapped to
/// the configured minimum dot radius.
const REFERENCE_MIN_RADIUS_M: f32 = 1.0;
/// Physical radius (m) of the "large" reference object (Jupiter, mean radius),
/// mapped to the configured maximum dot radius.
const REFERENCE_MAX_RADIUS_M: f32 = 6.9911e7;
/// Cross-sectional area proxies (∝ r²) for the two reference objects.
const REFERENCE_MIN_AREA: f32 = REFERENCE_MIN_RADIUS_M * REFERENCE_MIN_RADIUS_M;
const REFERENCE_MAX_AREA: f32 = REFERENCE_MAX_RADIUS_M * REFERENCE_MAX_RADIUS_M;

/// Reference distance for brightness falloff (in scaled Bevy units).
/// At this distance from a star, brightness is at 50% due to distance alone.
/// ~150 units ≈ 1 AU at default 1e-9 scale.
const DISTANCE_FALLOFF_REFERENCE: f32 = 150.0;

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
    // Read the body's `Transform` (set this frame by `position_bodies`) rather than
    // its `GlobalTransform`, which isn't propagated until PostUpdate and would lag the
    // camera/body meshes by one frame. Bodies are root entities, so Transform == world.
    bodies: Query<(Entity, &Transform, &Appearance, &BodyPointLink, Option<&BodyWireframeLink>, Option<&OccluderLink>, &crate::body::motive::info::BodyInfo), Without<BodyPointMesh>>,
    star_cache: Res<StarLightingFrameCache>,
    mut points: Query<(&BodyPointMesh, &mut Transform, &mut Visibility, &MeshMaterial3d<BodyPointMaterial>), Without<BodyPointLink>>,
    mut wireframes: Query<(&mut Visibility, &MeshMaterial3d<BodyWireframeMaterial>), (With<super::BodyWireframeMesh>, Without<BodyPointMesh>, Without<OccluderMesh>)>,
    mut occluders: Query<(&mut Visibility, &MeshMaterial3d<OccluderMaterial>), (With<OccluderMesh>, Without<BodyPointMesh>, Without<super::BodyWireframeMesh>)>,
    mut materials: ResMut<Assets<BodyPointMaterial>>,
    mut wireframe_materials: ResMut<Assets<BodyWireframeMaterial>>,
    mut occluder_materials: ResMut<Assets<OccluderMaterial>>,
    settings: Res<Settings>,
) {
    let brightness_floor = settings.display.body_brightness_floor;
    let min_radius_px = settings.display.body_radius_min;
    let max_radius_px = settings.display.body_radius_max;
    let fade_start_px = settings
        .display
        .model_fade_start_px
        .max(settings.display.model_fade_end_px);
    let fade_end_px = settings.display.model_fade_end_px.min(fade_start_px);
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

    // Use cached star data
    let max_intensity = star_cache.max_intensity;

    let camera_pos = camera_global.translation();

    for (_body_entity, body_transform, appearance, point_link, wireframe_link, occluder_link, _body_info) in bodies.iter() {
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

        // Calculate fade factor for model->dot handoff:
        // 0 at/above fade_start_px, 1 at/below fade_end_px.
        let fade_factor = if screen_radius >= fade_start_px {
            0.0
        } else if screen_radius <= fade_end_px {
            1.0
        } else if fade_start_px > fade_end_px {
            (fade_start_px - screen_radius) / (fade_start_px - fade_end_px)
        } else {
            1.0
        };

        // Update visibility
        if fade_factor > 0.0 {
            // Show point
            *point_visibility = Visibility::Visible;

            // Update point position (copy from body)
            point_transform.translation = body_center;

            // Size the dot by the body's physical radius, mapped between two reference
            // objects: a 1 m sphere -> body_radius_min, Jupiter -> body_radius_max. The
            // dot's AREA interpolates with the object's cross-sectional area (proportional
            // to r^2), so the drawn disc scales like the object's apparent size would.
            // Super-Jupiter bodies cap at max; sub-1m bodies are allowed to shrink below
            // min (no lower floor), so tiny objects can disappear.
            let physical_radius_m = appearance.radius() as f32;
            let area = physical_radius_m * physical_radius_m;
            let area_t = ((area - REFERENCE_MIN_AREA) / (REFERENCE_MAX_AREA - REFERENCE_MIN_AREA))
                .min(1.0);
            let dot_area = min_radius_px * min_radius_px
                + (max_radius_px * max_radius_px - min_radius_px * min_radius_px) * area_t;
            let point_px = dot_area.max(0.0).sqrt();

            // Expand very tiny dots to a minimum footprint so they do not alias/flicker.
            // Preserve total emitted energy by compensating brightness with area ratio.
            let render_point_px = point_px.max(POINT_AA_MIN_RADIUS_PX);
            let aa_energy_compensation = if render_point_px > 0.0 {
                (point_px * point_px) / (render_point_px * render_point_px)
            } else {
                0.0
            };

            // Calculate point scale for the target screen size
            // target_px / viewport_height * fov_y = desired_radius / distance
            // desired_radius = (target_px / viewport_height) * fov_y * distance * 0.5
            let desired_radius = (render_point_px / viewport_size.y) * (fov_y * 0.5) * distance;
            point_transform.scale = Vec3::splat(desired_radius);

            // Calculate phase brightness from all stars with distance falloff
            let mut total_brightness = 0.0f32;

            if max_intensity > 0.0 {
                for star in &star_cache.stars {
                    // Direction from body to camera (camera is at origin in world space)
                    let to_camera = (-body_center).normalize();
                    // Direction from body to star
                    let to_star = (star.bevy_position - body_center).normalize();

                    // Phase angle brightness: (1 + cos(phase)) / 2
                    let cos_phase = to_camera.dot(to_star);
                    let phase_brightness = (1.0 + cos_phase) / 2.0;

                    // Distance falloff: soft inverse-square
                    // At reference distance, factor = 0.5; approaches 0 at infinity
                    let body_to_star_dist = (star.bevy_position - body_center).length();
                    let dist_sq = body_to_star_dist * body_to_star_dist;
                    let ref_sq = DISTANCE_FALLOFF_REFERENCE * DISTANCE_FALLOFF_REFERENCE;
                    let distance_factor = ref_sq / (dist_sq + ref_sq);

                    // Weight by relative intensity
                    let relative_intensity = star.intensity / max_intensity;
                    total_brightness += phase_brightness * distance_factor * relative_intensity;
                }
            }

            // Apply minimum floor and clamp
            total_brightness = total_brightness.max(brightness_floor).min(1.0);

            // Update material brightness (phase brightness * fade factor)
            // Only update if value changed to avoid spurious asset change detection
            let new_brightness = total_brightness * fade_factor * aa_energy_compensation;
            if let Some(material) = materials.get(point_material_handle.id()) {
                if (material.brightness - new_brightness).abs() > 0.001 {
                    if let Some(material) = materials.get_mut(point_material_handle.id()) {
                        material.brightness = new_brightness;
                    }
                }
            }

            // Crossfade model->dot over the configured fade range.
            let model_fade = 1.0 - fade_factor;

            // Fade wireframe emissive strength smoothly, then hide once negligible.
            if let Some(wireframe_link) = wireframe_link {
                if let Ok((mut wf_vis, wf_material_handle)) = wireframes.get_mut(wireframe_link.0) {
                    *wf_vis = if model_fade > 0.001 {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                    let new_emission = WIREFRAME_EMISSION_STRENGTH * model_fade;
                    if let Some(material) = wireframe_materials.get(wf_material_handle.id()) {
                        if (material.emission_strength - new_emission).abs() > 0.001 {
                            if let Some(material) = wireframe_materials.get_mut(wf_material_handle.id()) {
                                material.emission_strength = new_emission;
                            }
                        }
                    }
                }
            }

            // Fade occluder alpha smoothly to avoid hard model popping.
            if let Some(occluder_link) = occluder_link {
                if let Ok((mut occ_vis, occ_material_handle)) = occluders.get_mut(occluder_link.0) {
                    *occ_vis = if model_fade > 0.001 {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                    if let Some(material) = occluder_materials.get(occ_material_handle.id()) {
                        let new_alpha = model_fade;
                        if (material.base_color.alpha - new_alpha).abs() > 0.001 {
                            if let Some(material) = occluder_materials.get_mut(occ_material_handle.id()) {
                                material.base_color.alpha = new_alpha;
                            }
                        }
                    }
                }
            }
        } else {
            // Hide point, ensure wireframe and occluder visible
            *point_visibility = Visibility::Hidden;

            if let Some(wireframe_link) = wireframe_link {
                if let Ok((mut wf_vis, wf_material_handle)) = wireframes.get_mut(wireframe_link.0) {
                    *wf_vis = Visibility::Visible;
                    if let Some(material) = wireframe_materials.get(wf_material_handle.id()) {
                        if (material.emission_strength - WIREFRAME_EMISSION_STRENGTH).abs() > 0.001 {
                            if let Some(material) = wireframe_materials.get_mut(wf_material_handle.id()) {
                                material.emission_strength = WIREFRAME_EMISSION_STRENGTH;
                            }
                        }
                    }
                }
            }

            // Show occluder when point is hidden
            if let Some(occluder_link) = occluder_link {
                if let Ok((mut occ_vis, occ_material_handle)) = occluders.get_mut(occluder_link.0) {
                    *occ_vis = Visibility::Visible;
                    if let Some(material) = occluder_materials.get(occ_material_handle.id()) {
                        if (material.base_color.alpha - 1.0).abs() > 0.001 {
                            if let Some(material) = occluder_materials.get_mut(occ_material_handle.id()) {
                                material.base_color.alpha = 1.0;
                            }
                        }
                    }
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
