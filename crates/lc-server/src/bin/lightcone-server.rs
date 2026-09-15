//! A shard as a process: bind a socket, load a world, tick.
//!
//! Thin on purpose. Everything worth testing is in the library, which is why the tick loop
//! here is six lines and has no logic of its own.

use std::io::Read;
use std::time::Duration;

use lc_proto::ClientId;
use lc_server::journal::Memory;
use lc_server::server::{Server, TICK_MS};
use lc_server::ticket::Trusted;
use lc_server::websocket::WebSocketServer;
use lc_server::world::World;
use lc_world::sky::{AuthoredStars, StarProvider};

const USAGE: &str = "\
lightcone-server — one shard

  --bind <addr>       where to listen (default 127.0.0.1:8080)
  --audience <name>   the audience tickets must name (default shard-1)
  --jwks <url|path>   the broker's published keys, fetched at boot
  --sky <url|path>    the packed catalogue this shard is authoritative over
  --open              admit connections with no valid ticket — DEVELOPMENT ONLY

Point --sky at the **same chunk the promoted client downloads**, which is
<cdn>/game/<build>/assets/sky/hyg-v42.lcsky. Both ends place craft into systems by position
against the same shell radius, so two different catalogues is two different answers to which
system a ship is in — and nothing reports the disagreement.
";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return Ok(());
    }
    let after = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let flag = |name: &str| args.iter().any(|a| a == name);

    let bind = after("--bind").unwrap_or_else(|| "127.0.0.1:8080".into());
    let audience = after("--audience").unwrap_or_else(|| "shard-1".into());
    let mut server = Server::new(Memory::default(), 0, 1);

    match after("--jwks") {
        Some(source) => {
            let mut trusted = Trusted::new(&audience);
            let learned = trusted.learn(&read_jwks(&source)?);
            if learned == 0 {
                // Starting anyway would mean refusing every ticket, which looks from the
                // outside exactly like everyone's credentials being wrong at once.
                return Err(format!("{source} published no key this server can use").into());
            }
            eprintln!("trusting {learned} key(s) from {source} for audience {audience}");
            server.trust(trusted);
        }
        None if flag("--open") => {}
        None => return Err("no --jwks and no --open: nobody could connect".into()),
    }

    if flag("--open") {
        server.admit_without_tickets(true);
        eprintln!(
            "WARNING: --open admits anyone with no ticket. Development only, and every \
             reconnection is a new ship."
        );
    }

    let stars = match after("--sky") {
        Some(source) => {
            let provider = lc_world::sky::chunk::ChunkProvider::decode(&read_bytes(&source)?)
                .map_err(|why| format!("{source}: {why:?}"))?;
            eprintln!("sky: {} stars from {source}", provider.stars().len());
            provider.stars().to_vec()
        }
        // Three hand-written stars. Fine for a shard nobody connects a real client to, and
        // wrong for every other case: a client loading the real catalogue will disagree with
        // this about which system it is in, and neither end will say so.
        None => {
            eprintln!("WARNING: no --sky, so this shard's world is the authored sample. A client");
            eprintln!("         with a real catalogue will not agree with it about anything.");
            AuthoredStars::sample().stars().to_vec()
        }
    };
    server.load_world(World::new(stars));

    let mut wire = WebSocketServer::bind(&bind).await?;
    eprintln!("listening on {}", wire.local_addr);

    let mut ticker = tokio::time::interval(Duration::from_millis(TICK_MS as u64));
    // The tick is the clock. Falling behind must not make the server sprint to catch up,
    // because every skipped tick is a slice of coordinate time nothing was read in.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        for ClientId(id) in wire.accepted() {
            eprintln!("connection {id} opened");
        }
        for client in wire.departed() {
            eprintln!("connection {} closed", client.0);
            server.disconnected(client);
        }
        server.tick(&mut wire).await?;
    }
}

/// The broker's key set, from a URL or a file.
///
/// A file is not a convenience: it is how a shard starts when the broker is down, which is a
/// state the game server is supposed to survive — it verifies locally and never asks per
/// connection. See `lightcone/docs/16-identity.md`.
fn read_jwks(source: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&read_bytes(source)?)?)
}

/// Bytes from a URL or a file, which is how every input this takes is named.
///
/// A URL matters for the sky in particular: pointing a shard at the CDN path of the promoted
/// build is what makes "both ends hold the same catalogue" a fact rather than a convention
/// somebody has to keep.
fn read_bytes(source: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let mut bytes = Vec::new();
        ureq::get(source).call()?.into_reader().read_to_end(&mut bytes)?;
        Ok(bytes)
    } else {
        Ok(std::fs::read(source)?)
    }
}
