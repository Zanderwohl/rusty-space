//! Trajectory gizmo rendering system.

use bevy::prelude::*;
use bevy::color::Srgba;
use bevy::math::{DVec3, FloatExt};
use bevy::render::view::ColorGrading;
use num_traits::Pow;
use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{Motive, MotiveSelection};
use crate::body::universe::save::ViewSettings;
use crate::camera::{PlanetariumCamera, Freecam};
use crate::sim::SimTime;
use crate::gui::settings::{DisplayGlow, Settings};
use crate::util::bevystuff::GlamVec;

/// Renders trajectory lines as Bevy gizmos with brightness variation.
pub fn render_trajectories(
    bodies: Query<(&BodyState, &BodyInfo, &Motive, Option<&Appearance>)>,
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
    for (state, info, motive, appearance) in bodies.iter() {
        if !(view_settings.show_trajectories || view_settings.body_in_any_trajectory_tag(&info.id)) {
            continue;
        }
        if let Some(trajectory) = &state.trajectory {
            let frac = match trajectory.periodicity() {
                None => 0.0,
                Some(periodicity) => {
                    periodicity.cycle_fraction(sim_time.time.to_j2000_seconds())
                }
            };

            // Get the primary_id if this is a Keplerian motive
            let primary_id = match motive.motive_at(current_time) {
                (_, MotiveSelection::Keplerian(k)) => Some(&k.primary_id),
                _ => None,
            };

            // Collect trajectory points; we may insert a transient point at the
            // body's current position so the line always passes through the body.
            let mut points: Vec<(f64, DVec3)> = trajectory.iter().map(|(t, d)| (t, *d)).collect();

            if let (Some(local_pos), Some(periodicity)) = (state.current_local_position, trajectory.periodicity()) {
                let current_relative_time = frac * periodicity.interval_size;
                let body_radius = appearance.map(|a| a.radius()).unwrap_or(0.0);

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

            // All trajectory displacements are relative to the primary; offset by
            // the primary's current global position when drawing.
            let primary_offset: Option<DVec3> = primary_id
                .and_then(|id| {
                    bodies.iter().find(|(_, info, _, _)| &info.id == id)
                })
                .and_then(|(primary_state, _, _, _)| {
                    if primary_state.trajectory.is_none() { return None; }
                    Some(primary_state.current_position)
                });

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
                gizmos.line(d1.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos), d2.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos), color);
            }
        }
    }
}
