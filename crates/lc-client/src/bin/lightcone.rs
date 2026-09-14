//! The Lightcone client.

use std::path::PathBuf;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use lc_client::action::Action;
use lc_client::app::{Catalogue, ClientPlugin, DevEntry};

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
    let flag = |name: &str| args.iter().position(|a| a == name);
    fn value<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1)?.parse().ok()
    }
    let mut actions = Vec::new();
    if let Some(preset) = value::<usize>(&args, "--band") {
        actions.push(Action::SetBandPreset(preset));
    }
    if let Some(rate) = value::<f64>(&args, "--rate") {
        actions.push(Action::SetTimeRate(rate));
    }
    if flag("--tune").is_some() {
        actions.push(Action::OpenPanel(lc_client::ui::Panel::Tuning));
    }
    if flag("--watch").is_some() || flag("--swarm").is_some() {
        actions.push(Action::SelectNearest);
        actions.push(Action::OpenPanel(lc_client::ui::Panel::Telescope));
    }
    if let Some(b) = value::<usize>(&args, "--curve") {
        if let Some(band) = em_spectra::Band::ALL.get(b) {
            actions.push(Action::SetCurveBand(*band));
        }
    }
    if flag("--fly").is_some() {
        // Index 0 of the sorted sky is the Sun in the full catalogue; 1 is interstellar.
        actions.push(Action::FlyToNearest);
    }
    let dev = DevEntry {
        observe_immediately: flag("--observe").is_some()
            || flag("--shot").is_some()
            || flag("--at").is_some(),
        target_swarm: flag("--swarm").is_some(),
        at_body: flag("--at").and_then(|i| args.get(i + 1).cloned()),
        screenshot: flag("--shot").and_then(|i| args.get(i + 1).cloned()),
        after_frames: value::<u32>(&args, "--frames").unwrap_or(120),
        actions,
    };
    let catalogue = args.first().filter(|a| !a.starts_with("--")).cloned();
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
