//! Putting a scene in the world, and running it.
//!
//! **The one thing the authority may do that no client may.** Every order on the wire is a
//! request a ship makes about itself, validated against what its owner is entitled to; a craft
//! appearing somewhere by fiat is not that, and there is no order for it because there must not
//! be. So a scene is staged here, on the side that decides what happened, and the craft it puts
//! in the world are seen by exactly the rule everything else is seen by.
//!
//! What runs is [`lc_world::scenario`], which is data. Nothing below animates anything: a beat
//! becomes the same [`Change`] a player's order becomes, folded by the same function, and a
//! craft told to fly somewhere flies there and takes as long as it takes.
//!
//! Split from [`crate::server`] the way [`crate::chase`] is, and for the same reason it gives:
//! a `Fleet` will not lend itself twice, so the decision comes out as a list and is acted on
//! afterwards.

use std::sync::Arc;

use glam::DVec3;
use lc_proto::{ClientId, Outbound, Refusal, ShipId};
use lc_world::craft::{Craft, CraftId};
use lc_world::motion::{self, Change, Motive};
use lc_world::navigation::{Course, Waypoint};
use lc_world::scenario::{Act, Member, Scenario, Slot, Start};
use lc_world::system::{LocalSystem, M_PER_LY};

use crate::journal::Journal;
use crate::server::{BURN_POWER_W, KIND_BURN, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// A staged scene, and how far through it we are.
pub struct Director {
    scenario: &'static Scenario,
    /// Coordinate microseconds the cast was put in the world. `None` until there is somebody to
    /// put it around: every scene is arranged about the player's own craft, and at a launch
    /// flag's staging nobody has signed in yet.
    started_t: Option<i64>,
    /// Beats already fired. An index rather than a drain, because the list is static.
    next: usize,
    /// Which craft the player turned out to be.
    pov: Option<CraftId>,
}

impl Director {
    pub fn new(scenario: &'static Scenario) -> Self {
        Self {
            scenario,
            started_t: None,
            next: 0,
            pov: None,
        }
    }

    pub fn scenario(&self) -> &'static Scenario {
        self.scenario
    }

    /// Which craft a slot is. Cast identifiers are their position in the scene, offset clear of
    /// anything a sign-in mints.
    fn craft_in(&self, slot: Slot) -> Option<CraftId> {
        match slot {
            Slot::Pov => self.pov,
            Slot::Cast(at) => (at < self.scenario.cast.len())
                .then(|| CraftId(lc_world::scenario::BASE_ID + at as i64)),
        }
    }

    /// Every identifier this scene would have used, for clearing one out before staging another.
    fn cast_ids(scenario: &Scenario) -> impl Iterator<Item = CraftId> + '_ {
        (0..scenario.cast.len()).map(|at| CraftId(lc_world::scenario::BASE_ID + at as i64))
    }
}

/// Why a scene could not be staged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Staging {
    /// No scene by that name.
    NoSuchScene,
    /// The shard is not authoritative over the star the scene names, or has no sky at all.
    NoSuchStar,
}

impl<J: Journal> Server<J> {
    /// Put a scene in the world.
    ///
    /// The cast is not placed here. Every scene is arranged about the player's craft — a
    /// quarry forty hull lengths off the shoulder has nowhere to be until there is a shoulder —
    /// and at a launch flag's staging nobody has signed in. So this records the scene and the
    /// rate, and the first tick that finds an owned craft stages around it. The same path
    /// serves a scene staged from a panel, where the player is already there.
    pub fn stage(&mut self, scenario: &'static Scenario) -> Result<(), Staging> {
        self.world
            .star_named(scenario.star)
            .ok_or(Staging::NoSuchStar)?;
        // Whatever was standing about from the last scene goes, or two casts would share a
        // sky and the player would be told about craft nothing was still running. The scene
        // that was *running* rather than the one arriving: a shorter cast replacing a longer
        // one would otherwise leave the tail of the old one adrift and unaccounted for.
        let leaving = self
            .director
            .as_ref()
            .map(|d| d.scenario)
            .unwrap_or(scenario);
        for id in Director::cast_ids(leaving).chain(Director::cast_ids(scenario)) {
            self.fleet.remove(id);
            self.pursuits.remove(&id);
        }
        self.set_rate(scenario.rate);
        self.director = Some(Director::new(scenario));
        Ok(())
    }

    /// A client asking for a scene.
    pub fn staged(&mut self, from: ClientId, name: &str, wire: &mut impl Transport) {
        let staged = self.may(from, crate::ability::Act::Stage, None)
            && Scenario::named(name).is_some_and(|scene| self.stage(scene).is_ok());
        if !staged {
            // One answer for "this shard does not do that", "no scene by that name" and "no
            // such star", as every other refusal here is one answer: a client learning which
            // is a client learning what a shard it is not entitled to ask has in it.
            wire.send(
                from,
                Outbound::Refused {
                    ship_id: ShipId(0),
                    reason: Refusal::Impossible,
                },
            );
        }
    }

    /// Stage what has not been staged, and fire what is due.
    pub(crate) fn direct(
        &mut self,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let Some(mut director) = self.director.take() else {
            return;
        };
        if director.started_t.is_none() {
            self.open_scene(&mut director, wire);
        }
        if let Some(started_t) = director.started_t {
            let due: Vec<(CraftId, &'static Act)> = director.scenario.beats[director.next..]
                .iter()
                .take_while(|beat| started_t + (beat.after_s * 1.0e6) as i64 <= self.now_t())
                .filter_map(|beat| Some((director.craft_in(beat.actor)?, &beat.act)))
                .collect();
            // Counted against the beats, not against what was found: a beat about a craft that
            // is no longer there is spent, or it would be retried for ever.
            director.next += director.scenario.beats[director.next..]
                .iter()
                .take_while(|beat| started_t + (beat.after_s * 1.0e6) as i64 <= self.now_t())
                .count();
            for (id, act) in due {
                self.perform(&director, id, act, wire, events, deliveries);
            }
        }
        self.director = Some(director);
    }

    /// Put the cast in the world, once there is a player to arrange it about.
    fn open_scene(&mut self, director: &mut Director, wire: &mut impl Transport) {
        // Whoever is signed in. Staging is development-only, so there is one of them.
        let Some(pov) = self.owners.keys().copied().min() else {
            return;
        };
        let Some(at) = self.world.star_named(director.scenario.star) else {
            return;
        };
        // **From the shard's own cache, never a system built here.** Craft are counted as
        // sharing a system by pointer, and `resync_systems` will not replace an `Arc` whose
        // star already matches — so a cast handed a private copy would be in the right place,
        // in the right system by name, and invisible to everyone for ever.
        let Some(system) = self.world.system_at(at) else {
            return;
        };
        let now_s = self.now_t() as f64 * 1.0e-6;

        let pov_member = director.scenario.pov;
        {
            let Some(craft) = self.fleet.get_mut(pov) else {
                return;
            };
            craft.motion.drive.accel_g = pov_member.accel_g;
            // Entered before a motive is set: a course resolved against no system is refused,
            // and waiting for `resync_systems` would leave the craft a tick with nothing to do.
            craft.enter(Some(system.clone()), now_s);
            place(craft, &system, &pov_member, None, now_s);
        }
        let shoulder = self.fleet.get(pov).cloned();

        for (at_slot, member) in director.scenario.cast.iter().enumerate() {
            let id = CraftId(lc_world::scenario::BASE_ID + at_slot as i64);
            let at = shoulder
                .as_ref()
                .map(|c| c.motion.position_ly)
                .unwrap_or_default();
            let mut craft = Craft::at(id, member.kind, at);
            craft.name = Some(member.name.to_string());
            craft.length_m = member.length_m;
            craft.motion.drive = craft.kind.drive();
            craft.motion.drive.accel_g = member.accel_g;
            // Entered before a motive is set: a course resolved against no system is refused,
            // and waiting for `resync_systems` would leave the craft a tick with nothing to do.
            craft.enter(Some(system.clone()), now_s);
            place(&mut craft, &system, member, shoulder.as_ref(), now_s);
            self.fleet.insert(craft);
        }

        director.pov = Some(pov);
        director.started_t = Some(self.now_t());
        // The player's craft has been put somewhere it did not ask to be, which is a state it
        // cannot reach by folding anything it sent. Handed one, and it takes it.
        self.tell_flying(wire, pov);
    }

    fn perform(
        &mut self,
        director: &Director,
        id: CraftId,
        act: &Act,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let at_t = self.now_t();
        let now_s = at_t as f64 * 1.0e-6;
        let change = match act {
            Act::Fly(spelling) => {
                let Some(course) = Course::parse(spelling) else {
                    return;
                };
                let Some(craft) = self.fleet.get(id) else {
                    return;
                };
                Some(Change::SetCourse {
                    course,
                    drive: craft.turning(craft.motion.drive),
                })
            }
            Act::Cut => Some(Change::CutDrive),
            Act::Chase(on) => {
                let Some(quarry) = director.craft_in(*on) else {
                    return;
                };
                self.pursuits.insert(
                    id,
                    crate::chase::Pursuit {
                        quarry: ShipId(quarry.0),
                        closeness: lc_world::pursuit::Closeness::Company,
                        // Never planned, so the guidance loop takes it this tick and solves the
                        // first approach itself. A second solve here would be a second
                        // implementation of the only standing order there is.
                        last_plan_t: i64::MIN,
                        last_seen: None,
                    },
                );
                None
            }
            Act::BreakOff => {
                self.pursuits.remove(&id);
                None
            }
        };
        if let Some(change) = change {
            let event = motion::Event {
                ship: motion::ShipId(id.0),
                at_t: now_s,
                change,
            };
            let Some(craft) = self.fleet.get_mut(id) else {
                return;
            };
            if craft.apply(&event).is_err() {
                return;
            }
        }
        // A burn, and burns are the loudest thing a ship does. Everyone in range learns that
        // this craft maneuvered, at light delay, exactly as they would for any other.
        self.emit(
            id,
            KIND_BURN,
            BURN_POWER_W,
            "{}".into(),
            at_t,
            events,
            deliveries,
        );
        self.tell_flying(wire, id);
    }
}

/// Put one craft where its scene says it starts.
fn place(
    craft: &mut Craft,
    system: &Arc<LocalSystem>,
    member: &Member,
    beside: Option<&Craft>,
    now_s: f64,
) {
    let shoulder = beside.map(|c| c.motion.position_ly).unwrap_or_default();
    match member.start {
        // Nothing to do, which is the point: whatever put it there knew better than this does.
        Start::AsFound => {}
        Start::Alongside { lengths, .. } => {
            let Some(other) = beside else { return };
            let Motive::Holding(Waypoint::Orbit(orbit)) = &other.motion.motive else {
                return;
            };
            if orbit.radius_m <= 0.0 {
                return;
            }
            let mut here = orbit.clone();
            // Arc length into angle. A standoff quoted in hull lengths is the same picture at
            // any radius, which one quoted in meters is not.
            here.phase_rad += lengths * other.length_m / orbit.radius_m;
            let here = Waypoint::Orbit(here);
            let Some(at) = here.place_at(system, now_s) else {
                return;
            };
            craft.motion.position_ly = at;
            craft.motion.begin_holding(here);
        }
        Start::Holding(spelling) => {
            let Some(course) = Course::parse(spelling) else {
                return;
            };
            let from = craft.motion.position_ly;
            let Some(waypoint) = course.resolve(system, from, now_s) else {
                return;
            };
            let waypoint = waypoint.nearest_to(from, system, now_s);
            let Some(at) = waypoint.place_at(system, now_s) else {
                return;
            };
            craft.motion.position_ly = at;
            craft.motion.begin_holding(waypoint);
        }
        Start::Beside { lengths, bearing } => {
            let bearing = DVec3::from(bearing).normalize_or(DVec3::X);
            let at = shoulder + bearing * (lengths * member.length_m / M_PER_LY);
            craft.motion.position_ly = at;
            craft.motion.begin_holding(Waypoint::Fixed(at));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use crate::world::World;
    use lc_proto::ClientId;
    use lc_world::craft::Kind;
    use lc_world::sky::CatalogueStar;

    /// A star the real solar system hangs off. `LocalSystem` keys the JPL-fitted preset off the
    /// name, so this is the measured two hundred and thirty bodies rather than a generated set
    /// — which is the only place Jupiter and Saturn exist to be orbited.
    fn sol() -> Option<CatalogueStar> {
        let mut star = crate::server::course_tests::a_star()?;
        star.provenance.name = Some(lc_world::system::SOL.to_string());
        Some(star)
    }

    /// A server with a sky, a player, and a scene asked for.
    fn staged(scene: &'static Scenario) -> Option<(Server<Memory>, Loopback, ClientId, CraftId)> {
        let star = sol()?;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.directing(true);
        let pov = CraftId(1);
        server.admit(
            ClientId(1),
            Craft::at(pov, Kind::Ship, star.position_ly),
            0.0,
        );
        server.load_world(World::new(vec![star]));
        server.stage(scene).expect("the scene stages");
        Some((server, Loopback::new(), ClientId(1), pov))
    }

    /// **The failure that has no symptom.** Craft are counted as sharing a system by pointer —
    /// `chase::in_sight` is an `Arc::ptr_eq` — while `resync_systems` skips any craft whose
    /// system already has the right *star*. A cast handed a system built here would therefore
    /// be in the right place, in the right system by name, never corrected, and invisible to
    /// everybody for ever: no contacts, no intercepts, no error.
    ///
    /// Checked against a craft the director never touched, whose system came from the shard's
    /// own cache. Checking the cast against the player proves nothing — they are placed by the
    /// same code, so a private copy would be a copy they agreed on.
    #[tokio::test]
    async fn a_staged_cast_shares_the_shards_system_and_not_a_copy_of_it() {
        let Some((mut server, mut wire, _, _)) = staged(&lc_world::scenario::MEETING) else {
            return;
        };
        // A bystander, placed into its system by `resync_systems` like anything else.
        let star_at = server.fleet.get(CraftId(1)).unwrap().motion.position_ly;
        server.admit(
            ClientId(2),
            Craft::at(CraftId(9), Kind::Probe, star_at),
            0.0,
        );
        server.tick(&mut wire).await.unwrap();
        server.tick(&mut wire).await.unwrap();

        let bystander = server.fleet.get(CraftId(9)).expect("the bystander");
        let cast = server
            .fleet
            .get(CraftId(lc_world::scenario::BASE_ID))
            .expect("the cast");
        assert!(
            bystander.system.is_some(),
            "premise: the bystander was placed"
        );
        assert!(
            Arc::ptr_eq(
                bystander.system.as_ref().unwrap(),
                cast.system.as_ref().expect("the cast is in a system"),
            ),
            "the cast is in a private copy of the system",
        );
        assert!(
            crate::chase::in_sight(bystander, cast),
            "nobody can see the cast"
        );
    }

    /// A scene is arranged about the player, so it waits for one. A flag stages before anybody
    /// has signed in, and a quarry forty hull lengths off the shoulder has nowhere to be until
    /// there is a shoulder.
    #[tokio::test]
    async fn a_scene_waits_for_somebody_to_arrange_it_around() {
        let Some(star) = sol() else { return };
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        server.load_world(World::new(vec![star]));
        server
            .stage(&lc_world::scenario::MEETING)
            .expect("the scene stages");

        server.tick(&mut wire).await.unwrap();
        assert!(
            server
                .fleet
                .get(CraftId(lc_world::scenario::BASE_ID))
                .is_none(),
            "a cast was placed around nobody",
        );
    }

    /// The player is put somewhere it did not ask to be, which is a state it cannot reach by
    /// folding anything it sent. It is handed one.
    #[tokio::test]
    async fn the_player_is_told_where_it_was_put() {
        let Some((mut server, mut wire, client, pov)) = staged(&lc_world::scenario::APPROACH)
        else {
            return;
        };
        let before = server.fleet.get(pov).unwrap().motion.position_ly;
        server.tick(&mut wire).await.unwrap();

        let after = server.fleet.get(pov).unwrap().motion.position_ly;
        assert_ne!(before, after, "the player was left where it started");
        let said = wire.take(client);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Flying { .. })),
            "the player was moved and not told: {said:?}",
        );
    }

    /// Beats are spent whether or not they landed, or one about a craft that has gone would be
    /// retried on every tick for the life of the scene.
    #[tokio::test]
    async fn a_beat_fires_once_and_is_not_fired_again() {
        let Some((mut server, mut wire, _, _)) = staged(&lc_world::scenario::MEETING) else {
            return;
        };
        for _ in 0..4 {
            server.tick(&mut wire).await.unwrap();
        }
        let director = server.director.as_ref().expect("a scene is running");
        assert_eq!(
            director.next,
            director.scenario.beats.len(),
            "beats were left unspent"
        );
    }

    /// Two casts in one sky would have the player told about craft nothing was still running.
    #[tokio::test]
    async fn staging_a_second_scene_clears_the_first_cast() {
        let Some((mut server, mut wire, _, _)) = staged(&lc_world::scenario::TRAFFIC) else {
            return;
        };
        server.tick(&mut wire).await.unwrap();
        let traffic = lc_world::scenario::TRAFFIC.cast.len();
        let last = CraftId(lc_world::scenario::BASE_ID + traffic as i64 - 1);
        assert!(
            server.fleet.get(last).is_some(),
            "premise: the first cast is there"
        );

        server
            .stage(&lc_world::scenario::MEETING)
            .expect("the second scene stages");
        server.tick(&mut wire).await.unwrap();

        assert!(
            server.fleet.get(last).is_none(),
            "the first cast stayed in the sky"
        );
        assert_eq!(
            server.rate(),
            lc_world::scenario::MEETING.rate,
            "the rate did not follow"
        );
    }

    /// A shard is not started for this, and a client that asks is told no rather than ignored.
    #[tokio::test]
    async fn staging_is_refused_by_a_shard_that_was_not_started_for_it() {
        let Some(star) = sol() else { return };
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let client = ClientId(1);
        server.admit(
            client,
            Craft::at(CraftId(1), Kind::Ship, star.position_ly),
            0.0,
        );
        server.load_world(World::new(vec![star]));

        wire.client_says(
            client,
            lc_proto::Inbound::Stage {
                scenario: "meeting".into(),
            },
        );
        server.tick(&mut wire).await.unwrap();

        assert!(
            server.director.is_none(),
            "a shard staged a scene it was not started for"
        );
        let said = wire.take(client);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Refused { .. })),
            "it was ignored rather than refused: {said:?}",
        );
    }

    /// A name nobody has is refused, and does not take down the scene already running.
    #[tokio::test]
    async fn a_scene_nobody_has_heard_of_is_refused() {
        let Some((mut server, mut wire, client, _)) = staged(&lc_world::scenario::MEETING) else {
            return;
        };
        wire.client_says(
            client,
            lc_proto::Inbound::Stage {
                scenario: "nonesuch".into(),
            },
        );
        server.tick(&mut wire).await.unwrap();

        let running = server.director.as_ref().map(|d| d.scenario.name);
        assert_eq!(running, Some("meeting"), "the running scene was lost");
        assert!(
            wire.take(client)
                .iter()
                .any(|m| matches!(m, Outbound::Refused { .. }))
        );
    }
    /// **The scene works or it does not.** Meeting somebody is ending up beside them, and the
    /// measure of that is a distance that stays: twelve hull lengths of the craft being met,
    /// held there while both of them go round Jupiter at forty kilometers a second.
    #[tokio::test]
    async fn the_meeting_holds_beside_the_player() {
        let Some((mut server, mut wire, _, pov)) = staged(&lc_world::scenario::MEETING) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let apart = |s: &Server<Memory>| {
            let (a, b) = (s.fleet.get(pov).unwrap(), s.fleet.get(cast).unwrap());
            a.motion.position_ly.distance(b.motion.position_ly) * lc_world::system::M_PER_LY
        };
        server.tick(&mut wire).await.unwrap();

        // Twelve lengths of a five-hundred-meter hull, and the arc is short enough that the
        // chord across it is the same number to well inside a per cent.
        let want = 12.0 * 500.0;
        assert!(
            (apart(&server) - want).abs() < want * 0.05,
            "opened {:.0} m apart",
            apart(&server)
        );

        // And it is a co-orbit rather than a coincidence: a fixed point would be left behind
        // within a tick at this speed. Four hundred ticks is a couple of days and many orbits.
        for _ in 0..400 {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            (apart(&server) - want).abs() < want * 0.05,
            "it drifted to {:.0} m",
            apart(&server),
        );
    }

    /// Being approached is a distance that shrinks. It does not have to arrive to be the
    /// picture — a hull growing in the window is the whole of it — but it must plainly close.
    #[tokio::test]
    async fn the_approach_closes() {
        let Some((mut server, mut wire, _, pov)) = staged(&lc_world::scenario::APPROACH) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let apart = |s: &Server<Memory>| {
            let (a, b) = (s.fleet.get(pov).unwrap(), s.fleet.get(cast).unwrap());
            a.motion.position_ly.distance(b.motion.position_ly) * lc_world::system::M_PER_LY
        };
        // Tick counts are game time at the design rate, and the scene runs slower than that.
        let ticks = |n: usize| (n as f64 / lc_world::scenario::APPROACH.rate).round() as usize;
        server.tick(&mut wire).await.unwrap();
        let opening = apart(&server);
        for _ in 0..ticks(1500) {
            server.tick(&mut wire).await.unwrap();
        }
        let closed = apart(&server);
        assert!(
            closed < opening / 20.0,
            "it barely closed: {opening:.0} to {closed:.0} m"
        );

        // **And then it holds its standoff, though it turns slowly.** A five-kilometer hull
        // takes six hundred seconds to come about, and while its plans ignored the planet every
        // correction was a flip against a quarry that had fallen ninety kilometers out of the
        // plan — so it could not get inside a hundred thousand. Reckoned along the quarry's
        // conic, it arrives once and stays: see `lc_world::consort`.
        let standoff = lc_world::pursuit::standoff_m(
            server.fleet.get(pov).unwrap().length_m,
            server.fleet.get(cast).unwrap().length_m,
        );
        let (mut nearest, mut furthest) = (f64::INFINITY, 0.0f64);
        for _ in 0..ticks(1000) {
            server.tick(&mut wire).await.unwrap();
            nearest = nearest.min(apart(&server));
            furthest = furthest.max(apart(&server));
        }
        assert!(
            nearest > standoff * 0.9 && furthest < standoff * 1.1,
            "it held {nearest:.0} to {furthest:.0} m off a {standoff:.0} m standoff",
        );
    }
    /// The reciprocal of the approach, and the direction that was missing: the small ship is
    /// the one doing the closing.
    ///
    /// **It reaches the standoff and stays on it**, which it could not do while the guidance
    /// was gated on a flat ten coordinate minutes — it rode a relative orbit a hundred to four
    /// hundred kilometers across instead — nor, to better than tens of kilometers, while its
    /// plans ignored the planet both ships were falling round.
    #[tokio::test]
    async fn the_small_ship_closes_on_the_large_one() {
        let Some((mut server, mut wire, _, pov)) = staged(&lc_world::scenario::CLOSING) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let apart = |s: &Server<Memory>| {
            let (a, b) = (s.fleet.get(pov).unwrap(), s.fleet.get(cast).unwrap());
            a.motion.position_ly.distance(b.motion.position_ly) * lc_world::system::M_PER_LY
        };
        server.tick(&mut wire).await.unwrap();
        let opening = apart(&server);

        for _ in 0..1500 {
            server.tick(&mut wire).await.unwrap();
        }
        let standoff = lc_world::pursuit::standoff_m(
            server.fleet.get(pov).unwrap().length_m,
            server.fleet.get(cast).unwrap().length_m,
        );
        let deadband = standoff * lc_world::pursuit::DRIFT_ALLOWANCE;

        // Two separate things, because they are two separate claims. It gets *onto* the
        // station: somewhere in here it is inside the deadband. And it then *stays in company*:
        // the deadband is where a craft decides to correct rather than where it sits, so it
        // wanders a little past before the next correction pulls it back, and the bound on that
        // is the one worth pinning — a five-kilometer hull at this range is sixty pixels of
        // ship rather than a mark.
        let (mut nearest, mut furthest) = (f64::INFINITY, 0.0f64);
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            nearest = nearest.min(apart(&server));
            furthest = furthest.max(apart(&server));
        }
        assert!(
            nearest < deadband,
            "it never reached the station: {nearest:.0} m"
        );
        assert!(
            (nearest - standoff).abs() < standoff * 0.02
                && (furthest - standoff).abs() < standoff * 0.02,
            "it did not stay on station: {nearest:.0} to {furthest:.0} m off {standoff:.0} m",
        );
        assert!(
            opening > 100.0 * deadband,
            "premise: it had a long way to come"
        );
    }

    /// **Hanging out within sight of the other hull.** Once alongside, closing in brings the
    /// small ship to a kilometer of clear space from the large one and holds it there, inside
    /// a quarter of a kilometer, for days of orbits round Jupiter.
    #[tokio::test]
    async fn closing_in_holds_a_kilometer_off_the_hull() {
        let Some((mut server, mut wire, client, pov)) = staged(&lc_world::scenario::CLOSING) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let apart = |s: &Server<Memory>| {
            let (a, b) = (s.fleet.get(pov).unwrap(), s.fleet.get(cast).unwrap());
            a.motion.position_ly.distance(b.motion.position_ly) * lc_world::system::M_PER_LY
        };
        for _ in 0..1500 {
            server.tick(&mut wire).await.unwrap();
        }
        wire.client_says(
            client,
            lc_proto::Inbound::Act(lc_proto::Intent {
                ship_id: ShipId(pov.0),
                order: lc_proto::Order::Intercept {
                    ship_id: ShipId(cast.0),
                    closeness: lc_proto::Closeness::Intimate,
                },
                issued_at_client_t: server.now_t(),
            }),
        );
        let (mine, theirs) = (
            server.fleet.get(pov).unwrap().length_m,
            server.fleet.get(cast).unwrap().length_m,
        );
        let standoff = lc_world::pursuit::Closeness::Intimate.standoff_m(mine, theirs);
        assert_eq!(
            standoff - 0.5 * (mine + theirs),
            1_000.0,
            "premise: a kilometer between hulls"
        );
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
        }
        let (mut nearest, mut furthest) = (f64::INFINITY, 0.0f64);
        for _ in 0..4000 {
            server.tick(&mut wire).await.unwrap();
            nearest = nearest.min(apart(&server));
            furthest = furthest.max(apart(&server));
        }
        let slack = lc_world::pursuit::INTIMATE_SLACK_M;
        assert!(
            nearest > standoff - slack && furthest < standoff + slack,
            "held {nearest:.0} to {furthest:.0} m, wanted {standoff:.0} ± {slack} m",
        );
        assert!(
            server.pursuits.contains_key(&pov),
            "the pursuit was given up"
        );
    }

    /// **The plume that flickered.** A pursuer chasing a quarry under thrust was handed a fresh
    /// "close the gap and stop" every tick, each short enough to be flown whole — turn, burn,
    /// flip, brake — so the drive reversed about once a tick while the ship plainly left the
    /// system. Measured, it reversed eighteen thousand times across a chase and never closed
    /// from two million kilometers. An escort matches the quarry's acceleration instead.
    ///
    /// Sampled *inside* each tick, because that is what a client draws: the plan is replayed
    /// between sightings, and sampling only at the tick saw each fresh plan's first instant.
    #[tokio::test]
    async fn chasing_a_burning_quarry_does_not_flicker() {
        use lc_world::motion::Motive;
        let Some((mut server, mut wire, _, pov)) = staged(&lc_world::scenario::CHASE) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let tick_s = 50.0 * 8_766.0 * 1.0e-3 * server.rate();
        let (mut reversals, mut last) = (0, 0i32);
        for n in 0..240 {
            server.tick(&mut wire).await.unwrap();
            let now_s = server.now_t() as f64 * 1.0e-6;
            let chaser = server.fleet.get(pov).unwrap();
            // Whatever the chaser is flying, asked the same question, so this pins the plume
            // rather than which motive happens to draw it.
            let sample = |t: f64| match &chaser.motion.motive {
                Motive::Escort(plan) => Some((plan.thrust_at(t), plan.state_at(t).1)),
                Motive::Rendezvous(plan) => Some((plan.thrust_at(t), plan.state_at(t).1)),
                _ => None,
            };
            if n < 8 {
                continue;
            }
            for k in 0..8 {
                let t = now_s + tick_s * k as f64 / 8.0;
                let Some((thrust, beta)) = sample(t) else {
                    continue;
                };
                let sign = match thrust.dot(beta) {
                    _ if thrust == DVec3::ZERO => 0,
                    along if along >= 0.0 => 1,
                    _ => -1,
                };
                if sign != 0 && last != 0 && sign != last {
                    reversals += 1;
                }
                if sign != 0 {
                    last = sign;
                }
            }
        }
        // Before the quarry turns round to brake there is nothing to reverse for.
        assert_eq!(
            reversals, 0,
            "the drive reversed {reversals} times while both were burning outward"
        );

        // And it caught up and stayed, rather than holding two million kilometers off.
        let standoff = lc_world::pursuit::standoff_m(500.0, 500.0);
        let (chaser, quarry) = (
            server.fleet.get(pov).unwrap(),
            server.fleet.get(cast).unwrap(),
        );
        let gap = chaser
            .motion
            .position_ly
            .distance(quarry.motion.position_ly)
            * lc_world::system::M_PER_LY;
        assert!(
            gap < 2.0 * standoff,
            "it is {gap:.0} m off a {standoff:.0} m standoff"
        );
    }

    /// **Hanging about is following.** Alongside a quarry with the same five-g drive, the
    /// quarry flies off to Io. The pursuer used to refuse a quarry pulling as hard as it could,
    /// give the pursuit up and fall ballistic behind. It follows the burn instead, and is back
    /// on station once the quarry settles into its new orbit.
    #[tokio::test]
    async fn a_quarry_that_leaves_is_followed_to_where_it_goes() {
        let Some((mut server, mut wire, _, pov)) = staged(&lc_world::scenario::CLOSING) else {
            return;
        };
        let cast = CraftId(lc_world::scenario::BASE_ID);
        let apart = |s: &Server<Memory>| {
            let (a, b) = (s.fleet.get(pov).unwrap(), s.fleet.get(cast).unwrap());
            a.motion.position_ly.distance(b.motion.position_ly) * lc_world::system::M_PER_LY
        };
        for _ in 0..2000 {
            server.tick(&mut wire).await.unwrap();
        }
        let (mine, theirs) = (
            server.fleet.get(pov).unwrap().length_m,
            server.fleet.get(cast).unwrap().length_m,
        );
        let standoff = lc_world::pursuit::Closeness::Company.standoff_m(mine, theirs);
        assert!(
            (apart(&server) - standoff).abs() < 0.05 * standoff,
            "premise: on station"
        );

        let now_s = server.now_t() as f64 * 1.0e-6;
        let quarry = server.fleet.get_mut(cast).unwrap();
        let drive = quarry.turning(quarry.motion.drive);
        assert_eq!(
            drive.accel_g,
            server.fleet.get(pov).unwrap().motion.drive.accel_g,
            "premise: evenly matched"
        );
        let leave = motion::Event {
            ship: motion::ShipId(cast.0),
            at_t: now_s,
            change: Change::SetCourse {
                course: Course::parse("orbit:Io:low").unwrap(),
                drive,
            },
        };
        server.fleet.get_mut(cast).unwrap().apply(&leave).unwrap();

        let mut arrived = None;
        for n in 0..1000 {
            server.tick(&mut wire).await.unwrap();
            assert!(
                server.pursuits.contains_key(&pov),
                "the pursuit was given up {n} ticks in"
            );
            if arrived.is_none()
                && matches!(
                    server.fleet.get(cast).unwrap().motion.motive,
                    Motive::Holding(_)
                )
            {
                arrived = Some(n);
            }
        }
        assert!(arrived.is_some(), "premise: the quarry got to Io");
        let gap = apart(&server);
        assert!(
            (gap - standoff).abs() < 0.05 * standoff,
            "{gap:.0} m off a {standoff:.0} m station at Io"
        );
    }
}
