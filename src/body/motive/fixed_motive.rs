use bevy::prelude::*;
use bevy::math::DVec3;
use bevy_egui::egui::Ui;
use crate::body::motive::info::{BodyInfo, BodyState};

#[derive(Component)]
pub struct FixedMotive {
    pub position: DVec3,
}

impl FixedMotive {
    pub fn display(&self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.label(format!("x: {} m", crate::util::format::sci_not(self.position.x)));
            ui.label(format!("y: {} m", crate::util::format::sci_not(self.position.y)));
            ui.label(format!("z: {} m", crate::util::format::sci_not(self.position.z)));
        });
    }
}

pub fn calculate(
    mut fixed_bodies: Query<(&mut BodyState, &BodyInfo, &FixedMotive),
        Or<(Changed<FixedMotive>, Added<FixedMotive>)>>,
) {
    for (mut state, _, motive) in fixed_bodies.iter_mut() {
        state.current_position = motive.position;
        state.last_step_position = motive.position;
    }
}
