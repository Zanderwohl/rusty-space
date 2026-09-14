//! The tick loop, intent validation, and the one place anything is released to a client.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::{
    ClientId, Cleared, Inbound, Intent, Order, Outbound, PROTOCOL_VERSION, Refusal, ShipId,
    Sighting, Withheld,
};
use lc_store::id::Minter;

use crate::journal::{Journal, JournalError, PREPARE_AHEAD_US};
use crate::rate::Budget;
use crate::transport::Transport;
use crate::world::{Event, Scheduled, Ship, schedule};
use lc_world::motion::ShipState;

/// Real milliseconds a tick covers.
pub const TICK_MS: i64 = 50;

/// Coordinate microseconds a tick covers: fifty milliseconds at the design rate.
///
/// Only step 1 of the tick is affected by this, and it affects nothing, because bodies are
/// propagated analytically. A server that drops to 10 Hz produces identical world state with
/// coarser event timestamps -- the step never enters an integrator, so it cannot accumulate.
pub const TICK_US: i64 = TICK_MS * 8766 * 1_000;

/// Ticks in a real second, for the window the rate counters measure over.
pub const TICKS_PER_SECOND: u32 = (1_000 / TICK_MS) as u32;

/// What the server remembers about one connection.
#[derive(Clone, Debug)]
pub struct Connected {
    pub ship: ShipId,
    /// The arrival time of the last sighting this client was actually sent.
    ///
    /// Not "the last thing that happened": what a client can prove it knows. An intent stamped
    /// earlier than this is a claim to have acted on information the client did not have.
    pub last_reception_t: i64,
    /// How far its delivery stream has been read.
    pub cursor_t: i64,
}

pub struct Server<J: Journal> {
    now_t: i64,
    ships: Vec<Ship>,
    clients: HashMap<ClientId, Connected>,
    journal: J,
    minter: Minter,
    /// Written by this tick, and handed to the journal at the end of it.
    pending: Vec<Event>,
    /// What each connection is allowed to send. Kept per `ClientId` rather than per admitted
    /// client, so a connection that has not been given a ship still cannot flood.
    budgets: HashMap<ClientId, Budget>,
}

impl<J: Journal> Server<J> {
    /// A world at coordinate time `start_t`, with a shard identifier for its event ids.
    pub fn new(journal: J, start_t: i64, shard: u64) -> Self {
        Self {
            now_t: start_t,
            ships: Vec::new(),
            clients: HashMap::new(),
            journal,
            minter: Minter::new(shard).expect("a shard inside the identifier's field"),
            pending: Vec::new(),
            budgets: HashMap::new(),
        }
    }

    pub fn now_t(&self) -> i64 {
        self.now_t
    }

    pub fn journal(&self) -> &J {
        &self.journal
    }

    pub fn ship(&self, id: ShipId) -> Option<&Ship> {
        self.ships.iter().find(|s| s.id == id)
    }

    /// What a client has actually sent. The measurement that will one day replace the guess in
    /// [`crate::rate`] with evidence.
    pub fn usage(&self, client: ClientId) -> Option<crate::rate::Usage> {
        self.budgets.get(&client).map(|b| b.usage)
    }

    /// Put a ship in the world and give it to a client.
    ///
    /// The identifier comes from the caller rather than from here, because who is connected is
    /// the transport's fact: a connection exists before the world has anything to say about it,
    /// and authentication will one day decide what it is called.
    pub fn admit(&mut self, owner: ClientId, ship_id: ShipId, motion: ShipState, noise_floor: f32) {
        self.ships.push(Ship { id: ship_id, owner, motion, system: None, noise_floor });
        self.clients.insert(owner, Connected {
            ship: ship_id,
            // Nothing received yet, so nothing is provable: an intent may be stamped anywhere
            // from the beginning of time up to now.
            last_reception_t: i64::MIN,
            // And nothing sent yet. Not `now_t`, which would mean "told everything up to this
            // instant" and would swallow an event stamped at exactly this instant -- a ship's
            // own act, on the tick it acts. The same start serves a brand-new client, which is
            // the catch-up path run from the beginning.
            cursor_t: i64::MIN,
        });
    }

    /// One tick. The order is the whole of it.
    pub async fn tick(&mut self, wire: &mut impl Transport) -> Result<(), JournalError> {
        // 1. Advance.
        self.now_t += TICK_US;
        // Room to write into, kept ahead rather than made on demand. Cheap: the journal holds
        // the range it has already made and this is a comparison until the window moves.
        self.journal.prepare(self.now_t, self.now_t + PREPARE_AHEAD_US).await?;
        // 2. Drain intents, validate, write events, schedule deliveries.
        let mut events = Vec::new();
        let mut deliveries = Vec::new();
        for (from, message) in wire.poll() {
            // Charged before the message is read, so a malformed one costs its sender as much
            // as a valid one and there is nothing to gain by sending rubbish quickly.
            let budget = self.budgets.entry(from).or_insert_with(|| Budget::new(TICK_MS));
            if !budget.charge() {
                let retry_after_ticks = budget.retry_after_ticks();
                wire.send(from, Outbound::Throttled { retry_after_ticks });
                continue;
            }
            self.handle(from, message, wire, &mut events, &mut deliveries);
        }
        for budget in self.budgets.values_mut() {
            budget.advance(TICKS_PER_SECOND);
        }
        self.journal.write(&events, &deliveries).await?;
        self.pending = events;
        // 3 and 4. Everything that has arrived since the last tick, through the gate.
        self.flush(wire).await
    }

    fn handle(
        &mut self,
        from: ClientId,
        message: Inbound,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        match message {
            Inbound::Hello { protocol } => {
                if protocol != PROTOCOL_VERSION {
                    wire.send(from, Outbound::WrongProtocol { server: PROTOCOL_VERSION });
                    return;
                }
                if let Some(state) = self.clients.get(&from) {
                    wire.send(from, Outbound::Welcome {
                        client_id: from,
                        protocol: PROTOCOL_VERSION,
                        ship_id: state.ship,
                        now_t: self.now_t,
                    });
                }
            }
            Inbound::Act(intent) => {
                if let Err(reason) = self.act(from, intent, events, deliveries) {
                    wire.send(from, Outbound::Refused { ship_id: intent.ship_id, reason });
                }
            }
            Inbound::ResumeFrom { arrive_t } => {
                // A client that missed an hour missed eight thousand in-game hours. Winding its
                // cursor back is the whole of catch-up; the next flush replays from there.
                if let Some(state) = self.clients.get_mut(&from) {
                    state.cursor_t = arrive_t.min(self.now_t);
                }
            }
        }
    }

    /// Validate an intent, and if it stands, make it an event.
    fn act(
        &mut self,
        from: ClientId,
        intent: Intent,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<(), Refusal> {
        let state = self.clients.get(&from).ok_or(Refusal::NotYours)?;
        if state.ship != intent.ship_id {
            return Err(Refusal::NotYours);
        }
        let index = self
            .ships
            .iter()
            .position(|s| s.id == intent.ship_id && s.owner == from)
            .ok_or(Refusal::NotYours)?;

        // The clamp. Not later than now, and not earlier than the moment this client's stream
        // has already been resolved to.
        //
        // Stronger than doc 08's rule, which floors at the last event the client can prove it
        // received, and stronger for a reason the rule alone does not cover: a delivery is
        // matched against a cursor that advances every tick, so an event stamped behind that
        // cursor would be written, scheduled, and then never looked at again. The client's own
        // transmission would vanish. The last reception is always at or behind the cursor, so
        // this floor implies the doc's and adds the part that keeps deliveries findable.
        let floor = state.cursor_t.saturating_add(1);
        let at = intent.issued_at_client_t.clamp(floor.min(self.now_t), self.now_t);

        let (kind, power_w, payload) = match intent.order {
            Order::Transmit { power_w } => {
                // `<= 0.0` is false for NaN, so the finite check is not redundant with it.
                if power_w <= 0.0 || !power_w.is_finite() {
                    return Err(Refusal::Impossible);
                }
                (KIND_TRANSMIT, power_w, format!("{{\"power_w\":{power_w}}}"))
            }
            Order::Burn { beta } => {
                let beta = DVec3::from_array(beta);
                if !beta.is_finite() || beta.length() >= 1.0 {
                    return Err(Refusal::Impossible);
                }
                let here = self.ships[index].position_at(at as f64);
                self.ships[index].motion = crate::world::coasting(here, beta, at);
                // A burn is not silent -- it is the most visible thing a ship does -- but what
                // it radiates is the drive's business. Nominal, until there is a drive model.
                (KIND_BURN, BURN_POWER_W, format!("{{\"beta\":{beta:?}}}"))
            }
        };

        let source = self.ships[index].id;
        let at_position = self.ships[index].position_at(at as f64);
        let id = self.minter.mint(at).ok_or(Refusal::Impossible)?.get();
        let event = Event { id, source, t: at, at: at_position, kind, power_w, payload };
        for observer in &self.ships {
            if let Some(scheduled) = schedule(&event, observer) {
                deliveries.push(scheduled);
            }
        }
        events.push(event);
        Ok(())
    }

    /// Release what has arrived. **The only place anything reaches a client.**
    ///
    /// Every sighting here goes through [`Cleared::clear`], which is the only constructor of
    /// the only type [`Outbound::Sightings`] can hold. Adding a second path out would mean
    /// adding a second way to build a `Cleared`, and there is not one.
    async fn flush(&mut self, wire: &mut impl Transport) -> Result<(), JournalError> {
        let now = self.now_t;
        // Cloned out first: the journal read borrows `self`, and the state update writes it.
        let connections: Vec<(ClientId, Connected)> =
            self.clients.iter().map(|(id, state)| (*id, state.clone())).collect();

        for (id, state) in connections {
            let Some(ship) = self.ships.iter().find(|s| s.id == state.ship).cloned() else {
                continue;
            };
            // The proven read: one range scan over `(observer_id, arrive_t)`, already ordered.
            let due = self.journal.due(ship.id, state.cursor_t, now).await?;

            let mut cleared = Vec::new();
            let mut latest = state.last_reception_t;
            for (scheduled, event) in due {
                let direction =
                    (event.at - ship.position_at(scheduled.arrive_t as f64)).normalize_or_zero();
                let sighting = Sighting {
                    event_id: event.id,
                    source_id: event.source.0,
                    arrive_t: scheduled.arrive_t,
                    emitted_t: event.t,
                    direction: direction.to_array(),
                    strength: scheduled.strength,
                    kind: event.kind,
                    payload: event.payload.clone(),
                };
                match Cleared::clear(sighting, now, ship.noise_floor) {
                    Ok(pass) => {
                        latest = latest.max(scheduled.arrive_t);
                        cleared.push(pass);
                    }
                    // Withheld for either reason, and the client is told nothing either way.
                    // A client that could tell the two apart could tell that *something*
                    // happened it was not told about, which is faster-than-light information.
                    Err(Withheld::StillInFlight | Withheld::BelowNoiseFloor) => {}
                }
            }
            if let Some(mine) = self.clients.get_mut(&id) {
                mine.cursor_t = now;
                mine.last_reception_t = latest;
            }
            if !cleared.is_empty() {
                wire.send(id, Outbound::Sightings(cleared));
            }
        }
        Ok(())
    }
}

/// Event kinds. Small integers on the wire; named here.
pub const KIND_TRANSMIT: i16 = 1;
pub const KIND_BURN: i16 = 2;

/// What a burn radiates, until there is a drive model to ask.
pub const BURN_POWER_W: f64 = 1.0e12;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Memory;
use crate::transport::Loopback;

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

    /// **The acceptance criterion.** Two clients, one acts, and the other learns about it at
    /// light delay and not before.
    ///
    /// The negative half is the one that matters: it is asserted on every tick from the act
    /// until the light arrives, not merely at the end. A gate that released everything one tick
    /// early would pass a test that only looked at the end.
    #[tokio::test]
    async fn one_client_acts_and_the_other_learns_at_light_delay_and_not_before() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let actor = ClientId(1);
        server.admit(actor, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);
        let watcher = ClientId(2);
        server
            .admit(watcher, ShipId(2), crate::world::still(DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)), 0.0);

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0e20 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        let emitted = server.journal().events[0].t;
        let arrives = emitted + TWO_LIGHT_HOURS as i64;
        assert!(arrives > server.now_t() + TICK_US * 8, "the delay is not worth testing");

        // The actor is standing where it happened, so it knows at once.
        assert_eq!(sightings(&wire.take(actor)).len(), 1, "a ship cannot be late to its own act");

        let mut told_at = None;
        for _ in 0..2_000 {
            let before = server.now_t();
            server.tick(&mut wire).await.unwrap();
            let seen = wire.take(watcher);
            let list = sightings(&seen);
            if list.is_empty() {
                assert!(
                    server.now_t() < arrives,
                    "nothing arrived by {} but the light landed at {arrives}",
                    server.now_t(),
                );
                continue;
            }
            // The negative case, asserted where it can actually fail.
            assert!(
                before < arrives && arrives <= server.now_t(),
                "told at t in ({before}, {}] but the light arrives at {arrives}",
                server.now_t(),
            );
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].emitted_t, emitted);
            assert_eq!(list[0].arrive_t, arrives);
            // And it points back the way it came.
            assert!(list[0].direction[0] < -0.999, "{:?}", list[0].direction);
            told_at = Some(server.now_t());
            break;
        }
        assert!(told_at.is_some(), "the light never arrived at all");

        // And it is said once, not on every tick after.
        for _ in 0..5 {
            server.tick(&mut wire).await.unwrap();
            assert!(sightings(&wire.take(watcher)).is_empty(), "the same sighting came twice");
        }
    }

    /// A flood is stopped, told when to come back, and costs the sender whether or not the
    /// message was any good.
    ///
    /// The ceiling is a safety valve rather than a game rule — see [`crate::rate`] — so what
    /// this checks is that it exists and is not in the way, not that it is set to the right
    /// number. Nobody knows the right number yet, which is why `usage` counts.
    #[tokio::test]
    async fn a_flood_is_throttled_and_a_legitimate_burst_is_not() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        let order = |t| Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: t,
        });

        // A burst a player could plausibly produce goes through untouched.
        for step in 0..crate::rate::BURST as i64 {
            wire.client_says(client, order(step));
        }
        server.tick(&mut wire).await.unwrap();
        let answered = wire.take(client);
        assert!(
            !answered.iter().any(|m| matches!(m, Outbound::Throttled { .. })),
            "a burst inside the allowance was throttled",
        );
        assert_eq!(server.usage(client).unwrap().refused, 0);

        // A scripted client sending ten times that in one tick is not.
        for step in 0..(crate::rate::BURST as i64 * 10) {
            wire.client_says(client, order(step));
        }
        server.tick(&mut wire).await.unwrap();
        let answered = wire.take(client);
        let throttled: Vec<u32> = answered
            .iter()
            .filter_map(|m| match m {
                Outbound::Throttled { retry_after_ticks } => Some(*retry_after_ticks),
                _ => None,
            })
            .collect();
        assert!(!throttled.is_empty(), "a flood was not stopped");
        assert!(throttled.iter().all(|t| *t > 0), "told to retry at once after being refused");

        let usage = server.usage(client).unwrap();
        assert!(usage.refused > 0);
        assert!(usage.peak_per_tick >= crate::rate::BURST as u32 * 10, "the attempt went unmeasured");
    }

    /// An unknown connection is limited too. A socket that has never been given a ship can
    /// still send, and flooding costs the server the same whether the sender owns anything.
    #[tokio::test]
    async fn a_connection_with_no_ship_is_still_limited() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let stranger = ClientId(99);
        for _ in 0..(crate::rate::BURST as i64 * 3) {
            wire.client_says(stranger, Inbound::Hello { protocol: PROTOCOL_VERSION });
        }
        server.tick(&mut wire).await.unwrap();
        assert!(server.usage(stranger).unwrap().refused > 0, "an unadmitted flood was free");
    }

    /// A ship may not act for a ship that is not its own, and may not be told that it tried.
    #[tokio::test]
    async fn a_client_cannot_act_for_a_ship_it_does_not_own() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let first = ClientId(1);
        server.admit(first, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);
        server.admit(ClientId(2), ShipId(2), crate::world::still(DVec3::ZERO), 0.0);

        wire.client_says(first, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert!(server.journal().events.is_empty(), "an event was written for someone else's ship");
        assert!(matches!(
            wire.take(first).as_slice(),
            [Outbound::Refused { reason: Refusal::NotYours, .. }]
        ));
    }

    /// The clamp. An intent stamped earlier than the last thing the client can prove it
    /// received is a claim to have acted on information it did not have; one stamped later than
    /// now is a claim about the future.
    #[tokio::test]
    async fn an_intents_timestamp_is_clamped_at_both_ends() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        // The future: clamped down to now.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: i64::MAX / 4,
        }));
        server.tick(&mut wire).await.unwrap();
        assert_eq!(server.journal().events[0].t, server.now_t(), "an intent was stamped in the future");
        let _ = wire.take(client);

        // Its own transmission is received at once, so the floor is now that arrival.
        server.tick(&mut wire).await.unwrap();
        let floor = server.journal().events[0].t;

        // The past: clamped up to what it can prove it knew.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let second = server.journal().events.last().unwrap();
        assert!(
            second.t >= floor,
            "stamped at {} but the client had already received something at {floor}",
            second.t,
        );
    }

    /// An intent that arrives several ticks in, stamped in the deep past, still gets delivered.
    ///
    /// The regression for a delivery that could be written and then never looked at. Flush
    /// advances a cursor to `now` every tick; an event clamped to a time behind that cursor is
    /// scheduled into a window already passed, and the client's own transmission disappears.
    /// It needed a few ticks to appear at all, which is why it survived the first test that
    /// acted on tick one.
    #[tokio::test]
    async fn an_intent_stamped_in_the_deep_past_is_still_delivered() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        // Let the cursor run well past where the intent claims to have been issued.
        for _ in 0..5 {
            server.tick(&mut wire).await.unwrap();
        }
        let _ = wire.take(client);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0e9 },
            issued_at_client_t: i64::MIN,
        }));
        server.tick(&mut wire).await.unwrap();

        let messages = wire.take(client);
        let told = sightings(&messages);
        assert_eq!(told.len(), 1, "the act was written and then never looked at again");
        // Clamped into this tick's window rather than into the past it asked for.
        assert!(
            told[0].emitted_t > server.now_t() - TICK_US,
            "stamped at {} but the tick began at {}",
            told[0].emitted_t,
            server.now_t() - TICK_US,
        );
        assert!(told[0].emitted_t <= server.now_t());
    }

    /// An order the world cannot carry out is refused rather than clamped into something it
    /// can. There is no nearest legal burn to a superluminal one.
    #[tokio::test]
    async fn an_impossible_order_is_refused() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        for order in [
            Order::Burn { beta: [1.0, 0.0, 0.0] },
            Order::Burn { beta: [0.9, 0.9, 0.0] },
            Order::Burn { beta: [f64::NAN, 0.0, 0.0] },
            Order::Transmit { power_w: 0.0 },
            Order::Transmit { power_w: -5.0 },
        ] {
            wire.client_says(client, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order,
                issued_at_client_t: 0,
            }));
            server.tick(&mut wire).await.unwrap();
            assert!(server.journal().events.is_empty(), "{order:?} became an event");
            assert!(
                matches!(
                    wire.take(client).as_slice(),
                    [Outbound::Refused { reason: Refusal::Impossible, .. }]
                ),
                "{order:?} was not refused",
            );
        }
        // And a burn just inside `c` is fine.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Burn { beta: [0.99, 0.0, 0.0] },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert_eq!(server.journal().events.len(), 1);
        // And the burn left the ship moving, on the shared model's own terms.
        let after = &server.ship(ShipId(1)).unwrap().motion;
        assert!(matches!(after.motive, lc_world::motion::Motive::Drifting { .. }));
        assert!((after.beta.x - 0.99).abs() < 1.0e-12, "{}", after.beta.x);
    }

    /// Arrival is not detection. A signal that reaches a receiver below its noise floor is not
    /// sent, and the client cannot tell that from nothing having happened.
    #[tokio::test]
    async fn a_signal_under_the_noise_floor_arrives_and_is_not_sent() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let actor = ClientId(1);
        server.admit(actor, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);
        let deaf = ClientId(2);
        server.admit(
            deaf,
            ShipId(2),
            crate::world::still(DVec3::new(1_000_000.0, 0.0, 0.0)),
            1.0e6,
        );

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(actor);

        // Well past the arrival, and still nothing.
        for _ in 0..4 {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(server.now_t() > 1_000_000, "the test never reached the arrival");
        assert!(
            server.journal().deliveries.iter().any(|d| d.observer == ShipId(2)),
            "it was never even scheduled, so the floor is not what stopped it",
        );
        assert!(sightings(&wire.take(deaf)).is_empty(), "a signal under the floor was sent");
    }

    /// Catch-up. A client that was away winds its cursor back and is told everything again,
    /// still subject to the same gate.
    #[tokio::test]
    async fn resuming_replays_what_was_missed_and_nothing_more() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let actor = ClientId(1);
        server.admit(actor, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0e9 },
            // Stamped at "now", so the replay below can ask for everything after zero and
            // mean it. A reception at zero is one the client already has.
            issued_at_client_t: i64::MAX / 4,
        }));
        server.tick(&mut wire).await.unwrap();
        let first = sightings(&wire.take(actor)).len();
        assert_eq!(first, 1);

        // Nothing new while it is away.
        server.tick(&mut wire).await.unwrap();
        assert!(sightings(&wire.take(actor)).is_empty());

        wire.client_says(actor, Inbound::ResumeFrom { arrive_t: 0 });
        server.tick(&mut wire).await.unwrap();
        assert_eq!(sightings(&wire.take(actor)).len(), 1, "the replay did not come back");

        // A resume past `now` cannot be used to ask for the future.
        wire.client_says(actor, Inbound::ResumeFrom { arrive_t: i64::MAX });
        server.tick(&mut wire).await.unwrap();
        assert!(sightings(&wire.take(actor)).is_empty());
    }

    #[tokio::test]
    async fn a_client_on_the_wrong_protocol_is_told_so_and_not_welcomed() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, ShipId(1), crate::world::still(DVec3::ZERO), 0.0);

        wire.client_says(client, Inbound::Hello { protocol: PROTOCOL_VERSION + 1 });
        server.tick(&mut wire).await.unwrap();
        assert!(matches!(wire.take(client).as_slice(), [Outbound::WrongProtocol { .. }]));

        wire.client_says(client, Inbound::Hello { protocol: PROTOCOL_VERSION });
        server.tick(&mut wire).await.unwrap();
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Welcome { .. }]));
    }

    /// A coarser tick produces the same world, only with coarser timestamps. The step never
    /// enters an integrator, so it cannot accumulate error.
    #[test]
    fn the_tick_size_does_not_change_where_anything_is() {
        let at = DVec3::new(500_000.0, 0.0, 0.0);
        let beta = DVec3::new(0.3, -0.1, 0.0);
        let ship = Ship {
            id: ShipId(1),
            owner: ClientId(1),
            motion: crate::world::coasting(at, beta, 0),
            system: None,
            noise_floor: 0.0,
        };
        let after = 40 * TICK_US;
        // One step or forty, the closed form is the same place to the last bit -- and it stays
        // exact across the conversion into light-years the world model works in and back.
        assert_eq!(ship.position_at(after as f64), at + beta * after as f64);
    }
}
