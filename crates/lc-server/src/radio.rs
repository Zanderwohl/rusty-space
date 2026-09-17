//! Saying things, and everything that decides who may hear them.
//!
//! Split out of [`crate::server`] because it is a self-contained question. The tick loop knows
//! about worldlines and about a gate; this knows about aim, sealing, acknowledgement and the
//! keyring, and the only thing the two share is that a transmission is an event like any other.
//!
//! **Three independent choices, and keeping them independent is the design.** Who a message is
//! addressed to decides whose acknowledgements ride back with it and who can decrypt it. Where
//! it is *pointed* decides who hears it. Whether it is *sealed* decides who can read it. A
//! player who conflates them broadcasts a private message in clear across a system, and the
//! interface lets them: it is the mistake a real operator makes, and it is legible afterwards
//! because everyone in earshot saw it.
//!
//! **The keyring and the acknowledgement window are schedules, not sets.** Both live on
//! [`crate::server::Server`] and both are written the instant a transmission leaves, stamped
//! with the coordinate time its light *lands*, and refused by every reader until the clock has
//! reached that. So a key sent four light-years takes four years to become usable, and the rule
//! that says so is the same shape as the deliveries table's: work it out at write time, gate it
//! at read time.
//!
//! Scheduling them rather than learning them on arrival is also what makes them work for a
//! craft nobody is flying. Arrivals are only walked for connected clients — there is nobody to
//! tell otherwise — and a key that only landed when its owner happened to be signed in would be
//! a mechanic that depended on who was watching.
//!
//! The geometry is `lc_world::signal` and the storage is `lc_store::chat`. See
//! `lightcone/docs/05-observation.md`.

use lc_proto::{
    ACK_DEPTH, Aim, MESSAGE_LIMIT, Order, Outbound, Refusal, Said, Secrecy, ShipId, Spoken,
};
use lc_world::craft::CraftId;
use lc_world::motion::LIGHT_US_PER_LY;
use lc_world::signal::{Beam, Transmitter};

use crate::chase;
use crate::journal::{Journal, JournalError};
use crate::server::Server;
use crate::world::MICROS_PER_SECOND;

/// What a transmitter puts out, until a ship has a radio to specify.
///
/// A megawatt, which is a large dish and not an absurd one. It sets the range at which a
/// shout is heard and therefore how far a tight beam is worth the trouble; the ratio between
/// the two is [`lc_world::signal::Beam::gain`] and does not depend on this number.
pub const SIGNAL_POWER_W: f64 = 1.0e6;

/// The conversation half of an order, before its event has an identifier.
///
/// Held apart from the event because the two are written to different places for different
/// reasons — see [`crate::journal::Journal::record`] — and because the identifier is minted
/// after the order is validated.
pub(crate) struct Utterance {
    to: ShipId,
    sealed: bool,
    /// A key offer, which is a message with nothing in it.
    key: bool,
    body: String,
    acks: Vec<i64>,
}

/// What a transmission order becomes: an event to write, and a line for the transcript.
pub(crate) struct Transmission {
    pub kind: i16,
    pub payload: String,
    pub beam: Beam,
    pub said: Utterance,
    /// The order as applied, which for these two is the order as sent: there is nothing about
    /// a message the server clamps.
    pub applied: Order,
}

impl<J: Journal> Server<J> {
    /// Validate a [`Order::Say`] or [`Order::OfferKey`] and work out what it puts on the air.
    ///
    /// Nothing is validated about the addressee, and that is deliberate on both counts. A
    /// client may address a craft that does not exist: the message goes out and nobody it
    /// reaches is the addressee, which is the right answer. It is also the *safe* one —
    /// refusing an unknown identifier would turn this order into a probe for which ships the
    /// world holds.
    pub(crate) fn compose(
        &self,
        id: CraftId,
        from: ShipId,
        order: &Order,
        at: i64,
    ) -> Result<Transmission, Refusal> {
        match order {
            Order::Say { to, aim, secrecy, body } => {
                if body.is_empty() || body.len() > MESSAGE_LIMIT || *to == from {
                    return Err(Refusal::Impossible);
                }
                let sealed = matches!(secrecy, Secrecy::Sealed);
                if sealed && !self.holds_key(id, *to, at) {
                    return Err(Refusal::NoKey);
                }
                let beam = self.beam_for(id, aim, at)?;
                let acks = self.acks_for(id, *to, at);
                let spoken =
                    Spoken { to: to.0, sealed, body: Some(body.clone()), acks: acks.clone() };
                Ok(Transmission {
                    kind: lc_proto::kind::MESSAGE,
                    payload: serde_json::to_string(&spoken).unwrap_or_else(|_| "{}".into()),
                    beam,
                    said: Utterance {
                        to: *to,
                        sealed,
                        key: false,
                        body: body.clone(),
                        acks,
                    },
                    applied: order.clone(),
                })
            }
            Order::OfferKey { to, aim } => {
                if *to == from {
                    return Err(Refusal::Impossible);
                }
                let beam = self.beam_for(id, aim, at)?;
                let spoken = Spoken {
                    to: to.0,
                    sealed: false,
                    body: Some(String::new()),
                    acks: Vec::new(),
                };
                Ok(Transmission {
                    kind: lc_proto::kind::KEY,
                    payload: serde_json::to_string(&spoken).unwrap_or_else(|_| "{}".into()),
                    beam,
                    said: Utterance {
                        to: *to,
                        sealed: false,
                        key: true,
                        body: String::new(),
                        acks: Vec::new(),
                    },
                    applied: order.clone(),
                })
            }
            // Unreachable: `act` sends only the two above here.
            _ => Err(Refusal::Impossible),
        }
    }

    /// Write a transmission into the conversations it belongs to, and schedule what it teaches.
    ///
    /// Everything here is stamped with the time the light **lands**, never the time it left.
    /// The rows exist from the moment the signal does — there is nothing to compute later, and
    /// a craft with nobody flying it is served by the same path as one with a pilot watching —
    /// and every read of them is gated on the clock having reached the arrival. That is the
    /// same bargain the deliveries table makes, in a second place where it happens to be the
    /// whole of a game mechanic rather than an optimisation.
    pub(crate) fn remember(
        &mut self,
        event_id: i64,
        sender: CraftId,
        at: i64,
        said: &Utterance,
        landings: &[(CraftId, i64)],
    ) {
        self.said.push(lc_store::chat::Message {
            event_id,
            sender: sender.0,
            addressee: said.to.0,
            sealed: said.sealed,
            is_key: said.key,
            body: said.body.clone(),
            acks: said.acks.clone(),
            sent_t: at,
        });
        for (observer, arrive_t) in landings {
            if *observer == sender {
                continue;
            }
            self.receipts.push(lc_store::chat::Receipt {
                event_id,
                observer: observer.0,
                arrive_t: *arrive_t,
            });
            if said.key {
                // Anyone the offer reaches learns the key, addressee or not. That is what
                // "omnidirectional" costs, and it is the reason to point one at somebody.
                self.taught.push(lc_store::chat::Held {
                    holder: observer.0,
                    subject: sender.0,
                    learnt_t: *arrive_t,
                });
                self.keys
                    .entry(*observer)
                    .or_default()
                    .entry(ShipId(sender.0))
                    .and_modify(|learnt| *learnt = (*learnt).min(*arrive_t))
                    .or_insert(*arrive_t);
            } else {
                let window = self.heard.entry((*observer, ShipId(sender.0))).or_default();
                let place = window.partition_point(|(landed, _)| *landed <= *arrive_t);
                window.insert(place, (*arrive_t, event_id));
            }
        }
    }

    /// Whether `holder` may seal a message to `subject` at coordinate time `now`.
    ///
    /// The offer having been *sent* is not enough. Its light has to have arrived, which across
    /// four light-years is four years later, and that delay is the point of the mechanic rather
    /// than a consequence of it.
    pub(crate) fn holds_key(&self, holder: CraftId, subject: ShipId, now: i64) -> bool {
        self.keys
            .get(&holder)
            .and_then(|held| held.get(&subject))
            .is_some_and(|learnt| *learnt <= now)
    }

    /// The identifiers a message from `sender` to `to` acknowledges: the newest of the
    /// addressee's messages that have actually reached the sender by `now`.
    ///
    /// Safe to put in a payload a bystander may read, and it is worth being exact about why. An
    /// acknowledged message was emitted, arrived at the sender, and was then followed by this
    /// transmission — three links of a causal chain — so anything that hears *this* already has
    /// the acknowledged emission in its past light cone. There is nothing here a receiver
    /// learns sooner than it could have.
    pub(crate) fn acks_for(&self, sender: CraftId, to: ShipId, now: i64) -> Vec<i64> {
        let Some(window) = self.heard.get(&(sender, to)) else { return Vec::new() };
        let landed = window.partition_point(|(arrive_t, _)| *arrive_t <= now);
        window[landed.saturating_sub(ACK_DEPTH)..landed].iter().map(|(_, id)| *id).collect()
    }

    /// Which way a transmission is pointed, and how wide.
    ///
    /// The aim at a craft is the **advanced**-time solve of `lc_world::signal`, run against the
    /// sender's own sighting of it — which is stale, and is extrapolated forward across the
    /// flight time, so the aim is built on roughly twice the light delay in guesswork. A quarry
    /// that manoeuvres in between is missed, and the message goes past it into empty space.
    pub(crate) fn beam_for(&self, sender: CraftId, aim: &Aim, at: i64) -> Result<Beam, Refusal> {
        let Aim::Omni = aim else {
            let from = self.fleet.get(sender).ok_or(Refusal::NotYours)?.position_at(at as f64);
            let axis = match aim {
                // Unreachable: the `let else` above is this arm.
                Aim::Omni => return Ok(Beam::OMNI),
                Aim::Ship(target) => {
                    let seen = chase::sighting(&self.fleet, sender, *target, at)
                        .ok_or(Refusal::NotInSight)?;
                    lc_world::signal::aim_at(
                        from,
                        at as f64,
                        seen.position_ly * LIGHT_US_PER_LY,
                        seen.beta,
                        seen.emitted_s * MICROS_PER_SECOND as f64,
                    )
                    .ok_or(Refusal::Impossible)?
                }
                // A star does not manoeuvre, so this is the one aim that always lands — on
                // everybody in the system, which is what it is for.
                Aim::Star(star) => {
                    let to = self.world.star_at(*star).ok_or(Refusal::Impossible)?;
                    (to * LIGHT_US_PER_LY - from).try_normalize().ok_or(Refusal::Impossible)?
                }
            };
            return Ok(Beam::along(axis, Transmitter::SHIP.half_angle_rad()));
        };
        Ok(Beam::OMNI)
    }

    /// Hand this tick's transmissions to the journal, beside the events but not with them.
    ///
    /// Drained rather than copied, so a write that fails loses the tick's conversations rather
    /// than repeating them for ever. Receipts and keys are only ever produced alongside a
    /// message, so an empty `said` means there is nothing at all to write.
    pub(crate) async fn write_conversations(&mut self) -> Result<(), JournalError> {
        if self.said.is_empty() {
            return Ok(());
        }
        let said = std::mem::take(&mut self.said);
        let receipts = std::mem::take(&mut self.receipts);
        let taught = std::mem::take(&mut self.taught);
        self.journal.record(&said, &receipts, &taught).await
    }

    /// Take up the keyring and the acknowledgement window a previous run left behind.
    ///
    /// Both are in the store already — the keyring because a key offer writes one when it is
    /// transmitted, the window because every receipt is a row — so this is a read and not a
    /// reconstruction. It is separate from [`Server::adopt`] because a checkpoint is what the
    /// world *is* and these are what has been *said*, which are kept apart on purpose.
    ///
    /// Without it a shard coming back believes nobody holds anybody's key, and the first reply
    /// anyone sends acknowledges nothing — which the far end cannot tell from its own messages
    /// having been lost, and that is the one thing an acknowledgement exists to rule out.
    pub async fn resume_conversations(&mut self) -> Result<(), JournalError> {
        for held in self.journal.keyring().await? {
            self.keys
                .entry(CraftId(held.holder))
                .or_default()
                .entry(ShipId(held.subject))
                .and_modify(|learnt| *learnt = (*learnt).min(held.learnt_t))
                .or_insert(held.learnt_t);
        }
        // Arrival times are not read back: the window is already ordered oldest-arrival-first
        // by the query, and everything in it has landed or it would not be a receipt. What is
        // *not* in it is anything still in flight, which is exactly what must not be.
        for (observer, sender, event_id) in self.journal.ack_window().await? {
            let window = self.heard.entry((CraftId(observer), ShipId(sender))).or_default();
            // Stamped at the clock this shard came back on rather than at the arrival it
            // actually had. The window is read by [`Server::acks_for`], which asks only
            // whether a message has landed by now, and every one of these has.
            window.push((self.now_t, event_id));
            let excess = window.len().saturating_sub(ACK_DEPTH);
            window.drain(..excess);
        }
        Ok(())
    }

    /// Hand a connection the transcript its ship is party to, once.
    ///
    /// Called from the flush and not from the sign-in, because reading it is a query and a
    /// sign-in is not allowed to be one. Marked as sent before the read, so a store that keeps
    /// failing costs one attempt per connection rather than one per tick for ever.
    pub(crate) async fn send_backlog(
        &mut self,
        client: lc_proto::ClientId,
        ship: ShipId,
        now: i64,
        wire: &mut impl crate::transport::Transport,
    ) {
        if let Some(mine) = self.clients.get_mut(&client) {
            mine.backlog_sent = true;
        }
        match self.backlog(ship, now).await {
            // Nothing to say is said by not saying it, as an empty contact list is. A client's
            // log starts empty, so an empty backlog would be a message whose only content is
            // that there was no message.
            Ok(Some(backlog)) => wire.send(client, backlog),
            Ok(None) => {}
            // Not fatal and not retried. A shard with no store is the development case, and a
            // conversation nobody can read back is worth less than a connection that works.
            Err(why) => eprintln!("could not read {ship:?}'s transcript: {why}"),
        }
    }

    /// This ship's own copy of every conversation it is party to, as of `now`.
    ///
    /// Gated on arrival, not on emission. Everything the ship *sent* is here whatever the
    /// clock says — a sender knows what it transmitted the moment it transmits it — and
    /// everything it was told is here only once the light has landed. A message still in
    /// flight toward this ship is in the store and is not in this.
    ///
    /// Sealed messages this ship is not the addressee of come back with no body, exactly as
    /// they arrived. [`redact`] is the rule and this is the second place it applies, because a
    /// backlog is a second path out — which is a thing worth being uneasy about, and the reason
    /// both paths call one function rather than each deciding for itself.
    pub(crate) async fn backlog(&self, ship: ShipId, now: i64) -> Result<Option<Outbound>, JournalError> {
        let transcript = self.journal.transcript(ship).await?;
        let name_of = |id: i64| {
            self.fleet
                .get(CraftId(id))
                .map(|craft| craft.designation())
                .unwrap_or_else(|| format!("ship {id}"))
        };
        let mut messages: Vec<Said> = Vec::new();
        for m in transcript.sent {
            messages.push(Said {
                event_id: m.event_id,
                with: ShipId(m.addressee),
                with_name: name_of(m.addressee),
                mine: true,
                key: m.is_key,
                sealed: m.sealed,
                body: Some(m.body),
                acks: m.acks,
                sent_t: m.sent_t,
                arrive_t: None,
            });
        }
        for (m, arrive_t) in transcript.heard {
            if arrive_t > now {
                continue;
            }
            let readable = !m.sealed || m.addressee == ship.0;
            messages.push(Said {
                event_id: m.event_id,
                with: ShipId(m.sender),
                with_name: name_of(m.sender),
                mine: false,
                key: m.is_key,
                sealed: m.sealed,
                body: readable.then_some(m.body),
                acks: m.acks,
                sent_t: m.sent_t,
                arrive_t: Some(arrive_t),
            });
        }
        // By when this ship knew of each, which for a conversation across light delay is not
        // the order they were sent in: a reply can be composed before the message it crosses.
        messages.sort_by_key(|m| m.arrive_t.unwrap_or(m.sent_t));
        // From memory rather than from the transcript, because the keyring is what decides
        // whether an order may be sealed and that decision is synchronous. One copy of it.
        let keys = self
            .keys
            .get(&CraftId(ship.0))
            .into_iter()
            .flatten()
            .filter(|(_, learnt)| **learnt <= now)
            .map(|(subject, _)| *subject)
            .collect::<Vec<_>>();
        if messages.is_empty() && keys.is_empty() {
            return Ok(None);
        }
        Ok(Some(Outbound::Backlog { messages, keys }))
    }
}

/// A message payload as this observer may have it.
///
/// **The sealing, and the whole of it.** One event is stored with its body intact and each
/// receiver is handed a copy with the body removed unless they are the addressee. Doing it here
/// rather than at write time is what keeps one transmission one event: a sealed message
/// duplicated per receiver would be several events sharing an emission time, and everything
/// about the light cone rests on an event being a point.
///
/// An eavesdropper still learns that *something* was sent, by whom, to whom, and when — which
/// is the truth about a signal you cannot decrypt, not a concession. What they do not get is the
/// text.
///
/// Anything but a message passes through untouched, and a payload that will not parse does too:
/// this is a filter on what is released, not a validator of what was written.
pub(crate) fn redact(kind: i16, payload: &str, observer: ShipId) -> String {
    if kind != lc_proto::kind::MESSAGE && kind != lc_proto::kind::KEY {
        return payload.to_string();
    }
    let Ok(mut spoken) = serde_json::from_str::<Spoken>(payload) else {
        return payload.to_string();
    };
    if spoken.sealed && spoken.to != observer.0 {
        spoken.body = None;
    }
    serde_json::to_string(&spoken).unwrap_or_else(|_| payload.to_string())
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, Outbound, Sighting};

    use super::*;
    use crate::journal::Memory;
    use crate::server::TICK_US;
    use crate::transport::Loopback;
    use crate::world::still;

    /// Two light-hours, in light-microseconds. Far enough that the delay is many ticks.
    const TWO_LIGHT_HOURS: f64 = 7_200.0 * 1_000_000.0;

    fn sightings(messages: &[Outbound]) -> Vec<&Sighting> {
        messages
            .iter()
            .flat_map(|m| match m {
                Outbound::Sightings(list) => list.iter().map(|c| c.get()).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn spoken(messages: &[Outbound]) -> Vec<(i64, Spoken)> {
        sightings(messages)
            .into_iter()
            .filter(|s| s.kind == lc_proto::kind::MESSAGE || s.kind == lc_proto::kind::KEY)
            .filter_map(|s| Some((s.event_id, serde_json::from_str(&s.payload).ok()?)))
            .collect()
    }

    fn say(to: i64, aim: Aim, secrecy: Secrecy, body: &str) -> Order {
        Order::Say { to: ShipId(to), aim, secrecy, body: body.into() }
    }

    /// Two ships a light-hour apart and a third beside the second. An open message is read by
    /// both; a sealed one is read by its addressee and by nobody else.
    ///
    /// The eavesdropper still *hears* it — it is a signal falling on an antenna, and pretending
    /// otherwise would let a player learn that nothing was transmitted by not being told.
    #[tokio::test]
    async fn a_sealed_message_reaches_an_eavesdropper_with_no_body() {
        let mut wire = Loopback::new();
        let mut server = Server::new(Memory::default(), 0, 1);
        let (ada, bry, nosy) = (ClientId(1), ClientId(2), ClientId(3));
        let far = DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0);
        server.admit(ada, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(bry, crate::world::still(ShipId(2), far), 0.0);
        // A hundred light-seconds off the axis, so an omnidirectional shout reaches it.
        server.admit(nosy, crate::world::still(ShipId(3), far + DVec3::new(0.0, 1.0e8, 0.0)), 0.0);

        // Bry offers a key first, and the message waits for its light.
        wire.client_says(bry, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: Order::OfferKey { to: ShipId(1), aim: Aim::Omni },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        // Sealing before the key has landed is refused, and that is the mechanic.
        wire.client_says(ada, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: say(2, Aim::Omni, Secrecy::Sealed, "too soon"),
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert!(
            wire.take(ada).iter().any(|m| matches!(
                m,
                Outbound::Refused { reason: Refusal::NoKey, .. }
            )),
            "a key was usable before its light arrived",
        );

        // Run the clock past the key's arrival.
        while server.now_t() < TWO_LIGHT_HOURS as i64 + TICK_US {
            server.tick(&mut wire).await.unwrap();
        }
        wire.take(ada);
        wire.take(bry);
        wire.take(nosy);

        wire.client_says(ada, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: say(2, Aim::Omni, Secrecy::Sealed, "for you alone"),
            issued_at_client_t: server.now_t(),
        }));
        server.tick(&mut wire).await.unwrap();
        let sealed_at = server.now_t();
        while server.now_t() < sealed_at + TWO_LIGHT_HOURS as i64 + TICK_US * 2 {
            server.tick(&mut wire).await.unwrap();
        }

        let to_bry = spoken(&wire.take(bry));
        let (_, mine) = to_bry.iter().find(|(_, s)| s.sealed).expect("bry heard it");
        assert_eq!(mine.body.as_deref(), Some("for you alone"));

        let to_nosy = spoken(&wire.take(nosy));
        let (_, theirs) = to_nosy.iter().find(|(_, s)| s.sealed).expect("the eavesdropper heard it");
        assert_eq!(theirs.body, None, "an eavesdropper read a sealed message");
        assert_eq!(theirs.to, 2, "and could still see who it was for");
    }

    /// A beam is aimed and a bystander off it gets nothing at all — not a faint copy.
    #[tokio::test]
    async fn a_tight_beam_is_not_heard_off_its_axis() {
        let mut wire = Loopback::new();
        let mut server = Server::new(Memory::default(), 0, 1);
        let (ada, bry, nosy) = (ClientId(1), ClientId(2), ClientId(3));
        let far = DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0);
        server.admit(ada, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(bry, crate::world::still(ShipId(2), far), 0.0);
        // Well outside a milliradian at this range: the beam's whole spot is 7200 light-
        // microseconds across and this is a hundred million off the axis.
        server.admit(nosy, crate::world::still(ShipId(3), far + DVec3::new(0.0, 1.0e8, 0.0)), 0.0);

        wire.client_says(ada, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: say(2, Aim::Ship(ShipId(2)), Secrecy::Open, "just for you"),
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        while server.now_t() < TWO_LIGHT_HOURS as i64 + TICK_US * 2 {
            server.tick(&mut wire).await.unwrap();
        }

        assert_eq!(spoken(&wire.take(bry)).len(), 1, "the addressee did not hear the beam");
        assert!(spoken(&wire.take(nosy)).is_empty(), "a bystander heard a beam it was not on");
    }

    /// The acknowledgement rides back with the reply, names the message by its identifier, and
    /// covers no more than [`ACK_DEPTH`] of them.
    #[tokio::test]
    async fn a_reply_acknowledges_what_actually_arrived_and_nothing_else() {
        let mut wire = Loopback::new();
        let mut server = Server::new(Memory::default(), 0, 1);
        let (ada, bry) = (ClientId(1), ClientId(2));
        let far = DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0);
        server.admit(ada, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(bry, crate::world::still(ShipId(2), far), 0.0);

        let mut sent = Vec::new();
        for k in 0..(ACK_DEPTH + 3) {
            wire.client_says(ada, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: say(2, Aim::Omni, Secrecy::Open, &format!("message {k}")),
                issued_at_client_t: server.now_t(),
            }));
            server.tick(&mut wire).await.unwrap();
            let accepted = wire
                .take(ada)
                .into_iter()
                .find_map(|m| match m {
                    Outbound::Accepted { event_id, order: Order::Say { .. }, .. } => {
                        Some(event_id)
                    }
                    _ => None,
                })
                .expect("the message was accepted");
            sent.push(accepted);
        }

        // Bry replies before any of it has arrived: nothing to acknowledge.
        wire.client_says(bry, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: say(1, Aim::Omni, Secrecy::Open, "crossed in flight"),
            issued_at_client_t: server.now_t(),
        }));
        server.tick(&mut wire).await.unwrap();
        wire.take(ada);
        wire.take(bry);

        while server.now_t() < TWO_LIGHT_HOURS as i64 + TICK_US * (ACK_DEPTH as i64 + 6) {
            server.tick(&mut wire).await.unwrap();
        }
        let early = spoken(&wire.take(ada));
        let (_, crossed) = early.first().expect("the crossing reply arrived");
        assert!(crossed.acks.is_empty(), "a reply acknowledged light still in flight");

        // Now everything has landed, and the next reply says so.
        wire.client_says(bry, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: say(1, Aim::Omni, Secrecy::Open, "all received"),
            issued_at_client_t: server.now_t(),
        }));
        server.tick(&mut wire).await.unwrap();
        let at = server.now_t();
        while server.now_t() < at + TWO_LIGHT_HOURS as i64 + TICK_US * 2 {
            server.tick(&mut wire).await.unwrap();
        }
        let late = spoken(&wire.take(ada));
        let (_, answered) = late.last().expect("the second reply arrived");
        assert_eq!(answered.acks.len(), ACK_DEPTH, "the window is not bounded");
        assert_eq!(
            answered.acks,
            sent[sent.len() - ACK_DEPTH..],
            "the newest ten, by identifier, oldest first",
        );
    }

    /// A ship that signs in again is handed both halves of its conversation, and a sealed
    /// message it was never the addressee of still has no body in it.
    #[tokio::test]
    async fn a_reconnection_is_handed_the_transcript_it_left_behind() {
        let mut wire = Loopback::new();
        let mut server = Server::new(Memory::default(), 0, 1);
        let (ada, bry) = (ClientId(1), ClientId(2));
        server.admit(ada, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(bry, crate::world::still(ShipId(2), DVec3::new(1.0e6, 0.0, 0.0)), 0.0);

        wire.client_says(ada, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: say(2, Aim::Omni, Secrecy::Open, "anyone there"),
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        for _ in 0..4 {
            server.tick(&mut wire).await.unwrap();
        }
        wire.take(ada);
        wire.take(bry);

        // A second connection for the same ship, which is what a reconnection is.
        let again = ClientId(9);
        server.admit(again, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.tick(&mut wire).await.unwrap();
        let backlog = wire
            .take(again)
            .into_iter()
            .find_map(|m| match m {
                Outbound::Backlog { messages, .. } => Some(messages),
                _ => None,
            })
            .expect("no transcript on reconnecting");
        assert_eq!(backlog.len(), 1);
        assert!(backlog[0].mine, "its own message came back as somebody else's");
        assert_eq!(backlog[0].body.as_deref(), Some("anyone there"));
        assert_eq!(backlog[0].arrive_t, None, "a sender does not hear its own signal");
    }

    /// Talking to yourself, an empty message and one past the limit are all refused, and none
    /// of them becomes an event.
    #[tokio::test]
    async fn a_message_that_cannot_be_sent_is_refused_rather_than_written() {
        let mut wire = Loopback::new();
        let mut server = Server::new(Memory::default(), 0, 1);
        let client = ClientId(1);
        server.admit(client, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

        for order in [
            say(1, Aim::Omni, Secrecy::Open, "hello me"),
            say(2, Aim::Omni, Secrecy::Open, ""),
            say(2, Aim::Omni, Secrecy::Open, &"x".repeat(MESSAGE_LIMIT + 1)),
            // Nothing in sight to point at: the same answer an intercept gives.
            say(2, Aim::Ship(ShipId(2)), Secrecy::Open, "where are you"),
        ] {
            wire.client_says(client, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: order.clone(),
                issued_at_client_t: 0,
            }));
            server.tick(&mut wire).await.unwrap();
            assert!(
                wire.take(client).iter().any(|m| matches!(m, Outbound::Refused { .. })),
                "{order:?} was not refused",
            );
        }
        assert!(server.journal().messages.is_empty(), "a refused message was written down");
    }
}
