//! Where events and their scheduled deliveries are kept.
//!
//! A trait with two implementations. [`Memory`] is what the filter's own tests run against —
//! they are about causality and must not need a database. [`Postgres`] is the real one, and
//! reads deliveries through exactly the index-only range scan phase 7 demonstrated.
//!
//! Nothing here decides *what* may be sent. That is [`lc_proto::Cleared::clear`], and it sits
//! between this and the wire.

use glam::DVec3;
use lc_proto::ShipId;

use crate::world::{Event, Scheduled};

/// Anything that went wrong underneath. The server cannot fix any of it, so it is one type.
#[derive(Debug)]
pub struct JournalError(pub String);

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for JournalError {}

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

pub trait Journal {
    /// Make ready for writes in `[from_t, to_t]`.
    ///
    /// Called by the tick, ahead of the range it is about to write. A store that made room on
    /// demand would hide a stalled maintenance job until the disk filled, so the server does it
    /// deliberately: this *is* the job that pre-creates the window.
    fn prepare(
        &mut self,
        from_t: i64,
        to_t: i64,
    ) -> impl std::future::Future<Output = Result<(), JournalError>> + Send;

    /// Append events and the deliveries already worked out for them.
    fn write(
        &mut self,
        events: &[Event],
        deliveries: &[Scheduled],
    ) -> impl std::future::Future<Output = Result<(), JournalError>> + Send;

    /// What reaches one observer in `(after_t, until_t]`, in arrival order, with the event each
    /// one is about.
    ///
    /// Half-open below, because a tick asks for what arrived since the last tick and an
    /// inclusive bound would deliver the boundary twice.
    fn due(
        &self,
        observer: ShipId,
        after_t: i64,
        until_t: i64,
    ) -> impl std::future::Future<Output = Result<Vec<(Scheduled, Event)>, JournalError>> + Send;

    /// Every delivery of an event of `kind` that lands after `after_t`, with its event: what is
    /// still in flight. Read once, when a shard comes back, for whatever it acts on when light
    /// lands rather than merely shows.
    fn in_flight(
        &self,
        kind: i16,
        after_t: i64,
    ) -> impl std::future::Future<Output = Result<Vec<(Scheduled, Event)>, JournalError>> + Send;

    /// Append what was said, where it landed, and what keys that taught.
    ///
    /// Separate from [`Journal::write`] and deliberately so. An event and its deliveries are
    /// swept away once their light has passed every observer; a conversation is read long after
    /// that, by the two ships in it, whenever either signs in. Putting them through one call
    /// would put them under one retention policy. See `lc_store::chat`.
    ///
    /// Receipts are written by the *flush*, not by the tick that wrote the message: a receipt
    /// is the light landing, which is a different event from the light leaving and happens
    /// years later.
    fn record(
        &mut self,
        messages: &[lc_store::chat::Message],
        receipts: &[lc_store::chat::Receipt],
        keys: &[lc_store::chat::Held],
    ) -> impl std::future::Future<Output = Result<(), JournalError>> + Send;

    /// Everything this ship has said and been told.
    ///
    /// Asked once per sign-in, never in the tick's hot path. Keys are not here: the server
    /// holds the whole keyring in memory because it has to answer "may this be sealed" inside
    /// an order, which is synchronous. See [`Journal::keyring`].
    fn transcript(
        &self,
        ship: ShipId,
    ) -> impl std::future::Future<Output = Result<Transcript, JournalError>> + Send;

    /// Every keyring row there is: who will hold whose key, and from when.
    ///
    /// Read once, when a shard comes back. Whole rather than per ship because the rule it feeds
    /// is asked inside an order — synchronously, twenty times a second — and a query there would
    /// be a database round trip in the tick.
    fn keyring(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<lc_store::chat::Held>, JournalError>> + Send;

    /// The last [`lc_proto::ACK_DEPTH`] messages each observer has from each sender.
    ///
    /// Read once, when a shard comes back, to refill the acknowledgement window. Without it the
    /// first reply after a restart acknowledges nothing, which the far end cannot distinguish
    /// from its messages never having arrived.
    fn ack_window(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<(i64, i64, i64)>, JournalError>> + Send;
}

/// One ship's own copy of every conversation it is party to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    /// What it transmitted, oldest first.
    pub sent: Vec<lc_store::chat::Message>,
    /// What reached it, oldest arrival first, with when the light landed and how loudly.
    pub heard: Vec<(lc_store::chat::Message, i64, Option<f32>)>,
}

/// Everything in vectors. No ordering guarantees from storage, so it sorts.
#[derive(Default)]
pub struct Memory {
    pub events: Vec<Event>,
    pub deliveries: Vec<Scheduled>,
    pub messages: Vec<lc_store::chat::Message>,
    pub receipts: Vec<lc_store::chat::Receipt>,
    pub keys: Vec<lc_store::chat::Held>,
}

impl Journal for Memory {
    async fn prepare(&mut self, _from_t: i64, _to_t: i64) -> Result<(), JournalError> {
        Ok(())
    }

    async fn write(
        &mut self,
        events: &[Event],
        deliveries: &[Scheduled],
    ) -> Result<(), JournalError> {
        self.events.extend_from_slice(events);
        self.deliveries.extend_from_slice(deliveries);
        Ok(())
    }

    async fn due(
        &self,
        observer: ShipId,
        after_t: i64,
        until_t: i64,
    ) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        let mut out: Vec<(Scheduled, Event)> = self
            .deliveries
            .iter()
            .filter(|d| d.observer == observer && d.arrive_t > after_t && d.arrive_t <= until_t)
            .filter_map(|d| {
                self.events.iter().find(|e| e.id == d.event).map(|e| (*d, e.clone()))
            })
            .collect();
        out.sort_by_key(|(d, _)| d.arrive_t);
        Ok(out)
    }

    async fn in_flight(&self, kind: i16, after_t: i64) -> Result<Vec<(Scheduled, Event)>, JournalError> {
        let mut out: Vec<(Scheduled, Event)> = self
            .deliveries
            .iter()
            .filter(|d| d.arrive_t > after_t)
            .filter_map(|d| self.events.iter().find(|e| e.id == d.event && e.kind == kind).map(|e| (*d, e.clone())))
            .collect();
        out.sort_by_key(|(d, _)| d.arrive_t);
        Ok(out)
    }

    async fn record(
        &mut self,
        messages: &[lc_store::chat::Message],
        receipts: &[lc_store::chat::Receipt],
        keys: &[lc_store::chat::Held],
    ) -> Result<(), JournalError> {
        self.messages.extend_from_slice(messages);
        self.receipts.extend_from_slice(receipts);
        // First offer wins, as the keyring's primary key makes it in the real one.
        for key in keys {
            if !self.keys.iter().any(|k| k.holder == key.holder && k.subject == key.subject) {
                self.keys.push(*key);
            }
        }
        Ok(())
    }

    async fn transcript(&self, ship: ShipId) -> Result<Transcript, JournalError> {
        let find = |id: i64| self.messages.iter().find(|m| m.event_id == id).cloned();
        let mut heard: Vec<(lc_store::chat::Message, i64, Option<f32>)> = self
            .receipts
            .iter()
            .filter(|r| r.observer == ship.0)
            .filter_map(|r| Some((find(r.event_id)?, r.arrive_t, r.strength)))
            .collect();
        heard.sort_by_key(|(_, arrive_t, _)| *arrive_t);
        let mut sent: Vec<lc_store::chat::Message> =
            self.messages.iter().filter(|m| m.sender == ship.0).cloned().collect();
        sent.sort_by_key(|m| m.sent_t);
        Ok(Transcript { sent, heard })
    }

    async fn keyring(&self) -> Result<Vec<lc_store::chat::Held>, JournalError> {
        Ok(self.keys.clone())
    }

    async fn ack_window(&self) -> Result<Vec<(i64, i64, i64)>, JournalError> {
        let mut out = Vec::new();
        for receipt in &self.receipts {
            let Some(message) = self.messages.iter().find(|m| m.event_id == receipt.event_id)
            else {
                continue;
            };
            if message.is_key {
                continue;
            }
            out.push((receipt.observer, message.sender, receipt.event_id, receipt.arrive_t));
        }
        out.sort_by_key(|(observer, sender, _, arrive_t)| (*observer, *sender, *arrive_t));
        Ok(out.into_iter().map(|(observer, sender, event, _)| (observer, sender, event)).collect())
    }
}

/// How far ahead of `now` the tick keeps partitions made.
///
/// One span is thirty days of coordinate time, about four and a half real days at the design
/// rate. Two of them is enough that a server has to be down for a week before a write finds no
/// room.
pub const PREPARE_AHEAD_US: i64 = 2 * 2_592_000_000_000;

/// The real one.
pub struct Postgres {
    client: tokio_postgres::Client,
    /// The range already made ready, so the tick is one comparison rather than a round trip.
    prepared: Option<(i64, i64)>,
}

impl Postgres {
    /// Connect and bring the schema up to date.
    pub async fn open() -> Result<Self, JournalError> {
        let client = lc_store::connect().await?;
        lc_store::migrate::apply(&client).await?;
        Ok(Self { client, prepared: None })
    }

    /// The same, over a connection the caller already has.
    pub fn with(client: tokio_postgres::Client) -> Self {
        Self { client, prepared: None }
    }

    pub fn client(&self) -> &tokio_postgres::Client {
        &self.client
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
        self.prepared = Some((from_t, to_t));
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

    fn event(id: i64, t: i64, at: DVec3) -> Event {
        Event { id, source: ShipId(1), t, at, kind: 1, power_w: 1.0, payload: "{}".into() }
    }

    #[tokio::test]
    async fn memory_returns_the_window_in_arrival_order_and_excludes_its_start() {
        let mut journal = Memory::default();
        journal
            .write(
                &[event(1, 0, DVec3::ZERO), event(2, 0, DVec3::ZERO)],
                &[
                    Scheduled { observer: ShipId(9), event: 2, arrive_t: 300, strength: 1.0 },
                    Scheduled { observer: ShipId(9), event: 1, arrive_t: 100, strength: 1.0 },
                    Scheduled { observer: ShipId(8), event: 1, arrive_t: 150, strength: 1.0 },
                ],
            )
            .await
            .unwrap();

        let got = journal.due(ShipId(9), 0, 1_000).await.unwrap();
        assert_eq!(got.len(), 2, "the other observer's delivery came back");
        assert!(got[0].0.arrive_t < got[1].0.arrive_t, "out of arrival order");

        // Half-open: asking again from where the last answer ended repeats nothing.
        let next = journal.due(ShipId(9), 300, 1_000).await.unwrap();
        assert!(next.is_empty(), "the boundary was delivered twice");
        assert_eq!(journal.due(ShipId(9), 0, 100).await.unwrap().len(), 1, "and is inclusive above");
    }

    /// A delivery whose event is missing yields nothing rather than half a sighting.
    #[tokio::test]
    async fn a_delivery_with_no_event_is_dropped() {
        let mut journal = Memory::default();
        journal
            .write(&[], &[Scheduled { observer: ShipId(9), event: 404, arrive_t: 1, strength: 1.0 }])
            .await
            .unwrap();
        assert!(journal.due(ShipId(9), 0, 1_000).await.unwrap().is_empty());
    }

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
