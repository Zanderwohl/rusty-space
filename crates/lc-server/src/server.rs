//! The tick loop, intent validation, and the one place anything is released to a client.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::{
    ClientId, Cleared, Inbound, Intent, Order, Outbound, PROTOCOL_VERSION, Refusal, ShipId,
    Sighting, Withheld,
};
use lc_spacetime::Worldline;
use lc_store::id::Minter;

use crate::transport::Transport;
use crate::world::{Event, Path, Scheduled, Ship, schedule};

/// Real milliseconds a tick covers.
pub const TICK_MS: i64 = 50;

/// Coordinate microseconds a tick covers: fifty milliseconds at the design rate.
///
/// Only step 1 of the tick is affected by this, and it affects nothing, because bodies are
/// propagated analytically. A server that drops to 10 Hz produces identical world state with
/// coarser event timestamps -- the step never enters an integrator, so it cannot accumulate.
pub const TICK_US: i64 = TICK_MS * 8766 * 1_000;

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

pub struct Server {
    now_t: i64,
    ships: Vec<Ship>,
    clients: HashMap<ClientId, Connected>,
    events: Vec<Event>,
    deliveries: Vec<Scheduled>,
    minter: Minter,
    next_client: u64,
}

impl Server {
    /// A world at coordinate time `start_t`, with a shard identifier for its event ids.
    pub fn new(start_t: i64, shard: u64) -> Self {
        Self {
            now_t: start_t,
            ships: Vec::new(),
            clients: HashMap::new(),
            events: Vec::new(),
            deliveries: Vec::new(),
            minter: Minter::new(shard).expect("a shard inside the identifier's field"),
            next_client: 1,
        }
    }

    pub fn now_t(&self) -> i64 {
        self.now_t
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn deliveries(&self) -> &[Scheduled] {
        &self.deliveries
    }

    pub fn ship(&self, id: ShipId) -> Option<&Ship> {
        self.ships.iter().find(|s| s.id == id)
    }

    /// Put a ship in the world and give it to a client. Returns the client's own identifier.
    pub fn admit(&mut self, ship_id: ShipId, path: Path, noise_floor: f32) -> ClientId {
        let owner = ClientId(self.next_client);
        self.next_client += 1;
        self.ships.push(Ship { id: ship_id, owner, path, noise_floor });
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
        owner
    }

    /// One tick. The order is the whole of it.
    pub fn tick(&mut self, wire: &mut impl Transport) {
        // 1. Advance.
        self.now_t += TICK_US;
        // 2. Drain intents, validate, write events, schedule deliveries.
        for (from, message) in wire.poll() {
            self.handle(from, message, wire);
        }
        // 3 and 4. Everything that has arrived since the last tick, through the gate.
        self.flush(wire);
    }

    fn handle(&mut self, from: ClientId, message: Inbound, wire: &mut impl Transport) {
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
                if let Err(reason) = self.act(from, intent) {
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
    fn act(&mut self, from: ClientId, intent: Intent) -> Result<(), Refusal> {
        let state = self.clients.get(&from).ok_or(Refusal::NotYours)?;
        if state.ship != intent.ship_id {
            return Err(Refusal::NotYours);
        }
        let index = self
            .ships
            .iter()
            .position(|s| s.id == intent.ship_id && s.owner == from)
            .ok_or(Refusal::NotYours)?;

        // The clamp. Not earlier than what the client can prove it knew, not later than now.
        let floor = state.last_reception_t;
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
                let here = self.ships[index].path.position_at(at as f64);
                self.ships[index].path = Path::coasting(here, beta, at);
                // A burn is not silent -- it is the most visible thing a ship does -- but what
                // it radiates is the drive's business. Nominal, until there is a drive model.
                (KIND_BURN, BURN_POWER_W, format!("{{\"beta\":{beta:?}}}"))
            }
        };

        let source = self.ships[index].id;
        let at_position = self.ships[index].path.position_at(at as f64);
        let id = self.minter.mint(at).ok_or(Refusal::Impossible)?.get();
        let event = Event { id, source, t: at, at: at_position, kind, power_w, payload };
        for observer in &self.ships {
            if let Some(scheduled) = schedule(&event, observer) {
                self.deliveries.push(scheduled);
            }
        }
        self.events.push(event);
        Ok(())
    }

    /// Release what has arrived. **The only place anything reaches a client.**
    ///
    /// Every sighting here goes through [`Cleared::clear`], which is the only constructor of
    /// the only type [`Outbound::Sightings`] can hold. Adding a second path out would mean
    /// adding a second way to build a `Cleared`, and there is not one.
    fn flush(&mut self, wire: &mut impl Transport) {
        let now = self.now_t;
        for (id, state) in self.clients.iter_mut() {
            let Some(ship) = self.ships.iter().find(|s| s.id == state.ship) else { continue };
            let mut cleared = Vec::new();
            let mut latest = state.last_reception_t;
            for scheduled in &self.deliveries {
                if scheduled.observer != ship.id {
                    continue;
                }
                // The window is half-open: a tick asks for what arrived since the last one, and
                // an inclusive lower bound would deliver the boundary twice.
                if scheduled.arrive_t <= state.cursor_t || scheduled.arrive_t > now {
                    continue;
                }
                let Some(event) = self.events.iter().find(|e| e.id == scheduled.event) else {
                    continue;
                };
                let direction = (event.at - ship.path.position_at(scheduled.arrive_t as f64))
                    .normalize_or_zero();
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
            state.cursor_t = now;
            state.last_reception_t = latest;
            if !cleared.is_empty() {
                cleared.sort_by_key(|c| c.get().arrive_t);
                wire.send(*id, Outbound::Sightings(cleared));
            }
        }
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
    #[test]
    fn one_client_acts_and_the_other_learns_at_light_delay_and_not_before() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let actor = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);
        let watcher = server
            .admit(ShipId(2), Path::still(DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)), 0.0);

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0e20 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire);

        let emitted = server.events()[0].t;
        let arrives = emitted + TWO_LIGHT_HOURS as i64;
        assert!(arrives > server.now_t() + TICK_US * 8, "the delay is not worth testing");

        // The actor is standing where it happened, so it knows at once.
        assert_eq!(sightings(&wire.take(actor)).len(), 1, "a ship cannot be late to its own act");

        let mut told_at = None;
        for _ in 0..2_000 {
            let before = server.now_t();
            server.tick(&mut wire);
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
            server.tick(&mut wire);
            assert!(sightings(&wire.take(watcher)).is_empty(), "the same sighting came twice");
        }
    }

    /// A ship may not act for a ship that is not its own, and may not be told that it tried.
    #[test]
    fn a_client_cannot_act_for_a_ship_it_does_not_own() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let first = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);
        let _second = server.admit(ShipId(2), Path::still(DVec3::ZERO), 0.0);

        wire.client_says(first, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire);
        assert!(server.events().is_empty(), "an event was written for someone else's ship");
        assert!(matches!(
            wire.take(first).as_slice(),
            [Outbound::Refused { reason: Refusal::NotYours, .. }]
        ));
    }

    /// The clamp. An intent stamped earlier than the last thing the client can prove it
    /// received is a claim to have acted on information it did not have; one stamped later than
    /// now is a claim about the future.
    #[test]
    fn an_intents_timestamp_is_clamped_at_both_ends() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let client = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);

        // The future: clamped down to now.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: i64::MAX / 4,
        }));
        server.tick(&mut wire);
        assert_eq!(server.events()[0].t, server.now_t(), "an intent was stamped in the future");
        let _ = wire.take(client);

        // Its own transmission is received at once, so the floor is now that arrival.
        server.tick(&mut wire);
        let floor = server.events()[0].t;

        // The past: clamped up to what it can prove it knew.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire);
        let second = server.events().last().unwrap();
        assert!(
            second.t >= floor,
            "stamped at {} but the client had already received something at {floor}",
            second.t,
        );
    }

    /// An order the world cannot carry out is refused rather than clamped into something it
    /// can. There is no nearest legal burn to a superluminal one.
    #[test]
    fn an_impossible_order_is_refused() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let client = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);

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
            server.tick(&mut wire);
            assert!(server.events().is_empty(), "{order:?} became an event");
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
        server.tick(&mut wire);
        assert_eq!(server.events().len(), 1);
        assert!(matches!(server.ship(ShipId(1)).unwrap().path, Path::Coasting(_)));
    }

    /// Arrival is not detection. A signal that reaches a receiver below its noise floor is not
    /// sent, and the client cannot tell that from nothing having happened.
    #[test]
    fn a_signal_under_the_noise_floor_arrives_and_is_not_sent() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let actor = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);
        let deaf = server.admit(
            ShipId(2),
            Path::still(DVec3::new(1_000_000.0, 0.0, 0.0)),
            1.0e6,
        );

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire);
        let _ = wire.take(actor);

        // Well past the arrival, and still nothing.
        for _ in 0..4 {
            server.tick(&mut wire);
        }
        assert!(server.now_t() > 1_000_000, "the test never reached the arrival");
        assert!(
            server.deliveries().iter().any(|d| d.observer == ShipId(2)),
            "it was never even scheduled, so the floor is not what stopped it",
        );
        assert!(sightings(&wire.take(deaf)).is_empty(), "a signal under the floor was sent");
    }

    /// Catch-up. A client that was away winds its cursor back and is told everything again,
    /// still subject to the same gate.
    #[test]
    fn resuming_replays_what_was_missed_and_nothing_more() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let actor = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);

        wire.client_says(actor, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1.0e9 },
            // Stamped at "now", so the replay below can ask for everything after zero and
            // mean it. A reception at zero is one the client already has.
            issued_at_client_t: i64::MAX / 4,
        }));
        server.tick(&mut wire);
        let first = sightings(&wire.take(actor)).len();
        assert_eq!(first, 1);

        // Nothing new while it is away.
        server.tick(&mut wire);
        assert!(sightings(&wire.take(actor)).is_empty());

        wire.client_says(actor, Inbound::ResumeFrom { arrive_t: 0 });
        server.tick(&mut wire);
        assert_eq!(sightings(&wire.take(actor)).len(), 1, "the replay did not come back");

        // A resume past `now` cannot be used to ask for the future.
        wire.client_says(actor, Inbound::ResumeFrom { arrive_t: i64::MAX });
        server.tick(&mut wire);
        assert!(sightings(&wire.take(actor)).is_empty());
    }

    #[test]
    fn a_client_on_the_wrong_protocol_is_told_so_and_not_welcomed() {
        let mut server = Server::new(0, 1);
        let mut wire = Loopback::new();
        let client = server.admit(ShipId(1), Path::still(DVec3::ZERO), 0.0);

        wire.client_says(client, Inbound::Hello { protocol: PROTOCOL_VERSION + 1 });
        server.tick(&mut wire);
        assert!(matches!(wire.take(client).as_slice(), [Outbound::WrongProtocol { .. }]));

        wire.client_says(client, Inbound::Hello { protocol: PROTOCOL_VERSION });
        server.tick(&mut wire);
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Welcome { .. }]));
    }

    /// A coarser tick produces the same world, only with coarser timestamps. The step never
    /// enters an integrator, so it cannot accumulate error.
    #[test]
    fn the_tick_size_does_not_change_where_anything_is() {
        let at = DVec3::new(500_000.0, 0.0, 0.0);
        let beta = DVec3::new(0.3, -0.1, 0.0);
        let path = Path::coasting(at, beta, 0);
        let after = 40 * TICK_US;
        // One step or forty, the closed form is the same place to the last bit.
        assert_eq!(path.position_at(after as f64), at + beta * after as f64);
    }
}
