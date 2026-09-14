//! The Lightcone client, on a desktop.
//!
//! The browser build is `lightcone_web`; the two share everything but where their arguments
//! and their assets come from.

use std::path::PathBuf;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use lc_client::app::{Catalogue, ClientPlugin};

/// Where the client's own assets are.
///
/// Bevy's default resolves `assets` against `CARGO_MANIFEST_DIR` when cargo set it and against
/// the executable's directory otherwise, so running the built binary directly looked for
/// `target/debug/assets` and found nothing. Checking next to the executable first keeps a
/// packaged build working; the compile-time path is the development fallback.
fn asset_path() -> String {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("assets")))
        .filter(|p| p.is_dir());
    beside_exe
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")))
        .to_string_lossy()
        .into_owned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dev, catalogue) = lc_client::entry::parse(&args);

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "Lightcone".into(), ..default() }),
                    ..default()
                })
                .set(AssetPlugin { file_path: asset_path(), ..default() }),
        )
        .insert_resource(Catalogue(catalogue))
        .insert_resource(dev)
        .add_plugins(ClientPlugin)
        .run();
}
