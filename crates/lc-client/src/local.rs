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
use glam::DVec3;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::sky::CatalogueStar;
use lc_world::system::M_PER_LY;

/// Start one, and return the address to connect to.
///
/// Blocks only until the socket is bound, which is what produces the port — the operating
/// system picks it, so nothing here can collide with a shard already running.
///
/// `stars` should be the ones the client itself loaded. Both ends place craft into systems by
/// position against the same shell radius, so a server with a different sky would disagree with
/// the client about which system a ship is in.
/// How far apart the craft `--traffic` puts out are spread, metres.
///
/// A few tens of kilometres: far enough that they are separate contacts with their own marks,
/// near enough that a five-hundred-metre hull is still several pixels across.
const TRAFFIC_SPACING_M: f64 = 2.0e4;

/// How fast they drift, as a fraction of `c`. Thirty metres a second — slow enough to stay put
/// for a session, fast enough that each one has a velocity and therefore an attitude.
const TRAFFIC_BETA: f64 = 1.0e-7;

pub fn start(stars: Vec<CatalogueStar>, traffic: usize) -> Result<String, String> {
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
            runtime.block_on(serve(stars, traffic, tell));
        })
        .map_err(|why| why.to_string())?;

    // The thread sends exactly once, success or failure. A dropped sender means it panicked
    // before either, which is still an answer.
    address
        .recv()
        .unwrap_or_else(|_| Err("the local server stopped before it started".into()))
}

/// Craft to keep the player company, spread along a line near where they start.
///
/// There is no game reason for these and there never will be: they exist so that the thing
/// a screenshot is supposed to show — another ship, drawn, named and pointing somewhere — can
/// be photographed at all. Unowned, so the shard treats them as it treats a probe.
///
/// Identifiers well above the ones a sign-in mints, so a player's own ship cannot collide with
/// one of these.
fn company(near_ly: DVec3, count: usize) -> Vec<Craft> {
    (0..count)
        .map(|i| {
            let along = (i as f64 + 1.0) * TRAFFIC_SPACING_M / M_PER_LY;
            // Fanned across two axes rather than strung out along one, so they are not all
            // the same distance away and the nearest is not hiding the rest.
            let at = near_ly + DVec3::new(along, along * 0.35, along * -0.2);
            let mut craft = Craft::at(CraftId(1_000 + i as i64), Kind::Ship, at);
            // Each one heading somewhere different, which is the whole of what makes their
            // attitudes worth looking at.
            let heading = DVec3::new((i as f64).cos(), (i as f64).sin(), 0.2).normalize();
            craft.motion.beta = heading * TRAFFIC_BETA;
            craft.motion.set_adrift(0.0);
            craft
        })
        .collect()
}

async fn serve(
    stars: Vec<CatalogueStar>,
    traffic: usize,
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
    let world = World::new(stars);
    // Read before the world is handed over, because the server owns it afterwards.
    let start = world.start();
    server.load_world(world);
    if let Some(at) = start {
        for craft in company(at, traffic) {
            server.fleet_mut().insert(craft);
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
