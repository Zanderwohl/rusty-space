//! Events whose light a shard is still following, so a restarted shard follows it on.
//!
//! The event is copied whole: a delivery found decades later is read after the event's own
//! partition has been archived. See `lightcone/docs/02-event-store.md`.

use tokio_postgres::{Client, Error, GenericClient};

use crate::id::EventId;
use crate::store::Event;

/// One event's light.
#[derive(Clone, Debug, PartialEq)]
pub struct Front {
    pub event: Event,
    /// Coordinate microseconds after which nothing can detect it.
    pub until_t: i64,
    /// JSON the shard writes and reads; opaque here.
    pub spread: String,
}

/// One already kept is left as it was.
pub async fn save(client: &impl GenericClient, fronts: &[Front]) -> Result<u64, Error> {
    if fronts.is_empty() {
        return Ok(0);
    }
    let ids: Vec<i64> = fronts.iter().map(|f| f.event.id.get()).collect();
    let sources: Vec<i64> = fronts.iter().map(|f| f.event.source_id).collect();
    let times: Vec<i64> = fronts.iter().map(|f| f.event.t).collect();
    let gx: Vec<i64> = fronts.iter().map(|f| f.event.g[0]).collect();
    let gy: Vec<i64> = fronts.iter().map(|f| f.event.g[1]).collect();
    let gz: Vec<i64> = fronts.iter().map(|f| f.event.g[2]).collect();
    let kinds: Vec<i16> = fronts.iter().map(|f| f.event.kind).collect();
    let payloads: Vec<&str> = fronts.iter().map(|f| f.event.payload.as_str()).collect();
    let until: Vec<i64> = fronts.iter().map(|f| f.until_t).collect();
    let spreads: Vec<&str> = fronts.iter().map(|f| f.spread.as_str()).collect();
    client
        .execute(
            "INSERT INTO lc_fronts (event_id, source_id, t, gx, gy, gz, kind, payload, until_t, spread)
             SELECT event_id, source_id, t, gx, gy, gz, kind, payload::jsonb, until_t, spread::jsonb
               FROM unnest($1::bigint[], $2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[],
                           $6::bigint[], $7::smallint[], $8::text[], $9::bigint[], $10::text[])
                 AS u(event_id, source_id, t, gx, gy, gz, kind, payload, until_t, spread)
             ON CONFLICT DO NOTHING",
            &[&ids, &sources, &times, &gx, &gy, &gz, &kinds, &payloads, &until, &spreads],
        )
        .await
}

const COLUMNS: &str = "event_id, source_id, t, gx, gy, gz, kind, payload::text, until_t, spread::text";

fn front_from(row: &tokio_postgres::Row) -> Option<Front> {
    Some(Front {
        event: Event {
            id: EventId::from_raw(row.get(0))?,
            source_id: row.get(1),
            t: row.get(2),
            g: [row.get(3), row.get(4), row.get(5)],
            system_id: None,
            local: None,
            kind: row.get(6),
            payload: row.get(7),
        },
        until_t: row.get(8),
        spread: row.get(9),
    })
}

/// Every front something could still detect at `now_t`.
pub async fn live(client: &Client, now_t: i64) -> Result<Vec<Front>, Error> {
    let rows = client
        .query(&format!("SELECT {COLUMNS} FROM lc_fronts WHERE until_t >= $1 ORDER BY t"), &[&now_t])
        .await?;
    Ok(rows.iter().filter_map(front_from).collect())
}

/// For events the events table no longer holds.
pub async fn of(client: &Client, event_ids: &[i64]) -> Result<Vec<Front>, Error> {
    let rows = client
        .query(&format!("SELECT {COLUMNS} FROM lc_fronts WHERE event_id = ANY($1)"), &[&event_ids])
        .await?;
    Ok(rows.iter().filter_map(front_from).collect())
}

pub async fn forget(client: &Client, before_t: i64) -> Result<u64, Error> {
    client.execute("DELETE FROM lc_fronts WHERE until_t < $1", &[&before_t]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Minter;

    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        crate::migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    /// Kept whole, found by id, and forgotten once faded and not before.
    #[tokio::test]
    async fn a_front_is_kept_until_it_fades() {
        let Some(client) = store().await else { return };
        const SOURCE: i64 = 16_000;
        client.execute("DELETE FROM lc_fronts WHERE source_id = $1", &[&SOURCE]).await.unwrap();
        let mut minter = Minter::new(12).unwrap();
        let front = Front {
            event: Event {
                id: minter.mint(5_000_000).unwrap(),
                source_id: SOURCE,
                t: 5_000_000,
                g: [1, -2, 3],
                system_id: None,
                local: None,
                kind: 7,
                payload: "{}".into(),
            },
            until_t: 9_000_000,
            spread: "{}".into(),
        };
        assert_eq!(save(&client, std::slice::from_ref(&front)).await.unwrap(), 1);
        assert_eq!(save(&client, std::slice::from_ref(&front)).await.unwrap(), 0, "kept twice");

        let found = of(&client, &[front.event.id.get()]).await.unwrap();
        assert_eq!(found, vec![front.clone()]);
        assert!(live(&client, 9_000_000).await.unwrap().iter().any(|f| f.event.id == front.event.id));
        assert!(!live(&client, 9_000_001).await.unwrap().iter().any(|f| f.event.id == front.event.id));

        forget(&client, 9_000_000).await.unwrap();
        assert_eq!(of(&client, &[front.event.id.get()]).await.unwrap().len(), 1, "forgotten while bright");
        forget(&client, 9_000_001).await.unwrap();
        assert!(of(&client, &[front.event.id.get()]).await.unwrap().is_empty(), "kept after fading");
    }
}
