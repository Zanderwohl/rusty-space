//! Writing events and deliveries, and the one read the core loop makes.

use tokio_postgres::{Client, Error};

use crate::id::EventId;

/// One change of world state.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: EventId,
    pub source_id: i64,
    /// Coordinate time, microseconds.
    pub t: i64,
    /// Global grid position, light-microseconds.
    pub g: [i64; 3],
    /// The system it happened in, and where in that system, when it happened inside one.
    pub system_id: Option<i64>,
    pub local: Option<Local>,
    pub kind: i16,
    /// Whatever the event carries, as JSON text. Text rather than a parsed document because
    /// this crate has no opinion about what an event means; the server does.
    pub payload: String,
}

/// The exact local-frame values. Stored beside the global grid rather than derived from it:
/// the grid resolves 150 meters and a system's own frame does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    /// Meters from the system origin.
    pub position_m: [f64; 3],
    /// Seconds from the system epoch.
    pub t_s: f64,
}

/// One event reaching one observer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delivery {
    pub observer_id: i64,
    /// Coordinate time the signal arrives, microseconds.
    pub arrive_t: i64,
    pub event_id: EventId,
    /// For gating against the receiver's noise floor on arrival.
    pub strength: f32,
}

/// Create whatever partitions the range needs, on every partitioned table.
///
/// Has to run ahead of any write into the range: a row with no partition is an error, not a
/// table that grows one. At the design rate a partition is four and a half real days, so this
/// belongs on a schedule rather than at install.
pub async fn ensure_partitions(client: &Client, from_t: i64, to_t: i64) -> Result<i32, Error> {
    let mut made = 0;
    for table in ["events", "deliveries", "lc_samples"] {
        let row = client
            .query_one("SELECT lc_ensure_partitions($1, $2, $3)", &[&table, &from_t, &to_t])
            .await?;
        made += row.get::<_, i32>(0);
    }
    Ok(made)
}

/// Append events. One statement, whatever the count.
///
/// `unnest` rather than a row at a time: the write path is the one place the store is asked to
/// keep up with a tick, and a round trip per event would make that the bottleneck instead of
/// the disk.
pub async fn insert_events(client: &Client, events: &[Event]) -> Result<u64, Error> {
    if events.is_empty() {
        return Ok(0);
    }
    let ids: Vec<i64> = events.iter().map(|e| e.id.get()).collect();
    let sources: Vec<i64> = events.iter().map(|e| e.source_id).collect();
    let times: Vec<i64> = events.iter().map(|e| e.t).collect();
    let gx: Vec<i64> = events.iter().map(|e| e.g[0]).collect();
    let gy: Vec<i64> = events.iter().map(|e| e.g[1]).collect();
    let gz: Vec<i64> = events.iter().map(|e| e.g[2]).collect();
    let systems: Vec<Option<i64>> = events.iter().map(|e| e.system_id).collect();
    let lx: Vec<Option<f64>> = events.iter().map(|e| e.local.map(|l| l.position_m[0])).collect();
    let ly: Vec<Option<f64>> = events.iter().map(|e| e.local.map(|l| l.position_m[1])).collect();
    let lz: Vec<Option<f64>> = events.iter().map(|e| e.local.map(|l| l.position_m[2])).collect();
    let lt: Vec<Option<f64>> = events.iter().map(|e| e.local.map(|l| l.t_s)).collect();
    let kinds: Vec<i16> = events.iter().map(|e| e.kind).collect();
    let payloads: Vec<&str> = events.iter().map(|e| e.payload.as_str()).collect();

    client
        .execute(
            // The payload goes over as `text[]` and is cast in the statement: there is no
            // conversion from a Rust string to `jsonb` in the wire protocol, and building one
            // would mean this crate parsing payloads it has no opinion about.
            "INSERT INTO events
                 (event_id, source_id, t, gx, gy, gz, system_id, lx, ly, lz, lt, kind, payload)
             SELECT event_id, source_id, t, gx, gy, gz, system_id, lx, ly, lz, lt, kind,
                    payload::jsonb
               FROM unnest(
                 $1::bigint[], $2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[],
                 $6::bigint[], $7::bigint[], $8::float8[], $9::float8[], $10::float8[],
                 $11::float8[], $12::smallint[], $13::text[])
                 AS u(event_id, source_id, t, gx, gy, gz, system_id, lx, ly, lz, lt, kind,
                      payload)",
            &[
                &ids, &sources, &times, &gx, &gy, &gz, &systems, &lx, &ly, &lz, &lt, &kinds,
                &payloads,
            ],
        )
        .await
}

/// Schedule deliveries. Written when the event is written, not worked out when it is read.
pub async fn insert_deliveries(client: &Client, rows: &[Delivery]) -> Result<u64, Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let observers: Vec<i64> = rows.iter().map(|d| d.observer_id).collect();
    let arrivals: Vec<i64> = rows.iter().map(|d| d.arrive_t).collect();
    let events: Vec<i64> = rows.iter().map(|d| d.event_id.get()).collect();
    let strengths: Vec<f32> = rows.iter().map(|d| d.strength).collect();
    client
        .execute(
            "INSERT INTO deliveries (observer_id, arrive_t, event_id, strength)
             SELECT * FROM unnest($1::bigint[], $2::bigint[], $3::bigint[], $4::real[])
             ON CONFLICT DO NOTHING",
            &[&observers, &arrivals, &events, &strengths],
        )
        .await
}

/// What the server tick reads: everything reaching one observer in `(after_t, until_t]`.
///
/// The whole of the game's core loop, and the reason it is O(log n) rather than O(events): the
/// primary key is `(observer_id, arrive_t, event_id)`, so this is one range scan down one
/// B-tree, already in the order it is wanted.
pub const DELIVERY_QUERY: &str = "SELECT observer_id, arrive_t, event_id, strength
       FROM deliveries
      WHERE observer_id = $1 AND arrive_t > $2 AND arrive_t <= $3
   ORDER BY arrive_t";

pub async fn deliveries_for(
    client: &Client,
    observer_id: i64,
    after_t: i64,
    until_t: i64,
) -> Result<Vec<Delivery>, Error> {
    let rows = client.query(DELIVERY_QUERY, &[&observer_id, &after_t, &until_t]).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(Delivery {
                observer_id: row.get(0),
                arrive_t: row.get(1),
                event_id: EventId::from_raw(row.get(2))?,
                strength: row.get(3),
            })
        })
        .collect())
}

/// The execution plan for [`DELIVERY_QUERY`], as text.
///
/// Public because the claim that the core loop is a range scan is one a test has to be able to
/// check against the planner rather than against the shape of the SQL. A planner is free to
/// disagree with an index, and if it ever does that is a fact about this system worth failing on.
pub async fn explain_delivery_query(
    client: &Client,
    observer_id: i64,
    after_t: i64,
    until_t: i64,
) -> Result<String, Error> {
    let rows = client
        .query(
            &format!("EXPLAIN (ANALYZE, BUFFERS) {DELIVERY_QUERY}"),
            &[&observer_id, &after_t, &until_t],
        )
        .await?;
    Ok(rows.iter().map(|row| row.get::<_, &str>(0)).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Minter;
    use crate::migrate;

    /// One span is thirty days of coordinate time. Tests that cross a boundary use it.
    const SPAN_US: i64 = 2_592_000_000_000;

    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    /// Each test owns a band of identifiers and clears it first, so the suite can run in
    /// parallel against one database and twice in a row against the same one.
    async fn clear(client: &Client, band: i64) {
        client
            .execute("DELETE FROM deliveries WHERE observer_id BETWEEN $1 AND $2", &[
                &band,
                &(band + 999),
            ])
            .await
            .unwrap();
        client
            .execute("DELETE FROM events WHERE source_id BETWEEN $1 AND $2", &[&band, &(band + 999)])
            .await
            .unwrap();
    }

    fn event(minter: &mut Minter, source_id: i64, t: i64) -> Event {
        Event {
            id: minter.mint(t).expect("an identifier"),
            source_id,
            t,
            g: [t / 7, -t / 11, t / 13],
            system_id: None,
            local: None,
            kind: 1,
            payload: format!("{{\"at\":{t}}}"),
        }
    }

    /// Written rows are still there for a connection that knows nothing about the one that
    /// wrote them, and they are still there across a partition boundary.
    ///
    /// A second connection rather than a restarted server: what a restart would prove is that
    /// the rows were committed rather than held in one session, and a fresh connection proves
    /// exactly that without needing to stop the database a test does not own.
    #[tokio::test]
    async fn events_survive_the_connection_that_wrote_them_and_a_partition_rollover() {
        let Some(client) = store().await else { return };
        let band = 10_000i64;
        clear(&client, band).await;

        // Either side of a partition boundary, and exactly on it.
        let boundary = SPAN_US * 3;
        let times = [boundary - SPAN_US / 2, boundary - 1, boundary, boundary + 1];
        ensure_partitions(&client, times[0], *times.last().unwrap()).await.unwrap();

        let mut minter = Minter::new(1).unwrap();
        let written: Vec<Event> = times.iter().map(|t| event(&mut minter, band, *t)).collect();
        assert_eq!(insert_events(&client, &written).await.unwrap(), times.len() as u64);
        drop(client);

        let Some(fresh) = store().await else { return };
        let rows = fresh
            .query("SELECT t, tableoid::regclass::text FROM events WHERE source_id = $1 ORDER BY t", &[&band])
            .await
            .unwrap();
        assert_eq!(rows.len(), times.len(), "an event went missing");
        let landed: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(landed, times);

        // And they really are in different partitions, which is what makes it a rollover.
        let partitions: std::collections::HashSet<String> =
            rows.iter().map(|r| r.get::<_, String>(1)).collect();
        assert_eq!(partitions.len(), 2, "the boundary did not split them: {partitions:?}");
    }

    /// Writing into a range with no partition is an error rather than a table that quietly
    /// grows one. The scheduling is the point: a store that created partitions on demand would
    /// hide a stalled maintenance job until the disk filled.
    #[tokio::test]
    async fn a_write_past_the_last_partition_is_refused() {
        let Some(client) = store().await else { return };
        let mut minter = Minter::new(2).unwrap();
        // Far enough out that no other test will have made it, and dropped first so that a
        // second run of the suite starts from the same place a first one did.
        let far = SPAN_US * 900;
        client.batch_execute("DROP TABLE IF EXISTS events_p900, deliveries_p900").await.unwrap();
        let refused = insert_events(&client, &[event(&mut minter, 10_900, far)]).await;
        assert!(refused.is_err(), "a row with no partition was accepted");

        ensure_partitions(&client, far, far).await.unwrap();
        assert_eq!(insert_events(&client, &[event(&mut minter, 10_900, far)]).await.unwrap(), 1);
        client.execute("DELETE FROM events WHERE source_id = $1", &[&10_900i64]).await.unwrap();
    }

    #[tokio::test]
    async fn ensuring_partitions_twice_makes_them_once() {
        let Some(client) = store().await else { return };
        let from = SPAN_US * 800;
        let to = from + SPAN_US * 2;
        ensure_partitions(&client, from, to).await.unwrap();
        assert_eq!(ensure_partitions(&client, from, to).await.unwrap(), 0, "made again");
        // Backwards is a caller's mistake, not an empty range.
        assert!(ensure_partitions(&client, to, from).await.is_err());
    }

    /// The acceptance criterion, checked against the planner rather than asserted.
    ///
    /// "A single B-tree range scan" is a claim about what Postgres chooses to do, and it is
    /// free to choose otherwise. The plan has to show an index scan, no sequential scan, and no
    /// sort — the ordering has to fall out of the index rather than be imposed on the result,
    /// because a sort would mean reading the whole range before yielding the first row.
    #[tokio::test]
    async fn the_delivery_lookup_is_an_index_range_scan_with_no_sort() {
        let Some(client) = store().await else { return };
        let band = 12_000i64;
        clear(&client, band).await;
        ensure_partitions(&client, 0, SPAN_US).await.unwrap();

        // Enough rows, and enough observers, that a sequential scan would be the wrong choice
        // and the planner would take it if the index were not usable.
        let mut rows = Vec::new();
        let mut minter = Minter::new(3).unwrap();
        for step in 0..20_000i64 {
            rows.push(Delivery {
                observer_id: band + step % 40,
                arrive_t: (step * 7_919) % SPAN_US,
                event_id: minter.mint(step * 1_000).unwrap(),
                strength: 1.0,
            });
        }
        insert_deliveries(&client, &rows).await.unwrap();
        // What autovacuum does in a running system, and the test cannot wait for. Until a
        // vacuum marks pages all-visible an index-only scan still has to check the heap for
        // each row, the planner costs it accordingly, and it picks a bitmap scan instead --
        // which is to say the hot path stays index-only only while autovacuum keeps up. That
        // is a deployment requirement, and this is where it is written down.
        client.batch_execute("VACUUM ANALYZE deliveries").await.unwrap();

        let plan = explain_delivery_query(&client, band + 7, 0, SPAN_US / 4).await.unwrap();
        assert!(plan.contains("Index Only Scan"), "the index did not cover it:\n{plan}");
        assert!(!plan.contains("Seq Scan"), "it scanned the table:\n{plan}");
        assert!(!plan.contains("Sort"), "the order was imposed, not read:\n{plan}");
        // One partition, not all of them: the range prunes before the scan starts.
        // ` on deliveries_p`, not `deliveries_p`: the plan names the index as well as the
        // table, so the bare prefix counts every partition twice.
        let scanned = plan.matches(" on deliveries_p").count();
        assert_eq!(scanned, 1, "the query touched {scanned} partitions:\n{plan}");

        // And the rows themselves come back in arrival order, which is what the scan is for.
        let got = deliveries_for(&client, band + 7, 0, SPAN_US / 4).await.unwrap();
        assert!(!got.is_empty(), "the query found nothing to plan for");
        assert!(got.windows(2).all(|w| w[0].arrive_t <= w[1].arrive_t), "out of order");
        assert!(got.iter().all(|d| d.observer_id == band + 7));
        clear(&client, band).await;
    }

    /// The window is half-open on purpose: a tick asks for what arrived since the last tick,
    /// and an inclusive lower bound would deliver the boundary twice.
    #[tokio::test]
    async fn the_delivery_window_excludes_where_the_last_one_ended() {
        let Some(client) = store().await else { return };
        let band = 13_000i64;
        clear(&client, band).await;
        ensure_partitions(&client, 0, SPAN_US).await.unwrap();

        let mut minter = Minter::new(4).unwrap();
        let rows: Vec<Delivery> = (0..5i64)
            .map(|step| Delivery {
                observer_id: band,
                arrive_t: step * 1_000_000,
                event_id: minter.mint(step * 1_000_000).unwrap(),
                strength: 0.5,
            })
            .collect();
        insert_deliveries(&client, &rows).await.unwrap();

        let first = deliveries_for(&client, band, -1, 2_000_000).await.unwrap();
        let second = deliveries_for(&client, band, 2_000_000, 4_000_000).await.unwrap();
        assert_eq!(first.len(), 3, "0, 1 and 2 megaseconds");
        assert_eq!(second.len(), 2, "3 and 4, and not 2 again");
        let overlap: Vec<_> =
            first.iter().filter(|a| second.iter().any(|b| b.event_id == a.event_id)).collect();
        assert!(overlap.is_empty(), "a tick delivered {} events twice", overlap.len());
        clear(&client, band).await;
    }

    /// Everything an event carries comes back as it went in, including the local frame that
    /// the global grid cannot reproduce.
    #[tokio::test]
    async fn an_event_round_trips_with_its_local_frame_intact() {
        let Some(client) = store().await else { return };
        let band = 14_000i64;
        clear(&client, band).await;
        ensure_partitions(&client, 0, SPAN_US).await.unwrap();

        let mut minter = Minter::new(5).unwrap();
        let mut written = event(&mut minter, band, 1_234_567);
        written.system_id = Some(77);
        written.local = Some(Local { position_m: [1.5e11, -2.5e10, 3.0], t_s: 1.234_567 });
        insert_events(&client, std::slice::from_ref(&written)).await.unwrap();

        let row = client
            .query_one(
                "SELECT event_id, t, gx, gy, gz, system_id, lx, ly, lz, lt, kind, payload::text
                   FROM events WHERE source_id = $1",
                &[&band],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>(0), written.id.get());
        assert_eq!(row.get::<_, i64>(1), written.t);
        assert_eq!([row.get::<_, i64>(2), row.get::<_, i64>(3), row.get::<_, i64>(4)], written.g);
        assert_eq!(row.get::<_, Option<i64>>(5), Some(77));
        let local = written.local.unwrap();
        // Exactly. A local frame that came back rounded would be the 150 meters of the grid
        // reintroduced by the storage that exists to avoid it.
        assert_eq!(row.get::<_, f64>(6), local.position_m[0]);
        assert_eq!(row.get::<_, f64>(7), local.position_m[1]);
        assert_eq!(row.get::<_, f64>(8), local.position_m[2]);
        assert_eq!(row.get::<_, f64>(9), local.t_s);
        assert!(row.get::<_, String>(11).contains("1234567"));
        clear(&client, band).await;
    }
}
