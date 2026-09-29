//! Where events and their scheduled deliveries are kept.
//!
//! A trait with two implementations. [`Memory`] is what the filter's own tests run against —
//! they are about causality and must not need a database. [`Postgres`] is the real one, and
//! reads deliveries through exactly the index-only range scan phase 7 demonstrated, and is there
//! only with the `storage` feature.
//!
//! Nothing here decides *what* may be sent. That is [`lc_proto::Cleared::clear`], and it sits
//! between this and the wire.

use lc_proto::ShipId;

use crate::world::{Event, Scheduled};

#[cfg(feature = "storage")]
mod postgres;
#[cfg(feature = "storage")]
pub use postgres::{Postgres, Store};

/// Anything that went wrong underneath. The server cannot fix any of it, so it is one type.
#[derive(Debug)]
pub struct JournalError(pub String);

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for JournalError {}

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
    /// Read once, when a shard comes back, to refill the acknowledgment window. Without it the
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
        // The payload column is `jsonb`, so a payload that is not JSON is a row the real
        // journal refuses. A fake that took it let a burn built with `format!` out of a
        // `DVec3` past four hundred tests and fail on the shard.
        for event in events {
            if serde_json::from_str::<serde_json::Value>(&event.payload).is_err() {
                return Err(JournalError(format!(
                    "event {} carries a payload that is not JSON: {}",
                    event.id, event.payload,
                )));
            }
        }
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
            if message.content == lc_store::chat::Content::Key {
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
pub const PREPARE_AHEAD_US: i64 = 2 * lc_store::store::PARTITION_SPAN_US;


#[cfg(test)]
mod tests {
    use glam::DVec3;

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
}
