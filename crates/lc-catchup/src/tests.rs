use lc_proto::{Body, Spoken};
use lc_store::id::Minter;
use lc_store::retention::{attach, detach};
use lc_store::store::{self, Delivery, Event, PARTITION_SPAN_US as SPAN_US};

use super::*;

/// Slots, an observer and a shard each, as the store's tests keep: one test's cleanup must not drop
/// another's partitions, nor its identifiers collide.
const FIRST_SLOT: i64 = 640;
const FACTION: i64 = 31_900;

async fn store() -> Option<Client> {
    let client = lc_store::connect().await.ok()?;
    lc_store::migrate::apply(&client).await.expect("the schema applies");
    Some(client)
}

async fn clear(client: &Client, n: i64) {
    let (slot, observer) = (FIRST_SLOT + 2 * n, 31_000 + n);
    for slot in slot..slot + 2 {
        for table in ["events", "deliveries"] {
            let child = format!("{table}_s{slot}");
            let _ = attach(client, &child).await;
            let _ = client.batch_execute(&format!("DROP TABLE IF EXISTS {child} CASCADE")).await;
        }
    }
    let (lo, hi) = (slot * SPAN_US, (slot + 2) * SPAN_US);
    client.execute("DELETE FROM lc_archived_partitions WHERE from_t >= $1 AND from_t < $2", &[&lo, &hi]).await.unwrap();
    client.execute("DELETE FROM deliveries_archive WHERE observer_id = $1", &[&observer]).await.unwrap();
    client.execute("DELETE FROM events_archive WHERE t >= $1 AND t < $2", &[&lo, &hi]).await.unwrap();
    client.execute("DELETE FROM lc_faction_keys WHERE faction = $1", &[&(FACTION + n)]).await.unwrap();
    client.execute("DELETE FROM lc_factions WHERE faction = $1", &[&(FACTION + n)]).await.unwrap();
}

struct World {
    client: Client,
    minter: Minter,
    n: i64,
    base: i64,
    observer: ShipId,
}

impl World {
    async fn new(n: i64) -> Option<Self> {
        let client = store().await?;
        clear(&client, n).await;
        let base = (FIRST_SLOT + 2 * n) * SPAN_US;
        store::ensure_partitions(&client, base, base + 2 * SPAN_US - 1).await.unwrap();
        Some(Self { client, minter: Minter::new(31 + n as u64).unwrap(), n, base, observer: ShipId(31_000 + n) })
    }

    fn slot(&self) -> i64 {
        FIRST_SLOT + 2 * self.n
    }

    async fn clear(&self) {
        clear(&self.client, self.n).await;
    }

    /// An event at `t`, landing on the observer at `arrive_t` from along `+x`.
    async fn lands(&mut self, t: i64, arrive_t: i64, strength: f32, kind: i16, payload: &str) -> i64 {
        let id = self.minter.mint(t).unwrap();
        let event = Event { id, source_id: 7, t, g: [0, 0, 0], system_id: None, local: None, kind, payload: payload.into() };
        store::insert_events(&self.client, &[event]).await.unwrap();
        let delivery = Delivery { observer_id: self.observer.0, arrive_t, event_id: id, strength, direction: Some([1.0, 0.0, 0.0]) };
        store::insert_deliveries(&self.client, &[delivery]).await.unwrap();
        id.get()
    }

    /// The first slot's partitions, both tables, moved out of the live ones as retention moves them.
    async fn archive_the_first(&self) {
        for table in ["events", "deliveries"] {
            assert!(detach(&self.client, table, &format!("{table}_s{}", self.slot())).await.unwrap(), "{table}");
        }
    }

    async fn read(&self, after_t: i64, until_t: i64, now_t: i64, most: usize) -> Page {
        page(&self.client, self.observer, 1.0e-3, (after_t, until_t), now_t, most).await.unwrap()
    }
}

fn ids(page: &Page) -> Vec<i64> {
    page.sightings.iter().map(|s| s.get().event_id).collect()
}

/// What landed while the pilot was away is told from the archive and the live tables alike, oldest
/// first, from the direction it was solved to come from, and whole.
#[tokio::test]
async fn what_landed_is_told_from_the_archive_and_the_live_tables_alike() {
    let Some(mut world) = World::new(0).await else { return };
    let base = world.base;
    let old = world.lands(base, base + 10, 1.0, 1, "{\"a\":1}").await;
    let older = world.lands(base, base + 5, 1.0, 1, "{}").await;
    let recent = world.lands(base + SPAN_US, base + SPAN_US + 5, 1.0, 1, "{}").await;
    world.archive_the_first().await;
    let now = base + 2 * SPAN_US;

    let page = world.read(base - 1, now, now, PAGE).await;
    assert_eq!(ids(&page), vec![older, old, recent]);
    assert!(page.is_done(now));
    let told = page.sightings[1].get();
    assert_eq!((told.arrive_t, told.emitted_t, told.source_id), (base + 10, base, 7));
    assert_eq!(told.direction, [1.0, 0.0, 0.0]);
    // Stored as `jsonb`, which keeps the document and not its spacing, as the tick reads it too.
    assert_eq!(serde_json::from_str::<serde_json::Value>(&told.payload).unwrap(), serde_json::json!({ "a": 1 }));
    world.clear().await;
}

/// The tick's gate, here too: nothing still on its way, and nothing under the receiver's floor.
#[tokio::test]
async fn nothing_withheld_is_told() {
    let Some(mut world) = World::new(1).await else { return };
    let base = world.base;
    let heard = world.lands(base, base + 10, 1.0, 1, "{}").await;
    world.lands(base, base + 20, 1.0e-6, 1, "{}").await;
    world.lands(base, base + SPAN_US + 30, 1.0, 1, "{}").await;
    world.archive_the_first().await;
    let now = base + SPAN_US;

    let page = world.read(base - 1, base + 2 * SPAN_US, now, PAGE).await;
    assert_eq!(ids(&page), vec![heard], "below the floor, or not yet here");
    assert_eq!(page.through_t, now, "and read no further than now");
    world.clear().await;
}

/// A message under a faction key reads only to a craft that held the key when it landed, by what
/// the store says, not by anything the shard has in memory.
#[tokio::test]
async fn a_sealed_message_reads_only_if_its_key_was_held_when_it_landed() {
    let Some(mut world) = World::new(2).await else { return };
    let base = world.base;
    let faction = FACTION + world.n;
    let said = format!("{{\"to\":null,\"sealed\":false,\"under\":{faction},\"body\":{{\"Text\":\"hello\"}}}}");
    let before = world.lands(base, base + 10, 1.0, lc_proto::kind::MESSAGE, &said).await;
    let after = world.lands(base, base + 30, 1.0, lc_proto::kind::MESSAGE, &said).await;
    world
        .client
        .execute("INSERT INTO lc_factions (faction, founder, name, lineage, founded_t) VALUES ($1, 1, 'Ring', '{}', 0)", &[&faction])
        .await
        .unwrap();
    world
        .client
        .execute("INSERT INTO lc_faction_keys (holder, faction, learned_t) VALUES ($1, $2, $3)", &[&world.observer.0, &faction, &(base + 20)])
        .await
        .unwrap();
    world.archive_the_first().await;

    let page = world.read(base - 1, base + SPAN_US, base + SPAN_US, PAGE).await;
    let body = |id: i64| {
        let told = page.sightings.iter().find(|s| s.get().event_id == id).unwrap().get();
        serde_json::from_str::<Spoken>(&told.payload).unwrap().body
    };
    assert_eq!(body(before), Body::Unreadable, "the key came after it");
    assert_eq!(body(after), Body::Text("hello".into()));
    world.clear().await;
}

/// A page ends between two instants, never inside one, and the next begins where it ended: read
/// page by page, everything is told once.
#[tokio::test]
async fn pages_split_between_instants_and_tell_everything_once() {
    let Some(mut world) = World::new(3).await else { return };
    let base = world.base;
    let mut all = Vec::new();
    for (k, arrive_t) in [10, 20, 20, 20, 30, 40, 40, 50].into_iter().enumerate() {
        all.push((arrive_t, world.lands(base + k as i64, base + arrive_t, 1.0, 1, "{}").await));
    }
    world.archive_the_first().await;
    let now = base + SPAN_US;

    let mut told = Vec::new();
    let mut after_t = base;
    let mut pages = 0;
    loop {
        let page = world.read(after_t, now, now, 2).await;
        assert!(page.through_t > after_t, "a page that ends where it began never ends");
        for s in &page.sightings {
            assert!(s.get().arrive_t <= page.through_t);
        }
        told.extend(ids(&page));
        after_t = page.through_t;
        pages += 1;
        if page.is_done(now) {
            break;
        }
    }
    all.sort();
    let mut expected: Vec<i64> = all.into_iter().map(|(_, id)| id).collect();
    expected.sort();
    told.sort();
    assert_eq!(told, expected);
    assert!(pages >= 4, "{pages} pages of two");
    world.clear().await;
}
