//! The Lightcone client.

use bevy::prelude::*;
use lc_client::app::{Catalogue, ClientPlugin};

fn main() {
    let catalogue = std::env::args().nth(1);
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightcone".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Catalogue(catalogue))
        .add_plugins(ClientPlugin)
        .run();
}
