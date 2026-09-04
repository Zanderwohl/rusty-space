use bevy::prelude::*;
use bevy::math::DVec3;
use bevy_egui::egui::Ui;
use crate::util::format::sci_not;

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
