//! A server in the box.
//!
//! The same `lc-server` a shard runs, on a loopback socket, in this process. Not a simulation
//! of one and not a second implementation: identical code, so anything tuned against it is
//! tuned against what a deployment does — the light-delay gate, the intent clamps, the tick.
//!
//! This is why the desktop build contains a server. For a game whose single-player mode *is* a
//! server with one player, that is the arrangement rather than a compromise. The browser build
//! has none: it is served from a CDN and always talks to a shard.

use std::sync::mpsc::channel;

use lc_server::journal::Memory;
use lc_server::server::{Server, TICK_MS};
use lc_server::websocket::WebSocketServer;
use lc_server::world::World;
use lc_world::scenario::Scenario;
use lc_world::sky::CatalogueStar;

/// Start one, and return the address to connect to.
///
/// Blocks only until the socket is bound, which is what produces the port — the operating
/// system picks it, so nothing here can collide with a shard already running.
///
/// `stars` should be the ones the client itself loaded. Both ends place craft into systems by
/// position against the same shell radius, so a server with a different sky would disagree with
/// the client about which system a ship is in.
pub fn start(stars: Vec<CatalogueStar>, demo: Option<String>) -> Result<String, String> {
    let (tell, address) = channel::<Result<String, String>>();
    std::thread::Builder::new()
        .name("lc-local-server".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(why) => {
                    let _ = tell.send(Err(why.to_string()));
                    return;
                }
            };
            runtime.block_on(serve(stars, demo, tell));
        })
        .map_err(|why| why.to_string())?;

    // The thread sends exactly once, success or failure. A dropped sender means it panicked
    // before either, which is still an answer.
    address
        .recv()
        .unwrap_or_else(|_| Err("the local server stopped before it started".into()))
}

/// The catalogue next to this build, if it ships one.
///
/// The base is the asset path the client already fetches books under, because a local shard
/// lends the files in its own directory and there is no CDN in the box.
fn shelf() -> Result<Option<lc_server::library::Library>, String> {
    let path = crate::entry::asset_root().join("books").join("books.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|why| format!("{}: {why}", path.display()))?;
    // `LC_SHELF_BASE` is the same variable a deployed shard is told its CDN with, so the server
    // in the box can be pointed at one and the client then fetches over HTTP exactly as the
    // browser build does. Unset, the shelf is the directory this catalogue was read from.
    let base = std::env::var("LC_SHELF_BASE").unwrap_or_else(|_| crate::library::SHELF.to_owned());
    lc_server::library::Library::from_toml(&base, &text).map(Some)
}

async fn serve(
    stars: Vec<CatalogueStar>,
    demo: Option<String>,
    tell: std::sync::mpsc::Sender<Result<String, String>>,
) {
    let mut wire = match WebSocketServer::bind("127.0.0.1:0").await {
        Ok(wire) => wire,
        Err(why) => {
            let _ = tell.send(Err(why.to_string()));
            return;
        }
    };
    if tell.send(Ok(format!("ws://{}", wire.local_addr))).is_err() {
        // Nobody is waiting, so there is nobody to serve.
        return;
    }

    let mut server = Server::new(Memory::default(), 0, 1);
    // There is no broker here and the only client is the process asking. A ticket would be one
    // this process minted for itself, which proves nothing.
    server.admit_without_tickets(true);
    server.load_world(World::new(stars));
    // **Development only, and only ever here.** A shard is never started for this: a client
    // that could stage a scene could put a craft wherever it liked, which is the one thing the
    // authority keeps for itself.
    server.directing(true);
    // The books beside the client that started this. A shard is told its catalogue on the
    // command line; the one in the box finds it the same way the asset server does, so
    // single-player exercises the same wire path a deployment does rather than a shortcut
    // around it.
    match shelf() {
        Ok(Some(library)) => server.library = library,
        Ok(None) => {}
        Err(why) => bevy::log::warn!("no shelf for the local shard: {why}"),
    }
    if let Some(scene) = demo.as_deref().and_then(Scenario::named) {
        if let Err(why) = server.stage(scene) {
            bevy::log::error!("could not stage {}: {why:?}", scene.name);
        }
    }

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(TICK_MS as u64));
    // A tick missed because the machine was busy is a slice of coordinate time nothing was read
    // in. Catching up by sprinting would read them all at once, which is not the same thing.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        for client in wire.departed() {
            server.disconnected(client);
        }
        if let Err(why) = server.tick(&mut wire).await {
            bevy::log::error!("the local server stopped: {why}");
            return;
        }
    }
}
