//! A shard as a process: bind a socket, load a world, tick.
//!
//! Thin on purpose. Everything worth testing is in the library, which is why the tick loop
//! here is six lines and has no logic of its own.

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
  --open              admit connections with no valid ticket — DEVELOPMENT ONLY
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

    // The authored sample, so a client started with no arguments is looking at the same three
    // stars the server is. A real catalogue is a later argument.
    server.load_world(World::new(AuthoredStars::sample().stars().to_vec()));

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
    if source.starts_with("http://") || source.starts_with("https://") {
        Ok(ureq::get(source).call()?.into_json()?)
    } else {
        Ok(serde_json::from_str(&std::fs::read_to_string(source)?)?)
    }
}
