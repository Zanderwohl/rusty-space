//! Taking old partitions out of the live tables.
//!
//! Three tiers, and the middle one is the reason this is not a date comparison. An event is
//! unreachable once its light cone has swept past every possible observer — a fixed lag behind
//! `t`, the light-crossing time of the playable volume. But light is in flight for years, so an
//! event long past that lag can still be the thing a scheduled delivery is *about*, and taking
//! it away would leave a tick holding an identifier with nothing behind it.
//!
//! And the third tier is archive, not drop: replays and the god view read old events, so it has
//! to be reversible. Deliveries are dropped. See `lightcone/docs/02-event-store.md`.

use tokio_postgres::{Client, Error};

/// One partition of one table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    pub child: String,
    pub parent: String,
    /// Coordinate microseconds, half-open as Postgres declares it.
    pub from_t: i64,
    pub to_t: i64,
}

/// Every live partition of a table, ordered outward in time.
pub async fn partitions(client: &Client, table: &str) -> Result<Vec<Partition>, Error> {
    let rows = client.query("SELECT child, from_t, to_t FROM lc_partitions($1)", &[&table]).await?;
    Ok(rows
        .into_iter()
        .map(|row| Partition {
            child: row.get(0),
            parent: table.to_string(),
            from_t: row.get(1),
            to_t: row.get(2),
        })
        .collect())
}

/// Partitions already detached, and therefore restorable.
pub async fn archived(client: &Client) -> Result<Vec<Partition>, Error> {
    let rows = client
        .query("SELECT child, parent, from_t, to_t FROM lc_archived_partitions ORDER BY from_t", &[])
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| Partition {
            child: row.get(0),
            parent: row.get(1),
            from_t: row.get(2),
            to_t: row.get(3),
        })
        .collect())
}

/// What a [`sweep`] keeps, and how much of a backlog one sweep may work off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Coordinate microseconds behind now that stay live: every delivery landing in it, and every
    /// event any delivery still kept is about.
    pub kept_us: i64,
    /// Partitions one sweep may retire. Each takes a lock on its parent, however briefly, so a
    /// store thousands behind catches up a sweep at a time rather than stalling the tick.
    pub most: usize,
}

/// What one [`sweep`] took away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub dropped: Vec<Partition>,
    pub archived: Vec<Partition>,
}

impl Swept {
    pub fn len(&self) -> usize {
        self.dropped.len() + self.archived.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Retire what `policy` no longer keeps at coordinate time `now_t`, oldest first.
///
/// Deliveries before events: an event partition stays while any kept delivery refers to it, so
/// dropping the old deliveries is what lets their events go in the same sweep.
pub async fn sweep(client: &Client, now_t: i64, policy: Policy) -> Result<Swept, Error> {
    let cutoff = now_t.saturating_sub(policy.kept_us.max(0));
    let mut swept = Swept::default();
    for partition in partitions(client, "deliveries").await? {
        if partition.to_t > cutoff || swept.len() >= policy.most {
            break;
        }
        if drop_partition(client, "deliveries", &partition.child).await? {
            swept.dropped.push(partition);
        }
    }
    for partition in older_and_unreferenced(client, cutoff, policy.most - swept.len()).await? {
        if detach(client, "events", &partition.child).await? {
            swept.archived.push(partition);
        }
    }
    Ok(swept)
}

/// Which event partitions may be taken out at coordinate time `now_t`, keeping `kept_us`.
///
/// Both tests, in the order that makes the cheap one first: entirely older than the cutoff, and
/// not referred to by any delivery landing after it — one still in flight, or one landed and
/// kept for a client catching up, which needs its event just as much.
pub async fn archivable(client: &Client, now_t: i64, kept_us: i64) -> Result<Vec<Partition>, Error> {
    older_and_unreferenced(client, now_t.saturating_sub(kept_us.max(0)), usize::MAX).await
}

async fn older_and_unreferenced(client: &Client, cutoff: i64, most: usize) -> Result<Vec<Partition>, Error> {
    let mut out = Vec::new();
    for partition in partitions(client, "events").await? {
        // `to_t` is exclusive, so a partition ending exactly at the cutoff is wholly before it.
        // Partitions do not overlap, so ordered by start they are ordered by end too.
        if partition.to_t > cutoff || out.len() >= most {
            break;
        }
        let referenced: bool = client
            .query_one(
                "SELECT lc_has_pending_deliveries($1, $2, $3)",
                &[&partition.from_t, &partition.to_t, &cutoff],
            )
            .await?
            .get(0);
        if !referenced {
            out.push(partition);
        }
    }
    Ok(out)
}

/// Detach one partition, moving its rows into `<table>_archive` and dropping it.
///
/// `false` when there is no such partition of that table, which is what a second call gets.
pub async fn detach(client: &Client, table: &str, child: &str) -> Result<bool, Error> {
    let row = client
        .query_one("SELECT lc_detach_partition($1, $2)", &[&table, &child])
        .await?;
    Ok(row.get(0))
}

/// Drop one partition and its rows. `false` when there is no such partition of that table.
pub async fn drop_partition(client: &Client, table: &str, child: &str) -> Result<bool, Error> {
    let row = client
        .query_one("SELECT lc_drop_partition($1, $2)", &[&table, &child])
        .await?;
    Ok(row.get(0))
}

/// Put one back, from the record made when it was detached and the rows in the archive.
pub async fn attach(client: &Client, child: &str) -> Result<bool, Error> {
    let row = client.query_one("SELECT lc_attach_partition($1)", &[&child]).await?;
    Ok(row.get(0))
}

/// Note where a detached partition's rows were shipped.
///
/// This crate never ships them: that needs a bucket and belongs to a deployment. Recording it
/// is the part that has to survive a restart, so that a restore knows where to look.
pub async fn record_location(client: &Client, child: &str, location: &str) -> Result<bool, Error> {
    let changed = client
        .execute("UPDATE lc_archived_partitions SET location = $2 WHERE child = $1", &[
            &child, &location,
        ])
        .await?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{EventId, Minter};
    use crate::store::{self, Delivery, Event};
    use crate::migrate;

    const SPAN_US: i64 = store::PARTITION_SPAN_US;

    async fn store_client() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    /// Start from nothing: an archived partition's record and rows outlive the run, so a second
    /// run would otherwise find them.
    ///
    /// Each test owns a band of slots. They share one database and run in parallel, and a
    /// shared band means one test's cleanup drops the partition another just made.
    async fn clear(client: &Client, base_slot: i64) {
        for slot in base_slot..base_slot + 4 {
            for table in ["events", "deliveries"] {
                let child = format!("{table}_s{slot}");
                let _ = attach(client, &child).await;
                let _ = client
                    .batch_execute(&format!("DROP TABLE IF EXISTS {child} CASCADE"))
                    .await;
            }
        }
        client
            .execute(
                "DELETE FROM lc_archived_partitions WHERE from_t >= $1 AND from_t < $2",
                &[&(base_slot * SPAN_US), &((base_slot + 4) * SPAN_US)],
            )
            .await
            .unwrap();
    }

    fn event(minter: &mut Minter, source_id: i64, t: i64) -> Event {
        Event {
            id: minter.mint(t).expect("an identifier"),
            source_id,
            t,
            g: [0, 0, 0],
            system_id: None,
            local: None,
            kind: 1,
            payload: "{}".into(),
        }
    }

    /// The first tier, and the shape of the whole thing: old enough and unreferenced goes,
    /// recent stays, and what went is still there to be put back.
    #[tokio::test]
    async fn an_old_unreferenced_partition_detaches_and_comes_back() {
        let Some(client) = store_client().await else { return };
        const SLOT: i64 = 600;
        clear(&client, SLOT).await;
        let base = SLOT * SPAN_US;
        store::ensure_partitions(&client, base, base + SPAN_US * 2).await.unwrap();

        let mut minter = Minter::new(6).unwrap();
        let old = event(&mut minter, 20_000, base + 1_000);
        let recent = event(&mut minter, 20_000, base + SPAN_US + 1_000);
        store::insert_events(&client, &[old, recent]).await.unwrap();

        // A partition is archivable when the whole of it is behind the cutoff, and a partition
        // is a span wide -- so "now" has to be two spans past its start, not one.
        let now = base + SPAN_US * 2 + 2_000;
        let ready = archivable(&client, now, SPAN_US).await.unwrap();
        let names: Vec<&str> = ready.iter().map(|p| p.child.as_str()).collect();
        assert!(names.contains(&format!("events_s{SLOT}").as_str()), "{names:?}");
        assert!(!names.contains(&format!("events_s{}", SLOT + 1).as_str()), "the live one went");

        let child = format!("events_s{SLOT}");
        assert!(detach(&client, "events", &child).await.unwrap());
        // Gone from the live table...
        let live: i64 = client
            .query_one("SELECT count(*) FROM events WHERE source_id = 20000", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(live, 1, "only the recent event should still be queryable");
        // ...and not gone, though its table is.
        let kept: i64 = client
            .query_one("SELECT count(*) FROM events_archive WHERE source_id = 20000", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(kept, 1, "detaching destroyed the rows");
        let standing: Option<String> =
            client.query_one("SELECT to_regclass($1)::text", &[&child]).await.unwrap().get(0);
        assert_eq!(standing, None, "the partition stayed behind as a table");
        assert!(archived(&client).await.unwrap().iter().any(|p| p.child == child));

        // A replay asks for a year nobody has looked at since, and gets it.
        assert!(attach(&client, &child).await.unwrap());
        let back: i64 = client
            .query_one("SELECT count(*) FROM events WHERE source_id = 20000", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(back, 2, "the restore did not bring the rows back");
        let left: i64 = client
            .query_one("SELECT count(*) FROM events_archive WHERE source_id = 20000", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(left, 0, "the restore copied rather than moved");
        assert!(!archived(&client).await.unwrap().iter().any(|p| p.child == child));
        clear(&client, SLOT).await;
    }

    /// The second tier, which is the one a date comparison would get wrong. Light is in flight
    /// for years: an event far past the horizon can still be what a scheduled delivery is about,
    /// and archiving it would leave a tick holding an identifier with nothing behind it.
    #[tokio::test]
    async fn a_partition_a_pending_delivery_still_needs_stays() {
        let Some(client) = store_client().await else { return };
        const SLOT: i64 = 610;
        clear(&client, SLOT).await;
        let base = SLOT * SPAN_US;
        store::ensure_partitions(&client, base, base + SPAN_US * 3).await.unwrap();

        let mut minter = Minter::new(7).unwrap();
        let ancient = event(&mut minter, 20_100, base + 1_000);
        store::insert_events(&client, std::slice::from_ref(&ancient)).await.unwrap();

        let now = base + SPAN_US * 2;
        let child = format!("events_s{SLOT}");

        // With nothing scheduled, it is ready to go.
        let ready = archivable(&client, now, SPAN_US).await.unwrap();
        assert!(ready.iter().any(|p| p.child == child), "should be archivable with no deliveries");

        // Its light is still traveling: the delivery has not been read yet.
        store::insert_deliveries(&client, &[Delivery {
            observer_id: 20_100,
            arrive_t: now + SPAN_US / 2,
            event_id: ancient.id,
            strength: 1.0,
        }])
        .await
        .unwrap();
        let ready = archivable(&client, now, SPAN_US).await.unwrap();
        assert!(!ready.iter().any(|p| p.child == child), "archived an event still to be delivered");

        // Once that arrival is further back than is kept, nothing needs it any more.
        let later = now + SPAN_US * 2;
        let ready = archivable(&client, later, SPAN_US).await.unwrap();
        assert!(ready.iter().any(|p| p.child == child), "kept it after the delivery was read");
        clear(&client, SLOT).await;
    }

    /// The identifier range for a time range, which is what lets a question about *when* be
    /// asked of a table that only stores *which*. The packing is in two places and they have to
    /// agree; this is where that is checked.
    #[tokio::test]
    async fn the_sql_identifier_range_matches_the_packing() {
        let Some(client) = store_client().await else { return };
        for (from_t, to_t) in [(0i64, 999_999i64), (1_000_000, 5_999_999), (0, 0), (-500, 10)] {
            let row = client
                .query_one("SELECT lo, hi FROM lc_id_range($1, $2)", &[&from_t, &to_t])
                .await
                .unwrap();
            let (lo, hi): (i64, i64) = (row.get(0), row.get(1));

            let first_second = (from_t / 1_000_000).max(0);
            let last_second = (to_t / 1_000_000).max(0);
            assert_eq!(lo, EventId::new(first_second, 0, 0).unwrap().get());
            assert_eq!(
                hi,
                EventId::new(last_second, crate::id::MAX_SHARD, crate::id::MAX_SEQUENCE)
                    .unwrap()
                    .get()
            );

            // And every identifier a minter would produce for that range is inside it.
            let mut minter = Minter::new(3).unwrap();
            for t in [from_t.max(0), to_t.max(0)] {
                let id = minter.mint(t).unwrap().get();
                assert!((lo..=hi).contains(&id), "{id} outside [{lo}, {hi}] for {t}");
            }
        }
    }

    /// Detaching something that is not there, and restoring something that was never detached,
    /// both say so rather than raising.
    #[tokio::test]
    async fn detaching_and_attaching_what_is_not_there_is_not_an_error() {
        let Some(client) = store_client().await else { return };
        assert!(!detach(&client, "events", "events_p999999").await.unwrap());
        assert!(!attach(&client, "events_p999999").await.unwrap());
        assert!(!record_location(&client, "events_p999999", "s3://nowhere").await.unwrap());
        assert!(!drop_partition(&client, "deliveries", "deliveries_p999999").await.unwrap());
        // A table that is not a partition of the one named is not dropped through it.
        assert!(!drop_partition(&client, "deliveries", "lc_archived_partitions").await.unwrap());
    }

    /// Where the bytes went has to survive a restart, or a restore does not know where to look.
    #[tokio::test]
    async fn a_recorded_location_survives_the_connection_that_wrote_it() {
        let Some(client) = store_client().await else { return };
        const SLOT: i64 = 620;
        clear(&client, SLOT).await;
        let base = SLOT * SPAN_US;
        store::ensure_partitions(&client, base, base).await.unwrap();
        let child = format!("events_s{SLOT}");
        assert!(detach(&client, "events", &child).await.unwrap());
        assert!(record_location(&client, &child, "s3://lightcone/archive/1").await.unwrap());
        drop(client);

        let Some(fresh) = store_client().await else { return };
        let where_it_went: Option<String> = fresh
            .query_one("SELECT location FROM lc_archived_partitions WHERE child = $1", &[&child])
            .await
            .unwrap()
            .get(0);
        assert_eq!(where_it_went.as_deref(), Some("s3://lightcone/archive/1"));
        clear(&fresh, SLOT).await;
    }
}
