//! The Lightcone client, on a desktop.
//!
//! The browser build is `lightcone_web`; the two share everything but where their arguments
//! and their assets come from.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use lc_client::app::{Catalogue, ClientPlugin};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let entry = lc_client::entry::parse(&args);

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "Lightcone Frontier".into(), ..default() }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: lc_client::entry::asset_root().to_string_lossy().into_owned(),
                    ..default()
                }),
        )
        .insert_resource(Catalogue(entry.catalogue))
        // `--local` wins over `--server`: asking for one in this process is the more specific
        // request, and its address is not known until the socket is bound.
        .insert_resource(lc_client::uplink::ServerAddress(
            if entry.local { None } else { entry.server },
        ))
        .insert_resource(lc_client::uplink::LocalShard(entry.local))
        .insert_resource(lc_client::uplink::Demo(entry.demo))
        .insert_resource(entry.dev)
        .add_plugins(ClientPlugin)
        .run();
}
