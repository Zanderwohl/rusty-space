//! Where the server is, getting a link to it, and what to tell a player about that link.

// No panics in the game loop, as in `super`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use bevy::prelude::*;
use lc_proto::{ClientId, Inbound, Outbound, PROTOCOL_VERSION, ShipId};

use super::Uplink;

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
) {
    let greet_now = {
        let link = uplink.link.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        if matches!(
            message,
            Outbound::Accepted { .. } | Outbound::Refused { .. } | Outbound::AutoAcking { .. }
        )
            && let Some(asked_at) = uplink.asked_at.take()
        {
            uplink.round_trip_s = Some((now_s - asked_at).max(0.0));
        }
        super::fold(&mut uplink, &mut game, &mut ui, message);
    }

    // The server's word on an order, shown once. Written here rather than by the action fold
    // because until it arrives there is nothing true to say.
    if let Some(said) = uplink.applied.take() {
        let at = game.0.coordinate_time_s();
        ui.0.notify(said, at);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::tests::*;
    use crate::uplink::*;
    use crate::uplink::fold;
    use crate::link::{Offline, Status};

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
}
