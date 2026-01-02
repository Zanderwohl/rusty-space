use bevy::prelude::*;
use bevy::math::DVec3;
use bevy_egui::egui::Ui;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::universe::{Major, Minor};
use crate::body::universe::save::UniversePhysics;
use crate::gui::planetarium::time::SimTime;
use crate::util::format::sci_not;
use crate::util::gravity;

#[derive(Component)]
pub struct NewtonMotive {
    pub position: DVec3,
    pub velocity: DVec3,
}

impl NewtonMotive {
    pub fn display(&self, ui: &mut Ui) {
        ui.label("Position");
        ui.label(format!("\tx: {} m", sci_not(self.position.x)));
        ui.label(format!("\ty: {} m", sci_not(self.position.y)));
        ui.label(format!("\tz: {} m", sci_not(self.position.z)));

        ui.label("Velocity");
        ui.label(format!("\tx: {} m/s", sci_not(self.velocity.x)));
        ui.label(format!("\ty: {} m/s", sci_not(self.velocity.y)));
        ui.label(format!("\tz: {} m/s", sci_not(self.velocity.z)));
    }
}
/*
pub fn calculate(
    mut newton_bodies: Query<(&mut BodyState, &BodyInfo, &mut NewtonMotive, Option<&Major>, Option<&Minor>)>,
    mut other_bodies: Query<(&mut BodyState, &BodyInfo, &Major,), Without<NewtonMotive>>,
    sim_time: Res<SimTime>,
    sim_settings: Res<UniversePhysics>,
) {
    let step_size = sim_time.step;

    let mut major_bodies_prev_frame: std::collections::HashMap<String, (f64, DVec3)> = std::collections::HashMap::new();
    for (state, info, _, major, _) in newton_bodies.iter() {
        if major.is_some() {
            major_bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
        }
    }
    for (state, info, _) in other_bodies.iter() {
        major_bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
    }

    let delta_time = sim_time.time_seconds - sim_time.previous_time;
    for (mut state, info, mut motive, _, _) in newton_bodies.iter_mut() {
        if delta_time.abs() >= f64::EPSILON {
            let delta_v: DVec3 = major_bodies_prev_frame.iter().map(|(_, (other_mass, other_displacement))| {
                let g = gravity::newton_gravity_yeet(sim_settings.gravitational_constant, info.mass, motive.position, *other_mass, other_displacement);
                if g.is_nan() {
                    panic!("NaN velocity: (G = {}, g = {}, m = {}, M = {}, a->b = {}->{})", sim_settings.gravitational_constant, g, info.mass, *other_mass, motive.position, other_displacement);
                }
                g
            }).sum();
            if delta_v.is_nan() {
                panic!("NaN Delta V: {}", delta_v);
            }

            let starting_velocity = motive.velocity;
            let delta_v_with_t = delta_v * delta_time;
            motive.position += starting_velocity * delta_time;
            motive.velocity = starting_velocity + delta_v_with_t;
        }
        state.current_position = motive.position;
    }
}
*/
