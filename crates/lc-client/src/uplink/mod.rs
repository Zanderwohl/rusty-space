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

// Nothing in the game loop panics: startup may, and past it a wire message, a row or another
// craft's report is data. Every exception carries an `allow` with its reason.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use std::sync::Mutex;

use bevy::prelude::*;
use glam::DVec3;
use lc_proto::{Inbound, Outbound, ShipId, Sighting};

use lc_world::system::LocalSystem;

pub use crate::contact::{Contact, DriveAt};
use crate::link::{Link, Status};

mod connection;
mod orders;
mod sightings;
mod welcome;

pub use connection::{Demo, Joined, LocalShard, Note, ServerAddress, State, connect, note, out_of_reach, pump};
#[cfg(not(target_arch = "wasm32"))]
pub use connection::start_local;
pub use orders::refused;
pub use welcome::{MAX_SLEW, SERVER_RATE, Slew, clock_snap_us};

/// What a ship with no account behind it is called.
///
/// Offline there is no broker to have said a name, and every craft on the map is named
/// including this one. A name rather than a word for the reader, so that a real name can be
/// told from the absence of one at a glance.
pub const ANONYMOUS: &str = "Anonymous Ship";

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

/// Reckon every contact against this frame's clock and ship.
pub fn reckon_contacts(game: Res<crate::app::Game>, mut uplink: ResMut<Uplink>) {
    uplink.reckon(game.0.system.as_deref(), game.0.ship.motion.position_ly, game.0.coordinate_time_s());
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
    /// The same kind of copy as `chasing`.
    pub parked: bool,
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
    /// The clock error still being absorbed. See [`Slew`].
    pub slew: Slew,
    /// Drive events per craft, oldest first. Kept across statements, which replace contacts.
    drives: std::collections::HashMap<ShipId, Vec<DriveAt>>,
    /// The last account the server stated, re-applied with the placement for the same reason.
    pub fitting: Option<lc_proto::Fitting>,
    /// What the server solved from the ship's form with that account. See [`crate::parts::adopt_fitted`].
    pub hull: Option<lc_proto::Hull>,
    /// Every conversation this ship is in. See [`crate::chat`].
    pub chat: crate::chat::Chat,
    pub console: crate::console::Console,
    pub incoming: crate::field::Incoming,
    pub beams: crate::emit_panel::Beams,
}

impl Uplink {
    // The link's lock is only ever held within one call, so a poisoned one means a panic
    // elsewhere already; the link it guards is still the link, and is taken as it is.

    /// Take a link and start greeting over it. Replaces whatever was there.
    pub fn open(&mut self, link: Box<dyn Link + Send>) {
        *self.link.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(link);
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
        if let Some(link) = self.link.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) {
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
        let Some(link) = self.link.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) else {
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

fn fold(
    uplink: &mut Uplink,
    game: &mut crate::app::Game,
    ui: &mut crate::app::Ui,
    message: Outbound,
) {
    crate::emit_panel::fold(uplink, &message, game.0.ship.motion.position_ly);
    match message {
        message @ (Outbound::Welcome { .. }
        | Outbound::WrongProtocol { .. }
        | Outbound::Unauthenticated
        | Outbound::Clock { .. }) => welcome::fold(uplink, game, ui, message),
        message @ (Outbound::Present(_) | Outbound::Sightings(_)) => sightings::fold(uplink, game, ui, message),
        message @ (Outbound::Accepted { .. } | Outbound::Refused { .. }) => orders::fold(uplink, game, ui, message),
        Outbound::Flying { ship_id, ship } => {
            // Taken, not reconciled. This is the authority saying what this ship is doing,
            // about a solve the client has no way to reproduce — it cannot see the quarry the
            // way the server can, which is the whole reason the server flies the policy.
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                game.0.restore(&(&ship).into());
            }
        }
        Outbound::Throttled { retry_after_ticks } => {
            warn!(retry_after_ticks, "throttled");
        }
        Outbound::Pursuing { ship_id, pursuit } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                uplink.chasing = Some(pursuit);
            }
        }
        Outbound::Parked { ship_id } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                uplink.parked = true;
            }
        }
        // Held here and picked up by `crate::library`, which owns the shelf. The fold knows
        // about the world and a book is not part of it.
        Outbound::Library { base, books } => uplink.shelf = Some((base, books)),
        Outbound::Reading(marks) => uplink.bookmarks = Some(marks),
        // A report from this craft itself: what it has learned since the last one, with no hop.
        Outbound::Learned { report } => match lc_proto::decode::<lc_world::knowledge::Report>(&report) {
            Ok(report) => game.0.knowledge.absorb(&report),
            Err(why) => warn!(%why, "a knowledge page that would not parse"),
        },
        // Its own photometry, which no report carries.
        Outbound::Logged { logs } => match lc_proto::decode::<lc_world::knowledge::Logs>(&logs) {
            Ok(page) => {
                game.0.knowledge.copy_logs(&page.logs);
                for subject in page.retained {
                    game.0.knowledge.retain_raw(subject, true);
                }
            }
            Err(why) => warn!(%why, "a log page that would not parse"),
        },
        Outbound::Observing { duty, integration_s } => game.0.adopt_duty(&duty, integration_s),
        // Taken whole, like `Flying`.
        Outbound::Fitted { ship_id, fitting, hull, field } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                game.0.ship.fit(Some(lc_world::fitting::Fitting::from_wire(&fitting, field.as_ref())));
                uplink.fitting = Some(fitting);
                uplink.hull = Some(hull);
            }
        }
        Outbound::AutoAcking { ship_id, with } => {
            if uplink.joined().is_some_and(|joined| joined.ship_id == ship_id) {
                uplink.chat.auto_acking(with);
            }
        }
        Outbound::Answered { seq, ok, text } => uplink.console.answered(seq, ok, text),
        Outbound::Analyzing { left } => game.0.analyzing = left as usize,
        Outbound::Doing { observing, fitting } => {
            game.0.doing = crate::session::Doing {
                observing: observing.map(Into::into),
                fitting: fitting.into_iter().map(Into::into).collect(),
            };
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
        // The welcome to the successor follows.
        Outbound::Collapsed { at_t, .. } => ui.0.notify("The field collapsed", at_t as f64 * 1e-6),
        Outbound::Illuminated { ship_id, beam, bearing, spectrum, power_w, arrive_t } => {
            uplink.incoming.illuminated(ship_id, beam, bearing, spectrum, power_w, arrive_t)
        }
        Outbound::Presets(list) => game.0.presets = list,
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
            .add_systems(Update, (connect, pump, crate::parts::adopt_fitted).chain().in_set(crate::app::Stage::Link));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_proto::{Cleared, ClientId, Order, PROTOCOL_VERSION};

    pub(super) fn welcome(now_t: i64) -> Outbound {
        welcome_at(now_t, [0.0, 0.0, 0.0])
    }

    pub(super) fn welcome_at(now_t: i64, ship_at: [f64; 3]) -> Outbound {
        welcome_doing(now_t, ship_at, lc_proto::Motive::Drifting { from_ly: ship_at, since_t: 0.0 })
    }

    /// A welcome for a ship that is *doing* something, which is what a reconnect finds.
    pub(super) fn welcome_doing(now_t: i64, ship_at: [f64; 3], motive: lc_proto::Motive) -> Outbound {
        welcome_running_at(SERVER_RATE, now_t, ship_at, motive)
    }

    /// A welcome from a world running at some multiple of the design rate.
    pub(super) fn welcome_running_at(
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
                    slew_rate_rad_s: 0.05,
                },
                motive,
            },
        }
    }

    pub(super) fn app() -> (Uplink, crate::app::Game, crate::app::Ui) {
        (
            Uplink::default(),
            crate::app::Game(crate::session::Session::new(
                &lc_world::sky::AuthoredStars::sample(),
                3,
            )),
            crate::app::Ui(crate::ui::UiState::default()),
        )
    }

    pub(super) fn heard(event_id: i64, from: i64, spoken: lc_proto::Spoken, kind: i16) -> Outbound {
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

    /// Heat weighs: dropping the field leaves the client's ship lighter than the authority's.
    #[test]
    fn a_fitted_field_is_the_ships_heat() {
        use lc_world::fitting::{Account, Balance, Fitting};
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let b = Balance::DEFAULT;
        let full = Fitting::full(lc_world::form::Form::starting(), b, 0.0);
        let hot = Fitting::from_account(&Account { heat_j: 5.0 * b.module_energy_j(), ..full.account() }, b);
        let fitted = Outbound::Fitted {
            ship_id: ShipId(7),
            fitting: (&hot).into(),
            hull: (&hot).into(),
            field: Some((&hot).into()),
        };
        fold(&mut uplink, &mut game, &mut ui, fitted);
        let ship = &game.0.ship;
        assert_eq!(ship.fitting(), Some(&hot));
        let motion = &ship.motion;
        assert_eq!(ship.fitting().unwrap().mass_kg_at(motion, 0.0), hot.mass_kg_at(motion, 0.0));
    }

    /// 31 §Client: the map draws this ship's beams and those landing on it, and nothing else. Another
    /// craft's emission, seen or accepted for someone else, is a beam this ship does not know of.
    #[test]
    fn only_beams_this_ship_sent_or_is_in_are_drawn() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let drawn = |uplink: &Uplink| crate::emit_panel::on_map(&uplink.beams, &uplink.incoming, DVec3::ZERO, None, 1.0, 1.0e9).len();
        let emit = |ship_id, event_id| Outbound::Accepted {
            ship_id,
            event_id,
            at_t: 0,
            order: Order::Emit {
                aim: lc_proto::Aim::Bearing([1.0, 0.0, 0.0]),
                apertures: lc_proto::Apertures::Aft,
                power_w: 1.0e18,
                wavelength_m: 1.0e-6,
                spread_rad: 1.0e-3,
                duration_s: 60.0,
                lead: lc_proto::Lead::Coasting,
            },
        };
        let elsewhere = Sighting {
            event_id: 30,
            source_id: 2,
            arrive_t: 500_000,
            emitted_t: 0,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::EMIT,
            payload: "{}".into(),
        };
        let elsewhere = Outbound::Sightings(vec![Cleared::<Sighting>::clear(elsewhere, 500_000, 0.0).unwrap()]);
        fold(&mut uplink, &mut game, &mut ui, elsewhere);
        fold(&mut uplink, &mut game, &mut ui, emit(ShipId(2), 31));
        assert_eq!(drawn(&uplink), 0, "a beam somebody else lit");

        fold(&mut uplink, &mut game, &mut ui, emit(ShipId(7), 32));
        assert_eq!(drawn(&uplink), 1, "this ship's own");
        let lit = |power_w| Outbound::Illuminated {
            ship_id: ShipId(7),
            beam: 40,
            bearing: [0.0, 1.0, 0.0],
            spectrum: lc_proto::Spectrum::Line { wavelength_m: 1.0e-6 },
            power_w,
            arrive_t: 500_000,
        };
        fold(&mut uplink, &mut game, &mut ui, lit(1.0e17));
        assert_eq!(drawn(&uplink), 2, "one landing here");
        fold(&mut uplink, &mut game, &mut ui, lit(0.0));
        assert_eq!(drawn(&uplink), 1, "its light went out");
        fold(&mut uplink, &mut game, &mut ui, accepted_cut());
        assert_eq!(drawn(&uplink), 0, "put out");
    }

    fn accepted_cut() -> Outbound {
        Outbound::Accepted { ship_id: ShipId(7), event_id: 50, at_t: 500_000, order: Order::CutDrive }
    }

    #[test]
    fn the_shards_list_of_presets_is_the_sessions() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let preset = lc_proto::Preset { name: "Long".into(), form: (&lc_world::form::Form::starting()).into() };
        fold(&mut uplink, &mut game, &mut ui, Outbound::Presets(vec![preset.clone()]));
        assert_eq!(game.0.presets, vec![preset]);
        fold(&mut uplink, &mut game, &mut ui, Outbound::Presets(Vec::new()));
        assert!(game.0.presets.is_empty());
    }
}
