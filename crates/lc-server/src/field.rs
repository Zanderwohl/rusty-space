//! Collapse: a field that reaches `Q_max` destroys its ship. See `lightcone/docs/30-the-field.md`
//! §Collapse.
//!
//! Nothing is scheduled ahead or kept for it. The instant is a closed form of the account, which
//! every change of input settles first and a checkpoint keeps, so solving it again each tick is the
//! schedule, and a restart restores it with the account. What goes out is an ordinary event, and
//! observers learn of it at their own light delay; the wreck stays in the fleet with its worldline
//! ended, so its light already in flight goes on arriving until the last of it has passed.

use lc_proto::{Outbound, ShipId};
use lc_world::craft::{Craft, CraftId};

use crate::journal::Journal;
use crate::server::{KIND_COLLAPSE, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// 30 takes the spike as a second long.
const SPIKE_S: f64 = 1.0;

/// When `craft`'s field reaches `Q_max` by `until_s`, if it does, walking the account through the
/// day-long starlight segments it will be settled at.
pub fn collapse_by(craft: &Craft, until_s: f64) -> Option<f64> {
    let mut ahead: Option<Craft> = None;
    loop {
        let fitting = ahead.as_ref().unwrap_or(craft).fitting()?;
        let since_s = fitting.since_s();
        let segment_end_s = lc_world::solar::segment_end(since_s);
        if let Some(at_s) = fitting.collapse_s().filter(|&t| t <= segment_end_s.min(until_s)) {
            return Some(at_s);
        }
        if segment_end_s >= until_s {
            return None;
        }
        let next = ahead.get_or_insert_with(|| craft.clone());
        next.settle(segment_end_s);
        if next.fitting().is_none_or(|f| f.since_s() <= since_s) {
            return None;
        }
    }
}

impl<J: Journal> Server<J> {
    /// Destroy every craft whose field has reached `Q_max` by now, at the instant it did.
    pub(crate) fn collapse_fields(
        &mut self,
        after_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let due: Vec<(CraftId, f64)> =
            self.fleet.iter().filter_map(|craft| Some((craft.id, collapse_by(craft, now_s)?))).collect();
        for (id, at_s) in due {
            // Up, so the field is at its limit by the stamp, and never into the tick before, where
            // cursors already stand: only an input changed at a past instant asks for that.
            let at_t = ((at_s * 1.0e6).ceil() as i64).clamp(after_t + 1, self.now_t);
            self.collapse(id, at_t, wire, events, deliveries);
        }
    }

    fn collapse(
        &mut self,
        id: CraftId,
        at_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let at_s = at_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get_mut(id) else { return };
        craft.settle(at_s);
        let Some(fitting) = craft.fitting() else { return };
        let released_j = fitting.field().released_j(fitting.stored_j_at(&craft.motion, at_s));
        let spike_w = fitting.balance().collapse_spike_fraction * released_j / SPIKE_S;
        let name = craft.name.clone();
        craft.end(at_s);
        let payload = serde_json::to_string(&lc_proto::Released { released_j }).unwrap_or_else(|_| "{}".into());
        self.emit(id, KIND_COLLAPSE, spike_w, payload, at_t, events, deliveries);

        self.pursuits.remove(&id);
        self.refitting.remove(&id);
        self.reserved.remove(&id);
        self.instruments.aboard.remove(&id);
        self.auto_ack.remove(&id);
        self.owed.remove(&id);
        self.answered.remove(&id);
        self.destroyed.push(id.0);

        let account = self.by_account.iter().find(|(_, ship)| ship.0 == id.0).map(|(account, _)| account.clone());
        let owner = self.owners.remove(&id);
        if account.is_none() && owner.is_none() {
            return;
        }
        let successor = self.spawn(name.clone());
        if let Some(account) = account {
            self.by_account.insert(account, successor);
        }
        let Some(from) = owner else { return };
        let Some(state) = self.clients.get_mut(&from) else { return };
        let account = state.account.clone();
        *state = crate::server::Connected::new(successor, account.clone(), state.permission);
        self.owners.insert(CraftId(successor.0), from);
        wire.send(from, Outbound::Collapsed { ship_id: ShipId(id.0), at_t, released_j, successor });
        let name = name.unwrap_or_else(|| self.ship(successor).map(Craft::designation).unwrap_or_default());
        self.welcome(from, successor, name, &account, wire);
    }

    /// Drop each wreck once the light of its end has passed every craft there is.
    pub(crate) fn sweep_wrecks(&mut self) {
        let now_t = self.now_t as f64;
        let ended: Vec<(CraftId, f64)> =
            self.fleet.iter().filter_map(|craft| Some((craft.id, craft.ended_s()? * 1.0e6))).collect();
        for (id, end_t) in ended {
            let Some(at) = self.fleet.get(id).map(|wreck| wreck.position_at(end_t)) else { continue };
            let passed = self.fleet.iter().filter(|craft| craft.ended_s().is_none()).all(|craft| {
                lc_spacetime::arrival_time_at(end_t, at, &craft.worldline()).is_none_or(|t| t <= now_t)
            });
            if passed {
                self.fleet.remove(id);
            }
        }
    }

    /// Craft destroyed since this was last called, whose saved rows are to be deleted.
    pub fn take_destroyed(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.destroyed)
    }

    /// Hand back what [`Server::take_destroyed`] took, because deleting them failed.
    pub fn untake_destroyed(&mut self, ids: Vec<i64>) {
        self.destroyed.extend(ids);
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, Order, Refusal, Sighting};
    use lc_world::fitting::{Account, Balance, Fitting, RATED_LOAD_AU};
    use lc_world::form::{Form, PartId};
    use lc_world::motion::LIGHT_US_PER_LY;

    use super::*;
    use crate::journal::Memory;
    use crate::server::course_tests::{a_star, a_system};
    use crate::transport::Loopback;
    use crate::world::{World, still};

    const OWNER: ClientId = ClientId(1);
    const WATCHER: ClientId = ClientId(2);
    const DYING: ShipId = ShipId(1);
    const WATCHING: ShipId = ShipId(2);
    const AU_US: f64 = lc_world::system::UNIT_M / 299.792_458;
    /// Between the dying ship and the watcher: three light-days, a dozen ticks at sixty times.
    const APART_US: f64 = 3.0 * 86_400.0e6;
    const DAY_S: f64 = 86_400.0;

    /// A full starting ship close enough to the sample star to collapse in a couple of weeks, and
    /// a second one three light-days off. Sixty times the design rate, a few ticks a day.
    fn scene() -> Option<(Server<Memory>, Loopback)> {
        let star = a_star()?;
        let sun_w = lc_world::solar::SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * lc_world::system::UNIT_M.powi(2);
        let rated_au = RATED_LOAD_AU * (a_system()?.star_luminosity_w() / sun_w).sqrt();
        let near = star.position_ly * LIGHT_US_PER_LY + DVec3::X * 0.6 * rated_au * AU_US;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(vec![star]));
        server.set_rate(60.0);
        let mut dying = still(DYING, near);
        server.fit_new(&mut dying);
        server.admit(OWNER, dying, 0.0);
        server.admit(WATCHER, still(WATCHING, near + DVec3::Y * APART_US), 0.0);
        server.next_ship = 3;
        Some((server, Loopback::new()))
    }

    fn forecast(server: &Server<Memory>) -> Option<f64> {
        collapse_by(server.ship(DYING)?, server.now_t() as f64 * 1.0e-6 + 90.0 * DAY_S)
    }

    fn to_us(s: f64) -> i64 {
        (s * 1.0e6).ceil() as i64
    }

    fn act(ship: ShipId, order: Order) -> Inbound {
        Inbound::Act(Intent { ship_id: ship, order, issued_at_client_t: i64::MAX })
    }

    /// The ship as it was on the last tick before its collapse, and what its owner was told.
    struct Collapsed {
        before: Craft,
        at_t: i64,
        released_j: f64,
        successor: ShipId,
        said: Vec<Outbound>,
    }

    async fn until_collapse(server: &mut Server<Memory>, wire: &mut Loopback) -> Collapsed {
        for _ in 0..500 {
            let before = server.ship(DYING).unwrap().clone();
            server.tick(wire).await.unwrap();
            let said = wire.take(OWNER);
            let told = said.iter().find_map(|m| match m {
                Outbound::Collapsed { ship_id, at_t, released_j, successor } if *ship_id == DYING => {
                    Some((*at_t, *released_j, *successor))
                }
                _ => None,
            });
            if let Some((at_t, released_j, successor)) = told {
                return Collapsed { before, at_t, released_j, successor, said };
            }
        }
        panic!("no collapse");
    }

    fn collapse_events(server: &Server<Memory>) -> Vec<&Event> {
        server.journal().events.iter().filter(|e| e.kind == KIND_COLLAPSE).collect()
    }

    /// At the instant the account predicted, as an event where the ship was, releasing `Q_max` and
    /// everything stored, the energy committed to a plan among it.
    #[tokio::test]
    async fn a_collapse_fires_when_predicted_and_releases_everything_stored() {
        let Some((mut server, mut wire)) = scene() else { return };
        let craft = server.fleet.get_mut(CraftId(DYING.0)).unwrap();
        let fitting = craft.fitting().unwrap().clone();
        let committed_j = 5.0 * fitting.balance().module_energy_j();
        let account = Account { committed_j, ..fitting.account() };
        craft.fit(Some(Fitting::from_account(&account, *fitting.balance())));
        server.tick(&mut wire).await.unwrap();
        let predicted_s = forecast(&server).expect("close in, it collapses");
        assert!(predicted_s > server.now_t() as f64 * 1.0e-6 + 3.0 * DAY_S, "premise: days away, across starlight segments");

        let collapsed = until_collapse(&mut server, &mut wire).await;
        assert_eq!(collapsed.at_t, to_us(predicted_s));
        let [event] = collapse_events(&server)[..] else { panic!("{:?}", collapse_events(&server)) };
        assert_eq!((event.t, event.source), (collapsed.at_t, DYING));
        assert!(event.at.distance(collapsed.before.position_at(collapsed.at_t as f64)) < 1.0);

        let at_s = collapsed.at_t as f64 * 1.0e-6;
        let mut then = collapsed.before;
        then.settle(at_s);
        let fitting = then.fitting().unwrap();
        let heat_max_j = fitting.field().heat_max_j();
        assert!((fitting.heat_j_at(at_s) - heat_max_j).abs() < 1.0e-6 * heat_max_j, "not at the limit");
        let stored_j = fitting.stored_j_at(&then.motion, at_s);
        assert!(stored_j > committed_j, "premise: storage holds the commitment");
        assert!((collapsed.released_j - (heat_max_j + stored_j)).abs() < 1.0e-12 * collapsed.released_j);
        let released: lc_proto::Released = serde_json::from_str(&event.payload).unwrap();
        assert_eq!(released.released_j, collapsed.released_j);
    }

    /// Moving away before it is due moves it later, and it fires at the new instant.
    #[tokio::test]
    async fn a_change_of_input_moves_the_instant() {
        let Some((mut server, mut wire)) = scene() else { return };
        server.tick(&mut wire).await.unwrap();
        let first_s = forecast(&server).unwrap();
        let craft = server.ship(DYING).unwrap();
        let out_us = (craft.motion.position_ly - craft.system.as_ref().unwrap().star_position_ly()).length() * LIGHT_US_PER_LY;
        // A tenth of the way out again over the ten days or so it has.
        let beta = 0.1 * out_us / (10.0 * DAY_S * 1.0e6);
        wire.client_says(OWNER, act(DYING, Order::Burn { beta: [beta, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let moved_s = forecast(&server).expect("still close enough to collapse");
        assert!(moved_s > first_s + DAY_S / 4.0, "{first_s} {moved_s}");

        let collapsed = until_collapse(&mut server, &mut wire).await;
        assert_eq!(collapsed.at_t, to_us(moved_s));
    }

    /// A round whose vent is what crosses `Q_max` fires at the end of the step that vents.
    #[tokio::test]
    async fn a_vent_that_crosses_q_max_fires_at_its_steps_end() {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(60.0);
        let mut craft = still(DYING, DVec3::ZERO);
        server.fit_new(&mut craft);
        craft.drain(server.balance().module_energy_j(), 0.0);
        server.admit(OWNER, craft, 0.0);
        server.next_ship = 2;
        let mut wire = Loopback::new();
        let mut shrunk = Form::starting();
        shrunk.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 3.0 / 5.0;
        wire.client_says(OWNER, act(DYING, Order::Refit { target: (&shrunk).into() }));
        server.tick(&mut wire).await.unwrap();

        let fitting = server.ship(DYING).unwrap().fitting().unwrap().clone();
        let plan = fitting.refit().expect("the round is under way").clone();
        let [step] = plan.steps() else { panic!("{:?}", plan.steps()) };
        assert!(step.vented_j > 0.0, "premise: it vents");
        let end_s = plan.round().start_s + step.ends_s();
        let heat_max_j = fitting.heat_j_at(end_s) - 0.5 * step.vented_j;
        assert!(fitting.heat_j_at(end_s - 1.0e-3) < heat_max_j, "premise: only the vent crosses");
        let area_m2 = fitting.field().area_m2;
        server.set_balance(Balance { field_capacity: heat_max_j / area_m2, ..Balance::DEFAULT });

        let collapsed = until_collapse(&mut server, &mut wire).await;
        assert_eq!(collapsed.at_t, to_us(end_s));
    }

    /// Its light reaches a ship three light-days off three days later, and not before; until then
    /// that ship goes on seeing it, and afterwards not.
    #[tokio::test]
    async fn a_distant_ship_learns_of_a_collapse_when_its_light_arrives() {
        let Some((mut server, mut wire)) = scene() else { return };
        let mut seen_until_t = i64::MIN;
        let mut heard: Vec<(i64, Sighting)> = Vec::new();
        let mut at_t = None;
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            if at_t.is_none() {
                at_t = collapse_events(&server).first().map(|e| e.t);
            }
            for message in wire.take(WATCHER) {
                match message {
                    Outbound::Present(list) if list.iter().any(|p| p.get().ship_id == DYING) => {
                        seen_until_t = server.now_t();
                    }
                    Outbound::Sightings(list) => heard.extend(
                        list.into_iter().map(|s| s.get().clone()).filter(|s| s.kind == KIND_COLLAPSE).map(|s| (server.now_t(), s)),
                    ),
                    _ => {}
                }
            }
            let _ = wire.take(OWNER);
            if at_t.is_some_and(|at_t| server.now_t() as f64 > at_t as f64 + APART_US + 2.0 * DAY_S * 1.0e6) {
                break;
            }
        }
        let at_t = at_t.expect("it collapsed");
        let arrives_t = at_t + APART_US as i64;
        let [(told_t, sighting)] = &heard[..] else { panic!("{heard:?}") };
        assert_eq!((sighting.source_id, sighting.emitted_t), (DYING.0, at_t));
        assert_eq!(sighting.arrive_t, arrives_t);
        assert!(*told_t >= arrives_t);
        assert!(seen_until_t > at_t, "the ship vanished from sight when it collapsed, not when its light arrived");
        assert!(seen_until_t < arrives_t + 60 * crate::server::TICK_US, "still seen after its light had passed");
        assert!(server.ship(DYING).is_none(), "the wreck outlived its light");
    }

    /// Its owner flies a new starting ship at the spawn point, knowing nothing, and the wreck is
    /// no longer theirs to order about or saved.
    #[tokio::test]
    async fn the_owner_is_given_a_new_starting_ship() {
        let Some((mut server, mut wire)) = scene() else { return };
        let collapsed = until_collapse(&mut server, &mut wire).await;
        let successor = collapsed.successor;
        assert_eq!(successor, ShipId(3));
        let welcome = collapsed.said.iter().position(|m| matches!(m, Outbound::Welcome { ship_id, .. } if *ship_id == successor));
        let told = collapsed.said.iter().position(|m| matches!(m, Outbound::Collapsed { .. }));
        assert!(told < welcome && welcome.is_some(), "{:?}", collapsed.said);

        let craft = server.ship(successor).unwrap();
        assert!(craft.motion.position_ly.distance(server.world.start().unwrap()) < 1.0e-9);
        let fitting = craft.fitting().unwrap();
        assert_eq!(fitting.form(), &Form::starting());
        assert_eq!(fitting.stored_j_at(&craft.motion, server.now_t() as f64 * 1.0e-6), fitting.hull().capacities.storage_j);
        assert!(server.instruments.aboard.get(&CraftId(successor.0)).is_none_or(|a| a.knowledge.is_empty()));
        assert_eq!(server.ship(DYING).unwrap().ended_s(), Some(collapsed.at_t as f64 * 1.0e-6));

        wire.client_says(OWNER, act(DYING, Order::Burn { beta: [1.0e-6, 0.0, 0.0] }));
        wire.client_says(OWNER, act(successor, Order::Burn { beta: [1.0e-6, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(OWNER);
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { ship_id, reason: Refusal::NotYours } if *ship_id == DYING)), "{said:?}");
        assert!(said.iter().any(|m| matches!(m, Outbound::Accepted { ship_id, .. } if *ship_id == successor)), "{said:?}");

        let saved: Vec<i64> = server.checkpoint().ships.iter().map(|s| s.ship_id).collect();
        assert!(!saved.contains(&DYING.0) && saved.contains(&successor.0), "{saved:?}");
        assert_eq!(server.take_destroyed(), vec![DYING.0]);
    }

    /// Nothing but the account comes back from a checkpoint, and it collapses on time from it.
    #[tokio::test]
    async fn a_collapse_survives_a_restart() {
        let Some((mut server, mut wire)) = scene() else { return };
        server.tick(&mut wire).await.unwrap();
        server.tick(&mut wire).await.unwrap();
        let predicted_s = forecast(&server).unwrap();
        let checkpoint = server.checkpoint();
        drop(server);

        let mut restarted = Server::new(Memory::default(), 0, 2);
        restarted.load_world(World::new(vec![a_star().unwrap()]));
        restarted.set_rate(60.0);
        assert!(restarted.adopt(checkpoint).is_empty());
        let mut wire = Loopback::new();
        for _ in 0..500 {
            restarted.tick(&mut wire).await.unwrap();
            if !collapse_events(&restarted).is_empty() {
                break;
            }
        }
        let [event] = collapse_events(&restarted)[..] else { panic!("no collapse after the restart") };
        assert_eq!((event.t, event.source), (to_us(predicted_s), DYING));
    }
}
