//! The scenes panel.
//!
//! Buttons for [`lc_world::scenario`], which is where what they do is written. This draws a
//! list and asks; it decides nothing.
//!
//! Its own module rather than another arm in [`crate::panels`], which is the largest interface
//! file there is and has no room to spare.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::action::Action;
use crate::input::Requested;
use crate::ui::CameraPerspective;
use crate::uplink::Uplink;

/// Draw the list. Every button is the same ask, by name.
pub fn scenarios(
    ui: &mut egui::Ui,
    uplink: &Uplink,
    perspective: Option<CameraPerspective>,
    out: &mut MessageWriter<Requested>,
) {
    ui.label("Scenes to put in the world. Development only.");
    ui.separator();
    if uplink.joined().is_none() {
        // Said rather than hidden, because a panel whose buttons all do nothing is a panel
        // that looks broken. What is missing is an authority, and only it can place a craft.
        ui.label("No shard. Start with --local or --demo to stage one.");
        return;
    }
    for scene in lc_world::scenario::Scenario::ALL {
        if ui.button(scene.name).clicked() {
            super::panels::ask(out, Action::StageDemo(scene.name.to_string()));
        }
        ui.label(scene.blurb);
        if scene.rate != 1.0 {
            // The rate is the server's to state and it changes when a scene is staged, so what
            // the clock will do afterwards is part of what the button does.
            ui.label(format!("Runs at {:.0}x — the shard will say so.", scene.rate));
        }
        ui.separator();
    }
    watch_from(ui, uplink, perspective, out);
}

/// Which craft the camera is behind.
///
/// Only the eye moves — the light is still worked out from the player's own ship, which is why
/// this is here and not in a shipped build. See [`crate::ui::CameraPerspective::Pov`].
fn watch_from(
    ui: &mut egui::Ui,
    uplink: &Uplink,
    perspective: Option<CameraPerspective>,
    out: &mut MessageWriter<Requested>,
) {
    ui.label("Watch from");
    let aboard = perspective.is_none();
    if ui.add_enabled(!aboard, egui::Button::new("your own ship")).clicked() {
        super::panels::ask(out, Action::WatchFrom(None));
    }
    for contact in &uplink.contacts {
        let watching = perspective == Some(CameraPerspective::Pov(contact.ship_id));
        if ui.add_enabled(!watching, egui::Button::new(&contact.name)).clicked() {
            super::panels::ask(out, Action::WatchFrom(Some(contact.ship_id)));
        }
    }
    if uplink.contacts.is_empty() {
        ui.label("Nobody else in sight.");
    }
}
