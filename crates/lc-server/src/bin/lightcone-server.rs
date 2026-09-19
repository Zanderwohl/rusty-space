//! A shard as a process: bind a socket, load a world, tick.
//!
//! Thin on purpose. Everything worth testing is in the library, which is why the tick loop
//! here is six lines and has no logic of its own.

use std::io::Read;
use std::time::Duration;

use lc_proto::ClientId;
use tokio::signal::unix::{SignalKind, signal};
use lc_server::journal::{Memory, Postgres, Store};
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
  --shard <n>         this shard's number, which keys its saved state (default 1)
  --db <url>          where craft are kept, so the world outlives this process
                      (or LC_SHARD_DB, which is where it belongs: it is a password,
                       and an argument is visible to anything that can list processes)
  --admin-bind <addr> a read-only surface for the administration console, on its own port.
                      Needs --db and --jwks. Never publish a route to it: it is reached
                      over the container network by name and by nothing else.
  --open              admit connections with no valid ticket — DEVELOPMENT ONLY

Without --db a shard is a sandcastle: it runs, and everything in it is gone when it stops —
craft, the world's clock, and every conversation anyone has had. With one, craft are written
every few seconds and on the way out, events and conversations as they happen, and all of it is
read back at boot — including the world's clock, without which every saved craft reads as one
whose crossing has not begun.

Point --sky at the **same chunk the promoted client downloads**, which is
<cdn>/game/<build>/assets/sky/catalogue.lcsky. Both ends place craft into systems by position
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
    let admin_bind = after("--admin-bind");
    let audience = after("--audience").unwrap_or_else(|| "shard-1".into());
    let shard_id: i64 = after("--shard").and_then(|s| s.parse().ok()).unwrap_or(1);

    // The journal is chosen here because it cannot be chosen later: everything below holds a
    // `Server` and a server is generic over what it writes to. Its own connection, separate
    // from the checkpoint's below — two cheap sockets rather than one handle two borrows of
    // the same server have to share.
    let db = after("--db").or_else(|| std::env::var("LC_SHARD_DB").ok());
    let journal = match &db {
        Some(url) => {
            let client = connect(url).await?;
            lc_store::migrate::apply(&client).await?;
            Store::Durable(Postgres::with(client))
        }
        None => Store::Ephemeral(Memory::default()),
    };
    let mut server = Server::new(journal, 0, shard_id as u64);

    let mut admin_keys: Option<Trusted> = None;
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
            // Cloned before the server takes it: a second `Trusted` would be a second thing to keep
            // in step through a key rotation.
            admin_keys = Some(trusted.clone());
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
    // Shared with the administration surface below.
    let catalogue = std::sync::Arc::new(stars);
    server.load_world(World::from_shared(catalogue.clone()));

    // Refused rather than silently skipped: a console pointed at a shard that quietly declined
    // to listen is a card reading "unavailable" with nothing to explain it.
    if let Some(addr) = &admin_bind {
        let (Some(url), Some(keys)) = (db.as_ref(), admin_keys) else {
            return Err("--admin-bind needs both --db and --jwks".into());
        };
        let api = lc_server::admin::Api::new(
            std::sync::Arc::new(connect(url).await?),
            catalogue.clone(),
            std::sync::Arc::new(keys),
        );
        let listener = tokio::net::TcpListener::bind(addr).await?;
        eprintln!("administration surface on {addr}, for audience {audience}");
        tokio::spawn(async move {
            if let Err(why) = axum::serve(listener, lc_server::admin::router(api)).await {
                eprintln!("administration surface stopped: {why}");
            }
        });
    }

    // The shelf. A shard with no catalogue runs without one and says nothing about a library;
    // a shard with a catalogue and no base would send files hanging off nothing, so both are
    // required together or neither is taken.
    match (
        after("--library").or_else(|| std::env::var("LC_LIBRARY").ok()),
        after("--shelf-base").or_else(|| std::env::var("LC_SHELF_BASE").ok()),
    ) {
        (Some(path), Some(base)) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read the catalogue at {path}: {e}"))?;
            server.library = lc_server::library::Library::from_toml(&base, &text)?;
            eprintln!("shelf: {} books from {path}, served from {base}", server.library.books.len());
        }
        (Some(_), None) => {
            // Loudly: a shelf nobody can fetch from is a list of titles that do nothing.
            return Err("--library needs --shelf-base, or the client has nowhere to fetch from".into());
        }
        (None, _) => eprintln!("no --library, so this shard has no books to lend"),
    }

    // After the world, because a ballistic arc is re-solved against the system it is in and a
    // shard with no stars would bring every coasting craft back as a straight line.
    let store = match db {
        Some(url) => {
            let client = connect(&url).await?;
            match lc_store::ships::load_shard(&client, shard_id).await? {
                Some(shard) => {
                    let rows = lc_store::ships::load_ships(&client).await?;
                    let count = rows.len();
                    let refused = server.adopt(lc_server::persist::Checkpoint {
                        now_t: shard.now_t,
                        next_ship: shard.next_ship,
                        ships: rows,
                    });
                    for craft in &refused {
                        eprintln!(
                            "ERROR: ship {} ({}) will not load: {} — its account is refused \
                             rather than given a second ship",
                            craft.ship_id,
                            craft.account.as_deref().unwrap_or("no account"),
                            craft.why,
                        );
                    }
                    let marks = lc_store::reading::load(&client).await?;
                    if !marks.is_empty() {
                        eprintln!("resumed {} bookmarks", marks.len());
                    }
                    server.library.adopt(
                        marks
                            .into_iter()
                            .map(|m| {
                                (m.account, lc_proto::Bookmark {
                                    book: m.book,
                                    spine: m.spine.max(0) as u32,
                                    char_offset: m.char_offset.max(0) as u32,
                                    location: m.location.max(0) as u32,
                                    locations: m.locations.max(0) as u32,
                                })
                            })
                            .collect(),
                    );
                    eprintln!(
                        "resumed shard {shard_id} at t={} with {} of {count} craft",
                        shard.now_t,
                        count - refused.len(),
                    );
                }
                None => eprintln!("shard {shard_id} has no saved state; starting a new world"),
            }
            // After the clock is adopted, because the acknowledgement window is stamped
            // against it: a shard that read these first would date every message it had ever
            // been told to the instant before it knew what time it was.
            server.resume_conversations().await?;
            Some(client)
        }
        None => {
            eprintln!("WARNING: no --db, so nothing here survives this process. Every craft and");
            eprintln!("         every account's claim on one is gone when it stops.");
            None
        }
    };

    let mut wire = WebSocketServer::bind(&bind).await?;
    eprintln!("listening on {}", wire.local_addr);

    let mut ticker = tokio::time::interval(Duration::from_millis(TICK_MS as u64));
    // The tick is the clock. Falling behind must not make the server sprint to catch up,
    // because every skipped tick is a slice of coordinate time nothing was read in.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Interrupt and terminate both, because one is a keyboard and the other is `docker stop`,
    // and a world should survive being asked to stop politely by either.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut since_save = 0u32;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = interrupt.recv() => break,
            _ = terminate.recv() => break,
        }
        for ClientId(id) in wire.accepted() {
            eprintln!("connection {id} opened");
        }
        for client in wire.departed() {
            eprintln!("connection {} closed", client.0);
            server.disconnected(client);
        }
        server.tick(&mut wire).await?;

        since_save += 1;
        if since_save >= SAVE_EVERY_TICKS
            && let Some(client) = &store
        {
            since_save = 0;
            // A failed checkpoint is not a reason to stop the world. It is a reason to say so
            // every time, because a shard that has quietly stopped saving looks exactly like one
            // that is fine.
            if let Err(why) = checkpoint(client, shard_id, &mut server).await {
                eprintln!("ERROR: checkpoint failed: {why}");
            }
        }
    }

    if let Some(client) = &store {
        eprintln!("stopping; writing a last checkpoint");
        checkpoint(client, shard_id, &mut server).await?;
    }
    Ok(())
}

/// Ticks between checkpoints. Twenty seconds of real time at the design rate.
///
/// A crash loses at most this much, and what it loses is **orders**, not flight: every motive
/// is stamped in absolute coordinate time, so a stale checkpoint replayed forward puts a ship
/// exactly where it would have been. What does not survive is an order given in the gap.
const SAVE_EVERY_TICKS: u32 = 400;

async fn checkpoint(
    client: &tokio_postgres::Client,
    shard_id: i64,
    // `&mut` because a checkpoint drains the bookmarks written since the last one.
    server: &mut Server<Store>,
) -> Result<(), Box<dyn std::error::Error>> {
    let taken = server.checkpoint();
    lc_store::ships::save_ships(client, &taken.ships).await?;
    lc_store::ships::save_shard(client, shard_id, lc_store::ships::Shard {
        now_t: taken.now_t,
        next_ship: taken.next_ship,
    })
    .await?;
    // Only what changed since the last one. A page turn is a row and a reader turns a page a
    // minute; writing every account's whole shelf every twenty seconds would be writing nothing
    // new, forever.
    let marks: Vec<lc_store::reading::Bookmark> = server
        .library
        .take_dirty()
        .into_iter()
        .map(|(account, mark)| lc_store::reading::Bookmark {
            account,
            book: mark.book,
            spine: mark.spine as i32,
            char_offset: mark.char_offset as i32,
            location: mark.location as i32,
            locations: mark.locations as i32,
        })
        .collect();
    lc_store::reading::save(client, &marks).await?;
    Ok(())
}

/// Connect, and drive the connection in the background.
///
/// Not `lc_store::connect`, which reads the environment: a shard is told its database on the
/// command line beside everything else it is told.
async fn connect(url: &str) -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
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
///
/// **Name the service, not the site.** In a container deployment the public name resolves to
/// the host's own address, and reaching the host's published port from a container hairpins
/// through the NAT and hangs — so `http://lightcone-cdn:3101/...`, not
/// `https://cdn.example/...`. It is the same bytes either way, because it is the same
/// container; only the route differs. The timeouts below are what turn getting this wrong into
/// a process that fails rather than one that never finishes starting.
fn read_bytes(source: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let mut bytes = Vec::new();
        ureq::builder()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .get(source)
            .call()?
            .into_reader()
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    } else {
        Ok(std::fs::read(source)?)
    }
}
