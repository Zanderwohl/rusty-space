//! The connection, as the rest of the app sees it.
//!
//! One resource holding a [`Link`] and what the server has said about who we are. Nothing here
//! knows what a socket is — that is [`crate::link`] — and nothing above here knows there is a
//! thread.
//!
//! What the server does **not** send is ship positions. Both ends run the same `lc-world`
//! physics against the same clock, so the client computes where everything is and the server is
//! authoritative only where they disagree. What arrives instead is [`Sighting`]s: events, at
//! light delay. See `lightcone/docs/08-networking.md`.

use std::sync::Mutex;

use bevy::prelude::*;
use glam::DVec3;
use lc_proto::{
    ClientId, Inbound, Order, Outbound, PROTOCOL_VERSION, Presence, Refusal, ShipId, Sighting,
};

use lc_world::sighted::Reckoning;
use lc_world::system::LocalSystem;

use crate::link::{Link, Status};

/// What a ship with no account behind it is called.
///
/// Offline there is no broker to have said a name, and every craft on the map is named
/// including this one. A name rather than a word for the reader, so that a real name can be
/// told from the absence of one at a glance.
pub const ANONYMOUS: &str = "Anonymous Ship";

/// Where this client is with respect to a server.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    /// No server was named. The single-process game, which is every build before this one.
    #[default]
    Offline,
    Connecting,
    Joined(Joined),
    /// The server would not have us, in words for a person. Terminal: the reasons a server
    /// refuses — a ticket it will not take, a protocol it does not speak — are not things
    /// retrying fixes.
    Refused(String),
    /// It was open and is not. **Not** terminal in principle, though nothing reconnects yet.
    Lost(String),
}

/// Where and when the server put us.
///
/// Kept so it can be applied **again**. The client rebuilds its `Session` wholesale when the
/// sky finishes loading, and the connection is opened before that — deliberately, because a
/// ticket is worth sixty seconds and a sky is worth megabytes. So the rebuild lands after the
/// welcome and would otherwise drop all of it: the ship back at the origin, the clock back at
/// this process's own epoch, and `remote` back to false. The client then flies locally while
/// the interface still says LINKED, which is the worst of both — it looks connected and
/// nothing it does reaches the server.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub now_t: i64,
    /// The whole ship, including what it is doing. See [`lc_world::resume`].
    pub ship: lc_proto::Motion,
}

/// What a `Welcome` said.
#[derive(Clone, Debug, PartialEq)]
pub struct Joined {
    pub client_id: ClientId,
    pub ship_id: ShipId,
    /// The account's display name, as the broker knows it. A cache and not a fact: the broker
    /// owns it and a rename appears next session.
    pub name: String,
}

/// Another craft, as this ship currently sees it.
///
/// Everything here is **retarded**. The position is where the light arriving now left from, so
/// a contact under way is drawn behind where it actually is, and the faster it is going the
/// further behind. That is the game rather than a lag.
///
/// Reckoned forward between statements rather than held still — see [`lc_world::sighted`] for
/// why, and for what that gets wrong. [`Contact::reckon`] runs once a frame, after the clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub ship_id: ShipId,
    pub name: String,
    pub length_m: f64,
    /// Light-years from the world origin, where the light left.
    pub position_ly: DVec3,
    pub beta: DVec3,
    /// Unit vector the nose pointed along, as last stated.
    pub facing: DVec3,
    /// What its drive was putting into its exhaust, watts, as last stated. Zero when coasting.
    pub jet_power_w: f64,
    /// Coordinate seconds the light left.
    pub emitted_s: f64,
    reckoning: Reckoning,
    /// What the statement said the drive was doing, at the statement's own instant.
    stated_power_w: f64,
}

/// A drive event, as a contact's plume reads it: coordinate seconds, and the power from then.
pub type DriveAt = (f64, f64);

/// Drive events remembered per craft. Only the latest before the instant drawn matters, and
/// the instant drawn is never more than a tick or two behind the newest.
const REMEMBERED_DRIVES: usize = 16;

impl Contact {
    /// A contact from a statement. `system` is the one this ship is in, which a contact inside
    /// it is reckoned along a conic about.
    pub fn seen(presence: Presence, system: Option<&LocalSystem>) -> Self {
        let position_ly = DVec3::from_array(presence.at_ly);
        let beta = DVec3::from_array(presence.beta);
        let emitted_s = presence.emitted_t as f64 * 1.0e-6;
        let sighting = lc_world::pursuit::Sighting {
            target: lc_world::motion::ShipId(presence.ship_id.0),
            position_ly,
            beta,
            length_m: presence.length_m,
            emitted_s,
        };
        Self {
            ship_id: presence.ship_id,
            name: presence.name,
            length_m: presence.length_m,
            position_ly,
            beta,
            facing: DVec3::from_array(presence.facing).normalize_or_zero(),
            jet_power_w: presence.jet_power_w,
            emitted_s,
            reckoning: Reckoning::new(system, sighting),
            stated_power_w: presence.jet_power_w,
        }
    }

    /// Bring the contact up to what `observer_ly` sees at `now_s`. `drives` is this craft's
    /// drive events, oldest first.
    pub fn reckon(
        &mut self,
        system: Option<&LocalSystem>,
        observer_ly: DVec3,
        now_s: f64,
        drives: &[DriveAt],
    ) {
        let seen = self.reckoning.appearance_at(system, observer_ly, now_s);
        self.position_ly = seen.position_ly;
        self.beta = seen.beta;
        self.emitted_s = seen.emitted_s;
        // The latest word on the drive at the instant drawn, statement or event. The statement
        // stands when nothing has been said since, including when this clock is behind it.
        let stated_s = self.reckoning.seen.emitted_s;
        self.jet_power_w = drives
            .iter()
            .rev()
            .find(|(at_s, _)| *at_s <= seen.emitted_s)
            .filter(|(at_s, _)| *at_s > stated_s || seen.emitted_s < stated_s)
            .map_or(self.stated_power_w, |(_, power_w)| *power_w);
    }
}

/// Reckon every contact against this frame's clock and ship.
pub fn reckon_contacts(game: Res<crate::app::Game>, mut uplink: ResMut<Uplink>) {
    uplink.reckon(game.0.system.as_deref(), game.0.ship.motion.position_ly, game.0.coordinate_time_s());
}

/// Where the server is, if there is one.
#[derive(Resource, Default)]
pub struct ServerAddress(pub Option<String>);

/// Whether to run a shard in this process rather than connect to one. See [`crate::local`].
#[derive(Resource, Default)]
pub struct LocalShard(pub bool);

/// A scene for the in-process shard to stage, for `--demo`. Development only: a deployment's
/// traffic is other players.
#[derive(Resource, Default)]
pub struct Demo(pub Option<String>);

/// Start the in-process shard, and point [`ServerAddress`] at it.
///
/// On entering the world rather than at boot, because the shard has to be given the stars the
/// client actually ended up with: both ends put craft into systems by position against the same
/// shell radius, and two different skies would disagree about which system a ship is in.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_local(
    local: Res<LocalShard>,
    demo: Res<Demo>,
    game: Res<crate::app::Game>,
    mut address: ResMut<ServerAddress>,
) {
    if !local.0 || address.0.is_some() {
        return;
    }
    match crate::local::start(game.0.stars.clone(), demo.0.clone()) {
        Ok(at) => {
            info!("a shard is running in this process at {at}");
            address.0 = Some(at);
        }
        // Not fatal. Without an address the client stays the single-process game, which is
        // what it was a moment ago and is still playable.
        Err(why) => error!("could not start a local shard: {why}"),
    }
}

#[derive(Resource, Default)]
pub struct Uplink {
    /// `Mutex` because the native link holds an `mpsc::Receiver`, which is `Send` and not
    /// `Sync`, and a Bevy resource must be both.
    link: Mutex<Option<Box<dyn Link + Send>>>,
    pub state: State,
    /// Whether `Hello` has gone out on the current link. One per link, and a link is replaced
    /// rather than reconnected, so this never needs clearing on its own.
    greeted: bool,
    /// What has been seen, newest last. Kept so there is something to show while folding them
    /// into the world is still ahead.
    pub seen: Vec<Sighting>,
    /// Everybody else in sight, as of the last statement. Replaced wholesale rather than
    /// merged: the list is what the server can see of this ship's surroundings, and a contact
    /// missing from it is a contact that is no longer there.
    pub contacts: Vec<Contact>,
    /// Who this ship has a standing order to close on.
    ///
    /// The interface's copy of a policy the server owns, kept so a button can read as pressed
    /// the moment the order is accepted. Not authoritative: what the ship is actually *doing*
    /// is its motive, and the server drops the pursuit without saying so when the quarry goes
    /// out of sight — which is why this is cleared by a refusal and by losing the contact.
    pub chasing: Option<lc_proto::Pursuit>,
    /// The shelf, as the shard last stated it: its base and its books. Taken once.
    pub shelf: Option<(String, Vec<lc_proto::Book>)>,
    /// This account's places, most recently read first. Taken once.
    pub bookmarks: Option<Vec<lc_proto::Bookmark>>,
    /// What the server last said about an order, for the interface to show once and drop. The
    /// client cannot write its own here: an order's outcome is the server's to state.
    pub applied: Option<String>,
    /// The last placement the server gave, for [`Uplink::place`] to re-apply.
    placement: Option<Placement>,
    /// When the last order went out, in seconds since the app started.
    ///
    /// The client does not predict, so the delay between asking and seeing is the round trip
    /// and nothing else. That makes it the one number worth showing: without it, "this feels
    /// slow" and "this is slow" are the same report.
    asked_at: Option<f64>,
    /// How long the last order took to come back.
    pub round_trip_s: Option<f64>,
    /// Drive events per craft, oldest first. Kept across statements, which replace contacts.
    drives: std::collections::HashMap<ShipId, Vec<DriveAt>>,
    /// The last account the server stated, re-applied with the placement for the same reason.
    pub fitting: Option<lc_proto::Fitting>,
    /// Every conversation this ship is in. See [`crate::chat`].
    pub chat: crate::chat::Chat,
    /// Automatic acknowledgements this ship owes, drained by [`pump`] into orders.
    ///
    /// Collected rather than sent where they are decided, because folding the wire is not
    /// allowed to reach the wire: an answer is an order like any other and goes out through
    /// the one dispatcher every order does.
    owed: Vec<(ShipId, lc_proto::Aim)>,
}

/// The rate a shard runs at, and what a server that says nothing is taken to mean.
///
/// One, because `session::TIME_RATE` — the coordinate seconds a multiplier of 1 buys per real
/// second — is the same 8766 the server advances by every tick. The client's own default is
/// sixty times that, which is a development convenience whose own constant says "the server
/// owns the rate in a real session and this multiplier does not exist there".
///
/// It no longer exists there because of this constant, which would only ever have been a guess
/// that happened to be right. The rate arrives in the welcome and is restated with the clock;
/// this is the value a deployment sends, not the value a client assumes.
pub const SERVER_RATE: f64 = 1.0;

/// How far the clock may be out before it is pulled back, in coordinate microseconds.
///
/// One coordinate hour, which is about four tenths of a real second at the design rate. It has
/// to be comfortably more than a statement's own age — a tick plus the network, so a thousand
/// coordinate seconds or so — or every statement would drag the clock backwards by however long
/// it spent in flight. Below this the difference is smaller than the frames either side draws;
/// above it, positions disagree.
pub const CLOCK_SLACK_US: i64 = 3_600 * 1_000_000;

/// The same, against a world running at `rate` times the design rate.
///
/// A fixed hour stops being comfortably more than a statement's age as soon as the world runs
/// faster than one: at sixty, a single server tick is seven coordinate hours, so *every*
/// statement is further out than the slack and the client snaps back seven hours twenty times
/// a second, for ever, without the clock ever having drifted. Three ticks is the same margin
/// the fixed hour was, expressed in the thing it was always really measuring.
pub fn clock_slack_us(rate: f64) -> i64 {
    let tick = lc_server_tick_us(rate);
    CLOCK_SLACK_US.max(tick.saturating_mul(3))
}

/// Coordinate microseconds a server tick covers at `rate`. Mirrors `lc_server::server::TICK_US`,
/// which the browser build cannot see.
fn lc_server_tick_us(rate: f64) -> i64 {
    const DESIGN_TICK_US: f64 = 50.0 * 8766.0 * 1_000.0;
    (DESIGN_TICK_US * rate.max(0.0)) as i64
}

/// How many sightings are remembered. A bound rather than a policy: the fold that replaces
/// this will not keep a list at all.
const REMEMBERED: usize = 256;

impl Uplink {
    /// Take a link and start greeting over it. Replaces whatever was there.
    pub fn open(&mut self, link: Box<dyn Link + Send>) {
        *self.link.get_mut().unwrap() = Some(link);
        self.greeted = false;
        self.seen.clear();
        self.state = State::Connecting;
    }

    /// Put a session where the server has it, if the server has said.
    ///
    /// Idempotent, and called both when the welcome arrives and again whenever the session is
    /// rebuilt. Does nothing offline, which is the single-process game.
    pub fn place(&self, session: &mut crate::session::Session) {
        let Some(placement) = self.placement.as_ref() else { return };
        session.set_coordinate_time_us(placement.now_t);
        session.restore(&(&placement.ship).into());
        session.remote = true;
        if let Some(fitting) = &self.fitting {
            session.ship.fit(Some(fitting.into()));
        }
    }

    pub fn joined(&self) -> Option<&Joined> {
        match &self.state {
            State::Joined(joined) => Some(joined),
            _ => None,
        }
    }

    /// What this ship is called: the display name the account carries, which is the name every
    /// other client has for it. [`ANONYMOUS`] when nobody has said one.
    ///
    /// One answer for the two places that show it, the map's mark and the radio window's own
    /// lines.
    pub fn own_name(&self) -> String {
        self.joined()
            .map(|joined| joined.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| ANONYMOUS.to_string())
    }

    /// What a craft is called, from whatever this client knows of it.
    ///
    /// A contact's name first, because that is what the server last said; then the name a
    /// conversation already holds, which outlives the contact going out of sight; then its
    /// number, which is always true and never useful.
    pub fn name_of(&self, who: ShipId) -> String {
        self.contacts
            .iter()
            .find(|c| c.ship_id == who)
            .map(|c| c.name.clone())
            .or_else(|| self.chat.get(who).map(|c| c.name.clone()).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| format!("ship {}", who.0))
    }

    /// Note that an order has just gone out, so its answer can be timed.
    pub fn asked(&mut self, at_s: f64) {
        self.asked_at = Some(at_s);
    }

    /// Say something to the server. Silently does nothing with no link, which is the offline
    /// build and is not an error there.
    pub fn say(&mut self, message: Inbound) {
        if let Some(link) = self.link.get_mut().unwrap() {
            link.send(message);
        }
    }

    /// Read everything waiting, and fold it. Returns what arrived, for a caller that wants it.
    /// Bring every contact up to what `observer_ly` sees at `now_s`.
    pub fn reckon(&mut self, system: Option<&LocalSystem>, observer_ly: DVec3, now_s: f64) {
        for contact in &mut self.contacts {
            let drives = self.drives.get(&contact.ship_id).map_or(&[][..], Vec::as_slice);
            contact.reckon(system, observer_ly, now_s, drives);
        }
    }

    fn take(&mut self) -> Vec<Outbound> {
        let Some(link) = self.link.get_mut().unwrap() else {
            return Vec::new();
        };
        let status = link.status();
        let messages = link.poll();

        // Greet as soon as the socket is open, and exactly once. Before this the server has
        // nothing to call us and will answer nothing.
        if status.is_open() && !self.greeted {
            self.greeted = true;
            return messages;
        }
        if let Status::Closed(why) = status {
            // A refusal already explains itself; the socket closing afterwards is a consequence
            // and would otherwise overwrite the reason with "connection closed".
            if !matches!(self.state, State::Refused(_)) {
                self.state = State::Lost(why);
            }
        }
        messages
    }
}

/// Why a build that cannot play without a shard cannot go on, or `None` while it still might.
///
/// `Offline` with an address is the frame before [`connect`] runs, not a failure.
pub fn out_of_reach(state: &State, address: Option<&str>) -> Option<String> {
    match state {
        State::Refused(why) | State::Lost(why) => Some(why.clone()),
        State::Offline if address.is_none() => Some("no server was named for this page".into()),
        State::Offline | State::Connecting | State::Joined(_) => None,
    }
}

/// Open a connection when one is configured and there is not one.
pub fn connect(mut uplink: ResMut<Uplink>, address: Res<ServerAddress>) {
    let Some(address) = address.0.as_deref() else {
        return;
    };
    if uplink.state != State::Offline {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        info!("connecting to {address}");
        uplink.open(Box::new(crate::link::WebSocketLink::connect(address)));
    }
    #[cfg(target_arch = "wasm32")]
    {
        info!("connecting to {address}");
        match crate::link::BrowserLink::connect(address) {
            Ok(link) => uplink.open(Box::new(link)),
            // Opening throws only for an address the browser will not take at all — a bad URL,
            // or plain `ws://` from a page served over TLS. Not something retrying fixes.
            Err(why) => {
                error!("could not open a socket to {address}: {why}");
                uplink.state = State::Refused(why);
            }
        }
    }
}

/// Read the socket, and fold what it said.
pub fn pump(
    mut uplink: ResMut<Uplink>,
    ticket: Res<crate::Ticket>,
    time: Res<bevy::prelude::Time>,
    mut game: ResMut<crate::app::Game>,
    mut ui: ResMut<crate::app::Ui>,
    mut out: MessageWriter<crate::input::Requested>,
) {
    let greet_now = {
        let link = uplink.link.get_mut().unwrap();
        link.as_ref().is_some_and(|l| l.status().is_open()) && !uplink.greeted
    };
    if greet_now {
        uplink.greeted = true;
        let ticket = ticket.0.clone().unwrap_or_default();
        uplink.say(Inbound::Hello {
            protocol: PROTOCOL_VERSION,
            ticket,
        });
    }

    let now_s = time.elapsed_secs_f64();
    for message in uplink.take() {
        // An answer to an order stops the clock on it, whether the answer was yes or no.
        if matches!(message, Outbound::Accepted { .. } | Outbound::Refused { .. })
            && let Some(asked_at) = uplink.asked_at.take()
        {
            uplink.round_trip_s = Some((now_s - asked_at).max(0.0));
        }
        fold(&mut uplink, &mut game, &mut ui, message);
    }

    // The server's word on an order, shown once. Written here rather than by the action fold
    // because until it arrives there is nothing true to say.
    if let Some(said) = uplink.applied.take() {
        let at = game.0.coordinate_time_s();
        ui.0.notify(said, at);
    }

    // What arrived and is owed an answer. Through the ordinary request channel, so an automatic
    // acknowledgement is the same kind of thing as one a player sent, and is clamped, rate
    // limited and recorded exactly as one.
    for (to, aim) in std::mem::take(&mut uplink.owed) {
        out.write(crate::input::Requested(crate::action::Action::Say {
            to: Some(to),
            aim,
            // Never encrypted. There is nothing in it to protect — an acknowledgement is its
            // identifiers and no body — and encrypting one would mean holding a key this ship
            // may not have.
            secrecy: lc_proto::Secrecy::Open,
            body: String::new(),
            // Its own message, and a real one: it is minted a key so that a resend of *it*
            // would be the same message rather than another.
            idem: Some(
                lc_world::rng::hash(&[to.0 as u64, (game.0.coordinate_time_s() * 1.0e6) as u64])
                    .max(1),
            ),
        }));
    }
}

fn fold(
    uplink: &mut Uplink,
    game: &mut crate::app::Game,
    ui: &mut crate::app::Ui,
    message: Outbound,
) {
    match message {
        Outbound::Welcome {
            client_id,
            ship_id,
            now_t,
            name,
            rate,
            ship,
            ..
        } => {
            info!(?ship_id, %name, at = ?ship.at_ly, doing = ?ship.motive, "welcomed");
            // The server's clock, adopted whole. Both ends propagate analytically from a
            // coordinate time, so agreeing on it is the whole of agreeing about where anything
            // is — and the client's own clock started whenever this process did.
            // Remembered, then applied — and applied again every time the session is rebuilt.
            // See `Placement` for what goes wrong when it is only applied once.
            uplink.placement = Some(Placement { now_t, ship });
            uplink.place(&mut game.0);
            // A pursuit that outlived the last connection is stated straight after this.
            uplink.chasing = None;
            // **The server's rate, adopted.** Refusing to *change* the rate was not enough:
            // the client's own default is sixty times the server's, so a joined client ran
            // away from it at a hundred and forty coordinate hours a second without anybody
            // touching a key.
            ui.0.time_rate = rate;
            // Which craft this is, so a message addressed to it can be told from one that
            // merely reached it. See `crate::chat::Chat::me`.
            uplink.chat.i_am(ship_id);
            // What it knows is the shard's to say. Anything this client worked out before it was
            // welcomed was a guess made without it; the craft's own knowledge arrives in pages
            // of `Learnt` from here on.
            game.0.knowledge = lc_world::knowledge::Knowledge::new(lc_world::knowledge::Witness(ship_id.0 as u64));
            uplink.state = State::Joined(Joined {
                client_id,
                ship_id,
                name,
            });
        }
        Outbound::WrongProtocol { server } => {
            uplink.state = State::Refused(format!(
                "the server speaks protocol {server} and this client speaks {PROTOCOL_VERSION}"
            ));
        }
        Outbound::Unauthenticated => {
            uplink.state = State::Refused("the server did not accept this ticket".into());
        }
        Outbound::Flying { ship_id, ship } => {
            // Taken, not reconciled. This is the authority saying what this ship is doing,
            // about a solve the client has no way to reproduce — it cannot see the quarry the
            // way the server can, which is the whole reason the server flies the policy.
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                game.0.restore(&(&ship).into());
            }
        }
        Outbound::Present(cleared) => {
            let system = game.0.system.as_deref();
            uplink.contacts =
                cleared.into_iter().map(|c| Contact::seen(c.into_inner(), system)).collect();
            // The server drops a pursuit when its quarry goes out of sight and does not say
            // so — saying so would be a message about somewhere this client can no longer see.
            // Losing the contact is the same fact arriving the only way it can.
            if uplink
                .chasing
                .is_some_and(|p| !uplink.contacts.iter().any(|c| c.ship_id == p.quarry))
            {
                uplink.chasing = None;
            }
        }
        Outbound::Sightings(cleared) => {
            let seen: Vec<Sighting> = cleared.into_iter().map(|c| c.into_inner()).collect();
            for sighting in seen
                .iter()
                .filter(|s| matches!(s.kind, lc_proto::kind::MESSAGE | lc_proto::kind::KEY))
            {
                let Ok(spoken) = serde_json::from_str::<lc_proto::Spoken>(&sighting.payload)
                else {
                    continue;
                };
                let from = ShipId(sighting.source_id);
                let name = uplink.contacts.iter().find(|c| c.ship_id == from).map(|c| c.name.clone());
                let key = sighting.kind == lc_proto::kind::KEY;
                // Said before it is folded, because what the box shows is what this craft can
                // read — which for somebody else's sealed mail is the fact of it and no more.
                let who = name.clone().unwrap_or_else(|| uplink.name_of(from));
                // **Overheard traffic is announced and not quoted.** That two other craft are
                // talking is the news; what they said to each other is theirs, and repeating
                // it into this ship's own events box reads as if it had been said here.
                let notice = match uplink.chat.filing(spoken.to) {
                    crate::chat::Filing::Overheard => {
                        let to = spoken.to.map(|to| uplink.name_of(ShipId(to)));
                        format!("{who} -> {}", to.unwrap_or_else(|| "somebody".into()))
                    }
                    _ => {
                        let said = match (key, &spoken.body) {
                            (true, _) => "sent you its key".to_string(),
                            (false, Some(body)) => body.clone(),
                            (false, None) => "(encrypted, and not for you)".to_string(),
                        };
                        format!("{who}: {said}")
                    }
                };
                ui.0.heard(from, notice, sighting.arrive_t as f64 * 1e-6);
                // The bearing the signal came in on, which is what a dish answers down. Not
                // where they are now, and not where they will be: where the light left them.
                if let Some(aim) = uplink.chat.received(
                    from,
                    name.as_deref(),
                    sighting.event_id,
                    spoken,
                    key,
                    sighting.emitted_t as f64 * 1e-6,
                    sighting.arrive_t as f64 * 1e-6,
                    sighting.strength,
                    sighting.direction,
                ) {
                    uplink.owed.push((from, aim));
                }
            }
            // A report is not something anybody said, so it is not filed in a conversation and
            // never answered automatically. What it does is fold into what this craft knows,
            // stamped with the moment its light landed — which is where every record in it
            // gains the hop that says it came from over there. See
            // `lightcone/docs/22-provenance.md`.
            for sighting in seen.iter().filter(|s| s.kind == lc_proto::kind::REPORT) {
                let Ok(reported) = serde_json::from_str::<lc_proto::Reported>(&sighting.payload)
                else {
                    continue;
                };
                let from = ShipId(sighting.source_id);
                let who = uplink.name_of(from);
                let arrived_s = sighting.arrive_t as f64 * 1e-6;
                let Some(body) = reported.body.as_deref() else {
                    ui.0.heard(from, format!("{who} sent a sealed report"), arrived_s);
                    continue;
                };
                // Only counted here. The shard folds it into this craft's knowledge when its light
                // lands, signed in or not, and what it taught arrives in the next `Learnt`.
                let stars = serde_json::from_str::<lc_world::knowledge::Report>(body).map(|r| r.stars());
                let notice = match stars {
                    Ok(n) => format!("{who}: told you about {n} stars"),
                    Err(_) => format!("{who} sent a report that made no sense"),
                };
                ui.0.heard(from, notice, arrived_s);
            }
            for sighting in seen.iter().filter(|s| s.kind == lc_proto::kind::DRIVE) {
                let Ok(change) = serde_json::from_str::<lc_proto::DriveChange>(&sighting.payload) else {
                    continue;
                };
                let drives = uplink.drives.entry(ShipId(sighting.source_id)).or_default();
                let at = (sighting.emitted_t as f64 * 1e-6, change.power_w);
                let place = drives.partition_point(|(at_s, _)| *at_s <= at.0);
                drives.insert(place, at);
                let excess = drives.len().saturating_sub(REMEMBERED_DRIVES);
                drives.drain(..excess);
            }
            uplink.seen.extend(seen);
            let excess = uplink.seen.len().saturating_sub(REMEMBERED);
            uplink.seen.drain(..excess);
        }
        Outbound::Accepted { ship_id, event_id, at_t, order } => {
            // **Applied at the server's time, with the server's numbers.** Both are clamped
            // and neither is what was sent, so folding what was sent instead is how a client
            // ends up somewhere the server does not have it.
            let at_s = at_t as f64 * 1e-6;
            // The server drops a standing intercept on any flight order, so the pursuit the
            // interface shows is over too.
            if matches!(
                order,
                Order::SetCourse { .. } | Order::Cross { .. } | Order::CutDrive | Order::Burn { .. }
            ) {
                uplink.chasing = None;
            }
            let said = match &order {
                Order::SendReport { to, .. } => Some(match to.map(|t| uplink.name_of(t)) {
                    Some(who) => format!("report sent to {who}"),
                    None => "report broadcast".to_string(),
                }),
                Order::SetDuty { duty, integration_s } => {
                    game.0.adopt_duty(duty, *integration_s);
                    None
                }
                // What it did arrives in the next `Learnt`, with everything else it knows.
                Order::NameIt { .. } => None,
                Order::SetCourse { course, accel_g, max_beta } => {
                    let course: lc_world::navigation::Course = course.clone().into();
                    match game.0.set_course_at(at_s, &course, *accel_g, *max_beta) {
                        Some(label) => Some(format!("course: {label} at {accel_g:.0} g")),
                        None => Some("that course could not be flown".into()),
                    }
                }
                Order::Cross { star, accel_g, max_beta } => {
                    // Resolved here too, against this client's own catalogue — the same one
                    // the shard was given, which is what makes an id mean one thing on both
                    // ends. A star this build does not hold is a shard and a client that were
                    // handed different skies, and saying so is better than flying nowhere.
                    match game.0.star_by_raw(*star).map(|s| s.position_ly) {
                        Some(to_ly) => {
                            let name = game
                                .0
                                .star_by_raw(*star)
                                .map(|s| game.0.name_of(s.id))
                                .unwrap_or_else(|| "an undetected source".into());
                            match game.0.cross_to_at(at_s, to_ly, *accel_g, *max_beta) {
                                Some(cruise) => {
                                    let years =
                                        cruise.duration_s() / crate::flight::JULIAN_YEAR_S;
                                    let aboard = cruise.proper_duration_s()
                                        / crate::flight::JULIAN_YEAR_S;
                                    Some(format!(
                                        "{name}: {years:.2} years out, {aboard:.2} aboard"
                                    ))
                                }
                                None => Some("that crossing could not be flown".into()),
                            }
                        }
                        None => Some("this build does not have that star".into()),
                    }
                }
                Order::CutDrive => {
                    let note = match game.0.cut_drive_at(at_s) {
                        // What it says is where the ship ended up, because cutting does not
                        // stop it: it keeps its velocity and that velocity is now an orbit.
                        Some(coast) => format!("drive cut — {}", crate::hud::arc(&coast)),
                        None => "drive cut".to_string(),
                    };
                    Some(note)
                }
                // A standing intercept folds into nothing here. What it *does* arrives as a
                // motive, once per re-solve, through the same placement path a reconnect uses
                // — so the client is told the approach its ship is flying rather than working
                // one out from a quarry it can only see the past of.
                Order::Intercept { ship_id, closeness } => {
                    uplink.chasing = Some(lc_proto::Pursuit { quarry: *ship_id, closeness: *closeness });
                    Some(format!("closing on {}", ship_id.0))
                }
                // The server cut the drive of a ship that was flying the pursuit, and this folds
                // the same cut at the same instant.
                Order::BreakOff => {
                    uplink.chasing = None;
                    if game.0.ship.motion.pursuing().is_some() {
                        game.0.cut_drive_at(at_s);
                    }
                    Some("broke off".into())
                }
                // Nothing to fold into the ship's motion. A transmission is an event, and the
                // client learns of it the same way anyone else does: when its light arrives.
                Order::Transmit { .. } | Order::Burn { .. } => None,
                // What a refit does to the account arrives straight after, as `Fitted`.
                Order::Refit { .. } => Some("refit begun".into()),
                Order::CancelRefit => Some("refit stopped where it was".into()),
                // Recorded against the identifier the server minted, which is the only thing
                // an acknowledgement will ever name it by. Not shown in the events box: that
                // box is for what happened *to* this ship, and the chat window already has it.
                Order::Say { to, secrecy, body, idem, .. } => {
                    let name = to
                        .and_then(|t| uplink.contacts.iter().find(|c| c.ship_id == t))
                        .map(|c| c.name.clone());
                    uplink.chat.sent(
                        *to,
                        name.as_deref(),
                        event_id,
                        *idem,
                        Some(body.clone()),
                        matches!(secrecy, lc_proto::Secrecy::Sealed),
                        false,
                        at_s,
                    );
                    None
                }
                Order::OfferKey { to, .. } => {
                    let name = to
                        .and_then(|t| uplink.contacts.iter().find(|c| c.ship_id == t))
                        .map(|c| c.name.clone());
                    uplink.chat.sent(*to, name.as_deref(), event_id, 0, None, false, true, at_s);
                    None
                }
            };
            info!(?ship_id, at_t, ?order, "accepted");
            uplink.applied = said;
        }
        Outbound::Clock { now_t, rate } => {
            // A rate can change under a joined client: a development shard staging a scene is
            // the case, and a client still ticking at the old one runs away from the world
            // exactly as an unstated rate did.
            ui.0.time_rate = rate;
            // Only when it matters. Snapping to every statement would pull the clock back by
            // the statement's flight time, once a second, forever.
            let out_by = now_t - (game.0.coordinate_time_s() * 1e6) as i64;
            if out_by.abs() > clock_slack_us(rate) {
                game.0.correct_coordinate_time_us(now_t);
                let hours = out_by.abs() as f64 / 3.6e9;
                // Said out loud, because the world jumps when this happens and a jump nobody
                // explained reads as a bug in the physics.
                uplink.applied = Some(format!("clock corrected by {hours:.1} hours"));
            }
        }
        Outbound::Refused { ship_id, reason } => {
            warn!(?ship_id, ?reason, "an order was refused");
            if matches!(reason, Refusal::NotInSight | Refusal::TooFast) {
                uplink.chasing = None;
            }
            uplink.applied = Some(match reason {
                Refusal::Impossible => "the server refused that order".into(),
                Refusal::NotYours | Refusal::NotYou => "that is not your ship".into(),
                Refusal::NotInSight => "there is nothing there to close on".into(),
                Refusal::TooFast => "too fast to match; kill the closing speed first".into(),
                Refusal::NoEnergy => "not enough energy stored for that".into(),
                Refusal::Refitting => "the drones are working: cancel the refit to fly".into(),
                Refusal::UnderWay => "under way: cut the drive before refitting".into(),
                Refusal::Short(short) => crate::refit_panel::shortfall(short.into()).into(),
                // Their key has to arrive before it can be used, and asking for it is a
                // message like any other — which is to say, it takes as long as the light does.
                Refusal::NoKey => "no key for them yet; send yours and ask for theirs".into(),
                Refusal::NothingNew => "nothing new to report since the last one".into(),
            });
        }
        Outbound::Throttled { retry_after_ticks } => {
            warn!(retry_after_ticks, "throttled");
        }
        Outbound::Pursuing { ship_id, pursuit } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                uplink.chasing = Some(pursuit);
            }
        }
        // Held here and picked up by `crate::library`, which owns the shelf. The fold knows
        // about the world and a book is not part of it.
        Outbound::Library { base, books } => uplink.shelf = Some((base, books)),
        Outbound::Reading(marks) => uplink.bookmarks = Some(marks),
        // A report from this craft itself: what it has learnt since the last one, with no hop.
        Outbound::Learnt { report } => match serde_json::from_str::<lc_world::knowledge::Report>(&report) {
            Ok(report) => game.0.knowledge.absorb(&report),
            Err(why) => warn!(%why, "a knowledge page that would not parse"),
        },
        Outbound::Observing { duty, integration_s } => game.0.adopt_duty(&duty, integration_s),
        // Taken whole, like `Flying`: the authority's account, settled.
        Outbound::Fitted { ship_id, fitting } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                uplink.fitting = Some(fitting);
                game.0.ship.fit(Some((&fitting).into()));
            }
        }
        Outbound::Backlog { messages, keys } => {
            // Nothing is announced. A transcript is what was *already* said, and a box of
            // notifications about years-old messages on every sign-in would bury whatever is
            // actually happening now.
            let count = messages.len();
            uplink.chat.restore(messages, keys);
            if count > 0 {
                let at = game.0.coordinate_time_s();
                ui.0.notify(format!("{count} messages in the log"), at);
            }
        }
    }
}

/// How loudly a connection state should be shown.
///
/// The words live here and the palette lives in the interface, so "what does this state mean"
/// and "what color is that" stay separable. Every variant carries text: color is never the
/// only signal — see `lightcone/docs/18-ui-style.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Quiet,
    Working,
    Wrong,
}

/// What to show a player about the connection, if anything.
///
/// `None` offline, because no server was asked for. Saying "OFFLINE" there would imply
/// something had gone wrong with a thing nobody wanted.
pub fn note(state: &State, round_trip_s: Option<f64>) -> Option<(Note, String)> {
    match state {
        State::Offline => None,
        State::Connecting => Some((Note::Working, "CONNECTING".into())),
        State::Joined(joined) => {
            // The round trip, once there is one to show. Nothing here is predicted, so this is
            // exactly the delay between asking for something and seeing it — which makes it
            // the difference between "this feels slow" and "this is slow".
            let last = match round_trip_s {
                Some(seconds) => format!(" · {:.0} ms", seconds * 1e3),
                None => String::new(),
            };
            Some((Note::Quiet, format!("LINKED {}{last}", joined.name)))
        }
        State::Refused(why) => Some((Note::Wrong, format!("REFUSED — {why}"))),
        State::Lost(why) => Some((Note::Wrong, format!("LINK LOST — {why}"))),
    }
}

pub struct UplinkPlugin;

impl Plugin for UplinkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Uplink>()
            .init_resource::<ServerAddress>()
            .init_resource::<LocalShard>()
            .init_resource::<Demo>();
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(OnEnter(crate::app::AppState::InGame), start_local);
        app
            // Not gated on a state. The socket is not the game, and a connection that only
            // lived inside one screen would drop every time the player opened a menu.
            .add_systems(Update, (connect, pump).chain().in_set(crate::app::Stage::Link));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Offline;
    use lc_proto::{Cleared, Refusal};

    fn welcome(now_t: i64) -> Outbound {
        welcome_at(now_t, [0.0, 0.0, 0.0])
    }

    fn welcome_at(now_t: i64, ship_at: [f64; 3]) -> Outbound {
        welcome_doing(now_t, ship_at, lc_proto::Motive::Drifting { from_ly: ship_at, since_t: 0.0 })
    }

    /// A welcome for a ship that is *doing* something, which is what a reconnect finds.
    fn welcome_doing(now_t: i64, ship_at: [f64; 3], motive: lc_proto::Motive) -> Outbound {
        welcome_running_at(SERVER_RATE, now_t, ship_at, motive)
    }

    /// A welcome from a world running at some multiple of the design rate.
    fn welcome_running_at(
        rate: f64,
        now_t: i64,
        ship_at: [f64; 3],
        motive: lc_proto::Motive,
    ) -> Outbound {
        Outbound::Welcome {
            client_id: ClientId(3),
            protocol: PROTOCOL_VERSION,
            rate,
            ship_id: ShipId(7),
            now_t,
            name: "Ada".into(),
            ship: lc_proto::Motion {
                at_ly: ship_at,
                beta: [0.0; 3],
                attitude: [1.0, 0.0, 0.0],
                clock_s: 0.0,
                drive: lc_proto::Drive {
                    accel_g: 5.0,
                    max_beta: 0.999,
                    exhaust_v_m_s: 1.5e7,
                    slew_rate_rad_s: 0.05,
                },
                motive,
            },
        }
    }

    fn app() -> (Uplink, crate::app::Game, crate::app::Ui) {
        (
            Uplink::default(),
            crate::app::Game(crate::session::Session::new(
                &lc_world::sky::AuthoredStars::sample(),
                3,
            )),
            crate::app::Ui(crate::ui::UiState::default()),
        )
    }

    #[test]
    fn a_welcome_is_who_we_are() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let joined = uplink.joined().expect("joined");
        assert_eq!(joined.ship_id, ShipId(7));
        assert_eq!(joined.client_id, ClientId(3));
        assert_eq!(joined.name, "Ada");
    }

    /// The reason a `Welcome` carries a clock at all. A client whose own clock started when the
    /// process did would place every body somewhere the server does not have it.
    #[test]
    fn a_welcome_sets_the_clock_to_the_servers() {
        let (mut uplink, mut game, mut ui) = app();
        let a_while_in = 1_234_567_890_123;
        fold(&mut uplink, &mut game, &mut ui, welcome(a_while_in));
        assert_eq!(
            game.0.coordinate_time_s(),
            a_while_in as f64 * 1e-6,
            "the client kept its own clock",
        );
    }

    /// The other half of `Welcome`, and the one that was missing: a client that adopted only
    /// the clock flies a ship the server has somewhere else entirely.
    #[test]
    fn a_welcome_puts_the_ship_where_the_server_has_it() {
        let (mut uplink, mut game, mut ui) = app();
        let out_there = [4.2, -1.5, 0.25];
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, out_there));
        let at = game.0.ship.motion.position_ly;
        assert_eq!([at.x, at.y, at.z], out_there, "the client kept its own position");
        // And the observer follows the ship, or the sky is drawn from the old place.
        assert!(game.0.observer.x != 0, "the observer was left behind");
    }

    /// A ship that was *doing* something comes back doing it.
    ///
    /// The welcome used to carry a point, so a player who signed out of an orbit signed back
    /// into a drift — and drifted two and a half million kilometers off it in a day while the
    /// interface said LINKED.
    #[test]
    fn a_welcome_puts_the_ship_back_on_the_station_it_was_holding() {
        let (mut uplink, mut game, mut ui) = app();
        let station = lc_proto::Waypoint::Orbit {
            about: lc_proto::Anchor::Body("Earth".into()),
            radius_m: 1.2e7,
            pole: [0.0, 0.0, 1.0],
            phase_rad: 0.5,
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            welcome_doing(0, [4.2, 0.0, 0.0], lc_proto::Motive::Holding(station.clone())),
        );

        let expected = lc_world::resume::Snapshot::from(&lc_proto::Motion {
            at_ly: [4.2, 0.0, 0.0],
            beta: [0.0; 3],
            attitude: [1.0, 0.0, 0.0],
            clock_s: 0.0,
            drive: lc_proto::Drive {
                accel_g: 5.0,
                max_beta: 0.999,
                exhaust_v_m_s: 1.5e7,
                slew_rate_rad_s: 0.05,
            },
            motive: lc_proto::Motive::Holding(station),
        });
        let lc_world::resume::Recipe::Holding(waypoint) = expected.motive else {
            unreachable!("a station")
        };
        assert_eq!(
            game.0.ship.motion.motive,
            lc_world::motion::Motive::Holding(waypoint),
            "the client came back adrift",
        );
    }

    /// And the survivor of a rebuild is the whole state, not just the position: the sky load
    /// replaces the session after the welcome has already landed.
    #[test]
    fn a_rebuild_keeps_the_motive_and_not_only_the_position() {
        let (mut uplink, mut game, mut ui) = app();
        let doing = lc_proto::Motive::Drifting { from_ly: [1.0, 0.0, 0.0], since_t: 12.0 };
        fold(&mut uplink, &mut game, &mut ui, welcome_doing(0, [4.2, 0.0, 0.0], doing));

        game.0 = crate::session::Session::new(&lc_world::sky::AuthoredStars::sample(), 3);
        uplink.place(&mut game.0);

        match game.0.ship.motion.motive {
            lc_world::motion::Motive::Drifting { from_ly, since_t } => {
                assert_eq!(from_ly, glam::DVec3::new(1.0, 0.0, 0.0), "the line was redrawn");
                assert_eq!(since_t, 12.0);
            }
            other => unreachable!("{other:?}"),
        }
    }

    /// **The bug manual QA found.** The connection opens before the sky finishes loading — on
    /// purpose, because a ticket is worth sixty seconds — so the welcome lands first and the
    /// sky load then replaces the whole session. Without re-applying, the ship goes back to the
    /// origin with `remote` false: flying locally, in the wrong system, while the interface
    /// still says LINKED.
    #[test]
    fn a_session_rebuilt_after_a_welcome_is_still_the_servers() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(9_000_000, [4.2, 0.0, 0.0]));

        // What `enter_game` does when the sky arrives.
        game.0 = crate::session::Session::new(&lc_world::sky::AuthoredStars::sample(), 3);
        assert_eq!(game.0.ship.motion.position_ly, glam::DVec3::ZERO, "premise");
        assert!(!game.0.remote, "premise");

        uplink.place(&mut game.0);

        let at = game.0.ship.motion.position_ly;
        assert_eq!([at.x, at.y, at.z], [4.2, 0.0, 0.0], "the rebuild kept the origin");
        assert_eq!(game.0.coordinate_time_s(), 9.0, "the rebuild kept its own clock");
        assert!(game.0.remote, "it went back to flying locally while saying LINKED");
    }

    /// Offline, a rebuild is left alone — that is the single-process game and nobody has said
    /// where the ship is.
    #[test]
    fn a_session_rebuilt_with_no_server_is_not_touched() {
        let (uplink, mut game, mut _ui) = app();
        game.0.place_at(glam::DVec3::new(1.0, 2.0, 3.0));
        uplink.place(&mut game.0);
        assert_eq!(game.0.ship.motion.position_ly, glam::DVec3::new(1.0, 2.0, 3.0));
        assert!(!game.0.remote);
    }

    /// **The bug behind "clock corrected by 140 hours", every second.** Refusing to let a
    /// player *change* the rate was not enough: the client's own default was sixty times the
    /// server's, so a joined client ran away from it without anybody touching a key — and the
    /// correction fired every second and never fixed anything, because it re-diverged as fast
    /// as it was pulled back.
    ///
    /// The default is the design rate now, so the two agree before anything is sent. This
    /// still has to hold: `--rate` and the ladder can both leave a client running fast, and
    /// joining has to bring it back whatever put it there.
    #[test]
    fn joining_adopts_the_servers_rate() {
        let (mut uplink, mut game, mut ui) = app();
        // Sixty: what the offline default used to be, and what `--rate 60` still does.
        ui.0.time_rate = 60.0;
        assert!(ui.0.time_rate > SERVER_RATE, "premise: this outruns the server");

        fold(&mut uplink, &mut game, &mut ui, welcome(0));

        assert_eq!(ui.0.time_rate, SERVER_RATE, "the client kept its own rate");
    }

    /// **Whatever it says, not whatever was assumed.** The client used to hold a constant that
    /// happened to agree with a shard; a development shard staging a scene at sixty would have
    /// been joined by a client confidently running at one.
    #[test]
    fn a_client_runs_at_the_rate_the_welcome_states() {
        for rate in [SERVER_RATE, 60.0, 0.5] {
            let (mut uplink, mut game, mut ui) = app();
            let drifting = lc_proto::Motive::Drifting { from_ly: [0.0; 3], since_t: 0.0 };
            let said = welcome_running_at(rate, 0, [0.0; 3], drifting);

            fold(&mut uplink, &mut game, &mut ui, said);

            assert_eq!(ui.0.time_rate, rate, "the client did not take the stated rate");
        }
    }

    /// A rate can change under a joined client, and a clock statement is where it says so.
    #[test]
    fn a_restated_rate_is_adopted_without_a_second_welcome() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        assert_eq!(ui.0.time_rate, SERVER_RATE, "premise: joined at the design rate");

        let now_t = (game.0.coordinate_time_s() * 1e6) as i64;
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t, rate: 60.0 });

        assert_eq!(ui.0.time_rate, 60.0, "the client kept the rate it joined at");
    }

    /// **The correction storm.** At sixty times the design rate one server tick is seven
    /// coordinate hours, and a fixed one-hour slack makes every statement look like a runaway:
    /// the client snaps back seven hours twenty times a second, for ever, having never drifted.
    #[test]
    fn a_fast_clock_is_not_corrected_by_every_statement() {
        let (mut uplink, mut game, mut ui) = app();
        let drifting = lc_proto::Motive::Drifting { from_ly: [0.0; 3], since_t: 0.0 };
        fold(&mut uplink, &mut game, &mut ui, welcome_running_at(60.0, 0, [0.0; 3], drifting));

        let before = game.0.coordinate_time_s();
        // One tick of a world running at sixty, which is what a healthy statement is behind by.
        let tick_us = (50.0 * 8766.0 * 1_000.0 * 60.0) as i64;
        assert!(tick_us > CLOCK_SLACK_US, "premise: a tick outruns the fixed slack");
        let now_t = (before * 1e6) as i64 + tick_us;

        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t, rate: 60.0 });

        assert_eq!(game.0.coordinate_time_s(), before, "a healthy offset moved the clock");
        assert!(uplink.applied.is_none(), "it complained about nothing");
    }

    /// The arithmetic that made the number recognizable, kept so the correspondence is pinned
    /// rather than remembered: a multiplier of one is the server's 8766 coordinate seconds per
    /// real second, and the offline default used to be sixty of those — 143.7 coordinate hours
    /// a second, which is what the report said. It is one now, and this records what it cost.
    #[test]
    fn the_servers_rate_is_the_one_the_clocks_agree_at() {
        assert_eq!(crate::session::TIME_RATE * SERVER_RATE, 8766.0);
        // And a world that says sixty is a Julian year a minute, which is the number this
        // codebase already reaches for when it wants to watch something happen.
        assert!((crate::session::TIME_RATE * 60.0 * 60.0 - 31_557_600.0).abs() < 1.0);
        let gained_per_second = crate::session::TIME_RATE * (60.0 - SERVER_RATE);
        assert!(
            (gained_per_second / 3600.0 - 143.7).abs() < 0.1,
            "{} coordinate hours a second",
            gained_per_second / 3600.0,
        );
    }

    /// A clock statement in step with the client changes nothing. Snapping to every one would
    /// drag the clock backwards by the statement's own flight time, once a second, forever.
    #[test]
    fn a_clock_in_step_is_left_alone() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(10 * CLOCK_SLACK_US, [0.0; 3]));
        let before = game.0.coordinate_time_s();

        // A statement a fraction of the slack away, which is what a healthy connection looks
        // like: the message spent a tick and a network hop getting here.
        let close = (before * 1e6) as i64 - CLOCK_SLACK_US / 4;
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: close, rate: SERVER_RATE });

        assert_eq!(game.0.coordinate_time_s(), before, "a healthy offset moved the clock");
        assert!(uplink.applied.is_none(), "it complained about nothing");
    }

    /// **The bug behind the teleporting.** A client whose clock has run away — a warp, or a
    /// throttled background tab — is pulled back, and told, because the world jumps.
    #[test]
    fn a_clock_that_has_run_away_is_pulled_back() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));

        // A day of coordinate time ahead of the server, which a warp reaches in seconds.
        let server_t = 0;
        game.0.correct_coordinate_time_us(24 * CLOCK_SLACK_US);
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: server_t, rate: SERVER_RATE });

        assert_eq!(game.0.coordinate_time_s(), 0.0, "the client kept its own clock");
        let said = uplink.applied.clone().expect("a jump nobody explained reads as a bug");
        assert!(said.contains("clock corrected"), "{said}");
    }

    /// A correction moves the world's clock and **not** the crew's. The ship's proper time is
    /// however long they have actually lived through, and no amount of resynchronising the
    /// coordinate clock un-ages anybody.
    #[test]
    fn a_correction_does_not_un_age_the_crew() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));
        game.0.ship.motion.clock_s = 12_345.0;

        game.0.correct_coordinate_time_us(24 * CLOCK_SLACK_US);
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 0, rate: SERVER_RATE });

        assert_eq!(game.0.ship.motion.clock_s, 12_345.0, "the crew was un-aged");
    }

    /// A statement about other craft becomes the list the renderer and the reticle draw from,
    /// carrying the retarded position rather than a recipe for working out a present one.
    #[test]
    fn a_presence_becomes_a_contact_to_draw() {
        let (mut uplink, mut game, mut ui) = app();
        let presence = lc_proto::Presence {
            ship_id: ShipId(7),
            name: "Vela".into(),
            length_m: 1_200.0,
            at_ly: [1.0, 2.0, 3.0],
            beta: [0.0, 0.1, 0.0],
            facing: [0.0, 0.0, 2.0],
            jet_power_w: 4.2e17,
            emitted_t: 500_000,
            arrive_t: 1_000_000,
        };
        let cleared = lc_proto::Cleared::<lc_proto::Presence>::clear(presence, 1_000_000).unwrap();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Present(vec![cleared]));

        let [contact] = uplink.contacts.as_slice() else { panic!("{:?}", uplink.contacts) };
        assert_eq!(contact.name, "Vela");
        assert_eq!(contact.length_m, 1_200.0);
        assert_eq!(contact.position_ly, glam::DVec3::new(1.0, 2.0, 3.0));
        // Normalised on the way in, so nothing downstream has to wonder.
        assert_eq!(contact.facing, glam::DVec3::Z);
        assert_eq!(contact.emitted_s, 0.5);
        assert_eq!(contact.jet_power_w, 4.2e17, "it was seen burning");

        // Replaced wholesale, not merged: a contact missing from a statement is gone.
        fold(&mut uplink, &mut game, &mut ui, Outbound::Present(Vec::new()));
        assert!(uplink.contacts.is_empty(), "a dropped contact was kept");
    }

    /// **The flicker this exists for.** Two craft a kilometer and a half apart in low orbit of
    /// Jupiter, and a statement once a server tick — 438 coordinate seconds at the design rate,
    /// three frames at sixty. Held still, the contact fell up to twenty thousand kilometers
    /// behind the ship between statements and snapped back on each one. Reckoned, the range
    /// reads the formation.
    #[test]
    fn a_contact_in_formation_holds_its_range_between_statements() {
        use lc_world::navigation::{Course, Plane};
        use lc_world::system::M_PER_LY;

        let provider =
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = lc_world::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("the Sun");
        let mut system = lc_world::system::LocalSystem::for_star(sun).expect("the solar system");
        system.advance_to(0.0);
        let station =
            Course::Orbit { body: "Jupiter".into(), altitude_radii: 0.5, plane: Plane::Equatorial }
                .resolve(&system, DVec3::ZERO, 0.0)
                .expect("an orbit");

        let standoff = DVec3::new(900.0, -1_200.0, 0.0) / M_PER_LY;
        let observer_at = |t: f64| station.place_at(&system, t).unwrap() + standoff;
        let tick_s = lc_server_tick_us(1.0) as f64 * 1e-6;
        let frame_s = tick_s / 3.0;

        for statement in 0..3 {
            let emitted_s = 1_000.0 + statement as f64 * tick_s;
            let presence = lc_proto::Presence {
                ship_id: ShipId(2),
                name: "Vela".into(),
                length_m: 500.0,
                at_ly: station.place_at(&system, emitted_s).unwrap().to_array(),
                beta: lc_world::coast::beta_of(station.velocity_at(&system, emitted_s).unwrap())
                    .to_array(),
                facing: [1.0, 0.0, 0.0],
                jet_power_w: 0.0,
                emitted_t: (emitted_s * 1e6) as i64,
                arrive_t: (emitted_s * 1e6) as i64,
            };
            let mut contact = Contact::seen(presence, Some(&system));
            // Every frame drawn before the next statement lands, and frames from a client whose
            // clock is behind the shard's, which receives each sample from its own future.
            for frame in -3..4 {
                let now_s = emitted_s + frame as f64 * frame_s;
                let here = observer_at(now_s);
                contact.reckon(Some(&system), here, now_s, &[]);
                let range_m = here.distance(contact.position_ly) * M_PER_LY;
                assert!(
                    (range_m - 1_500.0).abs() < 5.0,
                    "{range_m:.0} m at {frame} frames after statement {statement}, not 1500"
                );
            }
        }
    }

    fn heard(event_id: i64, from: i64, spoken: lc_proto::Spoken, kind: i16) -> Outbound {
        let sighting = Sighting {
            event_id,
            source_id: from,
            arrive_t: 4_000_000,
            emitted_t: 1_000_000,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind,
            payload: serde_json::to_string(&spoken).unwrap(),
        };
        Outbound::Sightings(vec![
            lc_proto::Cleared::<Sighting>::clear(sighting, 4_000_000, 0.0).unwrap(),
        ])
    }

    /// What a craft knows is the shard's. A welcome replaces whatever this client worked out on
    /// its own with an empty copy owned by the craft, and `Learnt` fills it — a report from the
    /// craft itself, folded with no hop.
    #[test]
    fn a_welcome_hands_what_the_craft_knows_to_the_shard() {
        use lc_world::knowledge::{Bearing, Knowledge, Sighting as Seen, Witness};

        let (mut uplink, mut game, mut ui) = app();
        game.0.issue_charts(30.0);
        assert!(!game.0.knowledge.is_empty(), "a guess made before signing in");
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let me = uplink.joined().expect("welcomed").ship_id;
        assert_eq!(game.0.knowledge.owner, Witness(me.0 as u64));
        assert!(game.0.knowledge.is_empty(), "the shard says what it knows");

        let star = game.0.stars[0].id;
        let mut held = Knowledge::new(Witness(me.0 as u64));
        held.sighted(
            star,
            Seen {
                witness: Witness(me.0 as u64),
                observed_s: 1.0,
                bearing: Bearing { observer_ly: glam::DVec3::ZERO, toward: glam::DVec3::X, sigma_rad: 1e-9 },
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            },
        );
        let report = serde_json::to_string(&held.report(f64::NEG_INFINITY, 1.0)).unwrap();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Learnt { report });
        assert_eq!(game.0.knowledge.belief(star).unwrap().hops, 0, "its own, not relayed");
    }

    /// A report landing is a line in the events box, counted — and nothing more here. The shard
    /// folds it into the craft's knowledge when its light lands, signed in or not, and what it
    /// taught arrives in the next `Learnt`.
    #[test]
    fn a_report_arriving_is_counted_and_its_content_comes_from_the_shard() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let report = lc_world::knowledge::Report { from: lc_world::knowledge::Witness(7), sent_s: 1.0, entries: Vec::new() };
        let reported = lc_proto::Reported {
            to: Some(0),
            beamed: false,
            idem: 3,
            sealed: false,
            body: Some(serde_json::to_string(&report).unwrap()),
        };
        let sighting = Sighting {
            event_id: 21,
            source_id: 7,
            arrive_t: 4_000_000,
            emitted_t: 1_000_000,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::REPORT,
            payload: serde_json::to_string(&reported).unwrap(),
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Sightings(vec![lc_proto::Cleared::<Sighting>::clear(sighting, 4_000_000, 0.0).unwrap()]),
        );
        assert!(
            ui.0.notifications.iter().any(|n| n.text.contains("told you about 0 stars")),
            "{:?}",
            ui.0.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>(),
        );
        assert!(game.0.knowledge.is_empty(), "the shard folds it, not the client");
    }

    /// A sealed report for somebody else arrives as the fact of it. An eavesdropper learns
    /// that a survey went past and nothing about what is in it.
    #[test]
    fn a_sealed_report_teaches_an_eavesdropper_nothing() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let reported = lc_proto::Reported {
            to: Some(9),
            beamed: false,
            idem: 4,
            sealed: true,
            body: None,
        };
        let sighting = Sighting {
            event_id: 22,
            source_id: 7,
            arrive_t: 4_000_000,
            emitted_t: 1_000_000,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::REPORT,
            payload: serde_json::to_string(&reported).unwrap(),
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Sightings(vec![
                lc_proto::Cleared::<Sighting>::clear(sighting, 4_000_000, 0.0).unwrap(),
            ]),
        );
        assert!(game.0.knowledge.is_empty(), "nothing was learnt from it");
        assert!(
            ui.0.notifications
                .iter()
                .any(|n| n.text.contains("sealed report")),
            "but that it happened is not a secret",
        );
    }

    /// A message arriving is two things at once: a line in the transcript, and a green line in
    /// the events box that opens it. The second is what makes the first findable.
    #[test]
    fn a_message_arriving_is_both_a_line_and_a_notice() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 11,
            sealed: false,
            body: Some("are you there".into()),
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(99, 2, spoken, lc_proto::kind::MESSAGE));

        let conversation = uplink.chat.get(ShipId(2)).expect("a conversation with the sender");
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.lines[0].body.as_deref(), Some("are you there"));

        let note = ui.0.notifications.last().expect("nothing in the events box");
        assert_eq!(note.from, Some(ShipId(2)), "the notice does not open anything");
        assert!(note.text.contains("are you there"));
    }

    /// Somebody else's sealed mail is heard and not read, and the box says exactly that rather
    /// than staying silent — a signal falling on the antenna is a fact about the world.
    #[test]
    fn a_sealed_message_for_somebody_else_is_still_noticed() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken =
            lc_proto::Spoken {
                to: Some(99),
                beamed: false,
                idem: 12,
                sealed: true,
                body: None,
                acks: Vec::new(),
            };
        fold(&mut uplink, &mut game, &mut ui, heard(98, 2, spoken, lc_proto::kind::MESSAGE));

        // Somebody else's mail, so it is overheard rather than a conversation with the sender.
        let overheard = uplink.chat.overheard();
        let heard = overheard.first().expect("nothing was overheard");
        assert!(heard.line.sealed);
        assert_eq!(heard.line.body, None);
        assert_eq!(heard.to, Some(ShipId(99)), "it forgot who it was for");
        assert!(uplink.chat.get(ShipId(2)).is_none(), "it became a conversation with the sender");
        assert!(ui.0.notifications.last().is_some_and(|n| n.from == Some(ShipId(2))));
    }

    /// **Overheard traffic is announced, not quoted.** That two other craft are talking is the
    /// news; what they said to each other is theirs, and repeating it into this ship's own
    /// events box would read as if it had been said here.
    #[test]
    fn an_overheard_message_is_announced_without_its_contents() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken = lc_proto::Spoken {
            to: Some(99),
            beamed: false,
            idem: 21,
            sealed: false,
            body: Some("rendezvous at the third moon".into()),
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(97, 2, spoken, lc_proto::kind::MESSAGE));

        let note = ui.0.notifications.last().expect("nothing in the events box");
        assert!(!note.text.contains("rendezvous"), "it quoted somebody else's mail: {}", note.text);
        assert!(note.text.contains("->"), "it did not say who was talking to whom: {}", note.text);
        // And it still opens somewhere: the craft that transmitted it.
        assert_eq!(note.from, Some(ShipId(2)));
    }

    /// Sending is not receiving. A message this ship sent is in the transcript against the
    /// identifier the server minted — which is the only thing an acknowledgement can name — and
    /// is not in the events box, which is for what happened *to* this ship.
    #[test]
    fn an_accepted_message_is_recorded_against_the_identifier_it_will_be_acknowledged_by() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let before = ui.0.notifications.len();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Accepted {
            ship_id: ShipId(7),
            event_id: 4242,
            at_t: 2_000_000,
            order: Order::Say {
                to: Some(ShipId(2)),
                aim: lc_proto::Aim::Omni,
                secrecy: lc_proto::Secrecy::Open,
                body: "hello".into(),
                idem: 4242,
            },
        });
        let conversation = uplink.chat.get(ShipId(2)).expect("a conversation");
        assert_eq!(conversation.lines[0].event_ids, vec![4242]);
        assert!(conversation.lines[0].mine);
        let sent = conversation.lines[0].clone();
        assert!(!conversation.delivered(&sent), "unanswered, so not acknowledged");
        assert_eq!(ui.0.notifications.len(), before, "a sent message reported itself as news");

        // And the acknowledgement, when it comes back, names it.
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 13,
            sealed: false,
            body: Some("got it".into()),
            acks: vec![4242],
        };
        fold(&mut uplink, &mut game, &mut ui, heard(43, 2, spoken, lc_proto::kind::MESSAGE));
        assert!(uplink.chat.get(ShipId(2)).unwrap().delivered(&sent));
    }

    /// A key offer arriving is what puts a key in the ring, and the ring is what the interface
    /// reads to decide whether sealing can be offered at all.
    #[test]
    fn a_key_offer_arriving_is_what_lets_the_interface_offer_sealing() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        assert!(!uplink.chat.holds_key(ShipId(2)));
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 0,
            sealed: false,
            body: Some(String::new()),
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(50, 2, spoken, lc_proto::kind::KEY));
        assert!(uplink.chat.holds_key(ShipId(2)));
        assert!(uplink.chat.get(ShipId(2)).unwrap().lines[0].key, "it is in the transcript too");
    }

    /// A flight order ends a standing intercept on the server, so the interface stops showing
    /// one. A transmission is not a flight order.
    #[test]
    fn an_accepted_flight_order_ends_the_pursuit_shown() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let accepted = |order| Outbound::Accepted { ship_id: ShipId(7), event_id: 1, at_t: 0, order };
        let pursuit = lc_proto::Pursuit { quarry: ShipId(2), closeness: lc_proto::Closeness::Company };

        uplink.chasing = Some(pursuit);
        fold(&mut uplink, &mut game, &mut ui, accepted(Order::Transmit { power_w: 1.0 }));
        assert_eq!(uplink.chasing, Some(pursuit), "a transmission ended it");

        for order in [Order::CutDrive, Order::Burn { beta: [0.0, 1e-3, 0.0] }] {
            uplink.chasing = Some(pursuit);
            fold(&mut uplink, &mut game, &mut ui, accepted(order.clone()));
            assert_eq!(uplink.chasing, None, "{order:?} left the pursuit showing");
        }
    }

    /// **A flip between two statements still goes dark.** Sampled burning, then told by drive
    /// events that it went out at 100 s and lit again at 160 s: drawn in between, the plume is
    /// out, though no statement ever saw it so.
    #[test]
    fn a_contacts_plume_follows_its_drive_events_between_statements() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let burning = 4.0e17;
        let presence = lc_proto::Presence {
            ship_id: ShipId(2),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            jet_power_w: burning,
            emitted_t: 0,
            arrive_t: 0,
        };
        let present = |p: lc_proto::Presence| {
            let arrive_t = p.arrive_t;
            Outbound::Present(vec![Cleared::<lc_proto::Presence>::clear(p, arrive_t).unwrap()])
        };
        fold(&mut uplink, &mut game, &mut ui, present(presence.clone()));

        let drive = |event_id, at_s: f64, power_w| {
            let change = lc_proto::DriveChange { power_w, facing: [1.0, 0.0, 0.0] };
            let sighting = Sighting {
                event_id,
                source_id: 2,
                arrive_t: (at_s * 1e6) as i64,
                emitted_t: (at_s * 1e6) as i64,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: lc_proto::kind::DRIVE,
                payload: serde_json::to_string(&change).unwrap(),
            };
            Cleared::<Sighting>::clear(sighting, (at_s * 1e6) as i64, 0.0).unwrap()
        };
        let events = vec![drive(1, 100.0, 0.0), drive(2, 160.0, burning)];
        fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(events));

        let power_at = |uplink: &mut Uplink, now_s: f64| {
            uplink.reckon(None, DVec3::X * 1e3 / lc_world::system::M_PER_LY, now_s);
            uplink.contacts[0].jet_power_w
        };
        assert_eq!(power_at(&mut uplink, 50.0), burning, "before the flip");
        assert_eq!(power_at(&mut uplink, 130.0), 0.0, "in the flip");
        assert_eq!(power_at(&mut uplink, 170.0), burning, "after it");

        // A statement newer than every event is the latest word, and survives the next one.
        let later = lc_proto::Presence { jet_power_w: 0.0, emitted_t: 400_000_000, arrive_t: 400_000_000, ..presence };
        fold(&mut uplink, &mut game, &mut ui, present(later));
        assert_eq!(power_at(&mut uplink, 410.0), 0.0, "an older event outranked a newer statement");
    }

    #[test]
    fn a_protocol_mismatch_says_both_numbers() {
        let (mut uplink, mut game, mut ui) = app();
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::WrongProtocol { server: 99 },
        );
        let State::Refused(why) = &uplink.state else {
            panic!("{:?}", uplink.state)
        };
        assert!(
            why.contains("99") && why.contains(&PROTOCOL_VERSION.to_string()),
            "{why}"
        );
    }

    #[test]
    fn a_refused_ticket_is_terminal_and_says_so() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Unauthenticated);
        assert!(matches!(uplink.state, State::Refused(_)));
    }

    /// The socket closing after a refusal must not overwrite the reason: "connection closed"
    /// is true and tells a person nothing about what to do.
    #[test]
    fn a_close_after_a_refusal_keeps_the_refusal() {
        let (mut uplink, mut game, mut ui) = app();
        let mut link = Offline::new();
        link.status = Some(Status::Closed("connection closed".into()));
        uplink.open(Box::new(link));
        uplink.greeted = true;
        fold(&mut uplink, &mut game, &mut ui, Outbound::Unauthenticated);
        uplink.take();
        let State::Refused(why) = &uplink.state else {
            panic!("{:?}", uplink.state)
        };
        assert!(why.contains("ticket"), "the reason was overwritten: {why}");
    }

    #[test]
    fn a_link_that_closes_before_a_refusal_reports_the_close() {
        let (mut uplink, _game, mut _ui) = app();
        let mut link = Offline::new();
        link.status = Some(Status::Closed("connection refused".into()));
        uplink.open(Box::new(link));
        uplink.greeted = true;
        uplink.take();
        assert_eq!(uplink.state, State::Lost("connection refused".into()));
    }

    /// A bound, so a long session does not grow without limit before the fold exists.
    #[test]
    fn remembered_sightings_are_bounded_and_keep_the_newest() {
        let (mut uplink, mut game, mut ui) = app();
        for n in 0..(REMEMBERED as i64 + 10) {
            let sighting = Sighting {
                event_id: n,
                source_id: 1,
                arrive_t: n,
                emitted_t: 0,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: 0,
                payload: String::new(),
            };
            let cleared = Cleared::<Sighting>::clear(sighting, i64::MAX, 0.0).unwrap();
            fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(vec![cleared]));
        }
        assert_eq!(uplink.seen.len(), REMEMBERED);
        assert_eq!(
            uplink.seen.last().unwrap().event_id,
            REMEMBERED as i64 + 9,
            "it kept the oldest instead of the newest",
        );
    }

    /// The readout that turns "this feels slow" into a number. Absent until there is an order
    /// to have timed, because a zero would be a claim nobody measured.
    #[test]
    fn a_linked_note_shows_the_round_trip_once_there_is_one() {
        let joined = State::Joined(Joined {
            client_id: ClientId(1),
            ship_id: ShipId(1),
            name: "Ada".into(),
        });
        let (_, quiet) = note(&joined, None).unwrap();
        assert_eq!(quiet, "LINKED Ada", "it invented a measurement");

        let (_, timed) = note(&joined, Some(0.087)).unwrap();
        assert!(timed.contains("87 ms"), "{timed}");
        assert!(timed.contains("Ada"), "{timed}");
    }

    /// Offline is the single-process game, not a fault, and must not be dressed as one.
    #[test]
    fn nothing_is_said_about_a_connection_nobody_asked_for() {
        assert_eq!(note(&State::Offline, None), None);
    }

    #[test]
    fn every_other_state_says_something_a_person_can_read() {
        let states = [
            State::Connecting,
            State::Joined(Joined {
                client_id: ClientId(1),
                ship_id: ShipId(1),
                name: "Ada".into(),
            }),
            State::Refused("the server did not accept this ticket".into()),
            State::Lost("connection reset".into()),
        ];
        for state in states {
            let (_, words) = note(&state, None).expect("something to show");
            assert!(!words.is_empty());
            // Color is never the only signal, so the words have to carry it alone.
            assert!(
                words.chars().any(|c| c.is_ascii_uppercase()),
                "{words} reads as nothing",
            );
        }
        // And a refusal shows the reason rather than only that there was one.
        let (severity, words) = note(&State::Refused("no ticket".into()), None).unwrap();
        assert_eq!(severity, Note::Wrong);
        assert!(words.contains("no ticket"), "{words}");
    }

    /// The browser build strands on these, so a state that only *might* connect must not.
    #[test]
    fn only_a_dead_or_missing_link_is_out_of_reach() {
        let address = Some("wss://shard.example");
        assert_eq!(out_of_reach(&State::Offline, address), None, "connect has not run yet");
        assert_eq!(out_of_reach(&State::Connecting, address), None);
        assert!(out_of_reach(&State::Offline, None).is_some(), "no shard is never going to connect");
        let refused = out_of_reach(&State::Refused("no ticket".into()), address).unwrap();
        assert!(refused.contains("no ticket"), "{refused}");
        let lost = out_of_reach(&State::Lost("connection reset".into()), address).unwrap();
        assert!(lost.contains("connection reset"), "{lost}");
    }

    /// An order the server would not take is not a disconnection.
    #[test]
    fn a_refused_order_leaves_the_connection_alone() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Refused {
                ship_id: ShipId(7),
                reason: Refusal::Impossible,
            },
        );
        assert!(
            uplink.joined().is_some(),
            "a refused order dropped the connection"
        );
    }
}
