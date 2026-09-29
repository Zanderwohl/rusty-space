//! The journal over Postgres: [`Postgres`], and [`Store`] for a binary choosing at run time.

use glam::DVec3;
use lc_proto::ShipId;

use super::{Journal, JournalError, Memory, Transcript};
use crate::world::{Event, Scheduled};

impl From<tokio_postgres::Error> for JournalError {
    fn from(error: tokio_postgres::Error) -> Self {
        // The source, not just the summary. `tokio_postgres::Error` renders as "db error" and
        // nothing else; everything a person needs to act on -- the constraint, the table, the
        // detail -- is one level down.
        match std::error::Error::source(&error) {
            Some(why) => Self(format!("{error}: {why}")),
            None => Self(error.to_string()),
        }
    }
}

/// The real one.
pub struct Postgres {
    client: tokio_postgres::Client,
    /// The range already made ready, so the tick is one comparison rather than a round trip.
    prepared: Option<(i64, i64)>,
    /// Partitions made outside that range, for arrivals beyond it. Kept so a system talking to
    /// itself asks once rather than every tick.
    distant: std::collections::HashSet<i64>,
}

impl Postgres {
    /// Connect and bring the schema up to date.
    pub async fn open() -> Result<Self, JournalError> {
        let client = lc_store::connect().await?;
        lc_store::migrate::apply(&client).await?;
        Ok(Self::with(client))
    }

    /// The same, over a connection the caller already has.
    pub fn with(client: tokio_postgres::Client) -> Self {
        Self { client, prepared: None, distant: std::collections::HashSet::new() }
    }

    pub fn client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    /// Make room for rows landing outside the window [`Journal::prepare`] keeps.
    ///
    /// **A delivery's `arrive_t` is years out, not a time near now**, because that is how long
    /// the light takes — so the first thing said within earshot of another system lands past
    /// any rolling window, and a row with no partition is an error.
    ///
    /// One partition per arrival rather than the range up to it: every thirty-day slot between
    /// here and a star would be thousands of empty tables.
    async fn make_room(&mut self, times: impl Iterator<Item = i64>) -> Result<(), JournalError> {
        // Nothing prepared yet is an empty range, which every time falls outside.
        let (lo, hi) = self.prepared.unwrap_or((i64::MAX, i64::MIN));
        let wanted: std::collections::BTreeSet<i64> = times
            .filter(|t| *t < lo || *t > hi)
            .map(lc_store::store::partition_of)
            .filter(|slot| !self.distant.contains(slot))
            .collect();
        for slot in wanted {
            // The slot's own start, so the call makes exactly the one partition.
            let at = slot.saturating_mul(lc_store::store::PARTITION_SPAN_US);
            lc_store::store::ensure_partitions(&self.client, at, at).await?;
            self.distant.insert(slot);
        }
        Ok(())
    }
}

/// Light-microseconds are what the server works in and what the global grid counts, so the
/// conversion is a rounding and nothing else.
fn to_grid(at: DVec3) -> [i64; 3] {
    [at.x.round() as i64, at.y.round() as i64, at.z.round() as i64]
}

fn from_grid(g: [i64; 3]) -> DVec3 {
    DVec3::new(g[0] as f64, g[1] as f64, g[2] as f64)
}

impl Journal for Postgres {
    async fn prepare(&mut self, from_t: i64, to_t: i64) -> Result<(), JournalError> {
        if let Some((lo, hi)) = self.prepared
            && from_t >= lo
            && to_t <= hi
        {
            return Ok(());
        }
        lc_store::store::ensure_partitions(&self.client, from_t, to_t).await?;
        // To the end of the last partition made, which is what was actually made ready. Recording
        // `to_t` alone put the window's edge one tick behind the next ask, so every tick
        // went back to the store.
        let last = lc_store::store::partition_of(to_t);
        let hi = last.saturating_add(1).saturating_mul(lc_store::store::PARTITION_SPAN_US) - 1;
        self.prepared = Some((from_t, hi));
        Ok(())
    }

    async fn write(
        &mut self,
        events: &[Event],
        deliveries: &[Scheduled],
    ) -> Result<(), JournalError> {
        // Every id here was minted, so `from_raw` cannot refuse one; if it somehow did, the event
        // is not written rather than the shard stopping.
        let rows: Vec<lc_store::store::Event> = events
            .iter()
            .filter_map(|e| Some((e, lc_store::id::EventId::from_raw(e.id)?)))
            .map(|(e, id)| lc_store::store::Event {
                id,
                source_id: e.source.0,
                t: e.t,
                // Rounded onto the grid, which is what the grid is for. The 150 m it resolves
                // is a fraction of a microradian of direction at any range a sighting happens
                // over, and doc 03 already makes this the global representation.
                g: to_grid(e.at),
                system_id: None,
                local: None,
                kind: e.kind,
                payload: e.payload.clone(),
            })
            .collect();
        // Both tables at once, and before either insert: an event is always inside the tick's
        // window, but the deliveries it schedules reach as far as the light does.
        self.make_room(
            events.iter().map(|e| e.t).chain(deliveries.iter().map(|d| d.arrive_t)),
        )
        .await?;
        lc_store::store::insert_events(&self.client, &rows).await?;

        let scheduled: Vec<lc_store::store::Delivery> = deliveries
            .iter()
            .filter_map(|d| {
                Some(lc_store::store::Delivery {
                    observer_id: d.observer.0,
                    arrive_t: d.arrive_t,
                    event_id: lc_store::id::EventId::from_raw(d.event)?,
                    strength: d.strength,
                })
            })
            .collect();
        lc_store::store::insert_deliveries(&self.client, &scheduled).await?;
        Ok(())
    }

    async fn due(
        &self,
        observer: ShipId,
        after_t: i64,
        until_t: i64,
    ) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        // The proven path: one index-only range scan over `(observer_id, arrive_t, event_id)`.
        let rows =
            lc_store::store::deliveries_for(&self.client, observer.0, after_t, until_t).await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        // And one more query for the events they are about, rather than one per delivery.
        let ids: Vec<i64> = rows.iter().map(|d| d.event_id.get()).collect();
        let events = self
            .client
            .query(
                "SELECT event_id, source_id, t, gx, gy, gz, kind, payload::text
                   FROM events WHERE event_id = ANY($1)",
                &[&ids],
            )
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for delivery in rows {
            let Some(row) = events.iter().find(|r| r.get::<_, i64>(0) == delivery.event_id.get())
            else {
                continue;
            };
            out.push((
                Scheduled {
                    observer,
                    event: delivery.event_id.get(),
                    arrive_t: delivery.arrive_t,
                    strength: delivery.strength,
                },
                Event {
                    id: row.get(0),
                    source: ShipId(row.get(1)),
                    t: row.get(2),
                    at: from_grid([row.get(3), row.get(4), row.get(5)]),
                    kind: row.get(6),
                    // Not stored: what a source radiated is in the delivery's own strength,
                    // which is the only thing anything downstream reads it for.
                    power_w: 0.0,
                    payload: row.get(7),
                },
            ));
        }
        Ok(out)
    }

    async fn in_flight(&self, kind: i16, after_t: i64) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        let rows = self
            .client
            .query(
                "SELECT d.observer_id, d.arrive_t, d.event_id, d.strength,
                        e.source_id, e.t, e.gx, e.gy, e.gz, e.kind, e.payload::text
                   FROM deliveries d JOIN events e ON e.event_id = d.event_id
                  WHERE e.kind = $1 AND d.arrive_t > $2
               ORDER BY d.arrive_t",
                &[&kind, &after_t],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    Scheduled {
                        observer: ShipId(row.get(0)),
                        arrive_t: row.get(1),
                        event: row.get(2),
                        strength: row.get(3),
                    },
                    Event {
                        id: row.get(2),
                        source: ShipId(row.get(4)),
                        t: row.get(5),
                        at: from_grid([row.get(6), row.get(7), row.get(8)]),
                        kind: row.get(9),
                        power_w: 0.0,
                        payload: row.get(10),
                    },
                )
            })
            .collect())
    }

    async fn record(
        &mut self,
        messages: &[lc_store::chat::Message],
        receipts: &[lc_store::chat::Receipt],
        keys: &[lc_store::chat::Held],
    ) -> Result<(), JournalError> {
        // Messages before receipts: a receipt references the message it is about, so the other
        // order fails the foreign key on the very first one.
        lc_store::chat::save_messages(&self.client, messages).await?;
        lc_store::chat::save_receipts(&self.client, receipts).await?;
        lc_store::chat::save_keys(&self.client, keys).await?;
        Ok(())
    }

    async fn transcript(&self, ship: ShipId) -> Result<Transcript, JournalError> {
        Ok(Transcript {
            sent: lc_store::chat::sent_by(&self.client, ship.0).await?,
            heard: lc_store::chat::heard_by(&self.client, ship.0).await?,
        })
    }

    async fn keyring(&self) -> Result<Vec<lc_store::chat::Held>, JournalError> {
        Ok(lc_store::chat::all_keys(&self.client).await?)
    }

    async fn ack_window(&self) -> Result<Vec<(i64, i64, i64)>, JournalError> {
        Ok(lc_store::chat::recent_heard(&self.client, lc_proto::ACK_DEPTH as i64).await?)
    }
}

/// Whichever of the two a shard was started with.
///
/// `Server` is generic over its journal, so a binary that chooses one at run time would have to
/// be generic all the way down or box a trait with async methods. This is the third option and
/// the cheapest: one type, two arms, and the choice made once where the arguments are read.
///
/// A shard with no database is not a shard that keeps less. It is a shard that keeps **nothing**
/// past the process, conversations included, and [`Store::Ephemeral`] is named so that reads
/// like the warning it is.
pub enum Store {
    Ephemeral(Memory),
    Durable(Postgres),
}

macro_rules! either {
    ($self:expr, $method:ident ( $($arg:expr),* )) => {
        match $self {
            Store::Ephemeral(inner) => inner.$method($($arg),*).await,
            Store::Durable(inner) => inner.$method($($arg),*).await,
        }
    };
}

impl Journal for Store {
    async fn prepare(&mut self, from_t: i64, to_t: i64) -> Result<(), JournalError> {
        either!(self, prepare(from_t, to_t))
    }

    async fn write(
        &mut self,
        events: &[Event],
        deliveries: &[Scheduled],
    ) -> Result<(), JournalError> {
        either!(self, write(events, deliveries))
    }

    async fn due(
        &self,
        observer: ShipId,
        after_t: i64,
        until_t: i64,
    ) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        either!(self, due(observer, after_t, until_t))
    }

    async fn in_flight(&self, kind: i16, after_t: i64) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        either!(self, in_flight(kind, after_t))
    }

    async fn record(
        &mut self,
        messages: &[lc_store::chat::Message],
        receipts: &[lc_store::chat::Receipt],
        keys: &[lc_store::chat::Held],
    ) -> Result<(), JournalError> {
        either!(self, record(messages, receipts, keys))
    }

    async fn transcript(&self, ship: ShipId) -> Result<Transcript, JournalError> {
        either!(self, transcript(ship))
    }

    async fn keyring(&self) -> Result<Vec<lc_store::chat::Held>, JournalError> {
        either!(self, keyring())
    }

    async fn ack_window(&self) -> Result<Vec<(i64, i64, i64)>, JournalError> {
        either!(self, ack_window())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_round_trips_to_within_its_own_resolution() {
        let at = DVec3::new(1_234_567.4, -98.6, 0.5);
        let back = from_grid(to_grid(at));
        assert!((back - at).abs().max_element() <= 0.5, "{back:?} against {at:?}");
    }

    /// The whole stack against a real database: the server writes through the journal, the
    /// gate reads back through the index-only range scan, and none of it is in memory.
    ///
    /// "Survives a restart" is what this is for. A second server, over a second connection,
    /// knowing nothing about the first, still delivers what the first scheduled.
    #[tokio::test]
    async fn a_second_server_delivers_what_the_first_one_scheduled() {
        use crate::server::{Server, TICK_US};
        use crate::transport::Loopback;
        use crate::world::still;
        use lc_proto::{Inbound, Intent, Order, Outbound};

        const ACTOR: i64 = 30_000;
        const WATCHER: i64 = 30_001;
        const SHARD: u64 = 9;
        // Two light-hours, so the arrival is many ticks out and cannot be an accident.
        let far = DVec3::new(7_200.0 * 1_000_000.0, 0.0, 0.0);

        let Ok(journal) = Postgres::open().await else { return };
        // Own band, cleared first, so the suite can run twice against one database.
        journal
            .client()
            .execute("DELETE FROM deliveries WHERE observer_id IN ($1, $2)", &[&ACTOR, &WATCHER])
            .await
            .unwrap();
        journal
            .client()
            .execute("DELETE FROM events WHERE source_id IN ($1, $2)", &[&ACTOR, &WATCHER])
            .await
            .unwrap();

        let mut wire = Loopback::new();
        let mut server = Server::new(journal, 0, SHARD);
        let actor = lc_proto::ClientId(1);
        server.admit(actor, still(ShipId(ACTOR), DVec3::ZERO), 0.0);
        server.admit(lc_proto::ClientId(2), still(ShipId(WATCHER), far), 0.0);

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(ACTOR),
            order: Order::Transmit { power_w: 1.0e20 },
            issued_at_client_t: i64::MAX / 4,
        }));
        server.tick(&mut wire).await.unwrap();
        let emitted = TICK_US;
        let arrives = emitted + far.x as i64;
        drop(server);

        // A new server, a new connection, and nothing carried over but the database.
        let Ok(journal) = Postgres::open().await else { return };
        let mut restarted = Server::new(journal, arrives - TICK_US, SHARD + 1);
        let watcher = lc_proto::ClientId(1);
        restarted.admit(watcher, still(ShipId(WATCHER), far), 0.0);
        let mut wire = Loopback::new();
        // It has read nothing, so it asks from before the arrival.
        wire.client_says(watcher, Inbound::ResumeFrom { arrive_t: 0 });

        // One tick carries it past the arrival.
        restarted.tick(&mut wire).await.unwrap();
        let told: Vec<_> = wire
            .take(watcher)
            .into_iter()
            .flat_map(|m| match m {
                Outbound::Sightings(list) => list,
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(told.len(), 1, "the restart lost the scheduled delivery");
        let sighting = told[0].get();
        assert_eq!(sighting.arrive_t, arrives);
        assert_eq!(sighting.emitted_t, emitted);
        assert_eq!(sighting.source_id, ACTOR);
        assert!(sighting.direction[0] < -0.999, "{:?}", sighting.direction);

        // And the negative case still holds against the database: a server whose clock has not
        // reached the arrival is told the same thing by the same query, and releases nothing.
        let Ok(journal) = Postgres::open().await else { return };
        let mut early = Server::new(journal, arrives - TICK_US * 3, SHARD + 2);
        let watcher = lc_proto::ClientId(1);
        early.admit(watcher, still(ShipId(WATCHER), far), 0.0);
        let mut wire = Loopback::new();
        wire.client_says(watcher, Inbound::ResumeFrom { arrive_t: 0 });
        early.tick(&mut wire).await.unwrap();
        assert!(early.now_t() < arrives, "the test never stayed before the arrival");
        assert!(wire.take(watcher).is_empty(), "a sighting was released before its light landed");
    }
}
