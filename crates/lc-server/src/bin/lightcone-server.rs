//! A shard as a process: bind a socket, load a world, tick.
//!
//! Thin on purpose. Everything worth testing is in the library, which is why the tick loop
//! here is six lines and has no logic of its own.

use std::io::Read;
use std::time::Duration;

use lc_proto::ClientId;
use tokio::signal::unix::{SignalKind, signal};
use lc_server::journal::{Memory, Postgres, Store};
use lc_server::server::{Server, TICK_MS, TICKS_PER_SECOND};
use lc_server::ticket::Trusted;
use lc_server::websocket::WebSocketServer;
use lc_server::world::World;
use lc_world::sky::{AuthoredStars, StarProvider};

const USAGE: &str = "\
lightcone-server — one shard

  --bind <addr>       where to listen (default 127.0.0.1:8080)
  --audience <name>   the audience tickets must name (default shard-1)
  --jwks <url|path>   the broker's published keys, fetched at boot
  --sky <url|path>    the packed catalog this shard is authoritative over
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
<cdn>/game/<build>/assets/sky/catalog.lcsky. Both ends place craft into systems by position
against the same shell radius, so two different catalogs is two different answers to which
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
        // wrong for every other case: a client loading the real catalog will disagree with
        // this about which system it is in, and neither end will say so.
        None => {
            eprintln!("WARNING: no --sky, so this shard's world is the authored sample. A client");
            eprintln!("         with a real catalog will not agree with it about anything.");
            AuthoredStars::sample().stars().to_vec()
        }
    };
    // Shared with the administration surface below.
    let catalog = std::sync::Arc::new(stars);
    server.load_world(World::from_shared(catalog.clone()));

    // Refused rather than silently skipped: a console pointed at a shard that quietly declined
    // to listen is a card reading "unavailable" with nothing to explain it.
    if let Some(addr) = &admin_bind {
        let (Some(url), Some(keys)) = (db.as_ref(), admin_keys) else {
            return Err("--admin-bind needs both --db and --jwks".into());
        };
        let api = lc_server::admin::Api::new(
            std::sync::Arc::new(connect(url).await?),
            catalog.clone(),
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

    // The shelf. A shard with no catalog runs without one and says nothing about a library;
    // a shard with a catalog and no base would send files hanging off nothing, so both are
    // required together or neither is taken.
    match (
        after("--library").or_else(|| std::env::var("LC_LIBRARY").ok()),
        after("--shelf-base").or_else(|| std::env::var("LC_SHELF_BASE").ok()),
    ) {
        (Some(path), Some(base)) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read the catalog at {path}: {e}"))?;
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
    let mut store = match db {
        Some(url) => {
            let client = connect(&url).await?;
            // Before anything can be journalled. The clock below comes back from the last
            // checkpoint, which may be behind the last event written, and an identifier minted
            // from a clock that has gone back is one the store already has.
            if let Some(last) = lc_store::store::last_event_id(&client).await? {
                server.resume_ids(last);
            }
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
                    // After the craft, whose instruments `adopt` has just put back: this fills in
                    // what each of them knew.
                    let files = lc_store::knowledge::load_files(&client).await?;
                    let samples = lc_store::knowledge::load_samples(&client).await?;
                    // Counted rather than listed: a format bump refuses every file a craft
                    // holds, and thousands of identical lines bury the rest of the boot.
                    let mut problems: std::collections::BTreeMap<String, usize> =
                        std::collections::BTreeMap::new();
                    for problem in server.adopt_knowledge(&files, &samples) {
                        *problems.entry(problem).or_default() += 1;
                    }
                    for (problem, count) in problems {
                        eprintln!("WARNING: knowledge not restored ({count}x): {problem}");
                    }
                    eprintln!("resumed {} files and {} samples of knowledge", files.len(), samples.len());
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
                    let presets = lc_store::presets::load(&client).await?;
                    if !presets.is_empty() {
                        eprintln!("resumed {} presets", presets.len());
                    }
                    for problem in server.presets.adopt(presets) {
                        eprintln!("WARNING: preset not restored: {problem}");
                    }
                    eprintln!(
                        "resumed shard {shard_id} at t={} with {} of {count} craft",
                        shard.now_t,
                        count - refused.len(),
                    );
                }
                None => eprintln!("shard {shard_id} has no saved state; starting a new world"),
            }
            // After the clock is adopted, because the acknowledgment window is stamped
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
    // A checkpoint being written. The connection is inside it until it finishes.
    let mut saving: Option<tokio::task::JoinHandle<Written>> = None;
    // Consecutive ticks whose journal write failed, so a store that has gone away is reported
    // rather than repeated twenty times a second.
    let mut failing = 0u32;
    let mut overruns = lc_server::timing::Overruns::new(
        Duration::from_millis(TICK_MS as u64),
        TICKS_PER_SECOND,
    );

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
        // **Not `?`.** A tick the store would not take is one tick's events lost; ending the
        // process here lost every connected player instead, socket closed with no close frame,
        // and came back on the next order anyone gave.
        match server.tick(&mut wire).await {
            Ok(()) => failing = 0,
            Err(why) => {
                if failing.is_multiple_of(COMPLAIN_EVERY_TICKS) {
                    eprintln!("ERROR: the tick could not be journalled: {why}");
                }
                failing += 1;
            }
        }
        if let Some(line) = overruns.note(server.last_tick()) {
            eprintln!("WARNING: {line}");
        }

        // A finished write hands the connection back, and on failure what it was writing.
        if saving.as_ref().is_some_and(|task| task.is_finished())
            && let Some(task) = saving.take()
        {
            store = Some(finish_checkpoint(task.await?, &mut server));
        }

        since_save += 1;
        if since_save >= SAVE_EVERY_TICKS
            && let Some(client) = store.take()
        {
            since_save = 0;
            // The snapshot is taken here, on the tick, so it is the shard at one tick; only the
            // writing is moved off it. It used to hold the tick for the whole transaction.
            let taken = take(&mut server);
            saving = Some(tokio::spawn(write(client, shard_id, taken)));
        }
    }

    if let Some(task) = saving.take() {
        store = Some(finish_checkpoint(task.await?, &mut server));
    }
    if let Some(client) = store.take() {
        eprintln!("stopping; writing a last checkpoint");
        let (_, taken, written) = write(client, shard_id, take(&mut server)).await;
        if let Err(why) = written {
            give_back(&mut server, taken);
            return Err(why.into());
        }
    }
    Ok(())
}

/// Ticks between checkpoints. Twenty seconds of real time at the design rate.
///
/// A crash loses at most this much, and what it loses is **orders**, not flight: every motive
/// is stamped in absolute coordinate time, so a stale checkpoint replayed forward puts a ship
/// exactly where it would have been. What does not survive is an order given in the gap.
const SAVE_EVERY_TICKS: u32 = 400;

/// Ticks between repeats of the same journal complaint. Twenty seconds of real time: often
/// enough that an operator sees a store outage going on, rarely enough to read.
const COMPLAIN_EVERY_TICKS: u32 = 400;

/// A checkpoint as taken, for [`write`] to write and [`give_back`] to return if it could not.
struct Taken {
    checkpoint: lc_server::persist::Checkpoint,
    /// What changed since the last checkpoint: files touched, samples taken and consumed.
    /// Drained, and handed back if the write fails, so nothing is lost to a failure but time: a
    /// file that never changes again would otherwise never be written.
    remembered: lc_server::archive::Remembered,
    marks: Vec<(String, lc_proto::Bookmark)>,
    presets: Vec<lc_server::presets::Change>,
}

fn take(server: &mut Server<Store>) -> Taken {
    Taken {
        checkpoint: server.checkpoint(),
        remembered: server.take_knowledge(),
        marks: server.library.take_dirty(),
        presets: server.presets.take_dirty(),
    }
}

fn give_back(server: &mut Server<Store>, taken: Taken) {
    server.untake_knowledge(taken.remembered);
    server.library.redirty(&taken.marks);
    server.presets.redirty(&taken.presets);
}

/// A checkpoint write's outcome, with the connection and what was being written handed back.
type Written = (tokio_postgres::Client, Taken, Result<(), tokio_postgres::Error>);

/// Owns the connection while it writes, so it can run beside the tick; hands both back.
async fn write(mut client: tokio_postgres::Client, shard_id: i64, taken: Taken) -> Written {
    let started = std::time::Instant::now();
    let rows: Vec<lc_store::reading::Bookmark> = taken
        .marks
        .iter()
        .map(|(account, mark)| lc_store::reading::Bookmark {
            account: account.clone(),
            book: mark.book.clone(),
            spine: mark.spine as i32,
            char_offset: mark.char_offset as i32,
            location: mark.location as i32,
            locations: mark.locations as i32,
        })
        .collect();
    let (presets, forgotten) = lc_server::presets::rows(&taken.presets);
    // One transaction: a checkpoint is the shard's state at one tick, and half of one — samples
    // written and their deletions not, say — would be reloaded as something that never was.
    let written = async {
        let transaction = client.transaction().await?;
        lc_store::ships::save_ships(&transaction, &taken.checkpoint.ships).await?;
        // The partitions the samples land in exist, because the journal keeps them ready ahead of
        // the clock every tick and nothing is learned in the future.
        lc_store::knowledge::save_files(&transaction, &taken.remembered.files).await?;
        lc_store::knowledge::save_samples(&transaction, &taken.remembered.samples).await?;
        lc_store::knowledge::delete_samples(&transaction, &taken.remembered.discarded).await?;
        lc_store::ships::save_shard(&transaction, shard_id, lc_store::ships::Shard {
            now_t: taken.checkpoint.now_t,
            next_ship: taken.checkpoint.next_ship,
        })
        .await?;
        lc_store::reading::save(&transaction, &rows).await?;
        lc_store::presets::save(&transaction, &presets).await?;
        for (account, name) in &forgotten {
            lc_store::presets::delete(&transaction, account, name).await?;
        }
        transaction.commit().await
    }
    .await;
    if written.is_ok() {
        eprintln!("checkpoint written in {:.0} ms", started.elapsed().as_secs_f64() * 1.0e3);
    }
    (client, taken, written)
}

/// A failed checkpoint is not a reason to stop the world. It is a reason to say so every time,
/// because a shard that has quietly stopped saving looks exactly like one that is fine.
fn finish_checkpoint((client, taken, written): Written, server: &mut Server<Store>) -> tokio_postgres::Client {
    if let Err(why) = written {
        eprintln!("ERROR: checkpoint failed: {why}");
        give_back(server, taken);
    }
    client
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
/// build is what makes "both ends hold the same catalog" a fact rather than a convention
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
