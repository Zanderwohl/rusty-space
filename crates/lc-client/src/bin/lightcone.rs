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
                // `DefaultPlugins` already carries this once the `https` feature is on, so it
                // is configured rather than added. It is what lets a book be fetched from the
                // shelf's own CDN rather than from the build's asset directory.
                //
                // Its warning is about loading URLs from untrusted places, and the answer is that
                // this client never receives one: it receives a base from its own shard and a
                // bare file name, and `Shelf::where_to_fetch` puts them together.
                .set(bevy::asset::io::web::WebAssetPlugin { silence_startup_warning: true })
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "Lightcone Frontier".into(), ..default() }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: lc_client::entry::asset_root().to_string_lossy().into_owned(),
                    // No asset here has a `.meta` sidecar, and the default is to probe for one
                    // beside every asset loaded. On a filesystem that is a wasted stat; a
                    // desktop client fetching a book from the shelf's CDN makes it a round trip
                    // and a cached 404, which is what the browser build already avoids. See
                    // lightcone/docs/14-hosting.md.
                    meta_check: bevy::asset::AssetMetaCheck::Never,
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
