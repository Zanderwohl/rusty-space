//! The Lightcone client, in a browser.
//!
//! Differs from the desktop binary in three things and nothing else:
//!
//! - **Assets come over HTTP.** `AssetPlugin::file_path` is a URL prefix on this target, so
//!   pointing it at a CDN is the whole of the CDN integration.
//! - **Arguments come from the query string**, because there is no `argv`.
//! - **No `.meta` probing**, because every one of those is a round trip and a 404.
//! - **WebGPU only.** There is no WebGL2 fallback and there will not be one: the renderer
//!   needs storage buffers and compute, and a silently degraded sky is worse than a refusal.
//!
//! The page is expected to have checked `navigator.gpu` before loading this, because the
//! check is free and the download is not.

#[cfg(target_arch = "wasm32")]
use bevy::asset::{AssetMetaCheck, AssetPlugin};
#[cfg(target_arch = "wasm32")]
use bevy::prelude::*;
#[cfg(target_arch = "wasm32")]
use bevy::render::RenderPlugin;
#[cfg(target_arch = "wasm32")]
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
#[cfg(target_arch = "wasm32")]
use lc_client::app::{Catalogue, ClientPlugin};

/// The canvas the game draws into. The page owns it; Bevy finds it by selector.
#[cfg(target_arch = "wasm32")]
const CANVAS: &str = "#lightcone";

/// Where assets are fetched from when the page does not say.
///
/// Relative, so a build served from a plain static directory works with no configuration. The
/// hosted build passes an absolute CDN URL instead.
#[cfg(target_arch = "wasm32")]
const DEFAULT_ASSET_BASE: &str = "assets";

#[cfg(target_arch = "wasm32")]
fn main() {
    // Without this a Rust panic is an unhelpful "unreachable executed" in the console.
    console_error_panic_hook::set_once();

    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let args = lc_client::entry::from_query(&search);
    let entry = lc_client::entry::parse(&args);

    // Off the launching page, not the query string: a ticket in a URL is a ticket in history,
    // in an access log, and in a `Referer`.
    let ticket = lc_client::Ticket(lc_client::entry::ticket_from_page());
    info!(signed_in = ticket.0.is_some(), "entry");

    let asset_base =
        lc_client::entry::param(&search, "assets").unwrap_or_else(|| DEFAULT_ASSET_BASE.to_owned());
    info!("assets from {asset_base}");

    App::new()
        .insert_resource(ticket)
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Lightcone".into(),
                        canvas: Some(CANVAS.into()),
                        // The canvas is sized by the page's CSS; let it drive.
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: asset_base,
                    // No asset here has a `.meta` sidecar, and the default is to probe for one
                    // beside every asset loaded. On a filesystem that is a wasted stat; over a
                    // CDN it is a round trip and a 404 per asset, cached as a negative entry.
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(WgpuSettings {
                        backends: Some(Backends::BROWSER_WEBGPU),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(Catalogue(entry.catalogue))
        .insert_resource(lc_client::uplink::ServerAddress(entry.server))
        .insert_resource(entry.dev)
        .add_plugins(ClientPlugin)
        .run();
}

/// Building this binary for a desktop is a mistake worth naming.
///
/// The target-gated `main` above leaves nothing behind on a native target, and a bin with no
/// `main` fails to compile — which `cargo test --workspace` hits, because it builds every
/// binary in the workspace whether or not anyone intends to run it.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "lightcone_web is the browser build; it runs in a browser and nowhere else.\n\
         On a desktop use `lightcone`. To build this one:\n\
         \n    tools/build-wasm.sh\n"
    );
    std::process::exit(2);
}
