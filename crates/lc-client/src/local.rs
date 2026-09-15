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
use lc_world::navigation::Waypoint;
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
/// How far the nearest craft `--traffic` puts out stands off, metres.
///
/// Eight kilometres, where a five-hundred-metre hull is some forty pixels across and reads as
/// a ship. Each one after it is half again as far, so the set spans the range over which a
/// hull stops being a shape and becomes a mark.
const TRAFFIC_NEAREST_M: f64 = 8.0e3;

/// How far off the axis the player starts looking along they are fanned, radians.
///
/// Inside the vertical half-field, so they are all on screen at once without being stacked on
/// one bearing — which is what the first version of this did, and four marks and four labels
/// on one point is not a picture of four ships.
const TRAFFIC_SPREAD_RAD: f64 = 0.28;



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
            let phase = std::f64::consts::TAU * i as f64 / count.max(1) as f64;
            // Ahead of where the player starts looking, fanned about that axis and stepped
            // back in range, so the four are four marks rather than one.
            let bearing = DVec3::new(
                1.0,
                TRAFFIC_SPREAD_RAD * phase.cos(),
                TRAFFIC_SPREAD_RAD * phase.sin(),
            )
            .normalize();
            let range = TRAFFIC_NEAREST_M * (1.0 + 0.8 * i as f64);
            let at = near_ly + bearing * (range / M_PER_LY);
            let mut craft = Craft::at(CraftId(1_000 + i as i64), Kind::Ship, at);
            // Held rather than drifting. A shard runs at 8766 times real time, so the slowest
            // speed worth calling a speed carries a craft out of sight in seconds — the first
            // version of this gave them thirty metres a second apiece and they were three
            // hundred kilometres away by the time the shutter opened. The cost is that a held
            // craft has no attitude anything decides, so they all point the same way; what a
            // nose following a drive looks like is the player's own ship under thrust.
            craft.motion.begin_holding(Waypoint::Fixed(at));
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
