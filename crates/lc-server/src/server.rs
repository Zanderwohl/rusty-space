//! The tick loop, intent validation, and the one place anything is released to a client.
//!
//! Line limit: 2000. The one module allowed past the thousand-line cap, because it is the
//! shard's core assembly: the tick, what an intent is allowed to do, and the single gate every
//! release to a client passes through. Splitting those apart would put the gate somewhere other
//! than the loop it guards, and a second place a release could be written from is exactly the
//! thing this file exists to prevent. Everything that can live elsewhere already does —
//! `instruments`, `radio`, `planets`, `chase`, `drive`, `fitting`, `persist` — so what is left
//! is the part with nowhere else to be.

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
use lc_world::signal::Beam;
use crate::chase::{self, Pursuit};
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
    /// The account this connection signed in as. Kept because a bookmark belongs to the player
    /// and not to the craft: the same account reading on two machines is one place in one book.
    pub account: String,
    /// The arrival time of the last sighting this client was actually sent.
    ///
    /// Not "the last thing that happened": what a client can prove it knows. An intent stamped
    /// earlier than this is a claim to have acted on information the client did not have.
    pub last_reception_t: i64,
    /// How far its delivery stream has been read.
    pub cursor_t: i64,
    /// Whether the last contact list sent had anything in it.
    ///
    /// So that emptying is stated once and emptiness is then silent. A client that had a
    /// contact and stops hearing about it must be told; one that has never had any needs no
    /// message twenty times a second to say so again.
    pub had_contacts: bool,
    /// What the account may do beyond playing, read from its ticket. See [`crate::ability`],
    /// which is where every rule about it lives; nothing compares these directly.
    pub permission: crate::ability::Level,
    /// Whether this connection has been handed the ship's transcript yet.
    ///
    /// Sent from the tick rather than from the sign-in, because reading it is a query and a
    /// sign-in is not allowed to be. One per connection: a reconnection is a fresh client with
    /// an empty log, and it needs the conversation back.
    pub backlog_sent: bool,
    /// Learned-time through which this client's copy of its craft's knowledge is current. See
    /// [`crate::instruments`]. From the beginning for a new connection, which is what pages a
    /// craft's whole knowledge to a client that has just signed in.
    pub learned: lc_world::knowledge::Mark,
    /// The same for the craft's own samples, which come separately. See [`crate::instruments`].
    pub logged_s: f64,
    /// Whether this connection has been told which subjects its craft keeps raw.
    pub retained_sent: bool,
    /// The last [`Outbound::Analyzing`] count this connection was sent. A client starts at
    /// zero, so a new connection has in effect been told that.
    pub analyzing_sent: usize,
    /// The last [`Outbound::Doing`] this connection was sent.
    pub doing_sent: Option<Outbound>,
}

pub struct Server<J: Journal> {
    pub(crate) now_t: i64,
    /// Every craft in the world. The physics is `lc-world`'s and this is the whole of it.
    pub(crate) fleet: Fleet,
    /// The stars this shard is authoritative over. Empty until one is loaded, which is the
    /// state every test that is not about systems runs in.
    pub(crate) world: World,
    /// Whose word this server takes about who someone is. **Empty means nobody's**: with no
    /// published key learned, every ticket is refused and only [`Server::admit`] can put a
    /// craft in play, which is the state a test runs in.
    trusted: Trusted,
    spent: Spent,
    /// Which craft belongs to which account, so signing in twice reaches the same ship.
    pub(crate) by_account: HashMap<String, ShipId>,
    pub(crate) next_ship: i64,
    /// Who owns what. The server's fact, not the world's: a probe has a worldline and no
    /// client, and a client is a connection rather than a thing in space.
    pub(crate) owners: HashMap<CraftId, ClientId>,
    pub(crate) clients: HashMap<ClientId, Connected>,
    pub(crate) journal: J,
    minter: Minter,
    /// Written by this tick, and handed to the journal at the end of it.
    pending: Vec<Event>,
    /// What each connection is allowed to send. Kept per `ClientId` rather than per admitted
    /// client, so a connection that has not been given a ship still cannot flood.
    budgets: HashMap<ClientId, Budget>,
    /// Whether a connection whose ticket does not verify is admitted anyway. See
    /// [`Server::admit_without_tickets`].
    open: bool,
    /// Accounts whose saved craft could not be read. See [`crate::persist::Unreadable`].
    pub(crate) blocked: HashMap<String, String>,
    /// Who is chasing whom.
    ///
    /// The one piece of *standing* state in a server otherwise made of events. It is here and
    /// not in the world because it is a policy rather than a fact about a worldline: the world
    /// only ever holds the approach a policy most recently produced, which is an ordinary
    /// motive both ends evaluate. Dropped when the quarry goes out of sight, so it cannot keep
    /// a client informed about somewhere it can no longer see.
    ///
    /// **Not** dropped when the pilot signs out. A ship hanging about with another goes on
    /// hanging about, and a checkpoint writes the policy down with the craft.
    pub(crate) pursuits: HashMap<CraftId, Pursuit>,
    /// How fast this world runs, as a multiple of the design rate. See [`Server::set_rate`].
    rate: f64,
    /// Whether a client may stage a scene. See [`Server::directing`].
    pub(crate) directs: bool,
    /// The scene being run, if one was staged. See [`crate::director`].
    pub(crate) director: Option<crate::director::Director>,
    /// The shelf, and where everyone is on it. Not part of the world: see [`crate::library`].
    ///
    /// Public because the binary fills it from a file and drains its writes into the store; the
    /// server itself only ever reads it and records what a client says.
    pub library: crate::library::Library,
    /// Every account's presets. Public for the same reason as `library`.
    pub presets: crate::presets::Presets,
    /// Ticks since this server started.
    ///
    /// Counted rather than derived from `now_t / TICK_US`, which stopped meaning anything once
    /// a tick became a rate times a constant: at any rate but one, `now_t` is no longer a
    /// clean multiple of anything and "about once a real second" came out as never.
    ticks: u64,
    /// Where the last tick's real time went. See [`crate::timing`].
    pub(crate) stages: crate::timing::Stages,
    /// Everything a conversation needs, held here because a `Server` is where state lives and
    /// read only by [`crate::radio`], whose module doc is where the reasoning for all five is.
    ///
    /// The short version, because it is the surprising part: `keys` and `heard` are
    /// **schedules**, stamped with the coordinate time each thing's light lands, and gated on
    /// the clock having reached it.
    pub(crate) keys: HashMap<CraftId, HashMap<ShipId, i64>>,
    pub(crate) heard: HashMap<(CraftId, ShipId), Vec<(i64, i64)>>,
    /// Transmitted this tick, and where each will land, handed to the journal at the end of it.
    pub(crate) said: Vec<lc_store::chat::Message>,
    pub(crate) receipts: Vec<lc_store::chat::Receipt>,
    pub(crate) taught: Vec<lc_store::chat::Held>,
    /// What every craft knows and what its instruments are doing. See [`crate::instruments`].
    pub(crate) instruments: crate::instruments::Instruments,
    /// Who each craft answers automatically. Standing state, like `pursuits`, and for the same
    /// reason: it goes on answering with nobody flying the ship. See [`crate::radio::Owed`].
    pub(crate) auto_ack: HashMap<CraftId, std::collections::BTreeSet<ShipId>>,
    /// Messages addressed to each craft that have not landed yet, each a possible answer.
    pub(crate) owed: HashMap<CraftId, Vec<crate::radio::Owed>>,
    /// The messages each craft has lately answered, so a resend is not answered again. In memory
    /// only: after a restart a resend of something already answered is answered once more.
    pub(crate) answered: HashMap<CraftId, std::collections::VecDeque<(ShipId, lc_proto::MessageKey)>>,
    /// The tunables every fitted ship is read under. See [`Server::set_balance`].
    pub(crate) balance: lc_world::fitting::Balance,
    /// Craft refitting, and how many of the round's steps their owner has been told of.
    pub(crate) refitting: HashMap<CraftId, usize>,
    /// The forms each refitting craft's remaining steps leave, being measured off the tick.
    pub(crate) reserved: HashMap<CraftId, lc_world::fitting::Reservation>,
    /// Console lines waiting for the tick.
    pub(crate) commands: std::collections::VecDeque<crate::command::Queued>,
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
            // Startup, which may panic: a shard number outside the field is a misconfiguration.
            #[allow(clippy::expect_used)]
            minter: Minter::new(shard).expect("a shard inside the identifier's field"),
            pending: Vec::new(),
            budgets: HashMap::new(),
            open: false,
            blocked: HashMap::new(),
            pursuits: HashMap::new(),
            keys: HashMap::new(),
            heard: HashMap::new(),
            said: Vec::new(),
            receipts: Vec::new(),
            taught: Vec::new(),
            instruments: Default::default(),
            auto_ack: HashMap::new(),
            owed: HashMap::new(),
            answered: HashMap::new(),
            rate: 1.0,
            directs: false,
            library: crate::library::Library::default(),
            presets: crate::presets::Presets::default(),
            director: None,
            ticks: 0,
            stages: Default::default(),
            balance: lc_world::fitting::Balance::DEFAULT,
            refitting: HashMap::new(),
            reserved: HashMap::new(),
            commands: std::collections::VecDeque::new(),
        }
    }

    /// How fast this world runs, as a multiple of the design rate.
    ///
    /// One for a shard, and one is the only value a deployment has any business at: the rate
    /// is the game's, not a dial. What it is here for is a development shard staging a scene,
    /// where sixty — one Julian year a minute — turns a three-month chase into fifteen seconds
    /// of watching.
    ///
    /// Nothing about the world depends on it. Every motive is a closed form evaluated at a
    /// coordinate time, so a faster tick buys coarser event timestamps and nothing else; the
    /// step never enters an integrator, so it cannot accumulate. What *does* depend on it is
    /// the clock the client runs between statements, which is why the rate is on the wire.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = if rate.is_finite() && rate > 0.0 { rate } else { 1.0 };
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Coordinate microseconds this server's tick covers.
    fn tick_us(&self) -> i64 {
        (TICK_US as f64 * self.rate) as i64
    }

    pub fn now_t(&self) -> i64 {
        self.now_t
    }

    /// Mint past every identifier the store already holds.
    ///
    /// Called at boot with what the journal has, for the reason in
    /// [`lc_store::id::Minter::resume_from`]: the clock comes back from a checkpoint that may
    /// be behind where the shard got to, and identifiers are minted from it.
    pub fn resume_ids(&mut self, last: lc_store::id::EventId) {
        self.minter.resume_from(last);
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

    /// **Development only: let a client stage a scene.**
    ///
    /// Off, and a shard never turns it on. A client that could stage a scene could put a craft
    /// wherever it liked, which is the one thing the authority keeps for itself — every other
    /// order a client sends is a request about its own ship. See [`crate::director`].
    pub fn directing(&mut self, yes: bool) {
        self.directs = yes;
    }

    /// Give the server the stars it is authoritative over.
    ///
    /// Craft are placed into systems by where they *are*, on the tick after this, rather than
    /// by being told — so loading a world mid-flight is the same operation as a ship crossing
    /// into one.
    pub fn load_world(&mut self, world: World) {
        self.world = world;
        // A sky cached from the last world would put this world's craft under the wrong stars.
        self.instruments.forget_sky(self.world.stars());
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

    /// The craft a connection flies, if it has one.
    pub(crate) fn owned_by(&self, client: ClientId) -> Option<CraftId> {
        self.clients.get(&client).map(|state| CraftId(state.ship.0))
    }

    /// The single door on to [`crate::ability`]: one place a level is read, one table saying what
    /// it means.
    pub(crate) fn may(
        &self,
        client: ClientId,
        act: crate::ability::Act,
        about: Option<CraftId>,
    ) -> bool {
        use crate::ability::{Asking, Directing, Standing};
        crate::ability::allows(
            act,
            Asking {
                level: self
                    .clients
                    .get(&client)
                    .map_or(crate::ability::Level::PLAYER, |c| c.permission),
                standing: match about {
                    Some(id) if self.flies(client, id) => Standing::Flies,
                    _ => Standing::Otherwise,
                },
            },
            Directing(self.directs),
        )
    }

    /// **The ownership structure.** Three facts, all needed — dropping the last was a real bug,
    /// a reconnection replacing the old claim while the entry outlived a removed craft.
    fn flies(&self, client: ClientId, craft: CraftId) -> bool {
        self.owners.get(&craft) == Some(&client)
            && self.fleet.get(craft).is_some()
            && self.clients.get(&client).is_some_and(|state| state.ship.0 == craft.0)
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
            // Admitted rather than signed in — a test, or a probe — so there is no account to
            // keep a place in a book under.
            account: String::new(),
            // Nothing received yet, so nothing is provable: an intent may be stamped anywhere
            // from the beginning of time up to now.
            last_reception_t: i64::MIN,
            // And nothing sent yet. Not `now_t`, which would mean "told everything up to this
            // instant" and would swallow an event stamped at exactly this instant -- a ship's
            // own act, on the tick it acts. The same start serves a brand-new client, which is
            // the catch-up path run from the beginning.
            cursor_t: i64::MIN,
            had_contacts: false,
            // Admitted, not ticketed: a test's client is a player, and `directing` is what lets it
            // develop.
            permission: crate::ability::Level::PLAYER,
            backlog_sent: false,
            learned: Default::default(),
            logged_s: f64::NEG_INFINITY,
            retained_sent: false,
            analyzing_sent: 0,
            doing_sent: None,
        });
        self.aboard(CraftId(ship_id.0));
    }

    /// One tick. The order is the whole of it.
    pub async fn tick(&mut self, wire: &mut impl Transport) -> Result<(), JournalError> {
        // 1. Advance.
        self.stages.restart();
        self.ticks += 1;
        self.now_t += self.tick_us();
        let now_s = self.now_t as f64 * 1.0e-6;
        // Membership before motion, as the client orders it: a station and a conic are both
        // positions *in* a system, and one resolved against the wrong system is a craft in the
        // wrong place.
        self.resync_systems(now_s);
        self.stages.mark("resync");
        // Nothing moved on the server before this. Reading a worldline never needed it — every
        // motive is a closed form — but the transitions do: a crossing that arrives becomes a
        // station, and a ballistic arc folds the patch it was solved for.
        self.fleet.advance(now_s, self.tick_us() as f64 * 1.0e-6);
        self.stages.mark("advance");
        // Room to write into, kept ahead rather than made on demand. Cheap: the journal holds
        // the range it has already made and this is a comparison until the window moves.
        self.journal.prepare(self.now_t, self.now_t + PREPARE_AHEAD_US).await?;
        self.stages.mark("prepare");
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
        self.stages.mark("intents");
        // After the intents, so a command sees every order that arrived with it already flown.
        self.run_commands(wire, &mut events, &mut deliveries);
        self.stages.mark("commands");
        // After motion, so every instrument looks from where its craft now is, and after the
        // intents, so a report composed this tick carries nothing its craft learns in it.
        self.run_instruments();
        // 3. A staged scene, if one is running. Before the pursuits and after the intents for
        // the same reason they are after the intents: a chase a beat ordered is not re-solved
        // against the plan it has only just made.
        self.direct(wire, &mut events, &mut deliveries);
        // After the intents, so an intercept ordered this tick is not immediately re-solved
        // against the plan it just made.
        self.steer_pursuits(wire, &mut events, &mut deliveries);
        // After the intents, so an auto-ack switched off this tick answers nothing more.
        self.answer_owed(self.now_t - self.tick_us(), &mut events, &mut deliveries);
        self.announce_drives(self.now_t - self.tick_us(), &mut events, &mut deliveries);
        self.keep_accounts(wire);
        self.stages.mark("scene");
        self.schedule_landings(&events, &deliveries);
        self.land_reports();
        self.stages.mark("landings");
        self.journal.write(&events, &deliveries).await?;
        self.write_conversations().await?;
        self.stages.mark("journal");
        self.pending = events;
        self.state_the_clock(wire);
        self.tell_learned(wire);
        if self.ticks % u64::from(TICKS_PER_SECOND) == 0 {
            self.tell_doing(wire);
        }
        self.stages.mark("tell");
        // 3 and 4. Everything that has arrived since the last tick, through the gate.
        let flushed = self.flush(wire).await;
        self.stages.mark("flush");
        flushed
    }

    /// Where the last tick's real time went.
    pub fn last_tick(&self) -> &crate::timing::Stages {
        &self.stages
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
                    Some((ship_id, name, account)) => {
                        let pursuing = self.pursuits.get(&CraftId(ship_id.0)).map(|p| lc_proto::Pursuit {
                            quarry: p.quarry,
                            closeness: p.closeness.into(),
                            approach: p.approach,
                        });
                        // What it is doing, not merely where it is: an account coming back
                        // finds its craft mid-orbit or mid-burn, and a welcome that said only
                        // the position put it back at rest there. See `lc_world::resume`.
                        let ship = self
                            .ship(ship_id)
                            .map(|craft| (&craft.motion.snapshot()).into())
                            .unwrap_or_else(adrift_at_the_origin);
                        wire.send(from, Outbound::Welcome {
                            client_id: from,
                            protocol: PROTOCOL_VERSION,
                            ship_id,
                            now_t: self.now_t,
                            name,
                            rate: self.rate,
                            ship,
                        });
                        // After the welcome, which is the message a client has to have first.
                        if let Some(pursuit) = pursuing {
                            wire.send(from, Outbound::Pursuing { ship_id, pursuit });
                        }
                        self.tell_fitted(wire, CraftId(ship_id.0));
                        self.tell_observing(wire, from, CraftId(ship_id.0));
                        if self.auto_ack.contains_key(&CraftId(ship_id.0)) {
                            self.tell_auto_acking(wire, from, ship_id);
                        }
                        // A shard with no shelf says nothing about one, and its clients show an
                        // empty bookcase rather than a broken one.
                        if !self.library.is_empty() {
                            wire.send(from, Outbound::Library {
                                base: self.library.base.clone(),
                                books: self.library.books.clone(),
                            });
                            wire.send(from, Outbound::Reading(self.library.marks_for(&account)));
                        }
                        wire.send(from, Outbound::Presets(self.presets.for_account(&account)));
                    }
                    None => wire.send(from, Outbound::Unauthenticated),
                }
            }
            Inbound::SetReading(mark) => {
                // Silently ignored from a connection with no account, which is every anonymous
                // one: there is nowhere to keep a place for somebody who will not be back.
                let account = self.clients.get(&from).map(|c| c.account.clone()).unwrap_or_default();
                if account.is_empty() {
                    return;
                }
                // Nothing is said back. A bookmark is not an order and there is no outcome to
                // report; the client already knows where it is, because it is the one reading.
                self.library.set(&account, mark);
            }
            Inbound::Act(Intent { ship_id, order: Order::AutoAck { with, on }, .. }) => {
                if self.set_auto_ack(from, ship_id, with, on) {
                    self.tell_auto_acking(wire, from, ship_id);
                } else {
                    wire.send(from, Outbound::Refused { ship_id, reason: Refusal::NotYours });
                }
            }
            Inbound::Act(intent) => {
                let ship_id = intent.ship_id;
                match self.act(from, intent, events, deliveries) {
                    Ok(applied) => {
                        // An intercept is a policy, so what it *did* — the first approach —
                        // is not something the client can work out from the order coming back.
                        if matches!(applied.order, Order::Intercept { .. }) {
                            self.tell_flying(wire, CraftId(ship_id.0));
                        }
                        wire.send(from, Outbound::Accepted {
                            ship_id,
                            event_id: applied.event_id,
                            at_t: applied.at_t,
                            order: applied.order,
                        });
                        // Every accepted order may have committed, spent or refunded energy.
                        self.tell_fitted(wire, CraftId(ship_id.0));
                    }
                    Err(reason) => wire.send(from, Outbound::Refused { ship_id, reason }),
                }
            }
            Inbound::Command { seq, line } => self.enqueue(from, seq, line, wire),
            Inbound::SavePreset { name, form } => self.keep_preset(from, name, Some(form), wire),
            Inbound::DeletePreset { name } => self.keep_preset(from, name, None, wire),
            Inbound::Stage { scenario } => self.staged(from, &scenario, wire),
            Inbound::Grant { joules } => self.granted(from, joules, wire),
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
        // **The gate.** `NotYours` whichever half failed: a client that could tell them apart could
        // probe for both.
        let id = CraftId(intent.ship_id.0);
        if !self.may(from, crate::ability::Act::of(&intent.order), Some(id)) {
            return Err(Refusal::NotYours);
        }
        let state = self.clients.get(&from).ok_or(Refusal::NotYours)?;

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
        let at_s = at as f64 * 1.0e-6;
        // Orders about what a craft knows or is watching put nothing on the air, so they are
        // not events. They still get an identifier, so an acceptance looks like any other.
        if matches!(intent.order, Order::SetDuty { .. } | Order::NameIt { .. } | Order::RetainRaw { .. } | Order::Analyze) {
            let order = self.act_on_knowledge(id, &intent.order, at_s)?;
            let event_id = self.minter.mint(at).ok_or(Refusal::Impossible)?.get();
            return Ok(Applied { event_id, at_t: at, order });
        }
        let lights_the_drive = matches!(
            intent.order,
            Order::Burn { .. } | Order::SetCourse { .. } | Order::Cross { .. } | Order::Intercept { .. }
        );
        if lights_the_drive && self.fleet.get(id).is_some_and(|craft| craft.is_refitting(at_s)) {
            return Err(Refusal::Refitting);
        }

        // Set by the two arms that transmit something aimed. Everything else a ship does is
        // seen in every direction, because a plume and a hull are not pointed at anyone.
        let mut beam = Beam::OMNI;
        let mut utterance: Option<crate::radio::Utterance> = None;
        // Whether this order put something on the air. Not `utterance.is_some()`: a survey
        // report is a transmission with no transcript line, and a transmitter still does not
        // hear itself.
        let mut transmitted = false;
        // How far a report has told its recipient. Held until the event exists: anything below
        // can still refuse, and a mark moved for a report that was never sent loses that slice
        // for that recipient for good.
        let mut reported = None;

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
                crate::fitting::afford_burn(craft, at_s, beta)?;
                let here = craft.position_at(at as f64) / lc_world::motion::LIGHT_US_PER_LY;
                // Edited rather than rebuilt. Replacing the craft lost everything about it
                // that was not motion — its name, its hull size — and, worse, threw away the
                // record of what it had been doing, so the burn rewrote its whole past.
                craft.drift_from(here, beta, at as f64 * 1.0e-6);
                // A burn is not silent -- it is the most visible thing a ship does -- but what
                // it radiates is the drive's business. Nominal, until there is a drive model.
                (
                    KIND_BURN,
                    BURN_POWER_W,
                    // Serialized, not formatted: `DVec3`'s `Debug` writes `DVec3(x, y, z)`,
                    // which the payload column will not take, and a store that refuses a row
                    // stops the shard.
                    serde_json::json!({ "beta": beta.to_array() }).to_string(),
                    Order::Burn { beta: beta.to_array() },
                )
            }
            Order::SetCourse { course, accel_g, max_beta } => {
                if !accel_g.is_finite() || *accel_g <= 0.0 || !(*max_beta > 0.0) {
                    return Err(Refusal::Impossible);
                }
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                // Asked for, not stated. The ceiling is the craft's own, so a client cannot
                // fly a better ship than it has by sending a larger number.
                let mut drive = craft.rated_drive(at_s);
                drive.accel_g = accel_g.min(drive.accel_g);
                drive.max_beta = max_beta.min(drive.max_beta);
                // Not shadowed: the proto course is wanted again below, to say back what was
                // applied. Only the drive is clamped, never the course itself.
                let flown: Course = course.clone().into();
                let drive = crate::fitting::within_budget(craft, at_s, drive, |drive| {
                    Change::SetCourse { course: flown.clone(), drive }
                })?;
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
                    Order::SetCourse {
                        course: course.clone(),
                        accel_g: drive.accel_g,
                        max_beta: drive.max_beta,
                    },
                )
            }
            Order::Cross { star, accel_g, max_beta } => {
                if !accel_g.is_finite() || *accel_g <= 0.0 || !(*max_beta > 0.0) {
                    return Err(Refusal::Impossible);
                }
                // Resolved here, against this shard's own catalog. A star it does not hold
                // is not somewhere anyone may fly to, whatever the client believes it has.
                let to_ly = self.world.star_at(*star).ok_or(Refusal::Impossible)?;
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                let mut drive = craft.rated_drive(at_s);
                drive.accel_g = accel_g.min(drive.accel_g);
                drive.max_beta = max_beta.min(drive.max_beta);
                let drive = crate::fitting::within_budget(craft, at_s, drive, |drive| {
                    Change::Cross { to_ly, drive }
                })?;
                craft
                    .apply(&Change_ {
                        ship: motion_id(id),
                        at_t: at as f64 * 1.0e-6,
                        change: Change::Cross { to_ly, drive },
                    })
                    .map_err(refusal_for)?;
                (
                    KIND_BURN,
                    BURN_POWER_W,
                    format!("{{\"cross\":{star},\"accel_g\":{}}}", drive.accel_g),
                    Order::Cross { star: *star, accel_g: drive.accel_g, max_beta: drive.max_beta },
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
                // The order puts nothing out. The plume going dark is still seen, by anyone
                // who could see it lit — `crate::drive` states it — and after that the ship is.
                (KIND_CUT, 0.0, "{}".to_string(), Order::CutDrive)
            }
            Order::Intercept { ship_id, closeness, approach } => {
                let quarry = *ship_id;
                // The first plan is made here rather than left to the next tick, so that a
                // player who presses the button sees the ship move on the same round trip as
                // any other order. What keeps it flying is the standing policy below.
                let now_s = at as f64 * 1.0e-6;
                let seen = chase::sighting(&self.fleet, id, quarry, at)
                    .ok_or(Refusal::NotInSight)?;
                // Closing in on the quarry already being chased keeps what has been measured of
                // it, so an escort does not lose its acceleration for a tick.
                let last_seen =
                    self.pursuits.get(&id).filter(|p| p.quarry == quarry).and_then(|p| p.last_seen);
                let craft = self.fleet.get(id).ok_or(Refusal::NotYours)?;
                let arrival = chase::arrival(*approach, &self.balance, craft, now_s);
                match chase::plan(&self.fleet, craft, &seen, last_seen.as_ref(), (*closeness).into(), arrival, now_s) {
                    Ok(plan) => {
                        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                        let before = craft.clone();
                        plan.fly(craft, now_s);
                        if !crate::fitting::can_pay_for_its_plan(craft, now_s) {
                            *craft = before;
                            return Err(Refusal::NoEnergy);
                        }
                    }
                    // Already alongside. The order still stands — it is a policy, and the
                    // policy's job from here is to keep it there.
                    Err(lc_world::pursuit::Refused::AlreadyThere) => {}
                    Err(lc_world::pursuit::Refused::TooFast) => return Err(Refusal::TooFast),
                }
                self.pursuits.insert(id, Pursuit {
                    quarry,
                    closeness: (*closeness).into(),
                    approach: *approach,
                    last_plan_t: at,
                    last_seen,
                });
                (
                    KIND_BURN,
                    BURN_POWER_W,
                    format!("{{\"intercept\":{}}}", quarry.0),
                    Order::Intercept { ship_id: quarry, closeness: *closeness, approach: *approach },
                )
            }
            Order::BreakOff => {
                self.pursuits.remove(&id);
                // No further corrections: whatever it was closing or keeping station on, it now
                // keeps the velocity it has, on whatever conic that is.
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                if craft.motion.pursuing().is_some() {
                    craft
                        .apply(&Change_ {
                            ship: motion_id(id),
                            at_t: at as f64 * 1.0e-6,
                            change: Change::CutDrive,
                        })
                        .map_err(refusal_for)?;
                }
                (KIND_CUT, 0.0, "{}".to_string(), Order::BreakOff)
            }
            Order::Refit { target } => {
                self.refit(id, target, at_s)?;
                // Drones are quiet. What a refit does to the hull is seen when its light arrives,
                // as the form any contact is drawn in; see `chase::contacts`.
                (KIND_CUT, 0.0, "{\"refit\":true}".to_string(), Order::Refit { target: target.clone() })
            }
            // H6 and E3 build these.
            Order::FieldMode { .. } | Order::Emit { .. } => return Err(Refusal::NotBuilt),
            Order::CancelRefit => {
                self.fleet.get_mut(id).ok_or(Refusal::NotYours)?.cancel_refit(at_s);
                self.refitting.remove(&id);
                (KIND_CUT, 0.0, "{\"refit\":false}".to_string(), Order::CancelRefit)
            }
            // Returned from above, before anything is put on the air.
            Order::SetDuty { .. } | Order::NameIt { .. } | Order::RetainRaw { .. } | Order::Analyze => {
                return Err(Refusal::Impossible);
            }
            Order::Say { .. } | Order::OfferKey { .. } | Order::SendReport { .. } => {
                // The whole of it in `crate::radio`, because everything a transmission needs
                // to decide — the keyring, the aim, the acknowledgment window — is that
                // module's and none of it is this one's.
                let spoken = self.compose(id, intent.ship_id, &intent.order, at)?;
                beam = spoken.beam;
                transmitted = true;
                utterance = spoken.said;
                reported = spoken.reported;
                (spoken.kind, crate::radio::SIGNAL_POWER_W, spoken.payload, intent.order.clone())
            }
            // Unreachable: `handle` takes it before it gets here, because it makes no event.
            Order::AutoAck { .. } => return Err(Refusal::Impossible),
        };

        // A flight order replaces whatever the ship was doing, the standing intercept included.
        // Left in place, the next tick steered the ship back to its quarry over this order.
        if matches!(
            applied,
            Order::Burn { .. } | Order::SetCourse { .. } | Order::Cross { .. } | Order::CutDrive
        ) {
            self.pursuits.remove(&id);
        }

        let event_id = self.put_on_air(
            id, at, kind, power_w, payload, &beam, transmitted, utterance, events, deliveries,
        )?;
        if let Some((to, through)) = reported {
            self.reported(id, to, through);
        }
        Ok(Applied { event_id, at_t: at, order: applied })
    }

    /// Write an event and schedule it to everyone it reaches, and file it as a conversation
    /// when it is one. `transmitted` is a radio transmission, which its sender does not hear.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put_on_air(
        &mut self,
        id: CraftId,
        at: i64,
        kind: i16,
        power_w: f64,
        payload: String,
        beam: &Beam,
        transmitted: bool,
        utterance: Option<crate::radio::Utterance>,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<i64, Refusal> {
        let at_position = self.fleet.get(id).ok_or(Refusal::NotYours)?.position_at(at as f64);
        let event = Event {
            id: self.minter.mint(at).ok_or(Refusal::Impossible)?.get(),
            source: ShipId(id.0),
            t: at,
            at: at_position,
            kind,
            power_w,
            payload,
        };
        let mut landings = Vec::new();
        for observer in self.fleet.iter() {
            // A transmitter does not receive its own transmission. True of a real radio, which
            // is not listening on the frequency it is shouting into — and load-bearing here,
            // because a sender that heard itself would have every message twice in its own
            // transcript: once as its own, and once as an arrival from a stranger with its
            // own identifier. Everything else a ship does, it does go on seeing.
            if transmitted && observer.id == id {
                continue;
            }
            if let Some(scheduled) = schedule(&event, beam, observer) {
                landings.push((observer.id, scheduled.arrive_t, scheduled.strength));
                deliveries.push(scheduled);
            }
        }
        let event_id = event.id;
        if let Some(said) = utterance {
            self.remember(event_id, id, at, &said, &landings);
            self.owe(id, event.at, !beam.is_omni(), &said, &landings);
        }
        events.push(event);
        Ok(event_id)
    }

    /// Verify a ticket and bind the connection to the account's craft.
    ///
    /// The client never names its own ship: the identifier comes back in `Welcome` and is
    /// looked up from the ticket's subject. A client that could ask for a `ShipId` could ask
    /// for someone else's.
    fn sign_in(&mut self, from: ClientId, ticket: &str) -> Option<(ShipId, String, String)> {
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
                name: format!("Traveler {}", from.0),
                exp: i64::MAX,
                jti: format!("anonymous:{}", from.0),
                perm: 0,
            },
            Err(_) => return None,
        };

        // A craft this shard could not read is still a craft this shard *has*. Minting a second
        // one for the account would put two rows on one account, and every checkpoint after
        // that would fail on the unique constraint -- so the shard would stop saving anything,
        // for everyone, quietly. Refusing one player loudly is the better failure.
        if let Some(why) = self.blocked.get(&claims.sub) {
            eprintln!("refusing {}: its saved craft will not load: {why}", claims.sub);
            return None;
        }

        let ship = match self.by_account.get(&claims.sub) {
            Some(ship) => *ship,
            None => {
                // First sign-in: the account gets a craft. Where a new player starts is a game
                // question and this is the crudest possible answer to it.
                let ship = ShipId(self.next_ship);
                self.next_ship += 1;
                // Inside a system if this server has one, because the origin is empty space
                // and a player put there has nothing to look at or fly to.
                let at = self.world.start().unwrap_or(DVec3::ZERO);
                let mut craft = Craft::at(CraftId(ship.0), Kind::Ship, at);
                craft.name = Some(claims.name.clone());
                self.fit_new(&mut craft);
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
            account: claims.sub.clone(),
            last_reception_t: i64::MIN,
            cursor_t: i64::MIN,
            had_contacts: false,
            permission: crate::ability::Level::from_claim(claims.perm),
            backlog_sent: false,
            learned: Default::default(),
            logged_s: f64::NEG_INFINITY,
            retained_sent: false,
            analyzing_sent: 0,
            doing_sent: None,
        });
        // Being welcomed is not the same fact as owning the craft, and `act` checks the
        // second. Without this a signed-in client is welcomed, given a ship, and then refused
        // `NotYours` on every order it sends — which no in-process test caught, because the
        // ones about orders call `admit` and the ones about tickets never send an order.
        self.owners.insert(CraftId(ship.0), from);
        Some((ship, claims.name, claims.sub))
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
            .map(|(id, at)| (id, self.world.system_at(at, now_s)))
            .collect();

        for (id, system) in placements {
            let Some(craft) = self.fleet.get_mut(id) else { continue };
            if craft.system.as_ref().map(|s| s.star) == system.as_ref().map(|s| s.star) {
                continue;
            }
            craft.enter(system, now_s);
        }

        // Once a second rather than every tick: the sweep walks every loaded system, and what
        // it is looking for takes a survey rotation to become true.
        if self.ticks % u64::from(TICKS_PER_SECOND) == 0 {
            // The fleet is the record of who is in what, so it is what pins a system. Read
            // after placement, or a craft that has just arrived is not counted as being there.
            // A system under survey is pinned too: past the cap, surveys would otherwise evict
            // each other's systems every second and rebuild them the next tick.
            let occupied: Vec<lc_world::sky::StarId> = self
                .fleet
                .iter()
                .filter_map(|craft| craft.system.as_ref().map(|s| s.star))
                .chain(self.instruments.aboard.values().filter_map(|a| a.observatory.duty.surveying()))
                .collect();
            self.world.sweep(occupied, now_s);
        }
    }

    /// Say what time it is, about once a real second.
    ///
    /// Cheap — one integer per client per second — and it is what keeps a client's own clock
    /// from wandering. A client draws frames far faster than this and must run its own clock
    /// between them; a browser tab in the background has its frames throttled and its clock
    /// nearly stops. Without a statement to come back to, that divergence is permanent, and a
    /// client that disagrees about *when* the ship is disagrees about where it is.
    fn state_the_clock(&mut self, wire: &mut impl Transport) {
        if self.ticks % u64::from(TICKS_PER_SECOND) != 0 {
            return;
        }
        let (now_t, rate) = (self.now_t, self.rate);
        for client in self.clients.keys().copied().collect::<Vec<_>>() {
            wire.send(client, Outbound::Clock { now_t, rate });
        }
    }


    /// Tell a craft's owner what it is now flying.
    ///
    /// Only for changes the owner did not ask for — everything else it folded itself when its
    /// order came back accepted, and saying it twice would be a second copy of an answer.
    pub(crate) fn tell_flying(&self, wire: &mut impl Transport, id: CraftId) {
        let Some(craft) = self.fleet.get(id) else { return };
        let Some(owner) = self.owners.get(&id).copied() else { return };
        wire.send(owner, Outbound::Flying {
            ship_id: ShipId(id.0),
            ship: (&craft.motion.snapshot()).into(),
        });
        // A new plan committed energy, and the client took the motion whole rather than folding
        // it, so its account is stale until told.
        self.tell_fitted(wire, id);
    }

    /// Fly every standing intercept one tick.
    ///
    /// Each craft steers by what **it** can see, which is the same sighting its owner is sent
    /// and no fresher. A quarry that maneuveres is therefore chased on stale information until
    /// the news arrives — seconds to hours across a system — and that delay is the game rather
    /// than a shortcoming of the guidance.
    ///
    /// What to do is [`chase::decide`]'s; this is the half that may touch the fleet.
    fn steer_pursuits(
        &mut self,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now = self.now_t;
        let now_s = now as f64 * 1.0e-6;
        for (id, plan) in chase::decide(&self.fleet, &mut self.pursuits, &self.balance, now) {
            let Some(craft) = self.fleet.get_mut(id) else { continue };
            let Some(plan) = plan else {
                self.pursuits.remove(&id);
                // Given up, so nothing is steering: a station kept beside a quarry nobody can
                // see is a ship holding formation with a guess. Silent, as cutting always is,
                // and its owner is told what it is doing now.
                if craft.motion.pursuing().is_some() {
                    let cut = Change_ { ship: motion_id(id), at_t: now_s, change: Change::CutDrive };
                    if craft.apply(&cut).is_ok() {
                        self.tell_flying(wire, id);
                    }
                }
                continue;
            };
            plan.fly(craft, now_s);
            if let Some(entry) = self.pursuits.get_mut(&id) {
                entry.last_plan_t = now;
            }
            // A burn, and burns are the loudest thing a ship does. Everyone in range learns
            // that this craft maneuvered, at light delay, exactly as they would for any other.
            self.emit(id, KIND_BURN, BURN_POWER_W, "{}".into(), now, events, deliveries);
            // Its owner learns *what* it is flying, at once and directly. Nobody else does:
            // this is a ship being told about itself.
            self.tell_flying(wire, id);
        }
    }

    /// Write an event for something a craft did, and schedule it to everyone who will see it.
    pub(crate) fn emit(
        &mut self,
        id: CraftId,
        kind: i16,
        power_w: f64,
        payload: String,
        at: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let Some(craft) = self.fleet.get(id) else { return };
        let from = craft.position_at(at as f64);
        self.emit_from(id, from, kind, power_w, payload, at, events, deliveries);
    }

    /// [`Server::emit`] from a stated position, light-microseconds: at a jump the worldline has
    /// two.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_from(
        &mut self,
        id: CraftId,
        from: DVec3,
        kind: i16,
        power_w: f64,
        payload: String,
        at: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let Some(event_id) = self.minter.mint(at) else { return };
        let event = Event {
            id: event_id.get(),
            source: ShipId(id.0),
            t: at,
            at: from,
            kind,
            power_w,
            payload,
        };
        for observer in self.fleet.iter() {
            // Omnidirectional: everything that reaches here is a plume, a hull or an engine
            // going out, and none of those are pointed at anybody.
            if let Some(scheduled) = schedule(&event, &Beam::OMNI, observer) {
                deliveries.push(scheduled);
            }
        }
        events.push(event);
    }

    /// Release what has arrived. **The only place anything reaches a client.**
    ///
    /// Every sighting and every contact here goes through a [`Cleared::clear`], which is the
    /// only constructor of the only type [`Outbound::Sightings`] and [`Outbound::Present`] can
    /// hold. Adding a second path out would mean adding a second way to build a `Cleared`, and
    /// there is not one.
    async fn flush(&mut self, wire: &mut impl Transport) -> Result<(), JournalError> {
        let now = self.now_t;
        let mut contacts = chase::contacts(&self.fleet, &self.clients, now);
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();

        let mut reading: Vec<(ClientId, ShipId, i64)> = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(state) = self.clients.get_mut(&id) else { continue };
            // Every tick there is anything to say, and once more when there stops being: a
            // client that has stopped hearing about a contact has to be able to tell that from
            // a message that did not arrive, and one that has never had any needs no message
            // twenty times a second repeating it.
            let seen = contacts.remove(&id).unwrap_or_default();
            if !seen.is_empty() || state.had_contacts {
                state.had_contacts = !seen.is_empty();
                wire.send(id, Outbound::Present(seen));
            }
            let (ship, cursor_t, backlog_sent) = (state.ship, state.cursor_t, state.backlog_sent);
            if self.fleet.get(CraftId(ship.0)).is_none() {
                continue;
            }
            // The transcript, once per connection. See `crate::radio`.
            if !backlog_sent {
                self.send_backlog(id, ship, now, wire).await;
            }
            reading.push((id, ship, cursor_t));
        }

        // The proven read, one range scan over `(observer_id, arrive_t)` per client, all asked
        // at once: one connection pipelines them, so the tick waits one round trip rather than
        // one per client in turn.
        let journal = &self.journal;
        let dues = futures_util::future::try_join_all(
            reading.iter().map(|&(_, ship, cursor_t)| journal.due(ship, cursor_t, now)),
        )
        .await?;

        for ((id, ship_id, _), due) in reading.into_iter().zip(dues) {
            let Some(ship) = self.fleet.get(CraftId(ship_id.0)) else { continue };
            let Some(state) = self.clients.get_mut(&id) else { continue };
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
                    payload: crate::radio::redact(event.kind, &event.payload, ship_id),
                };
                match Cleared::<Sighting>::clear(sighting, now, ship.noise_floor) {
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
/// A welcome for a ship the fleet does not have, which is a state that should not arise: the
/// sign-in that reached here either found a craft or made one. Still at rest at the origin
/// rather than a panic, because losing a connection is better than losing the shard.
fn adrift_at_the_origin() -> lc_proto::Motion {
    (&lc_world::motion::ShipState::at(DVec3::ZERO).snapshot()).into()
}

fn motion_id(id: CraftId) -> lc_world::motion::ShipId {
    lc_world::motion::ShipId(id.0)
}

pub(crate) fn refusal_for(rejected: Rejected) -> Refusal {
    match rejected {
        Rejected::NotInASystem | Rejected::NoSuchPlace | Rejected::AlreadyThere => {
            Refusal::Impossible
        }
    }
}

pub const KIND_TRANSMIT: i16 = lc_proto::kind::TRANSMIT;
pub const KIND_BURN: i16 = lc_proto::kind::BURN;
/// Cutting the engine. Distinct from a burn because it radiates nothing.
pub const KIND_CUT: i16 = lc_proto::kind::CUT;
pub const KIND_DRIVE: i16 = lc_proto::kind::DRIVE;

/// What a burn radiates, until there is a drive model to ask.
pub const BURN_POWER_W: f64 = 1.0e12;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Memory;
use crate::transport::Loopback;

    /// Two light-hours, in light-microseconds. Far enough that the delay is many ticks.
    const TWO_LIGHT_HOURS: f64 = 7_200.0 * 1_000_000.0;

    fn presences(messages: &[Outbound]) -> Vec<&lc_proto::Presence> {
        messages
            .iter()
            .flat_map(|m| match m {
                Outbound::Present(list) => list.iter().map(|c| c.get()).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn sightings(messages: &[Outbound]) -> Vec<&Sighting> {
        messages
            .iter()
            .flat_map(|m| match m {
                Outbound::Sightings(list) => list.iter().map(|c| c.get()).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// **A drive transition is an event, at the instant it happens.** A crossing lights, goes
    /// out for the flip, lights again and goes out on arrival. The flip is a sixty-second coast
    /// inside a 438-second tick, so a statement once a tick could never show it; each of the
    /// four has to reach the other ship as its own sighting, stamped when it happened.
    #[tokio::test]
    async fn a_flip_shorter_than_a_tick_reaches_an_observer_as_two_events() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let (flier, watcher) = (ClientId(1), ClientId(2));
        server.admit(flier, Craft::at(CraftId(1), lc_world::craft::Kind::Ship, DVec3::ZERO), 0.0);
        let beside = DVec3::new(0.0, 1.0e6 / lc_world::system::M_PER_LY, 0.0);
        server.admit(watcher, Craft::at(CraftId(2), lc_world::craft::Kind::Ship, beside), 0.0);

        // Twenty thousand kilometers, past the standoff a crossing stops short of a star by.
        let trip_ly = 2.0e7 / lc_world::system::M_PER_LY;
        let to_ly = DVec3::new(lc_world::flight::STANDOFF_LY + trip_ly, 0.0, 0.0);
        // Folded directly: between the stars there is no course to set, and an order to cross
        // would need a catalog star there.
        let craft = server.fleet_mut().get_mut(CraftId(1)).unwrap();
        let drive = craft.turning(craft.kind.drive());
        craft
            .apply(&lc_world::motion::Event {
                ship: lc_world::motion::ShipId(1),
                at_t: 1.0,
                change: lc_world::motion::Change::Cross { to_ly, drive },
            })
            .unwrap();
        let lc_world::motion::Motive::Crossing(cruise) = &server.ship(ShipId(1)).unwrap().motion.motive else {
            panic!("premise: a crossing")
        };
        let arrive_t = ((cruise.start_s + cruise.duration_s()) * 1e6) as i64;

        let mut seen = Vec::new();
        while server.now_t() < arrive_t + 3 * server.tick_us() {
            seen.extend(sightings(&wire.take(watcher)).into_iter().cloned());
            server.tick(&mut wire).await.unwrap();
        }
        seen.extend(sightings(&wire.take(watcher)).into_iter().cloned());
        let drive: Vec<(i64, f64)> = seen
            .iter()
            .filter(|s| s.kind == KIND_DRIVE && s.source_id == 1)
            .map(|s| {
                let change: lc_proto::DriveChange = serde_json::from_str(&s.payload).unwrap();
                (s.emitted_t, change.power_w)
            })
            .collect();

        let lit: Vec<bool> = drive.iter().map(|(_, power)| *power > 0.0).collect();
        assert_eq!(lit, [true, false, true, false], "{drive:?}");
        let coast_us = drive[2].0 - drive[1].0;
        assert!(
            coast_us > 0 && coast_us < server.tick_us(),
            "premise: a flip inside one tick, got {coast_us} us",
        );
        assert!((drive[3].0 - arrive_t).abs() < 1_000, "went out at {}, arrived at {arrive_t}", drive[3].0);
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
        assert!(wire.take(first).iter().any(|m| matches!(
            m,
            Outbound::Refused { reason: Refusal::NotYours, .. }
        )));
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
        assert!(matches!(wire.take(client).as_slice(), [Outbound::Welcome { .. }, Outbound::Fitted { .. }, Outbound::Observing { .. }, Outbound::Presets(_)]));
    }

    /// **A faster world is the same world.** Its tick buys coarser event timestamps and
    /// nothing else: every motive is a closed form evaluated at a coordinate time, so the step
    /// never enters an integrator and cannot accumulate. Sixty ticks at the design rate and one
    /// at sixty times it must put the same craft in the same place.
    #[tokio::test]
    async fn the_rate_does_not_change_where_anything_is() {
        let at = DVec3::new(500_000.0, 0.0, 0.0);
        let beta = DVec3::new(0.3, -0.1, 0.0);

        let mut slow = Server::new(Memory::default(), 0, 1);
        slow.fleet_mut().insert(crate::world::coasting(ShipId(1), at, beta, 0));
        let mut fast = Server::new(Memory::default(), 0, 1);
        fast.set_rate(60.0);
        fast.fleet_mut().insert(crate::world::coasting(ShipId(1), at, beta, 0));

        let mut wire = Loopback::new();
        for _ in 0..60 {
            slow.tick(&mut wire).await.unwrap();
        }
        fast.tick(&mut wire).await.unwrap();

        assert_eq!(slow.now_t(), fast.now_t(), "sixty slow ticks is not one fast one");
        let there = |s: &Server<Memory>| s.ship(ShipId(1)).unwrap().motion.position_ly;
        assert_eq!(there(&slow), there(&fast), "the rate moved the ship");
    }

    /// A rate nobody could run at is not adopted. Zero would stop the clock and a negative one
    /// would run it backwards, and both would reach a client as a statement about the world.
    #[test]
    fn a_nonsense_rate_is_refused_rather_than_kept() {
        let mut server = Server::new(Memory::default(), 0, 1);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            server.set_rate(bad);
            assert_eq!(server.rate(), 1.0, "{bad} was taken as a rate");
        }
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

    /// **Where another ship is drawn, and when.**
    ///
    /// The position a client is handed is retarded: a craft two light-hours away is reported
    /// where it was two hours ago, so a craft that is moving is reported somewhere it is not.
    /// Asserting the *gap* rather than merely that a contact arrived is the point — a server
    /// that reported everybody's present position would pass any test that only checked the
    /// name, and would hand every client a faster-than-light view of the traffic around it.
    #[tokio::test]
    async fn a_contact_is_reported_where_its_light_left_and_not_where_it_is() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let watcher = ClientId(2);
        server.admit(watcher, crate::world::still(ShipId(2), DVec3::ZERO), 0.0);
        // Drifting across the line of sight, so where it was and where it is differ in `y`
        // alone and the light delay is the same two hours throughout.
        let beta = DVec3::new(0.0, 0.1, 0.0);
        let adrift =
            crate::world::coasting(ShipId(1), DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0), beta, 0);
        server.admit(ClientId(1), adrift, 0.0);

        // Far enough in that the light now landing left after the epoch rather than before it.
        for _ in 0..40 {
            server.tick(&mut wire).await.unwrap();
        }
        // The last tick's alone: a contact is stated every tick, so the accumulated queue holds
        // one of these per tick and only the newest is about now.
        wire.take(watcher);
        server.tick(&mut wire).await.unwrap();
        let seen = wire.take(watcher);
        let contacts = presences(&seen);
        assert_eq!(contacts.len(), 1, "one other craft is in the system");
        let contact = contacts[0];
        assert_eq!(contact.ship_id, ShipId(1));
        assert_eq!(contact.length_m, Kind::Ship.length_m());

        let now = server.now_t();
        assert_eq!(contact.arrive_t, now, "this is the light landing now");
        // The delay *is* the distance to the reported point, which is the whole of the solve.
        // Not "about two light-hours": the craft drifts across the line of sight, so the true
        // range grows, and an assertion against the nominal separation would be asserting the
        // approximation rather than the answer.
        let delay = (now - contact.emitted_t) as f64;
        let reported = DVec3::from_array(contact.at_ly);
        let here = server.ship(ShipId(2)).unwrap().motion.position_ly;
        let crossed = (reported - here).length() * lc_world::motion::LIGHT_US_PER_LY;
        assert!(
            (delay - crossed).abs() < 1.0,
            "light took {delay} microseconds to cross {crossed}",
        );
        assert!(delay > TWO_LIGHT_HOURS, "and the separation is at least the two hours set up");

        // And the gap that delay opens up, against where the craft actually is now.
        let actually = server.ship(ShipId(1)).unwrap().motion.position_ly;
        let behind = (actually - reported).length();
        let expected = beta.length() * delay * 1.0e-6 / lc_world::flight::JULIAN_YEAR_S;
        assert!(
            (behind - expected).abs() < expected * 0.01,
            "reported {behind} light-years behind, expected {expected}",
        );
        assert!(behind > 0.0, "a moving contact reported at its present position");
    }

    /// Nothing at all is said about craft in another system.
    ///
    /// Silence rather than a filtered list: a client that could tell "nobody is here" from "the
    /// people here are out of range" would be reading something off the shape of what it was
    /// not told.
    #[tokio::test]
    async fn a_craft_in_another_system_is_not_a_contact() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let watcher = ClientId(2);
        server.admit(watcher, crate::world::still(ShipId(2), DVec3::ZERO), 0.0);
        let far = lc_world::system::LOCAL_SHELL_LY * 2.0 * lc_world::motion::LIGHT_US_PER_LY;
        server.admit(
            ClientId(1),
            crate::world::still(ShipId(1), DVec3::new(far, 0.0, 0.0)),
            0.0,
        );
        server.tick(&mut wire).await.unwrap();
        assert!(presences(&wire.take(watcher)).is_empty());
    }

    /// One light-second, in light-microseconds. Close enough that an approach completes in a
    /// handful of ticks.
    const ONE_LIGHT_SECOND: f64 = 1_000_000.0;

    /// Where a craft is at the server's present, light-years.
    fn at_now<J: Journal>(server: &Server<J>, ship: ShipId) -> DVec3 {
        server.ship(ship).unwrap().position_at(server.now_t() as f64)
            / lc_world::motion::LIGHT_US_PER_LY
    }

    fn gap<J: Journal>(server: &Server<J>) -> f64 {
        at_now(server, ShipId(1)).distance(at_now(server, ShipId(2)))
            * lc_world::system::M_PER_LY
    }

    /// The pursuer's standing plan, if it is flying one.
    fn plan<J: Journal>(server: &Server<J>, ship: ShipId) -> Option<lc_world::pursuit::Rendezvous> {
        match &server.ship(ship).unwrap().motion.motive {
            lc_world::motion::Motive::Rendezvous(plan) => Some(plan.clone()),
            _ => None,
        }
    }

    /// **An intercept, flown by the server.** It closes, it stops closing at the standoff, and
    /// it stays there — which is the whole of pursue, match, and hang about.
    #[tokio::test]
    async fn an_intercept_closes_to_a_standoff_and_stays() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let hunter = ClientId(1);
        server.admit(hunter, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(
            ClientId(2),
            crate::world::still(ShipId(2), DVec3::new(ONE_LIGHT_SECOND, 0.0, 0.0)),
            0.0,
        );
        let opening = gap(&server);

        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Intercept {
                ship_id: ShipId(2),
                closeness: lc_proto::Closeness::Company,
                approach: lc_proto::Approach::Direct,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert!(plan(&server, ShipId(1)).is_some(), "the order did not put it on an approach");

        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
        }

        let standoff = lc_world::pursuit::standoff_m(
            server.ship(ShipId(1)).unwrap().length_m,
            server.ship(ShipId(2)).unwrap().length_m,
        );
        let closed = gap(&server);
        assert!(closed < opening / 100.0, "it barely closed: {opening} to {closed}");
        assert!(
            closed < standoff * lc_world::pursuit::DRIFT_ALLOWANCE,
            "ended {closed} off, which is outside the deadband round {standoff}",
        );
        // And it is done flying rather than circling forever.
        assert!(plan(&server, ShipId(1)).is_none_or(|p| p.has_arrived(server.now_t() as f64 * 1e-6)));

        // Station-keeping: hundreds of ticks later it is still there, and it has not spent
        // them re-planning against itself.
        for _ in 0..300 {
            server.tick(&mut wire).await.unwrap();
        }
        let later = gap(&server);
        assert!(
            later < standoff * lc_world::pursuit::DRIFT_ALLOWANCE,
            "it wandered off station: {closed} to {later}",
        );
    }

    /// **A courteous intercept, flown by the server**: a main-drive leg that ends short, on the
    /// ingress sphere, then thrusters onto the station, and it stays. The legs are separate plans,
    /// so this is the chase noticing a leg that ended short and planning the next — and only then:
    /// a leg re-planned part-way starts from wherever the ship is moving, which courtesy does not
    /// cover.
    #[tokio::test]
    async fn a_courteous_intercept_goes_in_on_thrusters_and_stays() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let hunter = ClientId(1);
        server.admit(hunter, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(
            ClientId(2),
            crate::world::still(ShipId(2), DVec3::new(ONE_LIGHT_SECOND, 0.0, 0.0)),
            0.0,
        );
        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Intercept {
                ship_id: ShipId(2),
                closeness: lc_proto::Closeness::Company,
                approach: lc_proto::Approach::Courteous,
            },
            issued_at_client_t: 0,
        }));
        let standoff = lc_world::pursuit::standoff_m(
            server.ship(ShipId(1)).unwrap().length_m,
            server.ship(ShipId(2)).unwrap().length_m,
        );
        let rcs_g = server.balance().rcs_accel_g;
        let (mut ingress, mut thrusters) = (false, false);
        let mut legs: Vec<lc_world::pursuit::Rendezvous> = Vec::new();
        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
            if let Some(leg) = plan(&server, ShipId(1)) {
                if legs.last() != Some(&leg) {
                    legs.push(leg.clone());
                }
                let to_m = leg.cruise.to_ly.length() * lc_world::system::M_PER_LY;
                ingress |= leg.cruise.drive.accel_g > rcs_g && to_m > 2.0 * standoff;
                thrusters |= ingress && leg.cruise.drive.accel_g <= rcs_g;
            }
        }
        assert!(ingress, "never flew a main-drive leg to the ingress sphere");
        assert!(thrusters, "never went in on thrusters");
        assert_eq!(legs.len(), 2, "{:#?}", legs.iter().map(|l| l.cruise.to_ly).collect::<Vec<_>>());
        let closed = gap(&server);
        assert!((closed - standoff).abs() < standoff * 0.1, "ended {closed} off, wanted {standoff}");
        for _ in 0..300 {
            server.tick(&mut wire).await.unwrap();
        }
        let later = gap(&server);
        assert!((later - closed).abs() < standoff * 0.1, "it wandered off station: {closed} to {later}");
    }

    /// **Breaking off is no further corrections.** The ship is left on whatever its velocity
    /// makes of it, not flying out the rest of an approach and not holding a station on its own.
    #[tokio::test]
    async fn breaking_off_leaves_the_ship_ballistic() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let hunter = ClientId(1);
        server.admit(hunter, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(ClientId(2), crate::world::still(ShipId(2), DVec3::new(ONE_LIGHT_SECOND, 0.0, 0.0)), 0.0);
        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Intercept {
                ship_id: ShipId(2),
                closeness: lc_proto::Closeness::Company,
                approach: lc_proto::Approach::Direct,
            },
            issued_at_client_t: 0,
        }));
        for _ in 0..3 {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(server.ship(ShipId(1)).unwrap().motion.pursuing().is_some(), "premise: under way");

        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::BreakOff,
            issued_at_client_t: server.now_t(),
        }));
        server.tick(&mut wire).await.unwrap();
        let motion = &server.ship(ShipId(1)).unwrap().motion;
        assert!(
            matches!(motion.motive, lc_world::motion::Motive::Drifting { .. }),
            "still flying {:?}",
            motion.motive,
        );
        assert!(!server.pursuits.contains_key(&CraftId(1)));
        for _ in 0..20 {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(server.ship(ShipId(1)).unwrap().motion.pursuing().is_none(), "it was steered again");
    }

    /// **The rule the whole design turns on, applied to an autopilot.**
    ///
    /// A pursuer steers by the light that has reached it. When its quarry burns, the pursuer
    /// goes on flying the old solution — aimed at a quarry it still sees holding course — until
    /// the news arrives.
    ///
    /// Asserted as the arrival inequality rather than as "about two hours", because the
    /// pursuer is closing all the while and the light's true crossing is shorter than the one
    /// it was sent over.
    ///
    /// Verified to fail when it should: replacing the retarded solve with the quarry's present
    /// state makes it fail, which is what says it is testing the delay and not merely that
    /// something eventually happened. It also needed `Craft` to keep a history before it could
    /// pass at all — a motive evaluated before it was flown answers about a ship that did not
    /// exist yet, and `Drifting` extrapolating backwards made a burn rewrite its own past.
    /// **A new flight order is the end of the pursuit.** It used to leave the standing order in
    /// place, and the next tick steered the ship straight back to its quarry over what the player
    /// had just asked for. A transmission is not a flight order and changes nothing.
    #[tokio::test]
    async fn a_flight_order_cancels_a_standing_intercept() {
        let orders = [
            (Order::CutDrive, false),
            (Order::Burn { beta: [0.0, 1.0e-3, 0.0] }, false),
            (Order::Transmit { power_w: 1.0e6 }, true),
        ];
        for (order, keeps) in orders {
            let mut server = Server::new(Memory::default(), 0, 1);
            let mut wire = Loopback::new();
            let hunter = ClientId(1);
            server.admit(hunter, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
            server.admit(ClientId(2), crate::world::still(ShipId(2), DVec3::new(ONE_LIGHT_SECOND, 0.0, 0.0)), 0.0);
            wire.client_says(hunter, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::Intercept {
                    ship_id: ShipId(2),
                    closeness: lc_proto::Closeness::Company,
                    approach: lc_proto::Approach::Direct,
                },
                issued_at_client_t: 0,
            }));
            for _ in 0..3 {
                server.tick(&mut wire).await.unwrap();
            }
            assert!(server.ship(ShipId(1)).unwrap().motion.pursuing().is_some(), "premise: under way");

            wire.client_says(hunter, Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: order.clone(),
                issued_at_client_t: server.now_t(),
            }));
            for _ in 0..20 {
                server.tick(&mut wire).await.unwrap();
            }
            assert_eq!(server.pursuits.contains_key(&CraftId(1)), keeps, "{order:?}");
            // Only for the orders that cancel it: a kept intercept against a still quarry has
            // arrived by now and is drifting alongside, which is the policy working.
            if !keeps {
                assert!(
                    server.ship(ShipId(1)).unwrap().motion.pursuing().is_none(),
                    "{order:?} was steered back to the quarry: {:?}",
                    server.ship(ShipId(1)).unwrap().motion.motive,
                );
            }
        }
    }

    #[tokio::test]
    async fn a_pursuer_cannot_react_to_a_burn_before_its_light_arrives() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let hunter = ClientId(1);
        let prey = ClientId(2);
        server.admit(hunter, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(
            prey,
            crate::world::still(ShipId(2), DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)),
            0.0,
        );

        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Intercept {
                ship_id: ShipId(2),
                closeness: lc_proto::Closeness::Company,
                approach: lc_proto::Approach::Direct,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let first = plan(&server, ShipId(1)).expect("an approach");
        assert_eq!(first.frame_beta, DVec3::ZERO, "premise: it is chasing something at rest");

        // The quarry lights its drive and goes somewhere else.
        wire.client_says(prey, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: Order::Burn { beta: [0.0, 1.0e-3, 0.0] },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let burn_t = server.now_t();
        assert_ne!(
            server.ship(ShipId(2)).unwrap().motion.beta,
            DVec3::ZERO,
            "premise: the quarry actually burned",
        );

        // Where the news starts from. The light of the burn leaves this point, and the
        // pursuer may not act on it until it has traveled from here to wherever the pursuer
        // has got to — which is nearer than it was, because it has been closing all along.
        let from = at_now(&server, ShipId(2));

        let mut reacted = None;
        for _ in 0..600 {
            server.tick(&mut wire).await.unwrap();
            let Some(now) = plan(&server, ShipId(1)) else { continue };
            if now.frame_beta != DVec3::ZERO {
                reacted = Some((server.now_t(), at_now(&server, ShipId(1))));
                break;
            }
        }
        let (reacted_at, reacted_where) = reacted.expect("the news never arrived at all");

        // **The assertion.** The plan changed only once the light of the burn could have got
        // to where the pursuer was standing when it changed it.
        //
        // Stated as the inequality rather than as "about two hours", because the pursuer is
        // closing and the true crossing is shorter than the one it was sent over. One tick of
        // slack, because a re-solve happens on a tick boundary and not at the instant the
        // light lands.
        let traveled = reacted_where.distance(from) * lc_world::system::M_PER_LY;
        let earliest_us = traveled / lc_world::flight::C_M_S * 1.0e6;
        let waited_us = (reacted_at - burn_t) as f64;
        assert!(
            waited_us + TICK_US as f64 >= earliest_us,
            "reacted {waited_us} after the burn to light needing {earliest_us} to reach it",
        );
        assert!(waited_us > 0.0, "it reacted on the tick of the burn itself");
    }



    /// **The same rule, on the channel a player actually watches.**
    ///
    /// A contact's reported velocity may not change until the light of the burn that changed
    /// it has arrived. This is the one that had been wrong: a craft's worldline was its current
    /// motive evaluated at any time asked, so a burn rewrote its own past and every client in
    /// the system saw the maneuvere on the next tick, at any range.
    #[tokio::test]
    async fn a_contact_is_not_seen_to_maneuvere_before_its_light_arrives() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let watcher = ClientId(1);
        let mover = ClientId(2);
        server.admit(watcher, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(
            mover,
            crate::world::still(ShipId(2), DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)),
            0.0,
        );
        server.tick(&mut wire).await.unwrap();
        wire.take(watcher);

        wire.client_says(mover, Inbound::Act(Intent {
            ship_id: ShipId(2),
            order: Order::Burn { beta: [0.0, 1.0e-3, 0.0] },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let burn_t = server.now_t();
        let from = at_now(&server, ShipId(2));
        assert_ne!(
            server.ship(ShipId(2)).unwrap().motion.beta,
            DVec3::ZERO,
            "premise: the quarry actually burned",
        );

        let here = at_now(&server, ShipId(1));
        let mut seen_moving = None;
        for _ in 0..600 {
            server.tick(&mut wire).await.unwrap();
            let said = wire.take(watcher);
            let Some(beta) = presences(&said).last().map(|c| DVec3::from_array(c.beta)) else {
                continue;
            };
            if beta != DVec3::ZERO {
                seen_moving = Some(server.now_t());
                break;
            }
        }
        let seen_moving = seen_moving.expect("the news never arrived at all");

        // The watcher does not move, so the crossing is the one the burn was sent over.
        let traveled = here.distance(from) * lc_world::system::M_PER_LY;
        let earliest_us = traveled / lc_world::flight::C_M_S * 1.0e6;
        let waited_us = (seen_moving - burn_t) as f64;
        assert!(
            waited_us + TICK_US as f64 >= earliest_us,
            "saw the burn {waited_us} after it, to light needing {earliest_us} to arrive",
        );
    }


    /// **A chase at relativistic closing speed, flown by the server.**
    ///
    /// The quarry is running at four fifths of `c` and the pursuer starts at rest, so the
    /// relative velocity is most of `c` and nothing about this can be done by subtracting
    /// velocities and adding drifts. It closes anyway, and it ends *matched* — which is the
    /// part that says the plan was solved in a real boosted frame rather than an approximate
    /// one.
    #[tokio::test]
    async fn an_intercept_matches_a_quarry_running_at_four_fifths_of_c() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let hunter = ClientId(1);
        let mut chaser = crate::world::still(ShipId(1), DVec3::ZERO);
        // A torch, so the test runs in seconds. Shedding four fifths of `c` at five gravities
        // is three months of coordinate time and a hundred thousand ticks; the physics is the
        // same either way, and what is being checked is the match and not the schedule.
        chaser.motion.drive =
            lc_world::flight::Drive { accel_g: 1_000.0, ..Default::default() };
        server.admit(hunter, chaser, 0.0);
        let running = DVec3::new(0.0, 0.8, 0.0);
        server.admit(
            ClientId(2),
            crate::world::coasting(
                ShipId(2),
                DVec3::new(ONE_LIGHT_SECOND * 60.0, 0.0, 0.0),
                running,
                0,
            ),
            0.0,
        );

        wire.client_says(hunter, Inbound::Act(Intent {
            ship_id: ShipId(1),
            order: Order::Intercept {
                ship_id: ShipId(2),
                closeness: lc_proto::Closeness::Company,
                approach: lc_proto::Approach::Direct,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        assert!(plan(&server, ShipId(1)).is_some(), "the order did not put it on an approach");

        // Long enough to shed four fifths of `c`, fly back, and stop.
        for _ in 0..20_000 {
            server.tick(&mut wire).await.unwrap();
            if plan(&server, ShipId(1)).is_none() {
                break;
            }
        }

        let hunter_beta = server.ship(ShipId(1)).unwrap().motion.beta;
        assert!(
            (hunter_beta - running).length() < 0.01,
            "ended at {hunter_beta} rather than matched to {running}",
        );
        // And alongside, measured where a standoff means something: the frame they now share.
        let standoff = lc_world::pursuit::standoff_m(
            server.ship(ShipId(1)).unwrap().length_m,
            server.ship(ShipId(2)).unwrap().length_m,
        );
        let separation = (at_now(&server, ShipId(1)) - at_now(&server, ShipId(2)))
            * lc_world::flight::JULIAN_YEAR_S;
        let gap = lc_world::boost::separation_in_frame(separation, running)
            * lc_world::flight::C_M_S;
        assert!(
            gap < standoff * lc_world::pursuit::DRIFT_ALLOWANCE,
            "ended {gap} m off, outside the deadband round {standoff} m",
        );
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

    pub(crate) fn a_star() -> Option<lc_world::sky::CatalogStar> {
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
                max_beta: 0.999,
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
            order: Order::SetCourse { course: lc_proto::Course::LeaveSystem, accel_g: 5.0, max_beta: 0.999 },
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
                max_beta: 0.999,
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
                order: Order::SetCourse { course: lc_proto::Course::LeaveSystem, accel_g, max_beta: 0.999 },
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
            order: Order::SetCourse { course: asked.clone(), accel_g: 1000.0, max_beta: 0.999 },
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
        let Order::SetCourse { course, accel_g, max_beta: 0.999 } = order else { panic!("{order:?}") };
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
                max_beta: 0.999,
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
            order: Order::SetCourse { course: course.clone(), accel_g: 5.0, max_beta: 0.999 },
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
    use super::course_tests::{a_star, a_system, orbitable};
    use super::*;
    use crate::journal::Memory;
    use crate::world::World;
    use lc_world::motion::Motive;
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
        let [Outbound::Welcome { ship_id, name, client_id, .. }, Outbound::Fitted { .. }, Outbound::Observing { .. }, Outbound::Presets(_)] =
            said.as_slice()
        else {
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
        assert!(matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Welcome { .. }, Outbound::Fitted { .. }, Outbound::Observing { .. }, Outbound::Presets(_)]));

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
        assert!(matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Welcome { .. }, Outbound::Fitted { .. }, Outbound::Observing { .. }, Outbound::Presets(_)]));
    }

    /// A ticket minted for another shard is not a ticket here, however valid it is there.
    /// A player dropped at the origin sees nothing: it is empty interstellar space. The point
    /// of starting inside a system is that there is something there.
    #[tokio::test]
    async fn a_new_ship_starts_inside_a_system_when_the_world_has_one() {
        let star = crate::server::course_tests::a_star().expect("the sample star");
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.load_world(World::new(vec![star.clone()]));
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship_id = welcomed(&mut wire, ClientId(1));
        // A tick, because membership is resolved by where a craft is rather than by being told.
        server.tick(&mut wire).await.unwrap();

        let craft = server.ship(ship_id).expect("the ship");
        assert_eq!(
            craft.system.as_ref().map(|s| s.star),
            Some(star.id),
            "a new ship was left in interstellar space",
        );
        // And not inside the star it is orbiting.
        assert!(craft.motion.position_ly.distance(star.position_ly) > 0.0);
    }

    /// With no world there is nowhere better, and the origin is still the answer.
    #[tokio::test]
    async fn a_new_ship_starts_at_the_origin_when_there_is_no_world() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship_id = welcomed(&mut wire, ClientId(1));
        assert_eq!(server.ship(ship_id).expect("the ship").motion.position_ly, DVec3::ZERO);
    }

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

    /// Open house does not mean a valid ticket stops being honored. A real account still
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

    /// **What a reconnect is for.** A player who signs out of an orbit signs back into one.
    ///
    /// The welcome used to carry a position and nothing else, so this ship came back at rest at
    /// the point it had reached — adrift, at a station it was no longer holding. A day later
    /// the two were two and a half million kilometers apart, which is seven times the distance
    /// to the Moon.
    #[tokio::test]
    async fn signing_back_in_finds_the_ship_still_holding_its_orbit() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.load_world(World::new(vec![star.clone()]));
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        // Put it where the system is, since a new craft starts a few au out along one axis.
        server.fleet_mut().get_mut(CraftId(ship.0)).unwrap().motion.position_ly = star.position_ly;
        server.tick(&mut wire).await.unwrap();

        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body,
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 5.0,
                max_beta: 0.999,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let Motive::Crossing(cruise) = &server.ship(ship).unwrap().motion.motive else {
            panic!("the course did not become a crossing")
        };
        let ticks = ((cruise.duration_s() * 1.0e6 / TICK_US as f64).ceil() as usize + 2).min(20_000);
        for _ in 0..ticks {
            server.tick(&mut wire).await.unwrap();
        }
        let Motive::Holding(station) = server.ship(ship).unwrap().motion.motive.clone() else {
            panic!("it never arrived")
        };

        // The socket drops, and the same account comes back on a new one.
        server.disconnected(ClientId(1));
        wire.take(ClientId(1));
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        let welcome = wire
            .take(ClientId(2))
            .into_iter()
            .find_map(|m| match m {
                Outbound::Welcome { ship, .. } => Some(ship),
                _ => None,
            })
            .expect("a welcome");

        let restored = lc_world::resume::Snapshot::from(&welcome)
            .restore(Some(&system), server.now_t() as f64 / 1.0e6);
        assert_eq!(
            restored.motive,
            Motive::Holding(station),
            "the welcome put the ship back doing something else",
        );
    }

    /// **A trip finishes while nobody is watching.** The fleet is advanced by the tick, not by
    /// a connection, so a course set before signing out is a course flown while signed out —
    /// and signing back in finds the ship already on station.
    ///
    /// The whole ship-side of this is that `Server::disconnected` drops the *connection* and
    /// leaves the craft. What makes it visible is the welcome carrying the motive.
    #[tokio::test]
    async fn a_course_set_before_signing_out_is_flown_while_signed_out() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.load_world(World::new(vec![star.clone()]));
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        server.fleet_mut().get_mut(CraftId(ship.0)).unwrap().motion.position_ly = star.position_ly;
        server.tick(&mut wire).await.unwrap();

        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body,
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 5.0,
                max_beta: 0.999,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let Motive::Crossing(cruise) = &server.ship(ship).unwrap().motion.motive else {
            panic!("the course did not become a crossing")
        };
        let ticks = ((cruise.duration_s() * 1.0e6 / TICK_US as f64).ceil() as usize + 2).min(20_000);

        // Sign out **under way**, a long way from arriving.
        for _ in 0..(ticks / 8) {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            matches!(server.ship(ship).unwrap().motion.motive, Motive::Crossing(_)),
            "the premise is a ship still in transit when the socket drops",
        );
        server.disconnected(ClientId(1));
        wire.take(ClientId(1));

        // Nobody is connected at all, and the world goes on.
        for _ in 0..ticks {
            server.tick(&mut wire).await.unwrap();
        }

        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        let welcome = wire
            .take(ClientId(2))
            .into_iter()
            .find_map(|m| match m {
                Outbound::Welcome { ship, .. } => Some(ship),
                _ => None,
            })
            .expect("a welcome");
        let restored = lc_world::resume::Snapshot::from(&welcome)
            .restore(Some(&system), server.now_t() as f64 / 1.0e6);
        assert!(
            matches!(restored.motive, lc_world::motion::Motive::Holding(_)),
            "signing back in found the ship {:?}, not on station",
            restored.motive,
        );
    }

    /// A crossing **between stars** arrives at a standoff and stops there, with no station.
    ///
    /// Not an oversight: `Change::Cross` is given no waypoint, because "go to that star" names
    /// a system and not a place inside one. Arriving puts the ship in the new system at rest,
    /// and choosing an orbit there is a second order. Pinned because the opposite is the
    /// natural expectation.
    #[tokio::test]
    async fn crossing_to_a_star_arrives_in_its_system_at_rest_and_not_in_an_orbit() {
        let Some(here) = a_star() else { return };
        let mut there = here.clone();
        there.id = lc_world::sky::StarId::synthesize("test", 7);
        // Further apart than `LOCAL_SHELL_LY`, or the two shells overlap and being "in" one of
        // them is whichever the lookup reaches first rather than a fact about where the ship is.
        there.position_ly = here.position_ly + DVec3::new(2.0, 0.0, 0.0);

        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.load_world(World::new(vec![here.clone(), there.clone()]));
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        server.fleet_mut().get_mut(CraftId(ship.0)).unwrap().motion.position_ly = here.position_ly;
        server.tick(&mut wire).await.unwrap();

        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::Cross { star: there.id.get(), accel_g: 5.0, max_beta: 0.999 },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let Motive::Crossing(cruise) = &server.ship(ship).unwrap().motion.motive else {
            panic!("the crossing did not begin")
        };
        let ticks = (cruise.duration_s() * 1.0e6 / TICK_US as f64).ceil() as usize + 4;
        assert!(ticks < 400_000, "{ticks} ticks is too long to run in a test");

        server.disconnected(ClientId(1));
        for _ in 0..ticks {
            server.tick(&mut wire).await.unwrap();
        }

        let arrived = server.ship(ship).expect("the ship");
        assert!(
            matches!(arrived.motion.motive, Motive::Drifting { .. }),
            "it arrived {:?}",
            arrived.motion.motive,
        );
        assert_eq!(arrived.motion.beta, DVec3::ZERO, "arriving is stopping");
        assert_eq!(
            arrived.system.as_ref().map(|s| s.star),
            Some(there.id),
            "it should at least be in the system it crossed to",
        );
    }

    /// **A ship outlives the process flying it.** Set a course, checkpoint, throw the whole
    /// server away, build a new one from the checkpoint, and the crossing goes on from where it
    /// was — and finishes.
    ///
    /// The clock is the part that is easy to get wrong. Every motive is stamped in absolute
    /// coordinate time, so a shard that came back at zero would read a crossing that began at
    /// t = 876 as one that has not begun, and fly it again.
    #[tokio::test]
    async fn a_ship_survives_the_process_and_its_crossing_goes_on() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.load_world(World::new(vec![star.clone()]));
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        server.fleet_mut().get_mut(CraftId(ship.0)).unwrap().motion.position_ly = star.position_ly;
        server.tick(&mut wire).await.unwrap();

        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::SetCourse {
                course: lc_proto::Course::Orbit {
                    body,
                    altitude_radii: 2.0,
                    plane: lc_proto::Plane::Equatorial,
                },
                accel_g: 5.0,
                max_beta: 0.999,
            },
            issued_at_client_t: 0,
        }));
        server.tick(&mut wire).await.unwrap();
        let Motive::Crossing(cruise) = &server.ship(ship).unwrap().motion.motive else {
            panic!("the course did not become a crossing")
        };
        let ticks = ((cruise.duration_s() * 1.0e6 / TICK_US as f64).ceil() as usize + 2).min(20_000);
        for _ in 0..(ticks / 8) {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            matches!(server.ship(ship).unwrap().motion.motive, Motive::Crossing(_)),
            "the premise is a ship in transit when the process ends",
        );

        // The process ends here. Nothing of it survives but the checkpoint.
        let checkpoint = server.checkpoint();
        let was = server.ship(ship).unwrap().motion.clone();
        drop(server);
        drop(wire);

        let mut server = trusting(&broker);
        server.load_world(World::new(vec![star]));
        assert!(server.adopt(checkpoint).is_empty(), "a row would not read");
        let now = server.ship(ship).expect("the ship came back").motion.clone();
        assert_eq!(now.motive, was.motive, "it came back on a different flight");
        assert_eq!(now.position_ly, was.position_ly, "it came back somewhere else");
        assert_eq!(now.clock_s, was.clock_s, "the crew aged across a restart");

        let mut wire = Loopback::new();
        for _ in 0..ticks {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            matches!(server.ship(ship).unwrap().motion.motive, Motive::Holding(_)),
            "the crossing did not finish after the restart: {:?}",
            server.ship(ship).unwrap().motion.motive,
        );

        // And the account still finds it, rather than being handed a second ship.
        says(&mut server, &mut wire, ClientId(9), broker.mint("acct-1", SHARD, 60, "j9")).await;
        let came_back = wire
            .take(ClientId(9))
            .into_iter()
            .find_map(|m| match m {
                Outbound::Welcome { ship_id, .. } => Some(ship_id),
                _ => None,
            })
            .expect("a welcome");
        assert_eq!(came_back, ship, "the account got a new ship");
    }

    /// A row that will not read refuses its account rather than minting a second ship for it.
    ///
    /// Two rows on one account is a unique-constraint failure on every checkpoint from then on:
    /// the shard stops saving, for everyone, and says nothing.
    #[tokio::test]
    async fn an_account_whose_saved_ship_will_not_read_is_refused() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();

        let bad = lc_store::ships::Ship {
            ship_id: 3,
            account: Some("acct-1".into()),
            saved_t: 0,
            state: b"not postcard, and too short for this shape".to_vec(),
            format: crate::persist::SAVE_FORMAT,
        };
        let refused = server.adopt(crate::persist::Checkpoint {
            now_t: 5_000,
            next_ship: 4,
            ships: vec![bad],
        });
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].account.as_deref(), Some("acct-1"));

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        assert!(
            matches!(wire.take(ClientId(1)).as_slice(), [Outbound::Unauthenticated]),
            "it was given a ship anyway",
        );
        // A different account is unaffected: one bad row is one player's problem.
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-2", SHARD, 60, "j2")).await;
        welcomed(&mut wire, ClientId(2));
    }

    /// The clock comes back with the world. Without it every saved motive, stamped absolutely,
    /// reads as one that has not happened yet.
    #[tokio::test]
    async fn adopting_a_checkpoint_restores_the_worlds_clock() {
        let mut server = Server::new(Memory::default(), 0, 1);
        assert_eq!(server.now_t(), 0, "premise");
        server.adopt(crate::persist::Checkpoint {
            now_t: 8_766_000_000,
            next_ship: 17,
            ships: Vec::new(),
        });
        assert_eq!(server.now_t(), 8_766_000_000);

        // And the counter, or the next sign-in reissues an identifier that is already taken.
        let broker = Broker::new([1u8; 32]);
        let mut trusted = Trusted::new(SHARD);
        assert_eq!(trusted.learn(&broker.jwks()), 1);
        server.trust(trusted);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        assert_eq!(welcomed(&mut wire, ClientId(1)), ShipId(17));
    }

    /// **The universe exists outside the player.** The same order, at the same coordinate, ends
    /// in the same state whether or not anyone stayed to watch it.
    ///
    /// Not a property of anything written for it — the fold is the fold, and a connection is a
    /// socket rather than a thing in space. It is asserted because it is the kind of invariant
    /// that gets broken by an optimization: gate any of the tick on who is connected and this is
    /// the test that says what that cost.
    #[tokio::test]
    async fn watching_a_flight_does_not_change_it() {
        let Some(star) = a_star() else { return };
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };

        let course = |ship| {
            Inbound::Act(Intent {
                ship_id: ship,
                order: Order::SetCourse {
                    course: lc_proto::Course::Orbit {
                        body: body.clone(),
                        altitude_radii: 2.0,
                        plane: lc_proto::Plane::Equatorial,
                    },
                    accel_g: 5.0,
                    max_beta: 0.999,
                },
                issued_at_client_t: 0,
            })
        };

        // `watched` keeps its connection for the whole flight; `alone` drops it the moment the
        // order is in. Everything else about the two runs is identical.
        let mut ends = Vec::new();
        for watched in [true, false] {
            let broker = Broker::new([1u8; 32]);
            let mut server = trusting(&broker);
            server.load_world(World::new(vec![star.clone()]));
            let mut wire = Loopback::new();

            says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
            let ship = welcomed(&mut wire, ClientId(1));
            server.fleet_mut().get_mut(CraftId(ship.0)).unwrap().motion.position_ly =
                star.position_ly;
            server.tick(&mut wire).await.unwrap();

            wire.client_says(ClientId(1), course(ship));
            server.tick(&mut wire).await.unwrap();
            let Motive::Crossing(cruise) = &server.ship(ship).unwrap().motion.motive else {
                panic!("the course did not become a crossing")
            };
            let ticks =
                ((cruise.duration_s() * 1.0e6 / TICK_US as f64).ceil() as usize + 2).min(20_000);

            if !watched {
                server.disconnected(ClientId(1));
            }
            for _ in 0..ticks {
                server.tick(&mut wire).await.unwrap();
                // A watched run drains its socket, which an unwatched one has nothing to drain.
                // Dropping the messages rather than keeping them is the point: what a client is
                // told changes nothing about what is true.
                wire.take(ClientId(1));
            }
            ends.push(server.ship(ship).unwrap().motion.clone());
        }

        let (watched, alone) = (&ends[0], &ends[1]);
        assert_eq!(watched.motive, alone.motive, "the flights ended differently");
        assert_eq!(watched.position_ly, alone.position_ly, "they ended in different places");
        assert_eq!(watched.beta, alone.beta);
        assert_eq!(watched.clock_s, alone.clock_s, "the crews aged differently");
    }

    /// The ship a welcome named, out of whatever else the tick also sent.
    ///
    /// Searched rather than matched as the whole queue: a tick that welcomes a client may also
    /// state the traffic around it, and `Welcome` being first is the invariant that matters
    /// rather than it being alone.
    /// **A ship hanging about with another goes on doing so while its pilot is away**, and
    /// across the shard being restarted, and the pilot is told so on coming back.
    #[tokio::test]
    async fn a_pursuit_outlives_signing_out_and_a_restart() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let ship = welcomed(&mut wire, ClientId(1));
        let quarry = ShipId(500);
        // A light-second off, in the light-microseconds `still` places in.
        let at = server.ship(ship).unwrap().position_at(server.now_t() as f64);
        server.admit(ClientId(9), crate::world::still(quarry, at + DVec3::X * 1.0e6), 0.0);
        let at_now = |server: &Server<Memory>, id: ShipId| {
            server.ship(id).unwrap().position_at(server.now_t() as f64)
                / lc_world::motion::LIGHT_US_PER_LY
        };
        server.tick(&mut wire).await.unwrap();

        wire.client_says(ClientId(1), Inbound::Act(Intent {
            ship_id: ship,
            order: Order::Intercept {
                ship_id: quarry,
                closeness: lc_proto::Closeness::Intimate,
                approach: lc_proto::Approach::Direct,
            },
            issued_at_client_t: server.now_t(),
        }));
        server.tick(&mut wire).await.unwrap();
        server.disconnected(ClientId(1));
        wire.take(ClientId(1));
        // And the quarry leaves, so staying with it takes steering rather than the plan already
        // flying when the pilot went.
        let now_s = server.now_t() as f64 * 1.0e-6;
        let leaving = server.fleet_mut().get_mut(CraftId(quarry.0)).unwrap();
        let from = leaving.motion.position_ly;
        leaving.drift_from(from, DVec3::Y * 1.0e-5, now_s);

        // Nobody at the controls, and it still closes and keeps station.
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
        }
        let (mine, theirs) = (server.ship(ship).unwrap().length_m, server.ship(quarry).unwrap().length_m);
        let standoff = lc_world::pursuit::Closeness::Intimate.standoff_m(mine, theirs);
        let apart = at_now(&server, ship).distance(at_now(&server, quarry)) * lc_world::system::M_PER_LY;
        assert!(
            (apart - standoff).abs() < lc_world::pursuit::INTIMATE_SLACK_M,
            "{apart} m off a {standoff} m standoff with nobody signed in",
        );

        // Restarted from a checkpoint, the policy comes back with the craft.
        let mut restarted = trusting(&broker);
        assert!(restarted.adopt(server.checkpoint()).is_empty());
        let kept = restarted.pursuits.get(&CraftId(ship.0)).expect("the pursuit was not saved");
        assert_eq!((kept.quarry, kept.closeness), (quarry, lc_world::pursuit::Closeness::Intimate));

        // And signing back in says so, straight after the welcome.
        says(&mut restarted, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        let said = wire.take(ClientId(2));
        assert!(matches!(said.first(), Some(Outbound::Welcome { .. })), "{said:?}");
        assert_eq!(
            said.get(1),
            Some(&Outbound::Pursuing {
                ship_id: ship,
                pursuit: lc_proto::Pursuit {
                    quarry,
                    closeness: lc_proto::Closeness::Intimate,
                    approach: lc_proto::Approach::Direct,
                },
            }),
        );
    }

    const SHELF: &str = r#"
        [[book]]
        id = "the-gilded-age"
        title = "The Gilded Age"
        file = "gilded.epub"
    "#;

    fn mark(book: &str, at: u32) -> lc_proto::Bookmark {
        lc_proto::Bookmark {
            book: book.to_owned(),
            spine: 1,
            char_offset: at,
            location: at / 1024,
            locations: 400,
        }
    }

    #[tokio::test]
    async fn signing_in_hands_over_the_shelf_and_this_account_s_place_on_it() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.library = crate::library::Library::from_toml("https://cdn/library/", SHELF).unwrap();
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let said = wire.take(ClientId(1));
        let library = said.iter().find_map(|m| match m {
            Outbound::Library { base, books } => Some((base.clone(), books.len())),
            _ => None,
        });
        assert_eq!(library, Some(("https://cdn/library/".to_owned(), 1)), "{said:?}");
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Reading(marks) if marks.is_empty())),
            "a new account has read nothing, and is told so rather than left guessing: {said:?}"
        );

        wire.client_says(ClientId(1), Inbound::SetReading(mark("the-gilded-age", 4_096)));
        server.tick(&mut wire).await.unwrap();
        wire.take(ClientId(1));

        // The same account, on another socket, opens to the same sentence.
        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        let said = wire.take(ClientId(2));
        let marks = said.iter().find_map(|m| match m {
            Outbound::Reading(marks) => Some(marks.clone()),
            _ => None,
        });
        assert_eq!(marks.as_deref(), Some([mark("the-gilded-age", 4_096)].as_slice()), "{said:?}");
    }

    #[tokio::test]
    async fn a_shard_with_no_shelf_says_nothing_about_one() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let said = wire.take(ClientId(1));
        assert!(!said.iter().any(|m| matches!(m, Outbound::Library { .. })), "{said:?}");
    }

    #[tokio::test]
    async fn one_account_cannot_read_another_s_place() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        server.library = crate::library::Library::from_toml("https://cdn/library/", SHELF).unwrap();
        let mut wire = Loopback::new();

        says(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        wire.take(ClientId(1));
        wire.client_says(ClientId(1), Inbound::SetReading(mark("the-gilded-age", 9_000)));
        server.tick(&mut wire).await.unwrap();

        says(&mut server, &mut wire, ClientId(2), broker.mint("acct-2", SHARD, 60, "j2")).await;
        let said = wire.take(ClientId(2));
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Reading(marks) if marks.is_empty())),
            "the second account was handed the first one's bookmark: {said:?}"
        );
    }

    fn welcomed(wire: &mut Loopback, client: ClientId) -> ShipId {
        let said = wire.take(client);
        match said.first() {
            Some(Outbound::Welcome { ship_id, .. }) => *ship_id,
            _ => panic!("{said:?}"),
        }
    }
}
