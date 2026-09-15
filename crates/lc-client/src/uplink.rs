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
    info!("connecting to {address}");
    uplink.open(Box::new(crate::link::WebSocketLink::connect(address)));
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
        fold(&mut uplink, &mut game, message);
    }

    // The server's word on an order, shown once. Written here rather than by the action fold
    // because until it arrives there is nothing true to say.
    if let Some(said) = uplink.applied.take() {
        let at = game.0.coordinate_time_s();
        ui.0.notify(said, at);
    }
}

fn fold(uplink: &mut Uplink, game: &mut crate::app::Game, message: Outbound) {
    match message {
        Outbound::Welcome {
            client_id,
            ship_id,
            now_t,
            name,
            ..
        } => {
            info!(?ship_id, %name, "welcomed");
            // The server's clock, adopted whole. Both ends propagate analytically from a
            // coordinate time, so agreeing on it is the whole of agreeing about where anything
            // is — and the client's own clock started whenever this process did.
            game.0.set_coordinate_time_us(now_t);
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
        Outbound::Welcome {
            client_id: ClientId(3),
            protocol: PROTOCOL_VERSION,
            ship_id: ShipId(7),
            now_t,
            name: "Ada".into(),
        }
    }

    fn app() -> (Uplink, crate::app::Game) {
        (
            Uplink::default(),
            crate::app::Game(crate::session::Session::new(
                &lc_world::sky::AuthoredStars::sample(),
                3,
            )),
        )
    }

    #[test]
    fn a_welcome_is_who_we_are() {
        let (mut uplink, mut game) = app();
        fold(&mut uplink, &mut game, welcome(0));
        let joined = uplink.joined().expect("joined");
        assert_eq!(joined.ship_id, ShipId(7));
        assert_eq!(joined.client_id, ClientId(3));
        assert_eq!(joined.name, "Ada");
    }

    /// The reason a `Welcome` carries a clock at all. A client whose own clock started when the
    /// process did would place every body somewhere the server does not have it.
    #[test]
    fn a_welcome_sets_the_clock_to_the_servers() {
        let (mut uplink, mut game) = app();
        let a_while_in = 1_234_567_890_123;
        fold(&mut uplink, &mut game, welcome(a_while_in));
        assert_eq!(
            game.0.coordinate_time_s(),
            a_while_in as f64 * 1e-6,
            "the client kept its own clock",
        );
    }

    #[test]
    fn a_protocol_mismatch_says_both_numbers() {
        let (mut uplink, mut game) = app();
        fold(
            &mut uplink,
            &mut game,
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
        let (mut uplink, mut game) = app();
        fold(&mut uplink, &mut game, Outbound::Unauthenticated);
        assert!(matches!(uplink.state, State::Refused(_)));
    }

    /// The socket closing after a refusal must not overwrite the reason: "connection closed"
    /// is true and tells a person nothing about what to do.
    #[test]
    fn a_close_after_a_refusal_keeps_the_refusal() {
        let (mut uplink, mut game) = app();
        let mut link = Offline::new();
        link.status = Some(Status::Closed("connection closed".into()));
        uplink.open(Box::new(link));
        uplink.greeted = true;
        fold(&mut uplink, &mut game, Outbound::Unauthenticated);
        uplink.take();
        let State::Refused(why) = &uplink.state else {
            panic!("{:?}", uplink.state)
        };
        assert!(why.contains("ticket"), "the reason was overwritten: {why}");
    }

    #[test]
    fn a_link_that_closes_before_a_refusal_reports_the_close() {
        let (mut uplink, _game) = app();
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
        let (mut uplink, mut game) = app();
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
            fold(&mut uplink, &mut game, Outbound::Sightings(vec![cleared]));
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
        let (mut uplink, mut game) = app();
        fold(&mut uplink, &mut game, welcome(0));
        fold(
            &mut uplink,
            &mut game,
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
