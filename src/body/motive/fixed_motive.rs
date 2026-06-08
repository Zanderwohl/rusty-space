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
