//! Catching a client up on what landed on its craft before the retention cutoff.
//!
//! Retention moves old deliveries and events into `deliveries_archive` and `events_archive` rather
//! than dropping them, and each delivery keeps the direction its light came from. So what a craft
//! was told while its pilot was away is in the store, solved when it landed against the course the
//! craft actually flew, and catching up is reading it: no worldline, no fleet, no shard. That is
//! what lets this become a service of its own; today the shard calls it.
//!
//! The same gate as the tick's: every sighting goes out through [`Cleared::clear`], and a sealed
//! payload through [`lc_proto::redact`] against the faction keys the store says the craft held
//! when the light landed. See `lightcone/docs/02-event-store.md` §Catching up.

use std::collections::HashMap;

use lc_proto::{Cleared, FactionId, ShipId, Sighting};
use tokio_postgres::{Client, Error};

/// Deliveries read per page. A page is one message to the client, held well under
/// [`lc_proto::FRAME_LIMIT`] by the payloads it carries, and one round of reads to the store.
pub const PAGE: usize = 500;

/// One page of what landed, oldest first.
#[derive(Debug, Default)]
pub struct Page {
    pub sightings: Vec<Cleared<Sighting>>,
    /// How far the page reaches: everything landing in `(after_t, through_t]` has been read, so
    /// the next page starts here.
    pub through_t: i64,
}

impl Page {
    pub fn is_done(&self, until_t: i64) -> bool {
        self.through_t >= until_t
    }
}

struct Landed {
    arrive_t: i64,
    event_id: i64,
    strength: f32,
    direction: Option<[f32; 3]>,
}

struct Said {
    source_id: i64,
    t: i64,
    kind: i16,
    payload: String,
}

/// Live and archived alike: a partition is in one or the other, and a sweep may move it between
/// two reads.
const LANDED: &str = "SELECT arrive_t, event_id, strength, dx, dy, dz FROM deliveries
                       WHERE observer_id = $1 AND arrive_t > $2 AND arrive_t <= $3
                      UNION
                      SELECT arrive_t, event_id, strength, dx, dy, dz FROM deliveries_archive
                       WHERE observer_id = $1 AND arrive_t > $2 AND arrive_t <= $3
                      ORDER BY arrive_t, event_id
                      LIMIT $4";

const SAID: &str = "SELECT event_id, source_id, t, kind, payload::text FROM events WHERE event_id = ANY($1)
                    UNION ALL
                    SELECT event_id, source_id, t, kind, payload::text FROM events_archive WHERE event_id = ANY($1)";

/// What landed on `observer` in `(after_t, until_t]`, a page at a time, as the client is told it
/// at `now_t` by a craft whose receiver's floor is `noise_floor`.
///
/// A page never splits the deliveries of one arrival time, so `through_t` is always a whole
/// instant and the next page cannot repeat or skip one.
pub async fn page(
    client: &Client,
    observer: ShipId,
    noise_floor: f32,
    (after_t, until_t): (i64, i64),
    now_t: i64,
    most: usize,
) -> Result<Page, Error> {
    let until_t = until_t.min(now_t);
    if after_t >= until_t {
        return Ok(Page { sightings: Vec::new(), through_t: until_t.max(after_t) });
    }
    let mut landed = landed(client, observer, (after_t, until_t), most as i64 + 1).await?;
    let through_t = if landed.len() > most {
        let next_t = landed[most].arrive_t;
        landed.truncate(most);
        landed.retain(|l| l.arrive_t < next_t);
        if landed.is_empty() {
            // One instant holds more than a page: take the whole of it, or no page would end.
            landed = landed_at(client, observer, next_t).await?;
            next_t
        } else {
            next_t - 1
        }
    } else {
        until_t
    };

    let ids: Vec<i64> = landed.iter().map(|l| l.event_id).collect();
    let said: HashMap<i64, Said> = client
        .query(SAID, &[&ids])
        .await?
        .into_iter()
        .map(|row| (row.get(0), Said { source_id: row.get(1), t: row.get(2), kind: row.get(3), payload: row.get(4) }))
        .collect();
    let held = keys_held(client, observer).await?;

    let mut sightings = Vec::with_capacity(landed.len());
    for l in landed {
        // A delivery outliving its event is an archive shipped away and not restored.
        let Some(event) = said.get(&l.event_id) else { continue };
        // A row from before directions were kept: nothing can say where it came from now, and a
        // made-up bearing would be drawn and answered as a real one.
        let Some(direction) = l.direction else { continue };
        let holds = |key: FactionId| held.get(&key.0).is_some_and(|&learned_t| learned_t <= l.arrive_t);
        let sighting = Sighting {
            event_id: l.event_id,
            source_id: event.source_id,
            arrive_t: l.arrive_t,
            emitted_t: event.t,
            direction: direction.map(f64::from),
            strength: l.strength,
            kind: event.kind,
            payload: lc_proto::redact(event.kind, &event.payload, observer, holds),
        };
        if let Ok(pass) = Cleared::<Sighting>::clear(sighting, now_t, noise_floor) {
            sightings.push(pass);
        }
    }
    Ok(Page { sightings, through_t })
}

async fn landed(client: &Client, observer: ShipId, (after_t, until_t): (i64, i64), limit: i64) -> Result<Vec<Landed>, Error> {
    Ok(client.query(LANDED, &[&observer.0, &after_t, &until_t, &limit]).await?.iter().map(landed_row).collect())
}

async fn landed_at(client: &Client, observer: ShipId, at_t: i64) -> Result<Vec<Landed>, Error> {
    landed(client, observer, (at_t - 1, at_t), i64::MAX).await
}

fn landed_row(row: &tokio_postgres::Row) -> Landed {
    let axis = |k: usize| row.get::<_, Option<f32>>(3 + k);
    Landed {
        arrive_t: row.get(0),
        event_id: row.get(1),
        strength: row.get(2),
        direction: (|| Some([axis(0)?, axis(1)?, axis(2)?]))(),
    }
}

/// Each faction key the craft holds, and from when.
async fn keys_held(client: &Client, holder: ShipId) -> Result<HashMap<i64, i64>, Error> {
    let rows = client.query("SELECT faction, learned_t FROM lc_faction_keys WHERE holder = $1", &[&holder.0]).await?;
    Ok(rows.into_iter().map(|row| (row.get(0), row.get(1))).collect())
}

#[cfg(test)]
mod tests;
