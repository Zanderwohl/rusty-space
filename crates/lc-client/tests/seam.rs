//! The client and the server, over a real socket.
//!
//! Everything either side does is covered by its own tests against a loopback that cannot
//! misframe, mis-encode or disconnect. This is the one that runs the bytes through a kernel —
//! and it is the only test in the repository where `lc-client` and `lc-server` are both
//! present, which is deliberate: they share `lc-proto` and nothing else, and a test that needed
//! more than that would be evidence the seam had grown.

use std::time::Duration;

use lc_client::link::{Link, Status, WebSocketLink};
use lc_proto::{Inbound, Intent, Order, Outbound, PROTOCOL_VERSION};
use lc_server::journal::Memory;
use lc_server::server::{Server, TICK_MS};
use lc_server::websocket::WebSocketServer;

/// Longest any step below waits before failing. Generous: it is a local socket, and the point
/// of the bound is that a broken seam fails rather than hangs.
const PATIENCE: Duration = Duration::from_secs(5);

/// The sky both ends hold. The authored sample rather than a packed catalogue, because what
/// matters here is that they hold the *same* one and that a star id means one thing across the
/// wire — not which stars they are.
fn a_sky() -> Vec<lc_world::sky::CatalogueStar> {
    use lc_world::sky::StarProvider;
    lc_world::sky::AuthoredStars::sample().stars().to_vec()
}

/// Start a shard on a port the operating system picks, and tick it until the test ends.
async fn shard(open: bool) -> String {
    let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
    let address = format!("ws://{}", wire.local_addr);

    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit_without_tickets(open);
    server.load_world(lc_server::world::World::new(a_sky()));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(TICK_MS as u64));
        loop {
            ticker.tick().await;
            for client in wire.departed() {
                server.disconnected(client);
            }
            server.tick(&mut wire).await.expect("the journal");
        }
    });
    address
}

/// Wait for the socket to open, then greet.
async fn greet(link: &mut WebSocketLink, protocol: u32, ticket: &str) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        match link.status() {
            Status::Open => break,
            Status::Closed(why) => panic!("the socket closed before it opened: {why}"),
            Status::Connecting => {
                assert!(tokio::time::Instant::now() < deadline, "never connected");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    link.send(Inbound::Hello {
        protocol,
        ticket: ticket.to_owned(),
    });
}

/// The next thing the server says, or a failure that names what it was waiting for.
async fn hear(link: &mut WebSocketLink, what: &str) -> Outbound {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        // A ship's account follows its welcome and every order, and none of these tests is
        // about energy.
        let heard = link.poll().into_iter().find(|m| !matches!(m, Outbound::Fitted { .. }));
        if let Some(message) = heard {
            return message;
        }
        if let Status::Closed(why) = link.status() {
            panic!("the socket closed while waiting for {what}: {why}");
        }
        assert!(tokio::time::Instant::now() < deadline, "no {what} arrived");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The whole point: a client connects to a server it did not share a process with, says who it
/// is, and is given a ship.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_connects_to_a_server_and_is_welcomed() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let said = hear(&mut link, "a welcome").await;

    let Outbound::Welcome {
        protocol,
        ship_id,
        name,
        ..
    } = said
    else {
        panic!("the first thing a server says must be a welcome, not {said:?}");
    };
    assert_eq!(protocol, PROTOCOL_VERSION);
    assert!(ship_id.0 > 0, "no ship");
    assert!(!name.is_empty(), "no name");
}

/// An order goes out and the server's answer comes back, over the socket. The answer carries
/// the order **as applied** — here the timestamp, clamped from a client claiming to have acted
/// at the end of time.
#[tokio::test(flavor = "multi_thread")]
async fn an_order_is_answered_with_what_was_actually_done() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id, .. } = hear(&mut link, "a welcome").await else {
        panic!("no welcome");
    };

    link.send(Inbound::Act(Intent {
        ship_id,
        order: Order::Transmit { power_w: 1500.0 },
        issued_at_client_t: i64::MAX,
    }));

    let said = hear(&mut link, "an acceptance").await;
    let Outbound::Accepted { ship_id: whose, at_t, order, event_id } = said else {
        panic!("an order was not accepted: {said:?}");
    };
    assert_eq!(whose, ship_id);
    assert_eq!(order, Order::Transmit { power_w: 1500.0 });
    assert!(event_id > 0, "an accepted order names no event");
    assert_ne!(at_t, i64::MAX, "the timestamp was taken at face value");
    assert!(at_t > 0, "clamped to something before the world started: {at_t}");
}

/// And an order that cannot stand comes back refused rather than silently ignored.
#[tokio::test(flavor = "multi_thread")]
async fn an_impossible_order_is_answered_too() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id, .. } = hear(&mut link, "a welcome").await else {
        panic!("no welcome");
    };

    link.send(Inbound::Act(Intent {
        ship_id,
        order: Order::Transmit { power_w: -1.0 },
        issued_at_client_t: 0,
    }));

    let said = hear(&mut link, "a refusal").await;
    assert!(
        matches!(said, Outbound::Refused { .. }),
        "an impossible order was not refused: {said:?}",
    );
}

/// A crossing names a **star**, and the server answers with the acceleration it actually flew.
/// The id is only meaningful because both ends were given the same catalogue.
#[tokio::test(flavor = "multi_thread")]
async fn a_crossing_names_a_star_the_server_also_holds() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id, .. } = hear(&mut link, "a welcome").await else {
        panic!("no welcome");
    };

    // The last of the three authored stars, so this is not the one a ship starts at.
    let destination = a_sky().last().expect("a star").id.get();
    link.send(Inbound::Act(Intent {
        ship_id,
        // More than any craft can pull, so the answer has to differ from the request.
        order: Order::Cross { star: destination, accel_g: 1000.0, max_beta: 0.999 },
        issued_at_client_t: 0,
    }));

    let said = hear(&mut link, "an acceptance").await;
    let Outbound::Accepted { order: Order::Cross { star, accel_g, .. }, .. } = said else {
        panic!("a crossing was not accepted: {said:?}");
    };
    assert_eq!(star, destination, "it agreed to a different star");
    assert!(accel_g < 1000.0, "the acceleration was taken at face value");
    assert!(accel_g > 0.0, "it flew at nothing");
}

/// A star this shard does not hold is not somewhere anyone may fly to, whatever the client
/// believes it has. This is the whole reason the wire carries an id and not a position.
#[tokio::test(flavor = "multi_thread")]
async fn a_crossing_to_a_star_the_server_does_not_have_is_refused() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id, .. } = hear(&mut link, "a welcome").await else {
        panic!("no welcome");
    };

    link.send(Inbound::Act(Intent {
        ship_id,
        order: Order::Cross { star: 0xdead_beef_dead_beef, accel_g: 1.0, max_beta: 0.999 },
        issued_at_client_t: 0,
    }));

    let said = hear(&mut link, "a refusal").await;
    assert!(
        matches!(said, Outbound::Refused { .. }),
        "a made-up star was accepted: {said:?}",
    );
}

/// The server states its clock unprompted, which is what a client's own clock is corrected
/// against. Without it a client that drifts stays drifted forever.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_says_what_time_it_is_without_being_asked() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    assert!(matches!(hear(&mut link, "a welcome").await, Outbound::Welcome { .. }));

    // Nothing is sent from here: a clock statement is the server's own doing.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        if let Some(Outbound::Clock { now_t, .. }) = link
            .poll()
            .into_iter()
            .find(|m| matches!(m, Outbound::Clock { .. }))
        {
            assert!(now_t > 0, "the clock it stated was {now_t}");
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "the server never stated its clock");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// How long an order takes to come back, which is what a player feels when the interface waits
/// for the server rather than predicting.
///
/// The bound is deliberately loose — this runs on whatever a test machine is doing — but it is
/// far under the *seconds* a person would notice. A tick is 50 ms and the answer is sent on the
/// tick that reads the order, so anything in this range is the tick and not a stall.
#[tokio::test(flavor = "multi_thread")]
async fn an_order_is_answered_within_a_few_ticks() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id, .. } = hear(&mut link, "a welcome").await else {
        panic!("no welcome");
    };
    // Clear the welcome's trailing traffic so the measurement times the order alone.
    tokio::time::sleep(Duration::from_millis(120)).await;
    let _ = link.poll();

    let sent = std::time::Instant::now();
    link.send(Inbound::Act(Intent {
        ship_id,
        order: Order::Transmit { power_w: 1000.0 },
        issued_at_client_t: 0,
    }));

    let deadline = tokio::time::Instant::now() + PATIENCE;
    let waited = loop {
        if link.poll().into_iter().any(|m| matches!(m, Outbound::Accepted { .. })) {
            break sent.elapsed();
        }
        assert!(tokio::time::Instant::now() < deadline, "no answer at all");
        tokio::time::sleep(Duration::from_millis(1)).await;
    };

    println!("order answered in {:.0} ms ({} ms ticks)", waited.as_secs_f64() * 1e3, TICK_MS);
    assert!(
        waited < Duration::from_millis(TICK_MS as u64 * 8),
        "an order took {:.0} ms, which is a stall and not a tick",
        waited.as_secs_f64() * 1e3,
    );
}

/// The negotiation that exists so a stale client fails legibly instead of misreading bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_on_the_wrong_protocol_is_told_the_number() {
    let address = shard(true).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION + 1, "").await;
    let said = hear(&mut link, "a protocol refusal").await;

    assert_eq!(
        said,
        Outbound::WrongProtocol {
            server: PROTOCOL_VERSION
        },
        "a client from the future was welcomed",
    );
}

/// A server that was told to trust nobody trusts nobody, over a real socket as much as in
/// process. This is the default and the one that matters.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_admits_nobody_refuses_over_the_wire() {
    let address = shard(false).await;
    let mut link = WebSocketLink::connect(&address);

    greet(&mut link, PROTOCOL_VERSION, "not a ticket").await;
    let said = hear(&mut link, "a refusal").await;

    assert_eq!(said, Outbound::Unauthenticated);
}

/// Two clients, one server, two ships. The thing a single-process build could never show.
#[tokio::test(flavor = "multi_thread")]
async fn two_clients_reach_the_same_server_and_get_different_ships() {
    let address = shard(true).await;
    let mut one = WebSocketLink::connect(&address);
    let mut two = WebSocketLink::connect(&address);

    greet(&mut one, PROTOCOL_VERSION, "").await;
    greet(&mut two, PROTOCOL_VERSION, "").await;

    let first = hear(&mut one, "a welcome").await;
    let second = hear(&mut two, "a welcome").await;
    let (Outbound::Welcome { ship_id: a, .. }, Outbound::Welcome { ship_id: b, .. }) =
        (&first, &second)
    else {
        panic!("{first:?} / {second:?}");
    };
    assert_ne!(a, b, "two connections were given one ship");
}

/// The client half notices a server that is not there, rather than waiting on it forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_to_nothing_closes_with_a_reason() {
    // Port 1 on loopback: bindable only by root, and nothing is listening on it.
    let link = WebSocketLink::connect("ws://127.0.0.1:1");
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        match link.status() {
            Status::Closed(why) => {
                assert!(!why.is_empty(), "closed with nothing to show a person");
                return;
            }
            _ => {
                assert!(tokio::time::Instant::now() < deadline, "never gave up");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
}

/// A survey crosses the socket and lands in the other ship's knowledge.
///
/// The one test where a report goes all the way: a client builds one out of what it holds, the
/// bytes go through a kernel, the shard schedules it against the light cone, and the second
/// client folds it into a `Knowledge` that had never heard of the star. Two ships admitted at
/// the origin, so the crossing is a tick rather than a wait.
#[tokio::test(flavor = "multi_thread")]
async fn a_report_crosses_the_seam_and_is_learnt_at_the_far_end() {
    use lc_world::knowledge::{Bearing, Knowledge, Sighting, Witness};

    let address = shard(true).await;
    let mut sender = WebSocketLink::connect(&address);
    let mut receiver = WebSocketLink::connect(&address);
    greet(&mut sender, PROTOCOL_VERSION, "").await;
    greet(&mut receiver, PROTOCOL_VERSION, "").await;
    let Outbound::Welcome { ship_id: mine, .. } = hear(&mut sender, "a welcome").await else {
        panic!("no welcome");
    };
    let Outbound::Welcome {
        ship_id: theirs, ..
    } = hear(&mut receiver, "a welcome").await
    else {
        panic!("no welcome");
    };

    // What the sender has: six bearings to one star, from six places, which is a parallax.
    let star = lc_world::sky::StarId::synthesise("seam", 1);
    let mut held = Knowledge::new(Witness(mine.0 as u64));
    for k in 0..6u64 {
        let at = glam::DVec3::new(k as f64 * 0.2, 0.0, 0.0);
        held.sighted(
            star,
            Sighting {
                witness: Witness(mine.0 as u64),
                observed_s: k as f64,
                bearing: Bearing {
                    observer_ly: at,
                    toward: (glam::DVec3::new(0.0, 9.0, 0.0) - at).normalize(),
                    sigma_rad: 1e-9,
                },
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            },
        );
    }
    held.name_it(star, "Waystone", 6.0);
    let report = held.report(f64::NEG_INFINITY, 0.0);

    sender.send(Inbound::Act(Intent {
        ship_id: mine,
        order: Order::SendReport {
            to: Some(theirs),
            aim: lc_proto::Aim::Omni,
            secrecy: lc_proto::Secrecy::Open,
            report: serde_json::to_string(&report).expect("a report encodes"),
            idem: 77,
        },
        issued_at_client_t: 0,
    }));
    let said = hear(&mut sender, "an acceptance").await;
    assert!(matches!(said, Outbound::Accepted { .. }), "{said:?}");

    // The far end: wait for the sighting to arrive and fold it as the client does.
    let mut learnt = Knowledge::new(Witness(theirs.0 as u64));
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !learnt.knows(star) {
        for message in receiver.poll() {
            let Outbound::Sightings(cleared) = message else {
                continue;
            };
            for sighting in cleared.into_iter().map(|c| c.into_inner()) {
                if sighting.kind != lc_proto::kind::REPORT {
                    continue;
                }
                let reported: lc_proto::Reported =
                    serde_json::from_str(&sighting.payload).expect("a payload");
                let body = reported.body.expect("an open report is readable");
                let arrived: lc_world::knowledge::Report =
                    serde_json::from_str(&body).expect("a report decodes");
                learnt.receive(&arrived, sighting.arrive_t as f64 * 1e-6);
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "no report arrived");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let belief = learnt.belief(star).expect("it landed");
    assert_eq!(belief.hops, 1, "one hop: it came from over there");
    assert_eq!(belief.witnesses, 1, "and the witness is still the sender");
    assert!(
        belief.triangulated,
        "their bearings solve their parallax here"
    );
    assert_eq!(
        belief.name.as_ref().map(|n| n.name.as_str()),
        Some("Waystone"),
        "what they call it came with it",
    );
}
