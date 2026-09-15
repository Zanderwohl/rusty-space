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
use lc_proto::{ClientId, Inbound, Order, Outbound, PROTOCOL_VERSION, Refusal, ShipId, Sighting};

use crate::link::{Link, Status};

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub now_t: i64,
    pub ship_at: [f64; 3],
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

/// Where the server is, if there is one.
#[derive(Resource, Default)]
pub struct ServerAddress(pub Option<String>);

/// Whether to run a shard in this process rather than connect to one. See [`crate::local`].
#[derive(Resource, Default)]
pub struct LocalShard(pub bool);

/// Start the in-process shard, and point [`ServerAddress`] at it.
///
/// On entering the world rather than at boot, because the shard has to be given the stars the
/// client actually ended up with: both ends put craft into systems by position against the same
/// shell radius, and two different skies would disagree about which system a ship is in.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_local(
    local: Res<LocalShard>,
    game: Res<crate::app::Game>,
    mut address: ResMut<ServerAddress>,
) {
    if !local.0 || address.0.is_some() {
        return;
    }
    match crate::local::start(game.0.stars.clone()) {
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
    /// What the server last said about an order, for the interface to show once and drop. The
    /// client cannot write its own here: an order's outcome is the server's to state.
    pub applied: Option<String>,
    /// The last placement the server gave, for [`Uplink::place`] to re-apply.
    placement: Option<Placement>,
}

/// The rate multiplier at which the client's clock runs at the server's.
///
/// One, because `session::TIME_RATE` — the coordinate seconds a multiplier of 1 buys per real
/// second — is the same 8766 the server advances by every tick. The client's default is sixty
/// times that, which is a development convenience whose own constant says "the server owns the
/// rate in a real session and this multiplier does not exist there". It does not exist there
/// from here on: joining adopts this.
///
/// If the two ever stop corresponding, the symptom is the clock correction below firing every
/// second and never fixing anything, because the client re-diverges as fast as it is pulled
/// back. That is the alarm working; it is not drift.
pub const SERVER_RATE: f64 = 1.0;

/// How far the clock may be out before it is pulled back, in coordinate microseconds.
///
/// One coordinate hour, which is about four tenths of a real second at the design rate. It has
/// to be comfortably more than a statement's own age — a tick plus the network, so a thousand
/// coordinate seconds or so — or every statement would drag the clock backwards by however long
/// it spent in flight. Below this the difference is smaller than the frames either side draws;
/// above it, positions disagree.
pub const CLOCK_SLACK_US: i64 = 3_600 * 1_000_000;

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
        let Some(placement) = self.placement else { return };
        session.set_coordinate_time_us(placement.now_t);
        session.place_at(glam::DVec3::from_array(placement.ship_at));
        session.remote = true;
    }

    pub fn joined(&self) -> Option<&Joined> {
        match &self.state {
            State::Joined(joined) => Some(joined),
            _ => None,
        }
    }

    /// Say something to the server. Silently does nothing with no link, which is the offline
    /// build and is not an error there.
    pub fn say(&mut self, message: Inbound) {
        if let Some(link) = self.link.get_mut().unwrap() {
            link.send(message);
        }
    }

    /// Read everything waiting, and fold it. Returns what arrived, for a caller that wants it.
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
    mut game: ResMut<crate::app::Game>,
    mut ui: ResMut<crate::app::Ui>,
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

    for message in uplink.take() {
        fold(&mut uplink, &mut game, &mut ui, message);
    }

    // The server's word on an order, shown once. Written here rather than by the action fold
    // because until it arrives there is nothing true to say.
    if let Some(said) = uplink.applied.take() {
        let at = game.0.coordinate_time_s();
        ui.0.notify(said, at);
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
            ship_at,
            ..
        } => {
            info!(?ship_id, %name, ?ship_at, "welcomed");
            // The server's clock, adopted whole. Both ends propagate analytically from a
            // coordinate time, so agreeing on it is the whole of agreeing about where anything
            // is — and the client's own clock started whenever this process did.
            // Remembered, then applied — and applied again every time the session is rebuilt.
            // See `Placement` for what goes wrong when it is only applied once.
            uplink.placement = Some(Placement { now_t, ship_at });
            uplink.place(&mut game.0);
            // **The server's rate, adopted.** Refusing to *change* the rate was not enough:
            // the client's own default is sixty times the server's, so a joined client ran
            // away from it at a hundred and forty coordinate hours a second without anybody
            // touching a key.
            ui.0.time_rate = SERVER_RATE;
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
        Outbound::Sightings(cleared) => {
            uplink
                .seen
                .extend(cleared.into_iter().map(|c| c.into_inner()));
            let excess = uplink.seen.len().saturating_sub(REMEMBERED);
            uplink.seen.drain(..excess);
        }
        Outbound::Accepted { ship_id, at_t, order, .. } => {
            // **Applied at the server's time, with the server's numbers.** Both are clamped
            // and neither is what was sent, so folding what was sent instead is how a client
            // ends up somewhere the server does not have it.
            let at_s = at_t as f64 * 1e-6;
            let said = match &order {
                Order::SetCourse { course, accel_g } => {
                    let course: lc_world::navigation::Course = course.clone().into();
                    match game.0.set_course_at(at_s, &course, *accel_g) {
                        Some(label) => Some(format!("course: {label} at {accel_g:.0} g")),
                        None => Some("that course could not be flown".into()),
                    }
                }
                Order::Cross { star, accel_g } => {
                    // Resolved here too, against this client's own catalogue — the same one
                    // the shard was given, which is what makes an id mean one thing on both
                    // ends. A star this build does not hold is a shard and a client that were
                    // handed different skies, and saying so is better than flying nowhere.
                    match game.0.star_by_raw(*star).map(|s| s.position_ly) {
                        Some(to_ly) => {
                            let name = game
                                .0
                                .star_by_raw(*star)
                                .and_then(|s| s.name.clone())
                                .unwrap_or_else(|| "an unnamed star".into());
                            match game.0.cross_to_at(at_s, to_ly, *accel_g) {
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
                // Nothing to fold into the ship's motion. A transmission is an event, and the
                // client learns of it the same way anyone else does: when its light arrives.
                Order::Transmit { .. } | Order::Burn { .. } => None,
            };
            debug!(?ship_id, at_t, ?order, "accepted");
            uplink.applied = said;
        }
        Outbound::Clock { now_t } => {
            // Only when it matters. Snapping to every statement would pull the clock back by
            // the statement's flight time, once a second, forever.
            let out_by = now_t - (game.0.coordinate_time_s() * 1e6) as i64;
            if out_by.abs() > CLOCK_SLACK_US {
                game.0.correct_coordinate_time_us(now_t);
                let hours = out_by.abs() as f64 / 3.6e9;
                // Said out loud, because the world jumps when this happens and a jump nobody
                // explained reads as a bug in the physics.
                uplink.applied = Some(format!("clock corrected by {hours:.1} hours"));
            }
        }
        Outbound::Refused { ship_id, reason } => {
            warn!(?ship_id, ?reason, "an order was refused");
            uplink.applied = Some(match reason {
                Refusal::Impossible => "the server refused that order".into(),
                Refusal::NotYours | Refusal::NotYou => "that is not your ship".into(),
            });
        }
        Outbound::Throttled { retry_after_ticks } => {
            warn!(retry_after_ticks, "throttled");
        }
    }
}

/// How loudly a connection state should be shown.
///
/// The words live here and the palette lives in the interface, so "what does this state mean"
/// and "what colour is that" stay separable. Every variant carries text: colour is never the
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
pub fn note(state: &State) -> Option<(Note, String)> {
    match state {
        State::Offline => None,
        State::Connecting => Some((Note::Working, "CONNECTING".into())),
        State::Joined(joined) => Some((Note::Quiet, format!("LINKED {}", joined.name))),
        State::Refused(why) => Some((Note::Wrong, format!("REFUSED — {why}"))),
        State::Lost(why) => Some((Note::Wrong, format!("LINK LOST — {why}"))),
    }
}

pub struct UplinkPlugin;

impl Plugin for UplinkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Uplink>()
            .init_resource::<ServerAddress>()
            .init_resource::<LocalShard>();
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(OnEnter(crate::app::AppState::InGame), start_local);
        app
            // Not gated on a state. The socket is not the game, and a connection that only
            // lived inside one screen would drop every time the player opened a menu.
            .add_systems(Update, (connect, pump).chain());
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
        Outbound::Welcome {
            client_id: ClientId(3),
            protocol: PROTOCOL_VERSION,
            ship_id: ShipId(7),
            now_t,
            name: "Ada".into(),
            ship_at,
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
    /// player *change* the rate was not enough: the client's own default is sixty times the
    /// server's, so a joined client ran away from it without anybody touching a key — and the
    /// correction fired every second and never fixed anything, because it re-diverged as fast
    /// as it was pulled back.
    #[test]
    fn joining_adopts_the_servers_rate() {
        let (mut uplink, mut game, mut ui) = app();
        ui.0.time_rate = crate::ui::TEST_TIME_RATE;
        assert!(ui.0.time_rate > SERVER_RATE, "premise: the default outruns the server");

        fold(&mut uplink, &mut game, &mut ui, welcome(0));

        assert_eq!(ui.0.time_rate, SERVER_RATE, "the client kept its own rate");
    }

    /// The arithmetic that made the number recognisable, kept so the correspondence is pinned
    /// rather than remembered: a multiplier of one is the server's 8766 coordinate seconds per
    /// real second, and the old default was sixty of those — 143.7 coordinate hours a second,
    /// which is what the report said.
    #[test]
    fn the_servers_rate_is_the_one_the_clocks_agree_at() {
        assert_eq!(crate::session::TIME_RATE * SERVER_RATE, 8766.0);
        let gained_per_second =
            crate::session::TIME_RATE * (crate::ui::TEST_TIME_RATE - SERVER_RATE);
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
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: close });

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
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: server_t });

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
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 0 });

        assert_eq!(game.0.ship.motion.clock_s, 12_345.0, "the crew was un-aged");
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
            let cleared = Cleared::clear(sighting, i64::MAX, 0.0).unwrap();
            fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(vec![cleared]));
        }
        assert_eq!(uplink.seen.len(), REMEMBERED);
        assert_eq!(
            uplink.seen.last().unwrap().event_id,
            REMEMBERED as i64 + 9,
            "it kept the oldest instead of the newest",
        );
    }

    /// Offline is the single-process game, not a fault, and must not be dressed as one.
    #[test]
    fn nothing_is_said_about_a_connection_nobody_asked_for() {
        assert_eq!(note(&State::Offline), None);
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
            let (_, words) = note(&state).expect("something to show");
            assert!(!words.is_empty());
            // Colour is never the only signal, so the words have to carry it alone.
            assert!(
                words.chars().any(|c| c.is_ascii_uppercase()),
                "{words} reads as nothing",
            );
        }
        // And a refusal shows the reason rather than only that there was one.
        let (severity, words) = note(&State::Refused("no ticket".into())).unwrap();
        assert_eq!(severity, Note::Wrong);
        assert!(words.contains("no ticket"), "{words}");
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
