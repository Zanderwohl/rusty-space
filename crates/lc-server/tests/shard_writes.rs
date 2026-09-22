//! Orders through the real store.
//!
//! The server's own tests run against [`lc_server::journal::Memory`], which takes any row it is
//! given — so a payload the `jsonb` column will not parse, or an arrival with no partition,
//! passes every one of them and fails only against Postgres. It failed there loudly: the write
//! error came back through `tick` and out of the shard's `main`, so one order ended the process
//! and every connected client saw its socket close.
//!
//! Skipped with no database, the way every other store test is.

use glam::DVec3;
use lc_proto::{ClientId, Inbound, Intent, Order, Outbound, ShipId};
use lc_server::journal::{Postgres, Store};
use lc_server::server::Server;
use lc_server::transport::Loopback;
use lc_server::world::still;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::sky::{AuthoredStars, StarProvider};
use lc_world::system::LocalSystem;
use tokio_postgres::Client;

/// Light-microseconds in a light-year, which is what a craft's position is given in.
const LIGHT_US_PER_LY: f64 = lc_world::motion::LIGHT_US_PER_LY;

/// Each test owns a band of ship identifiers and a shard number of its own, and clears what it
/// wrote last time: the suite runs in parallel against one database and twice in a row against
/// the same one, and a shard number is what keeps two of these from minting the same event.
async fn shard(band: i64, shard_id: u64) -> Option<Server<Store>> {
    let client = lc_store::connect().await.ok()?;
    lc_store::migrate::apply(&client).await.expect("the schema applies");
    clear(&client, band).await;
    Some(Server::new(Store::Durable(Postgres::with(client)), 0, shard_id))
}

async fn clear(client: &Client, band: i64) {
    let range: [&(dyn tokio_postgres::types::ToSql + Sync); 2] = [&band, &(band + 999)];
    for statement in [
        "DELETE FROM deliveries WHERE observer_id BETWEEN $1 AND $2",
        "DELETE FROM events WHERE source_id BETWEEN $1 AND $2",
    ] {
        client.execute(statement, &range).await.expect("the band clears");
    }
}

fn acted(wire: &Loopback, client: ClientId) -> bool {
    wire.peek(client).iter().any(|m| matches!(m, Outbound::Accepted { .. }))
}

/// A burn's payload used to be built with `format!` from a `DVec3`, whose `Debug` writes
/// `DVec3(x, y, z)` — not JSON, and the column is `jsonb`.
#[tokio::test]
async fn a_burn_is_journalled() {
    let band = 9_100_000;
    let Some(mut server) = shard(band, 901).await else { return };
    let mut wire = Loopback::new();
    server.admit(ClientId(1), still(ShipId(band), DVec3::ZERO), 0.0);
    server.tick(&mut wire).await.expect("an empty tick");

    wire.client_says(ClientId(1), Inbound::Act(Intent {
        ship_id: ShipId(band),
        order: Order::Burn { beta: [1.0e-4, 0.0, 0.0] },
        issued_at_client_t: 0,
    }));
    server.tick(&mut wire).await.expect("the burn is written");
    assert!(acted(&wire, ClientId(1)), "{:?}", wire.peek(ClientId(1)));
}

/// **Light takes years between stars, and a delivery is stamped when it lands.** The partitions
/// were made for a window either side of now, so the first thing said within earshot of another
/// system had nowhere to go.
#[tokio::test]
async fn an_event_reaching_another_system_is_journalled() {
    let band = 9_200_000;
    let Some(mut server) = shard(band, 902).await else { return };
    let mut wire = Loopback::new();
    server.admit(ClientId(1), still(ShipId(band), DVec3::ZERO), 0.0);
    let far = DVec3::new(4.0 * LIGHT_US_PER_LY, 0.0, 0.0);
    server.admit(ClientId(2), still(ShipId(band + 1), far), 0.0);
    server.tick(&mut wire).await.expect("an empty tick");

    wire.client_says(ClientId(1), Inbound::Act(Intent {
        ship_id: ShipId(band),
        order: Order::Transmit { power_w: 1.0e12 },
        issued_at_client_t: 0,
    }));
    server.tick(&mut wire).await.expect("the delivery is written");
    assert!(acted(&wire, ClientId(1)), "{:?}", wire.peek(ClientId(1)));
}

/// **A restart is not a new world.** The clock comes back from a checkpoint that may be behind
/// the last event written, and `(event_id, t)` is a primary key: a shard that minted from zero
/// again collided with itself on the first order anyone gave.
#[tokio::test]
async fn a_shard_resuming_does_not_mint_an_identifier_it_has_used() {
    let band = 9_300_000;
    let Some(mut server) = shard(band, 903).await else { return };
    let order = || {
        Inbound::Act(Intent {
            ship_id: ShipId(band),
            order: Order::Transmit { power_w: 1.0e12 },
            issued_at_client_t: 0,
        })
    };
    let mut wire = Loopback::new();
    server.admit(ClientId(1), still(ShipId(band), DVec3::ZERO), 0.0);
    wire.client_says(ClientId(1), order());
    server.tick(&mut wire).await.expect("the first event is written");

    // The same world, come back up on the same clock: what a stop with no last checkpoint
    // leaves behind.
    let client = lc_store::connect().await.expect("a database");
    let last = lc_store::store::last_event_id(&client).await.expect("a query").expect("an event");
    let mut resumed = Server::new(Store::Durable(Postgres::with(client)), 0, 903);
    resumed.resume_ids(last);
    let mut wire = Loopback::new();
    resumed.admit(ClientId(1), still(ShipId(band), DVec3::ZERO), 0.0);
    wire.client_says(ClientId(1), order());
    resumed.tick(&mut wire).await.expect("the resumed shard writes its own event");
    assert!(acted(&wire, ClientId(1)), "{:?}", wire.peek(ClientId(1)));
}

/// The order a client actually sends to fly somewhere, all the way through the store.
#[tokio::test]
async fn a_course_is_journalled_from_the_order_to_the_arrival() {
    let band = 9_400_000;
    let Some(mut server) = shard(band, 904).await else { return };
    let mut wire = Loopback::new();
    let star = AuthoredStars::sample().stars().first().cloned().expect("a star");
    let system = std::sync::Arc::new(LocalSystem::for_star(&star).expect("a system"));
    let body = system
        .inventory()
        .iter()
        .find_map(|entry| match &entry.target {
            lc_world::navigation::Target::Body(name) => Some(name.clone()),
            _ => None,
        })
        .expect("a body to orbit");
    let mut craft = Craft::at(CraftId(band), Kind::Ship, star.position_ly);
    craft.enter(Some(system), 0.0);
    server.admit(ClientId(1), craft, 0.0);
    server.tick(&mut wire).await.expect("an empty tick");

    wire.client_says(ClientId(1), Inbound::Act(Intent {
        ship_id: ShipId(band),
        order: Order::SetCourse {
            course: lc_proto::Course::Orbit {
                body,
                altitude_radii: 2.0,
                plane: lc_proto::Plane::Equatorial,
            },
            accel_g: 5.0,
            max_beta: 0.999,
        },
        issued_at_client_t: 0,
    }));
    // Past the burn, the flip and the arrival, each of which is journalled on its own.
    for _ in 0..40 {
        server.tick(&mut wire).await.expect("the shard keeps writing");
    }
    assert!(acted(&wire, ClientId(1)), "{:?}", wire.peek(ClientId(1)));
}
