use bevy::prelude::*;
use bevy::color::Srgba;
use bevy::math::{DVec3, FloatExt};
use bevy::render::view::ColorGrading;
use itertools::Itertools;
use num_traits::Pow;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::universe::save::ViewSettings;
use crate::gui::planetarium::PlanetariumCamera;
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::{DisplayGlow, Settings};
use crate::gui::util::freecam::Freecam;
use crate::util::bevystuff::GlamVec;

pub fn render_trajectories(
    bodies: Query<(&BodyState, &BodyInfo, Option<&KeplerMotive>)>,
    mut gizmos: Gizmos,
    view_settings: Res<ViewSettings>,
    settings: Res<Settings>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
    color_grading: Single<&ColorGrading>,
) {
    let distance_scale = view_settings.distance_factor();

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
    for (state, info, kepler_motive) in bodies.iter() {
        if !(view_settings.show_trajectories || view_settings.body_in_any_trajectory_tag(&info.id)) {
            continue;
        }
        if let Some(trajectory) = &state.trajectory {
            let len = trajectory.len();
            let frac = match trajectory.periodicity() {
                None => 0.0,
                Some(periodicity) => {
                    periodicity.cycle_fraction(sim_time.time_seconds)
                }
            };

            // TODO: this doesn't track for the future.
            let primary_d: Option<Vec<DVec3>> = kepler_motive
                .map(|m| {
                    let id = &m.primary_id;
                    bodies.iter().find(|(_, info, _)| { &info.id == id })
                })
                .flatten()
                .map(|(primary_state, _, _)| {
                    if primary_state.trajectory.is_none() { return None; }
                    let primary_trajectory = primary_state.trajectory.as_ref().unwrap();
                    trajectory.iter().map(|(t, _)| {
                        // primary_trajectory.get_lerp(t)
                        Some(primary_state.current_position)
                    }).collect()
                })
                .flatten();

            for (idx, ((t1, d1), (t2, d2))) in trajectory.iter().tuple_windows().enumerate() {
                let (d1, d2) = match &primary_d {
                    None => (d1.clone(), d2.clone()),
                    Some(primary_d) => (d1 + primary_d[idx], d2 + primary_d[idx + 1])
                };

                // Calculate the fractional position of this trajectory segment
                let segment_frac = idx as f32 / len as f32;
                let next_segment_frac = (idx + 1) as f32 / len as f32;
                
                // Check if planet is currently within this segment
                let planet_in_segment = if next_segment_frac > segment_frac {
                    frac as f32 >= segment_frac && (frac as f32) < next_segment_frac
                } else {
                    // Handle wraparound case
                    frac as f32 >= segment_frac || (frac as f32) < next_segment_frac
                };
                
                let brightness_factor = if planet_in_segment {
                    // Smooth fade within current segment based on planet's position within it
                    let progress_through_segment = if next_segment_frac > segment_frac {
                        (frac as f32 - segment_frac) / (next_segment_frac - segment_frac)
                    } else {
                        // Handle wraparound
                        if frac as f32 >= segment_frac {
                            (frac as f32 - segment_frac) / (1.0 - segment_frac + next_segment_frac)
                        } else {
                            (frac as f32 + 1.0 - segment_frac) / (1.0 - segment_frac + next_segment_frac)
                        }
                    };
                    progress_through_segment // Fade from 0.0 to 1.0 as planet moves through segment
                } else {
                    // Use sharp discontinuity for all other segments
                    let forward_offset = (segment_frac - frac as f32 + 1.0) % 1.0;
                    if forward_offset <= 0.5 {
                        0.0  // Dark ahead of planet
                    } else {
                        (forward_offset - 0.5) * 2.0  // Brightens as we go behind planet (trail)
                    }
                };
                
                color = Srgba::new(0.0, 1.0, 0.0, min_brightness.lerp(max_brightness, brightness_factor));
                gizmos.line(d1.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos), d2.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos), color);
            }
        }
    }
}
