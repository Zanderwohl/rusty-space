//! Bevy systems driving the Keplerian motive.
//!
//! The elements themselves live in `em_sim::motive::kepler`; this is the ECS wiring.
//! Phase 5 folds this into the `System` arena and deletes the file.

use bevy::math::DVec3;
use bevy::prelude::*;
use em_sim::motive::kepler::KeplerMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::sim::{BodySelection, CalculateTrajectory, SimTime};
use crate::body::universe::save::{UniversePhysics, ViewSettings};
use em_foundations::time::Instant;
use em_sim::time_map::TimeMap;
use bevy_egui::egui::Ui;

/// Was `KeplerMotive::display`. It cannot be an inherent method any more — the type
/// lives in `em-sim`, which knows nothing about egui — so it is a free function here.
/// Still the stub it always was.
pub fn display_kepler_motive(_motive: &KeplerMotive, ui: &mut Ui) {
    ui.label("Shape");
    ui.label("Rotation");
    ui.label("Epoch");
}

pub fn calculate_trajectory(
    mut calcs: MessageReader<CalculateTrajectory>,
    mut bodies: Query<(&mut BodyState, &BodyInfo, &crate::body::motive::Motive)>,
    physics: Res<UniversePhysics>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
) {
    if calcs.is_empty() { return; }

    // First collect all body masses into a HashMap
    let mut body_masses: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for (_, info, _) in bodies.iter() {
        body_masses.insert(info.id.clone(), info.mass);
    }

    let current_time = sim_time.time;

    for calc in calcs.read() {
        for (mut state, info, motive) in bodies.iter_mut() {
            let do_this = match &calc.selection {
                BodySelection::All => true,
                BodySelection::Tag(tag) => info.tags.contains(tag),
                BodySelection::IDs(ids) => ids.contains(&info.id),
            };
            if !do_this { continue; }

            // Get the current motive selection
            let (_, selection) = motive.motive_at(current_time);
            
            // Only calculate trajectories for Keplerian bodies
            let kepler_motive = match selection {
                crate::body::motive::MotiveSelection::Keplerian(k) => k,
                _ => continue,
            };

            let primary_mass = body_masses.get(&kepler_motive.primary_id)
                .copied()
                .expect("Missing primary body mass");
            // mu = G(M + m): must match the value used to propagate the body itself,
            // or the drawn trajectory will not close on the body's actual position.
            let mu = physics.gravitational_constant * (primary_mass + info.mass);

            state.trajectory = Some(TimeMap::new());
            let map = state.trajectory.as_mut().unwrap();
            let period = kepler_motive.period(mu);

            let periapsis_time = kepler_motive.time_at_periapsis_passage(mu);

            if !kepler_motive.is_open() {
                map.set_periodicity(periapsis_time, period);
            }

            for i in 0..=view_settings.trajectory_resolution {
                let relative_time = (i as f64 / view_settings.trajectory_resolution as f64) * period.to_seconds();
                let absolute_time = Instant::from_seconds_since_j2000(periapsis_time.to_j2000_seconds() + relative_time);
                let displacement = kepler_motive.displacement(absolute_time, mu);
                if let Some(displacement) = displacement {
                    map.insert(relative_time, displacement); // Store using relative time as key
                }
            }
        }
    }
}
