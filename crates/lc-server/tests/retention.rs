//! Retention as the shard runs it: the journal keeping partitions ahead of the clock, and a sweep
//! taking them away behind it.
//!
//! In a database of its own, emptied first: a sweep retires everything behind its clock, and the
//! shared one holds every other test's partitions. Skipped with no database, or no right to make
//! one.

use glam::DVec3;
use lc_proto::ShipId;
use lc_server::journal::{DESIGN_HOUR_US, Journal, PREPARE_AHEAD_US, Postgres, RETENTION};
use lc_server::world::{Event, Scheduled};
use lc_store::id::Minter;
use lc_store::retention::{Policy, sweep};
use lc_store::store::PARTITION_SPAN_US;
use tokio_postgres::Client;

/// Each test empties the database the others would be using.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn own_database() -> Option<Client> {
    let shared = lc_store::connect().await.ok()?;
    // An error when it is already there, which is fine.
    let _ = shared.batch_execute("CREATE DATABASE lc_store_retention").await;
    let mut config: tokio_postgres::Config = lc_store::connection_string().parse().ok()?;
    config.dbname("lc_store_retention");
    let (client, connection) = config.connect(tokio_postgres::NoTls).await.ok()?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public").await.ok()?;
    lc_store::migrate::apply(&client).await.expect("the schema applies");
    Some(client)
}

async fn live(client: &Client, table: &str) -> i64 {
    client
        .query_one("SELECT count(*) FROM pg_inherits WHERE inhparent = $1::text::regclass", &[&table])
        .await
        .unwrap()
        .get(0)
}

fn said(minter: &mut Minter, t: i64) -> Event {
    Event {
        id: minter.mint(t).unwrap().get(),
        source: ShipId(1),
        t,
        at: DVec3::ZERO,
        kind: 1,
        power_w: 1.0,
        payload: "{}".into(),
    }
}

fn heard(observer: i64, event: &Event, arrive_t: i64) -> Scheduled {
    Scheduled { observer: ShipId(observer), event: event.id, arrive_t, strength: 1.0 }
}

/// The acceptance test. A month at the design rate is 720 coordinate years, which without
/// retention left a partition of each table per span: 730 of them. With it the count stays
/// where the policy puts it, while every event something still needs is still there.
#[tokio::test]
async fn a_month_of_real_time_holds_a_bounded_number_of_partitions() {
    let _one = ONE_AT_A_TIME.lock().await;
    let Some(client) = own_database().await else { return };
    let mut journal = Postgres::with(client);
    let mut minter = Minter::new(1).unwrap();
    const HOURS: i64 = 30 * 24;
    const FAR_US: i64 = 4 * DESIGN_HOUR_US;
    const DEEP_US: i64 = 50 * DESIGN_HOUR_US;
    let start = DESIGN_HOUR_US;
    let mut peak = (0, 0);
    let mut now = start;

    for hour in 0..HOURS {
        now = start + hour * DESIGN_HOUR_US;
        journal.prepare(now, now + PREPARE_AHEAD_US).await.unwrap();
        // Said every hour: heard a light-second off and four light-years off, and the first of
        // them fifty light-years off, so its light is in flight long after it is past keeping.
        let event = said(&mut minter, now);
        let mut deliveries = vec![heard(2, &event, now + 1_000_000), heard(3, &event, now + FAR_US)];
        if hour == 0 {
            deliveries.push(heard(4, &event, now + DEEP_US));
        }
        journal.write(&[event], &deliveries).await.unwrap();

        let swept = sweep(journal.client(), now, RETENTION).await.unwrap();
        assert!(swept.len() <= RETENTION.most);
        peak.0 = peak.0.max(live(journal.client(), "events").await);
        peak.1 = peak.1.max(live(journal.client(), "deliveries").await);

        if hour == 30 {
            let landing = journal.due(ShipId(4), now, start + DEEP_US).await.unwrap();
            assert_eq!(landing.len(), 1, "an event went while its light was still in flight");
        }
    }

    // Partitions meeting a stretch of time, plus the one the deep arrival keeps apart from it.
    let bound = |stretch_us: i64| stretch_us / PARTITION_SPAN_US + 2 + 1;
    // Deliveries from the cutoff to the farthest arrival. Events from as far behind the cutoff as
    // a kept arrival can be about, to as far ahead as the tick prepares.
    let deliveries = bound(RETENTION.kept_us + FAR_US);
    let events = bound(FAR_US + RETENTION.kept_us + PREPARE_AHEAD_US);
    assert!(peak.0 <= events, "{} event partitions against {events}", peak.0);
    assert!(peak.1 <= deliveries, "{} delivery partitions against {deliveries}", peak.1);
    assert!(events < 40, "a bound of {events} is not one worth having");

    // A client back after a day away is still sent everything it missed, events and all.
    let missed = journal.due(ShipId(2), now - RETENTION.kept_us, now).await.unwrap();
    assert_eq!(missed.len(), 24);

    // And nothing was lost: what is not live is archived.
    let client = journal.client();
    let live_rows: i64 = client.query_one("SELECT count(*) FROM events", &[]).await.unwrap().get(0);
    let archived: i64 =
        client.query_one("SELECT count(*) FROM events_archive", &[]).await.unwrap().get(0);
    assert_eq!(live_rows + archived, HOURS);
    assert!(archived > HOURS / 2, "only {archived} archived");
    let standing: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_class
              WHERE relkind = 'r' AND relname ~ '^(events|deliveries)_[sp]'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(standing, live(client, "events").await + live(client, "deliveries").await,
        "archiving left tables behind");
}

/// A store far behind is caught up a sweep at a time, never more than one sweep's worth.
#[tokio::test]
async fn a_backlog_is_worked_off_a_sweep_at_a_time() {
    let _one = ONE_AT_A_TIME.lock().await;
    let Some(client) = own_database().await else { return };
    const SLOTS: i64 = 100;
    lc_store::store::ensure_partitions(&client, 0, SLOTS * PARTITION_SPAN_US - 1).await.unwrap();
    let now = SLOTS * PARTITION_SPAN_US;
    let policy = Policy { most: 16, ..RETENTION };
    let kept = (policy.kept_us + PARTITION_SPAN_US - 1) / PARTITION_SPAN_US;
    let due = 2 * (SLOTS - kept);

    let mut sweeps = 0;
    let mut retired = 0;
    loop {
        let swept = sweep(&client, now, policy).await.unwrap();
        if swept.is_empty() {
            break;
        }
        assert!(swept.len() <= policy.most);
        retired += swept.len() as i64;
        sweeps += 1;
    }
    assert_eq!(retired, due);
    assert_eq!(sweeps, (due + policy.most as i64 - 1) / policy.most as i64);
    assert_eq!(live(&client, "events").await, kept);
}

/// A store from before the span widened has thirty-day partitions, named `_p`, ahead of its
/// clock as well as behind it. A wide one would overlap them, so the slot is filled out at their
/// width, and the old ones are retired like any other.
#[tokio::test]
async fn a_store_from_before_the_wider_span_keeps_taking_writes() {
    let _one = ONE_AT_A_TIME.lock().await;
    let Some(client) = own_database().await else { return };
    const OLD_SPAN_US: i64 = 2_592_000_000_000;
    // What the old tick would have left: its own slot and two ahead, all inside the wide slot 1.
    for slot in 12..15 {
        for table in ["events", "deliveries"] {
            client
                .batch_execute(&format!(
                    "CREATE TABLE {table}_p{slot} PARTITION OF {table}
                         FOR VALUES FROM ({}) TO ({})",
                    slot * OLD_SPAN_US,
                    (slot + 1) * OLD_SPAN_US,
                ))
                .await
                .unwrap();
        }
    }

    let mut journal = Postgres::with(client);
    let mut minter = Minter::new(1).unwrap();
    let now = 13 * OLD_SPAN_US + 1;
    journal.prepare(now, now + PREPARE_AHEAD_US).await.unwrap();
    let late = PARTITION_SPAN_US * 2 + 1;
    for t in [now, PARTITION_SPAN_US * 2 - OLD_SPAN_US / 2, late] {
        let event = said(&mut minter, t);
        let arrival = heard(2, &event, t + 1);
        journal.write(&[event], &[arrival]).await.unwrap();
    }
    let names: Vec<String> = journal
        .client()
        .query(
            "SELECT c.relname::text FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
              WHERE i.inhparent = 'events'::regclass ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert!(names.contains(&"events_p23".to_string()), "the slot was not filled out: {names:?}");
    assert!(names.contains(&"events_s2".to_string()), "{names:?}");

    // Long after, every one of them goes, old and new alike.
    let later = late + RETENTION.kept_us + PARTITION_SPAN_US * 2;
    let swept = sweep(journal.client(), later, RETENTION).await.unwrap();
    assert!(swept.archived.iter().any(|p| p.child == "events_p13"), "{swept:?}");
    assert!(swept.archived.iter().any(|p| p.child == "events_s2"), "{swept:?}");
    assert!(swept.dropped.iter().any(|p| p.child == "deliveries_p13"), "{swept:?}");
}
