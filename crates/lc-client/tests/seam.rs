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

/// Start a shard on a port the operating system picks, and tick it until the test ends.
async fn shard(open: bool) -> String {
    let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
    let address = format!("ws://{}", wire.local_addr);

    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit_without_tickets(open);
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
        if let Some(message) = link.poll().into_iter().next() {
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
