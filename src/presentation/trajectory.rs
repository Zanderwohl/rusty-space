//! Trajectory gizmo rendering system.

use bevy::prelude::*;
use bevy::color::Srgba;
use bevy::math::{DVec3, FloatExt};
use bevy::render::view::ColorGrading;
use num_traits::Pow;
use crate::body::universe::save::ViewSettings;
use crate::sim::world::{BodyRef, SimSystem, Trajectories};
use crate::camera::{PlanetariumCamera, Freecam};
use crate::sim::SimTime;
use crate::gui::settings::{DisplayGlow, Settings};
use crate::presentation::render_space::ToRender;

/// Renders trajectory lines as Bevy gizmos with brightness variation.
pub fn render_trajectories(
    bodies: Query<&BodyRef>,
    system: Res<SimSystem>,
    trajectories: Res<Trajectories>,
    mut gizmos: Gizmos,
    view_settings: Res<ViewSettings>,
    settings: Res<Settings>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
    color_grading: Single<&ColorGrading>,
) {
    let distance_scale = view_settings.distance_factor();
    let current_time = sim_time.time;

    let exposure = color_grading.global.exposure;

    let (min_brightness, max_brightness) = match settings.display.glow {
        DisplayGlow::None => { (0.1, 1.0) }
        DisplayGlow::Subtle => { (0.25, 1.2) }
        DisplayGlow::VFD => { (1.0, 4.0) }
        DisplayGlow::Defcon => { (0.2, 10.0) }
    };
    let exposure_adjust = 2f32.pow(-exposure);
    let min_brightness = min_brightness * exposure_adjust;
    let max_brightness = max_brightness * exposure_adjust;

    let mut color = Srgba::new(1.0, 0.0, 0.0, 1.0);
    for body in bodies.iter() {
        let Some(i) = system.0.index_of(body.0) else { continue };
        let info = system.0.info(i);
        if !(view_settings.show_trajectories || view_settings.body_in_any_trajectory_tag(&info.id)) {
            continue;
        }
        if let Some(trajectory) = trajectories.0.get(&body.0) {
            let frac = match trajectory.periodicity() {
                None => 0.0,
                Some(periodicity) => {
                    periodicity.cycle_fraction(sim_time.time)
                }
            };



            // Collect trajectory points; we may insert a transient point at the
            // body's current position so the line always passes through the body.
            let mut points: Vec<(f64, DVec3)> = trajectory.iter().map(|(t, d)| (t.to_seconds(), *d)).collect();

            if let (Some(local_pos), Some(periodicity)) = (system.0.local_position(i), trajectory.periodicity()) {
                let current_relative_time = frac * periodicity.interval_size.to_seconds();
                let body_radius = system.0.radius(i);

                if let Some(seg) = points.windows(2).position(|w| {
                    current_relative_time >= w[0].0 && current_relative_time < w[1].0
                }) {
                    let dist_before = (local_pos - points[seg].1).length();
                    let dist_after = (local_pos - points[seg + 1].1).length();

                    if dist_before >= body_radius && dist_after >= body_radius {
                        points.insert(seg + 1, (current_relative_time, local_pos));
                    }
                }
            }

            let len = points.len();

            // Trajectory samples are offsets from the primary, so shift them by where the
            // primary is now. A direct arena lookup, where this used to be a linear scan
            // over every body for every body that drew a trajectory.
            let primary_offset: Option<DVec3> = system.0.parent(i).map(|p| system.0.position(p));

            for (idx, window) in points.windows(2).enumerate() {
                let (d1, d2) = (window[0].1, window[1].1);
                let (d1, d2) = match primary_offset {
                    None => (d1, d2),
                    Some(offset) => (d1 + offset, d2 + offset),
                };

                let segment_frac = idx as f32 / len as f32;
                let next_segment_frac = (idx + 1) as f32 / len as f32;
                
                let planet_in_segment = if next_segment_frac > segment_frac {
                    frac as f32 >= segment_frac && (frac as f32) < next_segment_frac
                } else {
                    frac as f32 >= segment_frac || (frac as f32) < next_segment_frac
                };
                
                let brightness_factor = if planet_in_segment {
                    let progress_through_segment = if next_segment_frac > segment_frac {
                        (frac as f32 - segment_frac) / (next_segment_frac - segment_frac)
                    } else {
                        if frac as f32 >= segment_frac {
                            (frac as f32 - segment_frac) / (1.0 - segment_frac + next_segment_frac)
                        } else {
                            (frac as f32 + 1.0 - segment_frac) / (1.0 - segment_frac + next_segment_frac)
                        }
                    };
                    progress_through_segment
                } else {
                    let forward_offset = (segment_frac - frac as f32 + 1.0) % 1.0;
                    if forward_offset <= 0.5 {
                        0.0
                    } else {
                        (forward_offset - 0.5) * 2.0
                    }
                };
                
                color = Srgba::new(0.0, 1.0, 0.0, min_brightness.lerp(max_brightness, brightness_factor));
                gizmos.line(d1.to_render_relative(distance_scale, fcam.bevy_pos), d2.to_render_relative(distance_scale, fcam.bevy_pos), color);
            }
        }
    }
}
