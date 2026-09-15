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
use crate::ticket::{Spent, Trusted};
use crate::world::{Event, Scheduled, World, schedule};
use lc_world::craft::{Craft, CraftId, Fleet, Kind};
use lc_world::motion::{Change, Event as Change_, Rejected};
use lc_world::navigation::Course;
use lc_world::system::LocalSystem;
use std::sync::Arc;

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

/// What an accepted intent actually became.
struct Applied {
    event_id: i64,
    at_t: i64,
    order: Order,
}

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
    /// Every craft in the world. The physics is `lc-world`'s and this is the whole of it.
    fleet: Fleet,
    /// The stars this shard is authoritative over. Empty until one is loaded, which is the
    /// state every test that is not about systems runs in.
    world: World,
    /// Whose word this server takes about who someone is. **Empty means nobody's**: with no
    /// published key learned, every ticket is refused and only [`Server::admit`] can put a
    /// craft in play, which is the state a test runs in.
    trusted: Trusted,
    spent: Spent,
    /// Which craft belongs to which account, so signing in twice reaches the same ship.
    by_account: HashMap<String, ShipId>,
    next_ship: i64,
    /// Who owns what. The server's fact, not the world's: a probe has a worldline and no
    /// client, and a client is a connection rather than a thing in space.
    owners: HashMap<CraftId, ClientId>,
    clients: HashMap<ClientId, Connected>,
    journal: J,
    minter: Minter,
    /// Written by this tick, and handed to the journal at the end of it.
    pending: Vec<Event>,
    /// What each connection is allowed to send. Kept per `ClientId` rather than per admitted
    /// client, so a connection that has not been given a ship still cannot flood.
    budgets: HashMap<ClientId, Budget>,
    /// Whether a connection whose ticket does not verify is admitted anyway. See
    /// [`Server::admit_without_tickets`].
    open: bool,
}

impl<J: Journal> Server<J> {
    /// A world at coordinate time `start_t`, with a shard identifier for its event ids.
    pub fn new(journal: J, start_t: i64, shard: u64) -> Self {
        Self {
            now_t: start_t,
            fleet: Fleet::new(),
            world: World::default(),
            trusted: Trusted::default(),
            spent: Spent::default(),
            by_account: HashMap::new(),
            next_ship: 1,
            owners: HashMap::new(),
            clients: HashMap::new(),
            journal,
            minter: Minter::new(shard).expect("a shard inside the identifier's field"),
            pending: Vec::new(),
            budgets: HashMap::new(),
            open: false,
        }
    }

    pub fn now_t(&self) -> i64 {
        self.now_t
    }

    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// Whose signature this server accepts as proof of identity, and under what audience.
    ///
    /// Learned from the broker's published key set, never by asking it per connection. Until
    /// this is called no ticket verifies, which is the safe direction: a server that has not
    /// been told who to trust trusts no one.
    pub fn trust(&mut self, trusted: Trusted) {
        self.trusted = trusted;
    }

    /// **Development only: let anyone in, ticket or not.**
    ///
    /// The same decision the site makes when no broker is configured — a deployment with no
    /// identity service launches the game for whoever asks, which is the single-player and
    /// development case. Off by default, because the direction that failure should point is
    /// "nobody gets in".
    ///
    /// An anonymous connection is keyed by its `ClientId`, so reconnecting gets a *new* ship.
    /// That is the honest consequence of having no account to come back to, and it is a second
    /// reason this is not a deployment mode.
    pub fn admit_without_tickets(&mut self, yes: bool) {
        self.open = yes;
    }

    /// Give the server the stars it is authoritative over.
    ///
    /// Craft are placed into systems by where they *are*, on the tick after this, rather than
    /// by being told — so loading a world mid-flight is the same operation as a ship crossing
    /// into one.
    pub fn load_world(&mut self, world: World) {
        self.world = world;
    }

    /// The fleet, for a caller putting a craft into a system.
    ///
    /// Placing craft is not the tick loop's business — a world is loaded around it — so this is
    /// how whatever owns the world reaches in. Reading it is [`Server::ship`].
    pub fn fleet_mut(&mut self) -> &mut Fleet {
        &mut self.fleet
    }

    /// Forget a connection that has closed.
    ///
    /// The craft stays: a ship does not vanish because its pilot's socket did, and the account
    /// comes back to it. What goes is the per-connection state, which would otherwise
    /// accumulate one entry per connection for the life of the process.
    pub fn disconnected(&mut self, client: ClientId) {
        self.clients.remove(&client);
        self.budgets.remove(&client);
    }

    pub fn ship(&self, id: ShipId) -> Option<&Craft> {
        self.fleet.get(CraftId(id.0))
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
    pub fn admit(&mut self, owner: ClientId, mut craft: Craft, noise_floor: f32) {
        let ship_id = ShipId(craft.id.0);
        craft.noise_floor = noise_floor;
        self.owners.insert(craft.id, owner);
        self.fleet.insert(craft);
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
        let now_s = self.now_t as f64 * 1.0e-6;
        // Membership before motion, as the client orders it: a station and a conic are both
        // positions *in* a system, and one resolved against the wrong system is a craft in the
        // wrong place.
        self.resync_systems(now_s);
        // Nothing moved on the server before this. Reading a worldline never needed it — every
        // motive is a closed form — but the transitions do: a crossing that arrives becomes a
        // station, and a ballistic arc folds the patch it was solved for.
        self.fleet.advance(now_s, TICK_US as f64 * 1.0e-6);
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
            Inbound::Hello { protocol, ticket } => {
                if protocol != PROTOCOL_VERSION {
                    wire.send(from, Outbound::WrongProtocol { server: PROTOCOL_VERSION });
                    return;
                }
                match self.sign_in(from, &ticket) {
                    Some((ship_id, name)) => wire.send(from, Outbound::Welcome {
                        client_id: from,
                        protocol: PROTOCOL_VERSION,
                        ship_id,
                        now_t: self.now_t,
                        name,
                    }),
                    None => wire.send(from, Outbound::Unauthenticated),
                }
            }
            Inbound::Act(intent) => {
                let ship_id = intent.ship_id;
                match self.act(from, intent, events, deliveries) {
                    Ok(applied) => wire.send(from, Outbound::Accepted {
                        ship_id,
                        event_id: applied.event_id,
                        at_t: applied.at_t,
                        order: applied.order,
                    }),
                    Err(reason) => wire.send(from, Outbound::Refused { ship_id, reason }),
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
    ///
    /// Returns what was **actually done**, which is not always what was asked: both the
    /// timestamp and the acceleration are clamped below.
    fn act(
        &mut self,
        from: ClientId,
        intent: Intent,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<Applied, Refusal> {
        let state = self.clients.get(&from).ok_or(Refusal::NotYours)?;
        if state.ship != intent.ship_id {
            return Err(Refusal::NotYours);
        }
        let id = CraftId(intent.ship_id.0);
        if self.owners.get(&id) != Some(&from) || self.fleet.get(id).is_none() {
            return Err(Refusal::NotYours);
        }

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

        let (kind, power_w, payload, applied) = match &intent.order {
            Order::Transmit { power_w } => {
                let power_w = *power_w;
                // `<= 0.0` is false for NaN, so the finite check is not redundant with it.
                if power_w <= 0.0 || !power_w.is_finite() {
                    return Err(Refusal::Impossible);
                }
                (
                    KIND_TRANSMIT,
                    power_w,
                    format!("{{\"power_w\":{power_w}}}"),
                    Order::Transmit { power_w },
                )
            }
            Order::Burn { beta } => {
                let beta = DVec3::from_array(*beta);
                if !beta.is_finite() || beta.length() >= 1.0 {
                    return Err(Refusal::Impossible);
                }
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                let here = craft.position_at(at as f64);
                let noise_floor = craft.noise_floor;
                *craft = crate::world::coasting(intent.ship_id, here, beta, at);
                craft.noise_floor = noise_floor;
                // A burn is not silent -- it is the most visible thing a ship does -- but what
                // it radiates is the drive's business. Nominal, until there is a drive model.
                (
                    KIND_BURN,
                    BURN_POWER_W,
                    format!("{{\"beta\":{beta:?}}}"),
                    Order::Burn { beta: beta.to_array() },
                )
            }
            Order::SetCourse { course, accel_g } => {
                if !accel_g.is_finite() || *accel_g <= 0.0 {
                    return Err(Refusal::Impossible);
                }
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                // Asked for, not stated. The ceiling is the craft's own, so a client cannot
                // fly a better ship than it has by sending a larger number.
                let mut drive = craft.kind.drive();
                drive.accel_g = accel_g.min(drive.accel_g);
                // Not shadowed: the proto course is wanted again below, to say back what was
                // applied. Only the acceleration is clamped, never the course itself.
                let flown: Course = course.clone().into();
                let change = Change::SetCourse { course: flown, drive };
                // **The fold, not a second implementation.** The server works the crossing out
                // from the order exactly as the client will, because it is the same function.
                craft
                    .apply(&Change_ { ship: motion_id(id), at_t: at as f64 * 1.0e-6, change })
                    .map_err(refusal_for)?;
                (
                    KIND_BURN,
                    BURN_POWER_W,
                    format!("{{\"accel_g\":{}}}", drive.accel_g),
                    // The clamped acceleration, not the one that was asked for. This is the
                    // whole reason the message exists.
                    Order::SetCourse { course: course.clone(), accel_g: drive.accel_g },
                )
            }
            Order::CutDrive => {
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                craft
                    .apply(&Change_ {
                        ship: motion_id(id),
                        at_t: at as f64 * 1.0e-6,
                        change: Change::CutDrive,
                    })
                    .map_err(refusal_for)?;
                // Silent. Cutting the engine is the one manoeuvre that puts nothing out, which
                // is exactly why a player might choose it.
                (KIND_CUT, 0.0, "{}".to_string(), Order::CutDrive)
            }
        };

        let at_position = self.fleet.get(id).ok_or(Refusal::NotYours)?.position_at(at as f64);
        let event = Event {
            id: self.minter.mint(at).ok_or(Refusal::Impossible)?.get(),
            source: intent.ship_id,
            t: at,
            at: at_position,
            kind,
            power_w,
            payload,
        };
        for observer in self.fleet.iter() {
            if let Some(scheduled) = schedule(&event, observer) {
                deliveries.push(scheduled);
            }
        }
        let event_id = event.id;
        events.push(event);
        Ok(Applied { event_id, at_t: at, order: applied })
    }

    /// Verify a ticket and bind the connection to the account's craft.
    ///
    /// The client never names its own ship: the identifier comes back in `Welcome` and is
    /// looked up from the ticket's subject. A client that could ask for a `ShipId` could ask
    /// for someone else's.
    fn sign_in(&mut self, from: ClientId, ticket: &str) -> Option<(ShipId, String)> {
        let claims = match self.trusted.check(ticket) {
            Ok(claims) => {
                // Spent only after it verifies, or an invalid ticket could burn a valid one's
                // identifier.
                self.spent.claim(&claims, self.now_t / crate::world::MICROS_PER_SECOND).ok()?;
                claims
            }
            // No account to key an anonymous player by, so the connection is the account.
            Err(_) if self.open => crate::ticket::Claims {
                sub: format!("anonymous:{}", from.0),
                name: format!("Traveller {}", from.0),
                exp: i64::MAX,
                jti: format!("anonymous:{}", from.0),
            },
            Err(_) => return None,
        };

        let ship = match self.by_account.get(&claims.sub) {
            Some(ship) => *ship,
            None => {
                // First sign-in: the account gets a craft. Where a new player starts is a game
                // question and this is the crudest possible answer to it.
                let ship = ShipId(self.next_ship);
                self.next_ship += 1;
                let mut craft = Craft::at(CraftId(ship.0), Kind::Ship, DVec3::ZERO);
                craft.name = Some(claims.name.clone());
                self.fleet.insert(craft);
                self.by_account.insert(claims.sub.clone(), ship);
                ship
            }
        };
        // A reconnection replaces the old connection's claim on the craft rather than sharing
        // it: two sockets acting for one ship is two clients predicting different futures.
        self.clients.retain(|_, state| state.ship != ship);
        self.clients.insert(from, Connected {
            ship,
            last_reception_t: i64::MIN,
            cursor_t: i64::MIN,
        });
        // Being welcomed is not the same fact as owning the craft, and `act` checks the
        // second. Without this a signed-in client is welcomed, given a ship, and then refused
        // `NotYours` on every order it sends — which no in-process test caught, because the
        // ones about orders call `admit` and the ones about tickets never send an order.
        self.owners.insert(CraftId(ship.0), from);
        Some((ship, claims.name))
    }

    /// Put every craft in the system it is actually inside.
    ///
    /// By position and a shell radius, which is the whole rule and is the client's rule too.
    /// Nothing is sent about it: both sides hold the same positions and reach the same answer,
    /// the way both sides solve the same patch.
    fn resync_systems(&mut self, now_s: f64) {
        if self.world.is_empty() {
            return;
        }
        // Positions first, then systems, then craft: the world and the fleet cannot both be
        // borrowed at once, and a craft's system is looked up from where it is.
        let where_each: Vec<(CraftId, DVec3)> =
            self.fleet.iter().map(|craft| (craft.id, craft.motion.position_ly)).collect();
        let placements: Vec<(CraftId, Option<Arc<LocalSystem>>)> = where_each
            .into_iter()
            .map(|(id, at)| (id, self.world.system_at(at)))
            .collect();

        for (id, system) in placements {
            let Some(craft) = self.fleet.get_mut(id) else { continue };
            if craft.system.as_ref().map(|s| s.star) == system.as_ref().map(|s| s.star) {
                continue;
            }
            craft.enter(system, now_s);
        }
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
            let Some(ship) = self.fleet.get(CraftId(state.ship.0)).cloned() else {
                continue;
            };
            // The proven read: one range scan over `(observer_id, arrive_t)`, already ordered.
            let due = self.journal.due(state.ship, state.cursor_t, now).await?;

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

/// Why the world refused an order, as the client is told it.
///
/// A refusal and not a failure: the server has the authoritative view of the system, and a
/// client folding the same order against a staler one can legitimately reach a different
/// answer. It is told which, and corrects.
/// A craft's identifier as [`lc_world::motion`] names it. The two are the same number: a craft
/// is the thing a worldline belongs to, and `motion` predates the fleet that owns them.
fn motion_id(id: CraftId) -> lc_world::motion::ShipId {
    lc_world::motion::ShipId(id.0)
}

fn refusal_for(rejected: Rejected) -> Refusal {
    match rejected {
        Rejected::NotInASystem | Rejected::NoSuchPlace => Refusal::Impossible,
    }
}

/// Event kinds. Small integers on the wire; named here.
pub const KIND_TRANSMIT: i16 = 1;
pub const KIND_BURN: i16 = 2;
/// Cutting the engine. Distinct from a burn because it radiates nothing.
pub const KIND_CUT: i16 = 3;

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
        server.admit(actor, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        let watcher = ClientId(2);
        server
            .admit(watcher, crate::world::still(ShipId(2), DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)), 0.0);

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
        server.admit(client, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

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
            wire.client_says(stranger, Inbound::Hello {
                protocol: PROTOCOL_VERSION,
                ticket: "not a ticket".into(),
            });
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
        server.admit(first, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(ClientId(2), crate::world::still(ShipId(2), DVec3::ZERO), 0.0);

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
        server.admit(client, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

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
        server.admit(client, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

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
        server.admit(client, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

        for order in [
            Order::Burn { beta: [1.0, 0.0, 0.0] },
            Order::Burn { beta: [0.9, 0.9, 0.0] },
            Order::Burn { beta: [f64::NAN, 0.0, 0.0] },
            Order::Transmit { power_w: 0.0 },
            Order::Transmit { power_w: -5.0 },
        ] {
            wire.client_says(client, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: order.clone(),
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
        server.admit(actor, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        let deaf = ClientId(2);
        server.admit(deaf, crate::world::still(ShipId(2), DVec3::new(1_000_000.0, 0.0, 0.0)), 1.0e6);

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
        server.admit(actor, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);

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
        use crate::testing::Broker;
        let broker = Broker::new([3u8; 32]);
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = crate::ticket::Trusted::new("shard-1");
        trusted.learn(&broker.jwks());
        server.trust(trusted);
        let mut wire = Loopback::new();
        let client = ClientId(1);

        // The version is checked before the ticket, so a stale client is told which problem it
        // has rather than being told it is not signed in.
        wire.client_says(client, Inbound::Hello {
            protocol: PROTOCOL_VERSION + 1,
            ticket: broker.mint("acct-1", "shard-1", 60, "j1"),
        });
        server.tick(&mut wire).await.unwrap();
        assert!(matches!(wire.take(client).as_slice(), [Outbound::WrongProtocol { .. }]));

        wire.client_says(client, Inbound::Hello {
            protocol: PROTOCOL_VERSION,
            ticket: broker.mint("acct-1", "shard-1", 60, "j2"),
        });
        server.tick(&mut wire).await.unwrap();
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Welcome { .. }]));
    }

    /// A coarser tick produces the same world, only with coarser timestamps. The step never
    /// enters an integrator, so it cannot accumulate error.
    #[test]
    fn the_tick_size_does_not_change_where_anything_is() {
        let at = DVec3::new(500_000.0, 0.0, 0.0);
        let beta = DVec3::new(0.3, -0.1, 0.0);
        let ship = crate::world::coasting(ShipId(1), at, beta, 0);
        let after = 40 * TICK_US;
        // One step or forty, the closed form is the same place to the last bit -- and it stays
        // exact across the conversion into light-years the world model works in and back.
        assert_eq!(ship.position_at(after as f64), at + beta * after as f64);
    }
}

#[cfg(test)]
pub(crate) mod course_tests {
    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use lc_world::craft::Kind;
    use lc_world::motion::Motive;
    use lc_world::sky::{AuthoredStars, StarProvider};
    use lc_world::system::LocalSystem;
    use std::sync::Arc;

    pub(crate) fn a_star() -> Option<lc_world::sky::CatalogueStar> {
        AuthoredStars::sample().stars().first().cloned()
    }

    pub(crate) fn a_system() -> Option<Arc<LocalSystem>> {
        LocalSystem::for_star(&a_star()?).map(Arc::new)
    }

    pub(crate) fn orbitable(system: &LocalSystem) -> Option<String> {
        system.inventory().iter().find_map(|entry| match &entry.target {
            lc_world::navigation::Target::Body(name) => Some(name.clone()),
            _ => None,
        })
    }

    /// The point of putting a course on the wire: the server works the crossing out *itself*,
    /// from the order, with the same `lc_world::motion::apply` the client runs. Nothing about
    /// the trajectory is sent.
    #[tokio::test]
    async fn a_course_on_the_wire_becomes_a_crossing_the_server_solved() {
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);
        server.fleet_mut().get_mut(CraftId(1)).expect("the craft").enter(Some(system), 0.0);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body: body.clone(),
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 5.0,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        assert!(
            !wire.take(client).iter().any(|out| matches!(out, Outbound::Refused { .. })),
            "the course was refused",
        );
        let craft = server.ship(ShipId(1)).expect("the craft");
        let Motive::Crossing(cruise) = &craft.motion.motive else {
            panic!("a course is a crossing, not {:?}", craft.motion.motive)
        };
        assert!(cruise.duration_s() > 0.0, "a crossing that takes no time went nowhere");
        assert_eq!(server.journal().events.len(), 1, "and it is one event");
    }

    /// Cutting the engine leaves the velocity it had. With no system to be on a conic about,
    /// that is a straight line — and it is *not* a stop.
    #[tokio::test]
    async fn cutting_the_drive_keeps_the_velocity() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);

        let beta = [0.4, 0.0, 0.0];
        for order in [Order::Burn { beta }, Order::CutDrive] {
            wire.client_says(client, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order,
                issued_at_client_t: 0,
            }));
            server.tick(&mut wire).await.unwrap();
        }

        let craft = server.ship(ShipId(1)).expect("the craft");
        assert!(matches!(craft.motion.motive, Motive::Drifting { .. }), "{:?}", craft.motion.motive);
        assert!((craft.motion.beta.x - 0.4).abs() < 1.0e-12, "{}", craft.motion.beta.x);
    }

    /// A course with nowhere to resolve against is refused rather than silently dropped. The
    /// world's refusal is the client's refusal.
    #[tokio::test]
    async fn a_course_with_no_system_is_refused() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse { course: lc_proto::Course::LeaveSystem, accel_g: 5.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        assert!(
            wire.take(client).iter().any(|out| matches!(
                out,
                Outbound::Refused { reason: Refusal::Impossible, .. }
            )),
            "a course that cannot be resolved should be refused",
        );
        assert_eq!(server.journal().events.len(), 0, "and nothing was written");
    }

    /// The acceleration is asked for, not stated. A client sending a thousand g gets its own
    /// ship's ceiling, and the event records what was actually flown.
    #[tokio::test]
    async fn a_client_cannot_ask_for_a_better_ship_than_it_has() {
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);
        server.fleet_mut().get_mut(CraftId(1)).expect("the craft").enter(Some(system), 0.0);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body,
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 1000.0,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        let ceiling = Kind::Ship.drive().accel_g;
        let written = &server.journal().events[0].payload;
        assert!(written.contains(&format!("{ceiling}")), "{written} does not record {ceiling} g");

        // And a nonsense acceleration is refused rather than clamped into something flyable.
        for accel_g in [0.0, -1.0, f64::NAN] {
            wire.client_says(client, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::SetCourse { course: lc_proto::Course::LeaveSystem, accel_g },
                issued_at_client_t: 0,
            }));
            server.tick(&mut wire).await.unwrap();
            assert!(
                wire.take(client).iter().any(|out| matches!(out, Outbound::Refused { .. })),
                "{accel_g} g was not refused",
            );
        }
    }
}

#[cfg(test)]
mod world_tests {
    use super::course_tests::*;
    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use crate::world::World;
    use lc_world::craft::Kind;
    use lc_world::motion::{Change, Event as Change_, Motive};

    /// A craft is in a system because of where it *is*. Nothing is told, and nothing asks:
    /// the server applies the same shell radius the client does, so the two cannot disagree
    /// about whether a ship is in a system.
    /// The message's whole reason for existing: the client asked for a thousand g and got the
    /// craft's ceiling, and now it is **told** so rather than left to diverge by the difference.
    #[tokio::test]
    async fn an_accepted_order_says_the_acceleration_that_was_actually_applied() {
        // Not `else { return }`: a test that skips itself silently is a test that passes for
        // the wrong reason, and this one is the point of the whole message.
        let system = a_system().expect("the sample star makes a system");
        let body = orbitable(&system).expect("something to orbit");

        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);
        server.fleet_mut().get_mut(CraftId(1)).expect("the craft").enter(Some(system), 0.0);

        let asked = lc_proto::Course::Orbit {
            body,
            altitude_radii: 2.0,
            plane: lc_proto::Plane::Equatorial,
        };
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse { course: asked.clone(), accel_g: 1000.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        let said = wire.take(client);
        let Some(Outbound::Accepted { ship_id, event_id, order, .. }) = said
            .iter()
            .find(|out| matches!(out, Outbound::Accepted { .. }))
        else {
            panic!("no acceptance: {said:?}");
        };
        assert_eq!(*ship_id, ShipId(1));
        assert!(*event_id > 0, "an accepted order names no event");
        let Order::SetCourse { course, accel_g } = order else { panic!("{order:?}") };
        assert_eq!(
            *accel_g,
            Kind::Ship.drive().accel_g,
            "it echoed what was asked for instead of what was flown",
        );
        // The course itself is not clamped, only the acceleration.
        assert_eq!(course, &asked);
    }

    /// The other silent clamp. An intent stamped before the client's cursor is moved forward,
    /// and the client needs the moved value to know when its own order took effect.
    #[tokio::test]
    async fn an_accepted_order_says_the_time_it_actually_took_effect() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(client);

        // Far in the future, which is the direction that gets clamped to `now`.
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: 1000.0 },
            issued_at_client_t: i64::MAX,
        }));
        server.tick(&mut wire).await.unwrap();

        let said = wire.take(client);
        let Some(Outbound::Accepted { at_t, .. }) =
            said.iter().find(|out| matches!(out, Outbound::Accepted { .. }))
        else {
            panic!("no acceptance: {said:?}");
        };
        assert_eq!(*at_t, server.now_t(), "the clamped time was not reported");
        assert_ne!(*at_t, i64::MAX);
    }

    /// An order that does not stand is refused and **not** also accepted.
    #[tokio::test]
    async fn a_refused_order_is_not_accepted_as_well() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO), 0.0);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Transmit { power_w: -1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        let said = wire.take(client);
        assert!(said.iter().any(|out| matches!(out, Outbound::Refused { .. })), "{said:?}");
        assert!(
            !said.iter().any(|out| matches!(out, Outbound::Accepted { .. })),
            "a refused order was also accepted: {said:?}",
        );
    }

    #[tokio::test]
    async fn a_craft_is_placed_by_where_it_is() {
        let Some(star) = a_star() else { return };
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();

        // One at the star, one a good way outside its shell.
        server.admit(ClientId(1), Craft::at(CraftId(1), Kind::Ship, star.position_ly), 0.0);
        let far = star.position_ly + DVec3::new(50.0, 0.0, 0.0);
        server.admit(ClientId(2), Craft::at(CraftId(2), Kind::Probe, far), 0.0);
        server.load_world(World::new(vec![star.clone()]));

        assert!(server.ship(ShipId(1)).unwrap().system.is_none(), "nothing placed before a tick");
        server.tick(&mut wire).await.unwrap();

        let inside = server.ship(ShipId(1)).expect("the ship");
        assert_eq!(inside.system.as_ref().map(|s| s.star), Some(star.id), "it is at the star");
        assert!(
            server.ship(ShipId(2)).unwrap().system.is_none(),
            "fifty light-years out is not in anything",
        );
    }

    /// The transitions the server never used to reach, because it never advanced anything.
    /// Reading a worldline needs no stepping — every motive is a closed form — but a crossing
    /// only *becomes* a station by being stepped past its own arrival.
    #[tokio::test]
    async fn a_crossing_the_server_flew_arrives_and_becomes_a_station() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, star.position_ly), 0.0);
        server.load_world(World::new(vec![star]));
        server.tick(&mut wire).await.unwrap();

        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body,
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 5.0,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let Motive::Crossing(cruise) = &server.ship(ShipId(1)).unwrap().motion.motive else {
            panic!("the course did not become a crossing")
        };
        let duration_s = cruise.duration_s();
        assert!(duration_s > 0.0);

        // Far enough past the arrival that no rounding leaves it short.
        let ticks = ((duration_s * 1.0e6 / TICK_US as f64).ceil() as usize + 2).min(20_000);
        for _ in 0..ticks {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            matches!(server.ship(ShipId(1)).unwrap().motion.motive, Motive::Holding(_)),
            "after {ticks} ticks it is {:?}",
            server.ship(ShipId(1)).unwrap().motion.motive,
        );
    }

    /// **The property the whole seam exists for.** A client stepping at a frame rate and a
    /// server stepping at 438 seconds a tick, folding the same order at the same coordinate,
    /// end up in the same place.
    ///
    /// The reference is a bare `Craft` — the same type, run by hand at a different step —
    /// because that is exactly what a client is. If this ever fails, something in the fold
    /// depends on how often it is called, which is the one thing it may not do.
    #[tokio::test]
    async fn a_client_stepping_finely_agrees_with_the_server() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(client, Craft::at(CraftId(1), Kind::Ship, star.position_ly), 0.0);
        server.load_world(World::new(vec![star.clone()]));
        server.tick(&mut wire).await.unwrap();

        let course = lc_proto::Course::Orbit {
            body,
            altitude_radii: 2.0,
            plane: lc_proto::Plane::Equatorial,
        };
        wire.client_says(client, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::SetCourse { course: course.clone(), accel_g: 5.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        // What the server actually stamped it at. A client is told the coordinate; it does not
        // guess one.
        let stamped = server.journal().events[0].t;

        // Entered at zero where the server entered it at its first tick. Deliberately not
        // aligned: a client that joined a system at a slightly different moment still has to
        // agree, and entering from nothing leaves the motive alone either way.
        let mut mirror = Craft::at(CraftId(1), Kind::Ship, star.position_ly);
        mirror.enter(Some(system.clone()), 0.0);
        let mut drive = Kind::Ship.drive();
        drive.accel_g = 5.0_f64.min(drive.accel_g);
        mirror
            .apply(&Change_ {
                ship: lc_world::motion::ShipId(1),
                at_t: stamped as f64 * 1.0e-6,
                change: Change::SetCourse { course: course.into(), drive },
            })
            .expect("the same order the server took");

        // Run both to the same coordinate, one at 438 seconds a step and one at 61.
        let target_us = server.now_t() + 400 * TICK_US;
        while server.now_t() < target_us {
            server.tick(&mut wire).await.unwrap();
        }
        let end_s = server.now_t() as f64 * 1.0e-6;

        let mut now = 0.0;
        while now < end_s {
            let next = (now + 61.0).min(end_s);
            mirror.advance(next, next - now);
            now = next;
        }
        assert!((now - end_s).abs() < 1.0e-9, "the mirror stopped at {now}, not {end_s}");

        let flown = server.ship(ShipId(1)).expect("the ship");
        // To the bit. Anything less would mean the fold has a term that depends on how often
        // it is called, and a client predicting for a few seconds would slide off the server's
        // answer rather than track it.
        assert_eq!(
            flown.motion.position_ly, mirror.motion.position_ly,
            "they disagree by {:e} light-years",
            (flown.motion.position_ly - mirror.motion.position_ly).length(),
        );
        assert_eq!(flown.motion.motive, mirror.motion.motive);
        assert_eq!(flown.motion.beta, mirror.motion.beta);
    }
}

#[cfg(test)]
mod hello_tests {
    use super::*;
    use crate::journal::Memory;
    use crate::testing::Broker;
    use crate::ticket::Trusted;
    use crate::transport::Loopback;

    const SHARD: &str = "shard-1";

    fn trusting(broker: &Broker) -> Server<Memory> {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = Trusted::new(SHARD);
        assert_eq!(trusted.learn(&broker.jwks()), 1);
        server.trust(trusted);
        server
    }

    async fn says(server: &mut Server<Memory>, wire: &mut Loopback, from: ClientId, ticket: String) {
        wire.client_says(from, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket });
        server.tick(wire).await.unwrap();
    }

    /// The hole this closes. Before a ticket, `Hello` carried no identity and a craft went to
    /// whoever connected.
    #[tokio::test]
    async fn a_connection_with_no_ticket_gets_no_ship() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let client = ClientId(1);

        says(&mut server, &mut wire, client, "not a ticket".into()).await;
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Unauthenticated]));
        assert!(server.ship(ShipId(1)).is_none(), "a craft was handed out anyway");
    }

    /// A server that has not been told whose word to take takes nobody's.
    #[tokio::test]
    async fn a_server_that_trusts_nobody_admits_nobody() {
        let broker = Broker::new([1u8; 32]);
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);

        says(&mut server, &mut wire, client, broker.mint("acct-1", SHARD, 60, "j1")).await;
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Unauthenticated]));
    }

    /// A valid ticket gets a craft, and the client is never asked which one it wants.
    #[tokio::test]
    async fn a_ticket_names_the_account_and_the_server_names_the_ship() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let client = ClientId(1);

        says(&mut server, &mut wire, client, broker.mint("acct-1", SHARD, 60, "j1")).await;
        let said = wire.take(client);
        let [Outbound::Welcome { ship_id, name, client_id, .. }] = said.as_slice() else {
            panic!("no welcome: {said:?}")
        };
        assert_eq!(*client_id, client);
        assert_eq!(name, "Ada");
        assert!(server.ship(*ship_id).is_some(), "the ship it was given does not exist");
    }

    /// Signing in again reaches the same ship. A player who reconnects is not a new player.
    #[tokio::test]
    async fn the_same_account_comes_back_to_the_same_ship() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let first = welcomed(&mut wire, ClientId(1));

        // A different connection, a fresh ticket, the same account.
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        assert_eq!(welcomed(&mut wire, ClientId(2)), first, "the account got a second ship");

        // And a different account does not.
        says(&mut server, &mut wire, ClientId(3), broker.mint("acct-2", SHARD, 60, "j3")).await;
        assert_ne!(welcomed(&mut wire, ClientId(3)), first);
    }

    /// Two sockets acting for one ship is two clients predicting different futures, so the
    /// newer connection takes the craft and the older one stops owning it.
    #[tokio::test]
    async fn a_reconnection_displaces_the_connection_it_replaces() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        let _ = wire.take(ClientId(2));

        // The displaced connection can no longer act for it.
        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::Transmit { power_w: 1.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert!(
            wire.take(ClientId(1))
                .iter()
                .any(|out| matches!(out, Outbound::Refused { reason: Refusal::NotYours, .. })),
            "the replaced connection still acted for the ship",
        );
    }

    /// One ticket, one connection. A ticket in a log or a screenshot is worth nothing twice.
    #[tokio::test]
    async fn a_ticket_cannot_be_replayed() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let ticket = broker.mint("acct-1", SHARD, 60, "only-once");

        says(&mut server, &mut wire, ClientId(1), ticket.clone()).await;
        assert!(matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Welcome { .. }]));

        says(&mut server, &mut wire, ClientId(2), ticket).await;
        assert!(matches!(wire.take(ClientId(2)).as_slice(), [Outbound::Unauthenticated]));
    }

    /// A ticket that does not verify must not burn the identifier of one that would. Otherwise
    /// anyone who saw a `jti` could lock its owner out by presenting a forgery first.
    #[tokio::test]
    async fn a_forged_ticket_does_not_spend_a_real_ones_identifier() {
        let ours = Broker::new([1u8; 32]);
        let stranger = Broker::new([2u8; 32]);
        let mut server = trusting(&ours);
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(9), stranger.mint("acct-1", SHARD, 60, "j1")).await;
        assert!(matches!(wire.take(ClientId(9)).as_slice(), [Outbound::Unauthenticated]));

        // The real one, with the same identifier, still works.
        says(&mut server, &mut wire, ClientId(1), ours.mint("acct-1", SHARD, 60, "j1")).await;
        assert!(matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Welcome { .. }]));
    }

    /// A ticket minted for another shard is not a ticket here, however valid it is there.
    /// The bug the seam test found. Being welcomed and owning the craft are two facts, and
    /// `act` checks the second: a client that signed in with a ticket could be given a ship and
    /// then refused `NotYours` on everything it did with it.
    #[tokio::test]
    async fn a_client_that_signed_in_can_act_on_the_ship_it_was_given() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let client = ClientId(1);

        says(&mut server, &mut wire, client, broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship_id = welcomed(&mut wire, client);

        wire.client_says(client, Inbound::Act(Intent {
            ship_id,
            order: Order::Transmit { power_w: 1000.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();

        let said = wire.take(client);
        assert!(
            said.iter().any(|out| matches!(out, Outbound::Accepted { .. })),
            "a signed-in client could not act on its own ship: {said:?}",
        );
    }

    /// And a reconnection takes ownership with it, or the new socket inherits the refusal.
    #[tokio::test]
    async fn a_reconnection_can_act_on_the_ship_it_took_over() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship_id = welcomed(&mut wire, ClientId(1));
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        assert_eq!(welcomed(&mut wire, ClientId(2)), ship_id);

        wire.client_says(ClientId(2), Inbound::Act(Intent {
            ship_id,
            order: Order::Transmit { power_w: 1000.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(ClientId(2));
        assert!(
            said.iter().any(|out| matches!(out, Outbound::Accepted { .. })),
            "the reconnection could not act: {said:?}",
        );

        // And the displaced connection cannot act for it any more.
        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id,
            order: Order::Transmit { power_w: 1000.0 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let stale = wire.take(ClientId(1));
        assert!(
            stale.iter().any(|out| matches!(out, Outbound::Refused { reason: Refusal::NotYours, .. })),
            "a displaced connection still commanded the ship: {stale:?}",
        );
    }

    /// The default, and the one that matters: a server nobody configured lets nobody in.
    #[tokio::test]
    async fn admitting_without_tickets_is_off_unless_asked_for() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), "not a ticket".into()).await;
        assert!(
            matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Unauthenticated]),
            "a bad ticket was admitted by a server that was never told to",
        );
    }

    /// And when it is asked for, a bad ticket gets a ship anyway — which is the whole point,
    /// and the reason the flag is named after what it switches off.
    #[tokio::test]
    async fn admitting_without_tickets_gives_an_anonymous_connection_a_ship() {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit_without_tickets(true);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(7), String::new()).await;
        let welcome = wire.take(ClientId(7));
        let Some(Outbound::Welcome { ship_id, name, .. }) = welcome.first() else {
            panic!("no welcome: {welcome:?}");
        };
        assert_eq!(*ship_id, ShipId(1));
        assert!(name.contains('7'), "the name should say which connection it is: {name}");
        assert!(server.ship(ShipId(1)).is_some(), "the ship was not put in the world");
    }

    /// Two anonymous connections are two players, not one. They are keyed by connection
    /// because there is no account to key them by.
    #[tokio::test]
    async fn two_anonymous_connections_are_two_ships() {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit_without_tickets(true);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), String::new()).await;
        says(&mut server, &mut wire, ClientId(2), String::new()).await;
        assert_eq!(welcomed(&mut wire, ClientId(1)), ShipId(1));
        assert_eq!(welcomed(&mut wire, ClientId(2)), ShipId(2));
    }

    /// Open house does not mean a valid ticket stops being honoured. A real account still
    /// reaches its own ship, which is what keeps one code path on the client.
    #[tokio::test]
    async fn a_real_ticket_still_names_its_account_when_the_door_is_open() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.admit_without_tickets(true);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let first = welcomed(&mut wire, ClientId(1));
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        assert_eq!(welcomed(&mut wire, ClientId(2)), first, "the account lost its ship");
    }

    #[tokio::test]
    async fn a_ticket_for_another_shard_is_refused() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", "shard-2", 60, "j1")).await;
        assert!(matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Unauthenticated]));
    }

    fn welcomed(wire: &mut Loopback, client: ClientId) -> ShipId {
        match wire.take(client).as_slice() {
            [Outbound::Welcome { ship_id, .. }] => *ship_id,
            other => panic!("{other:?}"),
        }
    }
}
