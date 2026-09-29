//! Joining a shard, and keeping to its clock and rate.

// No panics in the game loop, as in `super`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use bevy::prelude::*;
use lc_proto::{Outbound, PROTOCOL_VERSION};

use super::{Joined, Placement, State, Uplink};

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

/// The most a slew changes the clock's rate by, as a fraction of it. Too little to see a moon's
/// orbit hurry, and it absorbs a coordinate hour — four tenths of a real second — in four seconds.
pub const MAX_SLEW: f64 = 0.1;

/// Real seconds a slew may take before a jump is the better answer: an error slewing cannot
/// absorb in this long is a background tab coming back, not drift.
const SLEW_WITHIN_S: f64 = 20.0;

/// How far the clock may be out before it is jumped rather than slewed, in coordinate
/// microseconds: two real seconds of the world's time at `rate`, about five coordinate hours at
/// the design rate. Scaled so it stays far above a statement's own age, which is a tick and a
/// network hop, at any rate.
pub fn clock_snap_us(rate: f64) -> i64 {
    (MAX_SLEW * SLEW_WITHIN_S * rate.max(0.0) * crate::session::TIME_RATE * 1e6) as i64
}

/// A clock error being absorbed by running a little fast or slow, rather than jumped.
///
/// Replaced, not added to, by each statement: the error is measured against the clock as it
/// already stands, so it already counts whatever was absorbed since the last. It carries the
/// statement's flight time with it, so the client settles that far behind the shard, which is
/// where a welcome puts it anyway.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slew {
    owed_us: i64,
}

impl Slew {
    pub fn owe(&mut self, us: i64) {
        self.owed_us = us;
    }

    pub fn owed_us(&self) -> i64 {
        self.owed_us
    }

    /// The coordinate time to advance by instead of `span_us`, paying off up to [`MAX_SLEW`] of
    /// it. Never below nine tenths of it, so the clock never stops or runs backwards.
    pub fn take(&mut self, span_us: i64) -> i64 {
        let limit = (span_us.max(0) as f64 * MAX_SLEW) as i64;
        let paid = self.owed_us.clamp(-limit, limit);
        self.owed_us -= paid;
        span_us + paid
    }
}

/// Coordinate microseconds a server tick covers at `rate`. Mirrors `lc_server::server::TICK_US`,
/// which the browser build cannot see.
#[cfg(test)]
pub(super) fn lc_server_tick_us(rate: f64) -> i64 {
    const DESIGN_TICK_US: f64 = 50.0 * 8766.0 * 1_000.0;
    (DESIGN_TICK_US * rate.max(0.0)) as i64
}

pub(super) fn fold(
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
            // A new session: an Apply sent on the old one will never be answered.
            ui.0.form.applying = crate::ledger::Applying::Idle;
            // The server's clock, adopted whole. Both ends propagate analytically from a
            // coordinate time, so agreeing on it is the whole of agreeing about where anything
            // is — and the client's own clock started whenever this process did.
            // Remembered, then applied — and applied again every time the session is rebuilt.
            // See `Placement` for what goes wrong when it is only applied once.
            uplink.placement = Some(Placement { now_t, ship });
            uplink.place(&mut game.0);
            uplink.slew = Slew::default();
            // A pursuit that outlived the last connection is stated straight after this.
            uplink.chasing = None;
            uplink.parked = false;
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
            // of `Learned` from here on.
            game.0.knowledge = lc_world::knowledge::Knowledge::new(lc_world::knowledge::Witness(ship_id.0 as u64));
            // Nobody, until the server says otherwise straight after this.
            uplink.chat.auto_acking(Vec::new());
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
        Outbound::Clock { now_t, rate } => {
            // A rate can change under a joined client: a development shard staging a scene is
            // the case, and a client still ticking at the old one runs away from the world
            // exactly as an unstated rate did.
            ui.0.time_rate = rate;
            let out_by = now_t - (game.0.coordinate_time_s() * 1e6) as i64;
            // Coordinate seconds, and what they are in real milliseconds at this rate.
            let real_ms = out_by as f64 * 1e-3 / (rate.max(f64::MIN_POSITIVE) * crate::session::TIME_RATE);
            debug!(out_by_s = out_by as f64 * 1e-6, real_ms, "clock stated");
            if out_by.abs() > clock_snap_us(rate) {
                game.0.correct_coordinate_time_us(now_t);
                uplink.slew = Slew::default();
                let hours = out_by.abs() as f64 / 3.6e9;
                let was = if out_by < 0 { "ahead" } else { "behind" };
                // Said out loud, because the world jumps when this happens and a jump nobody
                // explained reads as a bug in the physics.
                uplink.applied = Some(format!("clock corrected: was {hours:.1} hours {was}"));
            } else {
                uplink.slew.owe(out_by);
            }
        }
        // Routed elsewhere by `super::fold`.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::tests::*;
    use crate::uplink::*;
    use crate::uplink::fold;
    use lc_proto::ClientId;

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
    /// coordinate hours, and a fixed one-hour slack made every statement look like a runaway:
    /// the client snapped back seven hours twenty times a second, for ever, having never drifted.
    #[test]
    fn a_fast_clock_is_not_jumped_by_every_statement() {
        let (mut uplink, mut game, mut ui) = app();
        let drifting = lc_proto::Motive::Drifting { from_ly: [0.0; 3], since_t: 0.0 };
        fold(&mut uplink, &mut game, &mut ui, welcome_running_at(60.0, 0, [0.0; 3], drifting));

        let before = game.0.coordinate_time_s();
        // A statement three ticks of a world running at sixty out, far past a stale statement.
        let tick_us = lc_server_tick_us(60.0);
        assert!(tick_us > 3_600 * 1_000_000, "premise: a tick outruns an hour");
        let now_t = (before * 1e6) as i64 + 3 * tick_us;

        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t, rate: 60.0 });

        assert_eq!(game.0.coordinate_time_s(), before, "a healthy offset jumped the clock");
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

    /// **A small error is slewed, not jumped.** The clock does not move when the statement
    /// arrives; it runs slow until it has given back what it was ahead, and no faster than
    /// [`MAX_SLEW`] allows.
    #[test]
    fn a_clock_a_little_ahead_runs_slow_until_it_agrees() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));
        let hour_us = 3_600 * 1_000_000;
        game.0.correct_coordinate_time_us(hour_us);

        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 0, rate: SERVER_RATE });
        assert_eq!(game.0.coordinate_time_s(), 3_600.0, "the clock jumped");
        assert!(uplink.applied.is_none(), "a slew is not news");

        // Frames of a sixtieth of a real second, as `advance_clock` sees them.
        let frame_us = (crate::session::TIME_RATE * 1e6 / 60.0) as i64;
        let (mut real_us, mut frames) = (hour_us, 0);
        while uplink.slew.owed_us() != 0 {
            let step = uplink.slew.take(frame_us);
            assert!(step >= (frame_us as f64 * (1.0 - MAX_SLEW)) as i64, "slewed harder than allowed");
            game.0.advance_us(step);
            real_us += frame_us;
            frames += 1;
            assert!(frames < 60 * 10, "never caught up");
        }
        let at_us = (game.0.coordinate_time_s() * 1e6).round() as i64;
        assert_eq!(at_us, real_us - hour_us, "it slewed to the wrong place");
    }

    /// Each statement replaces what is owed: the error is measured against the clock as it
    /// stands, which already counts what the last one paid.
    #[test]
    fn a_statement_replaces_what_was_owed() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 1_000_000_000, rate: SERVER_RATE });
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 400_000_000, rate: SERVER_RATE });
        assert_eq!(uplink.slew.owed_us(), 400_000_000);
    }

    /// **The bug behind the teleporting.** A client whose clock has run away — a warp, or a
    /// throttled background tab — is pulled back, and told, because the world jumps.
    #[test]
    fn a_clock_that_has_run_away_is_pulled_back() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));

        // A day of coordinate time ahead of the server, which a background tab reaches in ten
        // real seconds.
        let server_t = 0;
        game.0.correct_coordinate_time_us(24 * 3_600 * 1_000_000);
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: server_t, rate: SERVER_RATE });

        assert_eq!(game.0.coordinate_time_s(), 0.0, "the client kept its own clock");
        let said = uplink.applied.clone().expect("a jump nobody explained reads as a bug");
        assert!(said.contains("clock corrected") && said.contains("ahead"), "{said}");
        assert_eq!(uplink.slew, Slew::default(), "a jump left a slew behind it");
    }

    /// A correction moves the world's clock and **not** the crew's. The ship's proper time is
    /// however long they have actually lived through, and no amount of resynchronizing the
    /// coordinate clock un-ages anybody.
    #[test]
    fn a_correction_does_not_un_age_the_crew() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome_at(0, [0.0; 3]));
        game.0.ship.motion.clock_s = 12_345.0;

        game.0.correct_coordinate_time_us(24 * 3_600 * 1_000_000);
        fold(&mut uplink, &mut game, &mut ui, Outbound::Clock { now_t: 0, rate: SERVER_RATE });

        assert_eq!(game.0.ship.motion.clock_s, 12_345.0, "the crew was un-aged");
    }

    /// What a craft knows is the shard's. A welcome replaces whatever this client worked out on
    /// its own with an empty copy owned by the craft, and `Learned` fills it — a report from the
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
                size: None,
                range_m: None,
                spin_s: None,
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            },
        );
        let report = lc_proto::encode(&held.report(lc_world::knowledge::Mark::default(), 1.0));
        fold(&mut uplink, &mut game, &mut ui, Outbound::Learned { report });
        assert_eq!(game.0.knowledge.belief(star).unwrap().hops, 0, "its own, not relayed");
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
}
