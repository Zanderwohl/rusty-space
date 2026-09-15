//! Deciding what the cursor is on.
//!
//! The rule is [`em_ui::picking`], shared with the other product: in reach when the cursor is
//! within a slack of the thing *as drawn*, then the lowest rank outright, then the nearest
//! centre. What lives here is the part that knows what an Exotic Matters thing is — a body, a
//! trajectory marker, a segment of a drawn orbit — and turns the answer back into
//! [`HoverState`].
//!
//! This replaced two hit tests run in parallel, one against a ray and one against screen
//! distance, whose results were merged by taking the **most massive** hit. Mass was standing in
//! for "the small one cannot be hit exactly"; the slack does that directly, and without making
//! a moon in front of its planet unselectable at any distance.

use bevy::input::mouse::MouseButton;
use bevy::math::{DQuat, DVec3, Vec2, Vec3};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;
use em_sim::motive::MotiveSelection;
use em_sim::system::System;
use em_ui::picking::{self, Candidate, rank};

use crate::body::universe::save::ViewSettings;
use crate::camera::freecam::Freecam;
use crate::camera::planetarium::PlanetariumCamera;
use crate::gui::planetarium::{FocusedBodyState, HoverState, HoveredTrajectoryMarkerKind, TrajectoryHitData};
use crate::presentation::render_space::ToRender;
use crate::presentation::{FocusedTrajectoryMarker, FocusedTrajectoryMarkerKind};
use crate::sim::SimTime;
use crate::sim::world::{BodyRef, SimSystem};

#[derive(Default)]
pub struct PickState {
    last_pick_id: Option<String>,
    last_pick_time: f64,
}

const DOUBLE_CLICK_WINDOW: f64 = 0.5;
const TRAJECTORY_PICK_PIXEL_RADIUS: f32 = 10.0;

/// What a candidate turned out to be. The index into these is the candidate's identifier.
enum Subject {
    Body(String),
    Marker(HoveredTrajectoryMarkerKind),
}

/// What sort of thing a body counts as.
///
/// A root is a star, and a star sits *below* the planets in front of it: from close in its disc
/// can be most of the screen, and a click next to Mercury means Mercury. That ordering is the
/// whole of what replaced the old tie-break on mass.
fn rank_of(system: &System, index: em_sim::id::BodyIndex) -> u8 {
    if system.parent(index).is_none() { rank::STAR } else { rank::BODY }
}

/// How large something is drawn, in pixels, from its size and how far off it is.
fn drawn_radius_px(radius: f32, distance: f32, rad_per_px: f32) -> f32 {
    if distance <= 0.0 || rad_per_px <= 0.0 {
        return 0.0;
    }
    ((radius / distance) / rad_per_px).max(0.0)
}

#[derive(Default)]
struct HoverPickResult {
    hovered_body_id: Option<String>,
    hovered_marker_kind: Option<HoveredTrajectoryMarkerKind>,
    trajectory_hit: Option<TrajectoryHitData>,
}

pub fn update_hover_target(
    primary_window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    markers: Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    focused_body_state: Res<FocusedBodyState>,
    mut egui_ctx: EguiContexts,
    mut hover_state: ResMut<HoverState>,
) {
    let hovered = pick_hover_target(
        &primary_window,
        &cameras,
        &bodies,
        &system.0,
        &markers,
        &trajectory_caches,
        &view_settings,
        &sim_time,
        &focused_body_state,
        &mut egui_ctx,
    );
    hover_state.hovered_body_id = hovered.hovered_body_id;
    hover_state.hovered_marker_kind = hovered.hovered_marker_kind;
    hover_state.hovered_trajectory_hit = hovered.trajectory_hit;
}

pub fn pick_body_on_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    markers: Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    mut egui_ctx: EguiContexts,
    time: Res<Time>,
    mut pick_state: Local<PickState>,
    mut focused_body_state: ResMut<FocusedBodyState>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    let hovered = pick_hover_target(
        &primary_window,
        &cameras,
        &bodies,
        &system.0,
        &markers,
        &trajectory_caches,
        &view_settings,
        &sim_time,
        &focused_body_state,
        &mut egui_ctx,
    );
    if let Some(selected_id) = hovered.hovered_body_id {
        let info = system.0.by_name(&selected_id).map(|i| system.0.info(i));
        if let Some(info) = info {
        let now = time.elapsed().as_secs_f64();
        let is_double = pick_state.last_pick_id.as_deref() == Some(&info.id)
            && (now - pick_state.last_pick_time) <= DOUBLE_CLICK_WINDOW;

        if is_double {
            focused_body_state.current_body_id = Some(info.id.clone());
            info!("Selected body: {} (id: {})", info.display_name(), info.id);
            pick_state.last_pick_id = None;
        } else {
            info!("Picked body: {} (id: {})", info.display_name(), info.id);
            pick_state.last_pick_id = Some(info.id.clone());
            pick_state.last_pick_time = now;
        }
        }
    }
}

fn pick_hover_target(
    primary_window: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    markers: &Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: &Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: &ViewSettings,
    sim_time: &SimTime,
    focused_body_state: &FocusedBodyState,
    egui_ctx: &mut EguiContexts,
) -> HoverPickResult {
    let egui_wants_pointer = egui_ctx.ctx_mut()
        .map_or(false, |ctx| ctx.wants_pointer_input());
    if egui_wants_pointer {
        return HoverPickResult::default();
    }

    let Ok(window) = primary_window.single() else {
        return HoverPickResult::default();
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return HoverPickResult::default();
    };
    let Ok((camera, projection, cam_transform, freecam)) = cameras.single() else {
        return HoverPickResult::default();
    };
    let Projection::Perspective(perspective) = projection else {
        return HoverPickResult::default();
    };

    let fresh_gt = GlobalTransform::from(*cam_transform);
    let Ok(ray) = camera.viewport_to_world(&fresh_gt, cursor_pos) else {
        return HoverPickResult::default();
    };
    let ray_origin = ray.origin;
    let ray_dir: Vec3 = *ray.direction;

    let Some(viewport) = camera.logical_viewport_size() else {
        return HoverPickResult::default();
    };
    let rad_per_px = 2.0 * (perspective.fov * 0.5).tan() / viewport.y.max(1.0);
    let distance_scale = view_settings.distance_factor();

    // Everything the cursor could be on, reduced to where it was drawn. One list and one rule,
    // where there used to be a ray cast and a screen-distance test run side by side and merged
    // by mass.
    let mut subjects: Vec<Subject> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();

    for (_, body_ref, _) in bodies.iter() {
        let Some(index) = system.index_of(body_ref.0) else { continue };
        let at = system.position(index).to_render_relative(distance_scale, freecam.bevy_pos);
        let Ok(screen) = camera.world_to_viewport(&fresh_gt, at) else { continue };
        let radius_px = drawn_radius_px(
            view_settings.body_scale_factor(system.radius(index)),
            at.distance(ray_origin),
            rad_per_px,
        );
        candidates.push(Candidate {
            id: subjects.len() as u64,
            at: screen,
            radius_px,
            rank: rank_of(system, index),
        });
        subjects.push(Subject::Body(system.info(index).id.clone()));
    }

    for (marker, marker_transform) in markers.iter() {
        let kind = match marker.kind {
            FocusedTrajectoryMarkerKind::Periapsis => HoveredTrajectoryMarkerKind::Periapsis,
            FocusedTrajectoryMarkerKind::Apoapsis => HoveredTrajectoryMarkerKind::Apoapsis,
            // Drawn by the hover itself; picking it would be picking its own output.
            FocusedTrajectoryMarkerKind::MouseHit => continue,
        };
        let at = marker_transform.translation;
        let Ok(screen) = camera.world_to_viewport(&fresh_gt, at) else { continue };
        let radius_px =
            drawn_radius_px(marker_transform.scale.x, at.distance(ray_origin), rad_per_px);
        // Above the bodies, the way a craft is: a marker is a few pixels of deliberate target
        // and is unreachable if anything it sits on outranks it.
        candidates.push(Candidate {
            id: subjects.len() as u64,
            at: screen,
            radius_px,
            rank: rank::CRAFT,
        });
        subjects.push(Subject::Marker(kind));
    }

    let (mut hovered_body_id, mut hovered_marker_kind) = (None, None);
    match picking::pick(&candidates, cursor_pos, picking::SLACK_PX).map(|id| &subjects[id as usize])
    {
        Some(Subject::Body(id)) => hovered_body_id = Some(id.clone()),
        Some(Subject::Marker(kind)) => hovered_marker_kind = Some(*kind),
        None => {}
    }

    // Fallback: trajectory segment cast only when no body or marker hit
    let trajectory_hit = if hovered_body_id.is_none() && hovered_marker_kind.is_none() {
        pick_trajectory_segment(
            focused_body_state,
            bodies,
            system,
            trajectory_caches,
            sim_time,
            view_settings,
            freecam,
            camera,
            &fresh_gt,
            cursor_pos,
            ray_origin,
            ray_dir,
        )
    } else {
        None
    };

    HoverPickResult {
        hovered_body_id,
        hovered_marker_kind,
        trajectory_hit,
    }
}

/// Raycast against the focused body's trajectory segments.
/// Returns the closest hit within the pick radius threshold.
fn pick_trajectory_segment(
    focused_body_state: &FocusedBodyState,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    trajectory_caches: &Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    sim_time: &SimTime,
    view_settings: &ViewSettings,
    freecam: &Freecam,
    camera: &Camera,
    camera_global_transform: &GlobalTransform,
    cursor_pos: Vec2,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<TrajectoryHitData> {
    let focused_id = focused_body_state.current_body_id.as_ref()?;

    // Find the focused body and its current motive selection.
    let focused_index = system.by_name(focused_id)?;
    let focused_body = system.id(focused_index);
    let focused_motive = system.motive(focused_index);
    let focused_entity = trajectory_caches
        .iter()
        .find_map(|(traj_mesh, cache)| {
            let Ok((_, body_ref, _)) = bodies.get(traj_mesh.body_entity) else {
                return None;
            };
            if body_ref.0 == focused_body && cache.valid && !cache.local_points.is_empty() {
                Some(traj_mesh.body_entity)
            } else {
                None
            }
        })?;

    // Find the trajectory cache for the focused body
    let cache = trajectory_caches
        .iter()
        .find_map(|(traj_mesh, cache)| {
            if traj_mesh.body_entity != focused_entity {
                return None;
            }
            if cache.valid && !cache.local_points.is_empty() {
                Some(cache)
            } else {
                None
            }
        })?;
    
    // Get primary position offset for Keplerian orbits
    let primary_offset: DVec3 = cache
        .primary_id
        .as_ref()
        .and_then(|pid| system.by_name(pid))
        .map(|i| system.position(i))
        .unwrap_or(DVec3::ZERO);

    // Rotate cached points by current perifocal->reference delta so picking
    // tracks the render-time trajectory transform.
    let current_perifocal_to_reference = match focused_motive.motive_at(sim_time.time) {
        (_, MotiveSelection::Keplerian(k)) => Some(DQuat::from_mat3(&k.perifocal_to_reference_matrix(sim_time.time))),
        _ => None,
    };
    let rotation_delta = if let Some(current_rot) = current_perifocal_to_reference {
        let base_rot = cache.base_perifocal_to_reference.unwrap_or(current_rot);
        current_rot * base_rot.inverse()
    } else {
        DQuat::IDENTITY
    };
    
    let distance_scale = view_settings.distance_factor();
    let camera_pos = freecam.bevy_pos;
    let ray_origin_d = DVec3::new(ray_origin.x as f64, ray_origin.y as f64, ray_origin.z as f64);
    let ray_dir_d = DVec3::new(ray_dir.x as f64, ray_dir.y as f64, ray_dir.z as f64).normalize_or_zero();
    
    // (pixel_distance, idx, seg_t, time_a, time_b, local_pos, bevy_pos)
    let mut best_hit: Option<(f32, usize, f64, f64, f64, DVec3, Vec3)> = None;
    
    // Use only cached trajectory points for picking (exclude transient points).
    let points = &cache.local_points;
    for i in 0..points.len().saturating_sub(1) {
        let (time_a, local_a) = points[i];
        let (time_b, local_b) = points[i + 1];
        
        // Transform to bevy space
        let world_a = rotation_delta * local_a + primary_offset;
        let world_b = rotation_delta * local_b + primary_offset;
        let bevy_a = world_a.to_render_relative(distance_scale, camera_pos);
        let bevy_b = world_b.to_render_relative(distance_scale, camera_pos);
        let seg_a_d = DVec3::new(bevy_a.x as f64, bevy_a.y as f64, bevy_a.z as f64);
        let seg_b_d = DVec3::new(bevy_b.x as f64, bevy_b.y as f64, bevy_b.z as f64);
        let ray_hit = ray_segment_closest_point(ray_origin_d, ray_dir_d, seg_a_d, seg_b_d);
        
        let Ok(screen_a) = camera.world_to_viewport(camera_global_transform, bevy_a) else {
            continue;
        };
        let Ok(screen_b) = camera.world_to_viewport(camera_global_transform, bevy_b) else {
            continue;
        };
        let (seg_t_screen, screen_dist) = closest_point_on_screen_segment(cursor_pos, screen_a, screen_b);
        let seg_t = ray_hit.map(|(t, _)| t).unwrap_or(seg_t_screen);

        if screen_dist <= TRAJECTORY_PICK_PIXEL_RADIUS {
            if best_hit.is_none() || screen_dist < best_hit.as_ref().unwrap().0 {
                // Interpolate local position
                let local_hit = (rotation_delta * local_a).lerp(rotation_delta * local_b, seg_t);
                let bevy_hit = bevy_a.lerp(bevy_b, seg_t as f32);
                best_hit = Some((screen_dist, i, seg_t, time_a, time_b, local_hit, bevy_hit));
            }
        }
    }
    
    // Handle closed orbit: check segment from last point back to first
    if cache.closed && points.len() >= 2 {
        let (time_a, local_a) = points[points.len() - 1];
        let (time_b, local_b) = points[0];
        
        let world_a = rotation_delta * local_a + primary_offset;
        let world_b = rotation_delta * local_b + primary_offset;
        let bevy_a = world_a.to_render_relative(distance_scale, camera_pos);
        let bevy_b = world_b.to_render_relative(distance_scale, camera_pos);
        let seg_a_d = DVec3::new(bevy_a.x as f64, bevy_a.y as f64, bevy_a.z as f64);
        let seg_b_d = DVec3::new(bevy_b.x as f64, bevy_b.y as f64, bevy_b.z as f64);
        let ray_hit = ray_segment_closest_point(ray_origin_d, ray_dir_d, seg_a_d, seg_b_d);
        
        if let (Ok(screen_a), Ok(screen_b)) = (
            camera.world_to_viewport(camera_global_transform, bevy_a),
            camera.world_to_viewport(camera_global_transform, bevy_b),
        ) {
            let (seg_t_screen, screen_dist) = closest_point_on_screen_segment(cursor_pos, screen_a, screen_b);
            let seg_t = ray_hit.map(|(t, _)| t).unwrap_or(seg_t_screen);

            if screen_dist <= TRAJECTORY_PICK_PIXEL_RADIUS {
                if best_hit.is_none() || screen_dist < best_hit.as_ref().unwrap().0 {
                    let local_hit = (rotation_delta * local_a).lerp(rotation_delta * local_b, seg_t);
                    let bevy_hit = bevy_a.lerp(bevy_b, seg_t as f32);
                    // For wrapped segment, time_b is actually the start of the period (0)
                    // but we want the logical continuation, so add period if needed
                    let adjusted_time_b = if let Some(interval_size) = cache.interval_size {
                        time_b + interval_size
                    } else {
                        time_b
                    };
                    best_hit = Some((screen_dist, points.len() - 1, seg_t, time_a, adjusted_time_b, local_hit, bevy_hit));
                }
            }
        }
    }
    
    best_hit.map(|(_, idx, seg_t, time_a, time_b, local_pos, bevy_pos)| {
        TrajectoryHitData {
            segment_start_idx: idx,
            start_time: time_a,
            end_time: time_b,
            t: seg_t,
            hit_position_local: local_pos,
            hit_position_bevy: bevy_pos,
        }
    })
}

/// Find the closest point on a 2D segment to a 2D point.
/// Returns (t, distance) where t is in [0, 1].
fn closest_point_on_screen_segment(point: Vec2, seg_a: Vec2, seg_b: Vec2) -> (f64, f32) {
    let seg = seg_b - seg_a;
    let seg_len_sq = seg.length_squared();
    if seg_len_sq <= f32::EPSILON {
        return (0.0, point.distance(seg_a));
    }

    let t = ((point - seg_a).dot(seg) / seg_len_sq).clamp(0.0, 1.0);
    let closest = seg_a + seg * t;
    (t as f64, point.distance(closest))
}

/// Closest distance between a ray and a segment.
fn ray_segment_closest_point(
    ray_origin: DVec3,
    ray_dir: DVec3,
    seg_a: DVec3,
    seg_b: DVec3,
) -> Option<(f64, f64)> {
    let seg_dir = seg_b - seg_a;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq <= f64::EPSILON || ray_dir.length_squared() <= f64::EPSILON {
        return None;
    }

    let w0 = ray_origin - seg_a;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(seg_dir);
    let c = seg_dir.dot(seg_dir);
    let d = ray_dir.dot(w0);
    let e = seg_dir.dot(w0);
    let denom = a * c - b * b;

    let mut seg_t = if denom.abs() < 1e-12 {
        (e / c).clamp(0.0, 1.0)
    } else {
        ((a * e - b * d) / denom).clamp(0.0, 1.0)
    };
    if !seg_t.is_finite() {
        seg_t = 0.0;
    }
    let closest_on_seg = seg_a + seg_dir * seg_t;
    let ray_t = (closest_on_seg - ray_origin).dot(ray_dir).max(0.0);
    let closest_on_ray = ray_origin + ray_dir * ray_t;
    Some((seg_t, (closest_on_seg - closest_on_ray).length()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use em_sim::presets::solar_system;

    fn built() -> System {
        let mut system = System::from_contents(&solar_system()).expect("the bundled system");
        // Derived columns -- `parent` among them -- are empty until the first propagation.
        em_sim::propagate::evaluate_at(&mut system, em_foundations::time::Instant::J2000);
        system
    }

    fn named(system: &System, name: &str) -> em_sim::id::BodyIndex {
        system.indices().find(|&i| system.name(i) == name).expect("a body by that name")
    }


    /// The Sun ranks below the planets in front of it, and the ranking is by hierarchy rather
    /// than by size: Luna is a body like Earth is, and a moon in front of its planet is now
    /// selectable because of it.
    #[test]
    fn a_root_is_a_star_and_everything_else_is_a_body() {
        let system = built();
        assert_eq!(rank_of(&system, named(&system, "Sol")), rank::STAR);
        assert_eq!(rank_of(&system, named(&system, "Earth")), rank::BODY);
        assert_eq!(rank_of(&system, named(&system, "Luna")), rank::BODY);
        assert!(rank::BODY < rank::STAR, "a planet beats the star it is drawn against");
        assert!(rank::CRAFT < rank::BODY, "a marker beats the body it sits on");
    }

    /// The old rule took the most massive of the hits, which made a moon in front of its
    /// planet unreachable at any distance. Luna is a millionth of the Sun's mass and outranks
    /// it now, and ties Earth — where the nearest centre decides, as it should.
    #[test]
    fn mass_no_longer_decides() {
        let system = built();
        let sun = named(&system, "Sol");
        let luna = named(&system, "Luna");
        assert!(system.mass(sun) > system.mass(luna) * 1.0e6, "the Sun is vastly heavier");
        assert!(rank_of(&system, luna) < rank_of(&system, sun), "and loses anyway");
        assert_eq!(rank_of(&system, luna), rank_of(&system, named(&system, "Earth")));
    }

    /// Angular size in pixels, which is what makes a candidate's reach match its drawn disc.
    #[test]
    fn a_things_drawn_radius_is_its_angular_size() {
        // A 1080-line viewport across ninety degrees: two milliradians a pixel.
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4).tan() / 1080.0;
        // Something one unit across at a hundred units subtends ten milliradians.
        let px = drawn_radius_px(1.0, 100.0, rad_per_px);
        assert!((px - 0.01 / rad_per_px).abs() < 1.0e-3, "{px}");
        // Twice as far is half as big.
        assert!((drawn_radius_px(1.0, 200.0, rad_per_px) - px * 0.5).abs() < 1.0e-3);
        // And nothing degenerate comes back as a NaN.
        assert_eq!(drawn_radius_px(1.0, 0.0, rad_per_px), 0.0);
        assert_eq!(drawn_radius_px(1.0, 100.0, 0.0), 0.0);
    }
}
