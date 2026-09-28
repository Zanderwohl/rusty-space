//! Collapse, and what a field does to its neighbors. See `lightcone/docs/30-the-field.md`
//! §Collapse and §Proximity.
//!
//! Nothing is scheduled ahead or kept for a collapse. The instant is a closed form of the account,
//! which every change of input settles first and a checkpoint keeps, so solving it again each tick
//! is the schedule, and a restart restores it with the account. What goes out is an ordinary event,
//! and observers learn of it at their own light delay; the wreck stays in the fleet with its
//! worldline ended, so its light already in flight goes on arriving until the last of it has passed.
//!
//! The spike rides the event's own fan-out: each delivery to a fitted craft is queued as a burst
//! and lands at that delivery's arrival. Landing is a change of input like any other, so the
//! receiver's collapse re-solves from it, and a cascade is nothing but collapses and landings taken
//! in time order.

use glam::DVec3;
use lc_proto::{Outbound, ShipId};
use lc_spacetime::LIGHT_MICROSECOND_M;
use lc_world::craft::{Craft, CraftId};
use lc_world::field::{Burst, CONTACT_FRACTION, received_fraction};

use crate::journal::Journal;
use crate::server::{KIND_COLLAPSE, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// 30 takes the spike as a second long.
const SPIKE_S: f64 = 1.0;

/// Of a receiver's rated load, glow below which a neighbor is not solved for: over a time constant
/// it moves `Q` by this fraction of `Q_max`.
const GLOW_FLOOR: f64 = 1.0e-9;

/// Energy let go of at once, on its way to one craft's field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arrival {
    observer: CraftId,
    /// Coordinate microseconds, from the fan-out's delivery.
    arrive_t: i64,
    /// Where it was let go of, light-microseconds.
    from: DVec3,
    /// All of it, isotropic.
    energy_j: f64,
}

/// When `craft`'s field reaches `Q_max` by `until_s`, if it does, walking the account through the
/// day-long starlight segments it will be settled at.
pub fn collapse_by(craft: &Craft, until_s: f64) -> Option<f64> {
    let mut ahead: Option<Craft> = None;
    loop {
        let fitting = ahead.as_ref().unwrap_or(craft).fitting()?;
        let since_s = fitting.since_s();
        let segment_end_s = lc_world::solar::segment_end(since_s);
        if let Some(at_s) = fitting.collapse_s(&ahead.as_ref().unwrap_or(craft).motion, segment_end_s.min(until_s)) {
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

/// `craft`'s shadow toward a source along `to_source`, m², in the attitude it holds at `t`: off the
/// shadow table at the roll the hull presents to its star, the only roll it holds. Zero unfitted.
pub fn shadow_toward_m2(craft: &Craft, to_source: DVec3, t: f64) -> f64 {
    let (Some(fitting), Some(nose)) = (craft.fitting(), craft.facing_at(t)) else { return 0.0 };
    lc_world::solar::shadow_m2(fitting.geometry(), nose.dot(to_source.normalize_or_zero()))
}

/// Of `energy_j` let go of at `from`, what arrives at `craft`'s field meeting it at `arrive_t`: onto
/// its shadow toward `from`, from where it is then.
pub fn received_j(craft: &Craft, from: DVec3, energy_j: f64, arrive_t: i64) -> f64 {
    let offset = craft.position_at(arrive_t as f64) - from;
    let shadow_m2 = shadow_toward_m2(craft, -offset, arrive_t as f64 * 1.0e-6);
    energy_j * received_fraction(shadow_m2, offset.length() * LIGHT_MICROSECOND_M)
}

/// A fitted craft as a source of glow at one instant, read once for every receiver.
struct Glowing<'a> {
    craft: &'a Craft,
    /// Light-microseconds.
    at: DVec3,
    /// `Q/τ` of its field, W.
    emitted_w: f64,
}

/// `receiver`, which is `here` at `at_t`, and each source's glow reaching it: the watts `Q/τ` of
/// its field puts out as its light left, and the fraction of them the receiver takes.
fn glow_on(receiver: &Craft, here: DVec3, sources: &[Glowing], at_t: i64) -> Vec<(f64, f64)> {
    let Some(to) = receiver.fitting() else { return Vec::new() };
    let at_s = at_t as f64 * 1.0e-6;
    let floor_w = GLOW_FLOOR * to.field().rated_load_w();
    let bound_m2 = to.field().area_m2;
    sources
        .iter()
        .filter(|source| source.craft.id != receiver.id)
        .filter(|source| source.emitted_w * received_fraction(bound_m2, source.at.distance(here) * LIGHT_MICROSECOND_M) >= floor_w)
        .filter_map(|source| {
            let from = source.craft.fitting()?;
            let left_t = lc_spacetime::retarded_times_at(at_t as f64, here, &source.craft.worldline()).last().copied()?;
            let offset = here - source.craft.position_at(left_t);
            let emitted_w = from.heat_j_at(&source.craft.motion, left_t * 1.0e-6) / from.field().tau_s;
            let shadow_m2 = shadow_toward_m2(receiver, -offset, at_s);
            Some((emitted_w, received_fraction(shadow_m2, offset.length() * LIGHT_MICROSECOND_M)))
        })
        .collect()
}

impl<J: Journal> Server<J> {
    /// Restate every field's glow from its neighbors, from `at_t`. Every account is settled no
    /// further than that.
    pub(crate) fn shine(&mut self, at_t: i64) {
        let at_s = at_t as f64 * 1.0e-6;
        let sources: Vec<Glowing> = self
            .fleet
            .iter()
            .filter_map(|craft| {
                let fitting = craft.fitting()?;
                let emitted_w = fitting.heat_j_at(&craft.motion, at_s) / fitting.field().tau_s;
                Some(Glowing { craft, at: craft.position_at(at_t as f64), emitted_w })
            })
            .collect();
        let changed: Vec<(CraftId, f64)> = sources
            .iter()
            .filter_map(|receiver| {
                let glow = glow_on(receiver.craft, receiver.at, &sources, at_t);
                // A receiver surrounded takes no more than one in contact: summed per source, ships
                // stacked at the spawn point would heat each other without bound.
                let taken: f64 = glow.iter().map(|(_, fraction)| fraction).sum();
                let scale = if taken > CONTACT_FRACTION { CONTACT_FRACTION / taken } else { 1.0 };
                let watts: f64 = glow.iter().map(|(emitted_w, fraction)| emitted_w * fraction * scale).sum();
                (watts != receiver.craft.fitting()?.lit_w()).then_some((receiver.craft.id, watts))
            })
            .collect();
        for (id, watts) in changed {
            if let Some(craft) = self.fleet.get_mut(id) {
                craft.adjust(at_t as f64 * 1.0e-6, |fitting| fitting.set_lit_w(watts));
            }
        }
    }

    /// Destroy every craft whose field has reached `Q_max` by now, at the instant it did, landing
    /// every spike due by now on the way, in time order.
    pub(crate) fn collapse_fields(
        &mut self,
        after_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let mut due: Vec<(CraftId, i64)> =
            self.fleet.iter().filter_map(|craft| Some((craft.id, self.stamp(collapse_by(craft, now_s)?, after_t)))).collect();
        loop {
            let collapsing = due.iter().enumerate().min_by_key(|(_, (_, at_t))| *at_t).map(|(k, &(id, at_t))| (k, id, at_t));
            let landing = self
                .arrivals
                .iter()
                .enumerate()
                .filter(|(_, a)| a.arrive_t <= self.now_t)
                .min_by_key(|(_, a)| a.arrive_t)
                .map(|(k, a)| (k, a.arrive_t));
            match (collapsing, landing) {
                (_, Some((k, arrive_t))) if collapsing.is_none_or(|(_, _, at_t)| arrive_t <= at_t) => {
                    let arrival = self.arrivals.swap_remove(k);
                    let id = arrival.observer;
                    due.retain(|(craft, _)| *craft != id);
                    if !self.land(arrival, wire, events, deliveries) {
                        let next = self.fleet.get(id).and_then(|craft| collapse_by(craft, now_s));
                        due.extend(next.map(|at_s| (id, self.stamp(at_s, after_t))));
                    }
                }
                (Some((k, id, at_t)), _) => {
                    due.swap_remove(k);
                    self.collapse(id, at_t, wire, events, deliveries);
                }
                (None, _) => break,
            }
        }
    }

    /// Up, so the field is at its limit by the stamp, and never into the tick before, where cursors
    /// already stand: only an input changed at a past instant asks for that.
    fn stamp(&self, at_s: f64, after_t: i64) -> i64 {
        ((at_s * 1.0e6).ceil() as i64).max(after_t + 1).min(self.now_t)
    }

    /// Land a spike on its field, collapsing it there if that takes it to `Q_max`. Whether it did.
    fn land(&mut self, arrival: Arrival, wire: &mut impl Transport, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) -> bool {
        let at_s = arrival.arrive_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get_mut(arrival.observer) else { return false };
        let arriving_j = received_j(craft, arrival.from, arrival.energy_j, arrival.arrive_t);
        craft.adjust(at_s, |fitting| fitting.take_burst(Burst::Arriving(arriving_j)));
        let Some(fitting) = craft.fitting() else { return false };
        if fitting.heat_j_at(&craft.motion, at_s) < fitting.field().heat_max_j() {
            self.tell_fitted(wire, arrival.observer);
            return false;
        }
        self.collapse(arrival.observer, arrival.arrive_t, wire, events, deliveries);
        true
    }

    /// Of `energy_j` let go of at once by `event`'s source, queue what each fitted craft its light
    /// reaches will take, as a burst at the arrival the fan-out scheduled for it. The isotropic case
    /// of an emission.
    fn irradiate(&mut self, event_id: i64, from: DVec3, energy_j: f64, deliveries: &[Scheduled]) {
        let reached = deliveries.iter().filter(|d| d.event == event_id).filter(|d| {
            self.fleet.get(CraftId(d.observer.0)).is_some_and(|craft| craft.fitting().is_some())
        });
        let arrivals: Vec<Arrival> = reached
            .map(|d| Arrival { observer: CraftId(d.observer.0), arrive_t: d.arrive_t, from, energy_j })
            .collect();
        self.arrivals.extend(arrivals);
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
        let spike_j = fitting.balance().collapse_spike_fraction * released_j;
        let name = craft.name.clone();
        let from = craft.position_at(at_t as f64);
        craft.end(at_s);
        let payload = serde_json::to_string(&lc_proto::Released { released_j }).unwrap_or_else(|_| "{}".into());
        if let Some(event_id) = self.emit(id, KIND_COLLAPSE, spike_j / SPIKE_S, payload, at_t, events, deliveries) {
            self.irradiate(event_id, from, spike_j, deliveries);
        }

        self.pursuits.remove(&id);
        self.refitting.remove(&id);
        self.reserved.remove(&id);
        self.instruments.aboard.remove(&id);
        self.auto_ack.remove(&id);
        self.owed.remove(&id);
        self.answered.remove(&id);

        let account = self.by_account.iter().find(|(_, ship)| ship.0 == id.0).map(|(account, _)| account.clone());
        // Connected, or nobody could ever reach a ship given to it.
        let owner = self.owners.remove(&id).filter(|from| self.clients.get(from).is_some_and(|state| state.ship.0 == id.0));
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
                self.destroyed.push(id.0);
            }
        }
    }

    /// Wrecks swept since this was last called, whose saved rows are to be deleted.
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
        let near = near_the_star()?;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(vec![a_star()?]));
        server.set_rate(60.0);
        let mut dying = still(DYING, near);
        server.fit_new(&mut dying);
        server.admit(OWNER, dying, 0.0);
        server.admit(WATCHER, still(WATCHING, near + DVec3::Y * APART_US), 0.0);
        server.next_ship = 3;
        Some((server, Loopback::new()))
    }

    /// Light-microseconds, well inside the sample star's rated load.
    fn near_the_star() -> Option<DVec3> {
        let star = a_star()?;
        let sun_w = lc_world::solar::SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * lc_world::system::UNIT_M.powi(2);
        let rated_au = RATED_LOAD_AU * (a_system()?.star_luminosity_w() / sun_w).sqrt();
        Some(star.position_ly * LIGHT_US_PER_LY + DVec3::X * 0.6 * rated_au * AU_US)
    }

    fn forecast(server: &Server<Memory>) -> Option<f64> {
        collapse_by(server.ship(DYING)?, server.now_t() as f64 * 1.0e-6 + 90.0 * DAY_S)
    }

    /// How far the dying ship is from its star, light-microseconds.
    fn out_us(server: &Server<Memory>) -> f64 {
        let craft = server.ship(DYING).unwrap();
        (craft.motion.position_ly - craft.system.as_ref().unwrap().star_position_ly()).length() * LIGHT_US_PER_LY
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
        assert!((fitting.heat_j_at(&then.motion, at_s) - heat_max_j).abs() < 1.0e-6 * heat_max_j, "not at the limit");
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
        // A tenth of the way out again over the ten days or so it has.
        let beta = 0.1 * out_us(&server) / (10.0 * DAY_S * 1.0e6);
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
        let heat_max_j = fitting.heat_j_at(&server.ship(DYING).unwrap().motion, end_s) - 0.5 * step.vented_j;
        assert!(fitting.heat_j_at(&server.ship(DYING).unwrap().motion, end_s - 1.0e-3) < heat_max_j, "premise: only the vent crosses");
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
        assert!(saved.contains(&DYING.0) && saved.contains(&successor.0), "{saved:?}");
        assert!(server.take_destroyed().is_empty(), "its row went at the collapse, not the sweep");
    }

    /// An owner signed out when it happens finds the new ship on signing in again.
    #[tokio::test]
    async fn a_signed_out_owner_signs_in_to_the_successor() {
        use crate::testing::Broker;
        let Some(near) = near_the_star() else { return };
        let broker = Broker::new([1u8; 32]);
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = crate::ticket::Trusted::new("shard-1");
        assert_eq!(trusted.learn(&broker.jwks()), 1);
        server.trust(trusted);
        server.load_world(World::new(vec![a_star().unwrap()]));
        server.set_rate(60.0);
        let mut wire = Loopback::new();
        let hello = |jti: &str| Inbound::Hello { protocol: lc_proto::PROTOCOL_VERSION, ticket: broker.mint("acct-1", "shard-1", 60, jti) };
        let welcomed = |said: Vec<Outbound>| {
            said.into_iter().find_map(|m| match m {
                Outbound::Welcome { ship_id, .. } => Some(ship_id),
                _ => None,
            })
        };

        wire.client_says(OWNER, hello("j1"));
        server.tick(&mut wire).await.unwrap();
        let first = welcomed(wire.take(OWNER)).expect("welcomed");
        let mut dying = still(first, near);
        server.fit_new(&mut dying);
        server.fleet.remove(CraftId(first.0));
        server.fleet.insert(dying);
        server.disconnected(OWNER);
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            if !collapse_events(&server).is_empty() {
                break;
            }
        }
        assert_eq!(collapse_events(&server).len(), 1, "premise: it collapsed");

        let again = ClientId(3);
        wire.client_says(again, hello("j2"));
        server.tick(&mut wire).await.unwrap();
        let second = welcomed(wire.take(again)).expect("welcomed again");
        assert_ne!(second, first);
        assert!(server.ship(second).is_some_and(|craft| craft.ended_s().is_none() && craft.fitting().is_some()));
    }

    /// A ship with no account whose pilot has gone leaves nobody to give another to.
    #[tokio::test]
    async fn an_unowned_ship_with_no_account_has_no_successor() {
        let Some((mut server, mut wire)) = scene() else { return };
        server.disconnected(OWNER);
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            if !collapse_events(&server).is_empty() {
                break;
            }
        }
        assert_eq!(collapse_events(&server).len(), 1);
        assert!(server.ship(ShipId(3)).is_none());
    }

    /// Nothing but the account comes back from a checkpoint, and it collapses on time from it.
    #[tokio::test]
    async fn a_collapse_survives_a_restart() {
        let Some((mut server, mut wire)) = scene() else { return };
        server.tick(&mut wire).await.unwrap();
        // Drifting, so the starlight in a segment depends on where in it it is sampled.
        let beta = 0.1 * out_us(&server) / (10.0 * DAY_S * 1.0e6);
        wire.client_says(OWNER, act(DYING, Order::Burn { beta: [beta, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        server.tick(&mut wire).await.unwrap();
        // Settled partway through a starlight segment, as a refit or a grant leaves it: the segment
        // was sampled from where it began, not from here.
        let now_s = server.now_t() as f64 * 1.0e-6;
        let craft = server.fleet.get_mut(CraftId(DYING.0)).unwrap();
        craft.settle(now_s);
        assert!(craft.fitting().unwrap().since_s() > lc_world::solar::segment_end(now_s) - DAY_S, "premise: mid-segment");
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

    /// Light-microseconds in a meter.
    const US_PER_M: f64 = 1.0 / LIGHT_MICROSECOND_M;

    /// A full starting ship at its limit at the origin, which collapses a microsecond into the first
    /// tick, and starting ships `at` light-microseconds off, each holding `heat` of `Q_max`, or idle
    /// for `None`. No star, so nothing else heats anybody.
    fn neighbors(at: &[(DVec3, Option<f64>)]) -> Server<Memory> {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut dying = still(DYING, DVec3::ZERO);
        server.fit_new(&mut dying);
        heat_to(&mut dying, 1.0);
        server.fleet.insert(dying);
        for (k, (place, heat)) in at.iter().enumerate() {
            let mut craft = still(ShipId(2 + k as i64), *place);
            server.fit_new(&mut craft);
            if let Some(heat) = heat {
                heat_to(&mut craft, *heat);
            }
            server.fleet.insert(craft);
        }
        server
    }

    fn heat_to(craft: &mut Craft, of_max: f64) {
        let fitting = craft.fitting().unwrap().clone();
        let heat_j = of_max * fitting.field().heat_max_j();
        craft.fit(Some(Fitting::from_account(&Account { heat_j, ..fitting.account() }, *fitting.balance())));
    }

    /// Of `DYING`'s spike, where a starting ship holding `heat` of `Q_max` would be killed along
    /// `toward`, light-microseconds.
    fn lethal_us(server: &Server<Memory>, heat: f64, toward: DVec3) -> f64 {
        let dying = server.ship(DYING).unwrap().fitting().unwrap();
        let spike_j = dying.balance().collapse_spike_fraction * dying.field().released_j(dying.hull().capacities.storage_j);
        let mut probe = still(ShipId(99), toward);
        server.fit_new(&mut probe);
        heat_to(&mut probe, heat);
        let fitting = probe.fitting().unwrap();
        let headroom_j = fitting.field().heat_max_j() - fitting.heat_j_at(&probe.motion, 0.0);
        let shadow_m2 = shadow_toward_m2(&probe, -toward, 0.0);
        lc_world::field::lethal_radius_m(fitting.absorptivity(), spike_j, shadow_m2, headroom_j) * US_PER_M
    }

    fn died_at(server: &Server<Memory>, ship: ShipId) -> Option<i64> {
        collapse_events(server).iter().find(|e| e.source == ship).map(|e| e.t)
    }

    /// Hot enough that the lethal radius is a dozen kilometers rather than a hundred meters.
    const HOT: f64 = 1.0 - 1.0e-4;

    /// Just inside the radius 30 gives, a spike kills; just outside, it does not.
    #[tokio::test]
    async fn a_spike_kills_inside_the_lethal_radius_and_not_outside_it() {
        let r_us = lethal_us(&neighbors(&[]), HOT, DVec3::Y);
        assert!(r_us * LIGHT_MICROSECOND_M > 5_000.0, "premise: hot widens it, to {} m", r_us * LIGHT_MICROSECOND_M);
        for (of_radius, dies) in [(0.99, true), (1.01, false)] {
            let mut server = neighbors(&[(DVec3::Y * of_radius * r_us, Some(HOT))]);
            server.tick(&mut Loopback::new()).await.unwrap();
            assert!(died_at(&server, DYING).is_some(), "premise: it collapsed");
            assert_eq!(died_at(&server, ShipId(2)).is_some(), dies, "at {of_radius} of the lethal radius");
        }
    }

    /// B, inside A's radius, dies when A's light reaches it; C, outside A's but inside B's, when B's
    /// reaches it. Each death is one light-time after the last.
    #[tokio::test]
    async fn a_cascade_spreads_at_the_speed_of_light() {
        let r_us = lethal_us(&neighbors(&[]), HOT, DVec3::Y);
        let (b, c) = (DVec3::Y * 0.6 * r_us, DVec3::Y * 1.2 * r_us);
        let mut server = neighbors(&[(b, Some(HOT)), (c, Some(HOT))]);
        server.tick(&mut Loopback::new()).await.unwrap();
        let a_t = died_at(&server, DYING).expect("A collapsed");
        let b_t = died_at(&server, ShipId(2)).expect("B died");
        let c_t = died_at(&server, ShipId(3)).expect("C died");
        assert_eq!(b_t, (a_t as f64 + b.length()).ceil() as i64);
        assert_eq!(c_t, (b_t as f64 + (c - b).length()).ceil() as i64);
        assert!(c_t - b_t > 10, "premise: light-times long enough to tell apart, {}", c_t - b_t);

        let mut alone = neighbors(&[(c, Some(HOT))]);
        alone.tick(&mut Loopback::new()).await.unwrap();
        assert_eq!(died_at(&alone, ShipId(2)), None, "C is outside A's radius, so B is what killed it");
    }

    /// Storage empty or full, a spike is all heat, `α` of what arrives on the shadow toward it.
    #[tokio::test]
    async fn a_spike_is_all_heat_whatever_storage_has_room_for() {
        let apart_us = 3.0 * lethal_us(&neighbors(&[]), 0.0, DVec3::Y);
        let mut server = neighbors(&[(DVec3::Y * apart_us, None), (-DVec3::Y * apart_us, None)]);
        server.fleet.get_mut(CraftId(3)).unwrap().drain(f64::INFINITY, 0.0);
        let before: Vec<Craft> = [2, 3].map(|id| server.fleet.get(CraftId(id)).unwrap().clone()).into();
        server.tick(&mut Loopback::new()).await.unwrap();
        let at_t = died_at(&server, DYING).unwrap();
        let from = before[0].position_at(0.0) - DVec3::Y * apart_us;
        let dying = neighbors(&[]);
        let dying = dying.ship(DYING).unwrap().fitting().unwrap();
        let spike_j = dying.balance().collapse_spike_fraction * dying.field().released_j(dying.hull().capacities.storage_j);
        let now_s = server.now_t() as f64 * 1.0e-6;
        for (k, mut quiet) in before.into_iter().enumerate() {
            let craft = server.fleet.get(quiet.id).unwrap();
            let arrive_t = (at_t as f64 + apart_us).ceil() as i64;
            let absorbed_j = craft.fitting().unwrap().absorptivity() * received_j(craft, from, spike_j, arrive_t);
            // The dying ship's glow, which the tick began with.
            let lit_w = craft.fitting().unwrap().lit_w();
            quiet.adjust(0.0, |fitting| fitting.set_lit_w(lit_w));
            quiet.settle(now_s);
            let stored_j = |c: &Craft| c.fitting().unwrap().stored_j_at(&c.motion, now_s);
            assert_eq!(stored_j(craft), stored_j(&quiet), "{k}: a burst converted into storage");
            let tau_s = quiet.fitting().unwrap().field().tau_s;
            let rose_j = craft.fitting().unwrap().heat_j_at(&craft.motion, now_s) - quiet.fitting().unwrap().heat_j_at(&quiet.motion, now_s);
            let want_j = absorbed_j * (-(now_s - arrive_t as f64 * 1.0e-6) / tau_s).exp();
            assert!((rose_j - want_j).abs() < 1.0e-9 * want_j, "{k}: rose {rose_j}, the spike {want_j}");
            assert!(want_j > 0.01 * craft.fitting().unwrap().field().heat_max_j(), "premise: a real burst");
        }
    }

    /// `Q/τ` of a neighbor's field, onto the shadow toward it, over `4π d²`: all heat into a full
    /// ship, and none of it without the neighbor.
    #[tokio::test]
    async fn a_neighbors_glow_is_intake() {
        let d_us = 2_000.0 * US_PER_M;
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut source = still(DYING, DVec3::ZERO);
        server.fit_new(&mut source);
        heat_to(&mut source, 0.5);
        server.fleet.insert(source);
        let mut receiver = still(WATCHING, DVec3::Y * d_us);
        server.fit_new(&mut receiver);
        let lone = receiver.clone();
        server.fleet.insert(receiver);
        let mut wire = Loopback::new();
        server.tick(&mut wire).await.unwrap();
        server.tick(&mut wire).await.unwrap();
        let lit_t = server.now_t() - crate::server::TICK_US;

        let source = server.ship(DYING).unwrap().fitting().unwrap();
        let receiver = server.ship(WATCHING).unwrap();
        let shadow_m2 = shadow_toward_m2(receiver, -DVec3::Y, lit_t as f64 * 1.0e-6);
        let d_m = d_us * LIGHT_MICROSECOND_M;
        let want_w = source.heat_j_at(&server.ship(DYING).unwrap().motion, lit_t as f64 * 1.0e-6) / source.field().tau_s * shadow_m2 / (4.0 * std::f64::consts::PI * d_m * d_m);
        let lit_w = receiver.fitting().unwrap().lit_w();
        assert!((lit_w - want_w).abs() < 1.0e-9 * want_w, "{lit_w} {want_w}");

        let now_s = server.now_t() as f64 * 1.0e-6;
        let mut lone = lone;
        lone.settle(now_s);
        let held_j = |craft: &Craft| {
            let fitting = craft.fitting().unwrap();
            fitting.heat_j_at(&craft.motion, now_s) + fitting.stored_j_at(&craft.motion, now_s)
        };
        let rose_j = held_j(receiver) - held_j(&lone);
        let lit_j = receiver.fitting().unwrap().absorptivity() * lit_w * 2.0 * crate::server::TICK_US as f64 * 1.0e-6;
        assert!((rose_j - lit_j).abs() < 1.0e-3 * lit_j, "rose {rose_j}, lit {lit_j}");
    }

    /// The shard as it comes back from a checkpoint of `server`, on the same world and rate.
    fn restart(server: &Server<Memory>) -> Server<Memory> {
        let checkpoint = server.checkpoint();
        let mut restarted = Server::new(Memory::default(), 0, 2);
        restarted.load_world(World::new(vec![a_star().unwrap()]));
        restarted.set_rate(60.0);
        assert!(restarted.adopt(checkpoint).is_empty());
        restarted
    }

    /// Nothing about a restored wreck is anyone's, nothing is resumed for it, and it has no field
    /// to collapse again.
    fn assert_inert(server: &Server<Memory>, id: CraftId) {
        let wreck = server.fleet.get(id).expect("the wreck came back");
        assert!(wreck.ended_s().is_some() && wreck.fitting().is_none());
        assert!(!server.owners.contains_key(&id) && !server.by_account.values().any(|ship| ship.0 == id.0));
        assert!(!server.pursuits.contains_key(&id) && !server.refitting.contains_key(&id) && !server.reserved.contains_key(&id));
        assert!(!server.instruments.aboard.contains_key(&id));
        assert!(!server.auto_ack.contains_key(&id) && !server.owed.contains_key(&id));
    }

    /// Restarted after the collapse and before its light reaches a ship three light-days off, the
    /// shard goes on showing that ship the wreck until the light arrives, and deletes its row then.
    #[tokio::test]
    async fn a_wreck_outlives_a_restart_until_its_light_arrives() {
        let Some((mut server, mut wire)) = scene() else { return };
        let star = a_star().unwrap().id.get();
        wire.client_says(OWNER, act(DYING, Order::SetDuty { duty: lc_proto::Duty::Stare { star }, integration_s: 1.0e4 }));
        for _ in 0..5 {
            server.tick(&mut wire).await.unwrap();
        }
        let knew = server.take_knowledge();
        let knew_of = |id: i64| knew.files.iter().any(|f| f.ship_id == id) || knew.samples.iter().any(|r| r.ship_id == id);
        assert!(knew_of(DYING.0), "premise: it knew something");
        let at_t = until_collapse(&mut server, &mut wire).await.at_t;
        server.tick(&mut wire).await.unwrap();
        let arrives_t = at_t + APART_US as i64;
        assert!(server.now_t() < arrives_t - 4 * 3_600_000_000, "premise: its light is still well short of the watcher");
        assert!(server.take_destroyed().is_empty());

        let row = server.checkpoint().ships.into_iter().find(|s| s.ship_id == DYING.0).expect("the wreck is saved");
        assert_eq!(row.account, None);
        assert_eq!(crate::persist::decode(&row).unwrap().ended_s, Some(at_t as f64 * 1.0e-6));

        let mut restarted = restart(&server);
        drop(server);
        assert!(restarted.adopt_knowledge(&knew.files, &knew.samples).is_empty());
        assert_inert(&restarted, CraftId(DYING.0));
        let watching = restarted.ship(WATCHING).unwrap().clone();
        restarted.admit(WATCHER, watching, 0.0);
        let mut wire = Loopback::new();
        let (mut seen_until_t, mut swept_t, mut last_t) = (i64::MIN, None, restarted.now_t());
        let mut tick_us = 0;
        for _ in 0..100 {
            restarted.tick(&mut wire).await.unwrap();
            tick_us = restarted.now_t() - last_t;
            last_t = restarted.now_t();
            for message in wire.take(WATCHER) {
                match message {
                    Outbound::Present(list) if list.iter().any(|p| p.get().ship_id == DYING) => seen_until_t = restarted.now_t(),
                    _ => {}
                }
            }
            if swept_t.is_none() && restarted.take_destroyed() == vec![DYING.0] {
                swept_t = Some(restarted.now_t());
            }
            if restarted.now_t() > arrives_t + 2 * tick_us {
                break;
            }
        }
        assert!(seen_until_t + tick_us >= arrives_t, "the restart ended its view early: {seen_until_t} {arrives_t}");
        assert!(seen_until_t < arrives_t + tick_us, "still seen after its light had passed");
        let swept_t = swept_t.expect("the wreck was never swept");
        assert!(swept_t >= arrives_t && swept_t < arrives_t + tick_us, "{swept_t} {arrives_t}");
        assert!(restarted.ship(DYING).is_none());
        assert!(collapse_events(&restarted).is_empty(), "it collapsed again");
    }

    /// Signed out through the collapse and a restart, the owner signs in to the successor; the
    /// wreck's row claims no account, and ordering it is refused.
    #[tokio::test]
    async fn after_a_restart_the_owner_still_flies_the_successor() {
        use crate::testing::Broker;
        let Some(near) = near_the_star() else { return };
        let broker = Broker::new([1u8; 32]);
        let shard = || {
            let mut server = Server::new(Memory::default(), 0, 1);
            let mut trusted = crate::ticket::Trusted::new("shard-1");
            assert_eq!(trusted.learn(&broker.jwks()), 1);
            server.trust(trusted);
            server.load_world(World::new(vec![a_star().unwrap()]));
            server.set_rate(60.0);
            server
        };
        let hello = |jti: &str| Inbound::Hello { protocol: lc_proto::PROTOCOL_VERSION, ticket: broker.mint("acct-1", "shard-1", 60, jti) };
        let welcomed = |said: Vec<Outbound>| {
            said.into_iter().find_map(|m| match m {
                Outbound::Welcome { ship_id, .. } => Some(ship_id),
                _ => None,
            })
        };
        let mut server = shard();
        let mut wire = Loopback::new();
        wire.client_says(OWNER, hello("j1"));
        server.tick(&mut wire).await.unwrap();
        let first = welcomed(wire.take(OWNER)).expect("welcomed");
        let mut dying = still(first, near);
        server.fit_new(&mut dying);
        server.fleet.remove(CraftId(first.0));
        server.fleet.insert(dying);
        // Clear of the identifiers the shard hands out.
        server.fleet.insert(still(ShipId(50), near + DVec3::Y * APART_US));
        server.disconnected(OWNER);
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            if !collapse_events(&server).is_empty() {
                break;
            }
        }
        let successor = server.by_account["acct-1"];
        assert_ne!(successor, first, "premise: it collapsed");

        let rows = server.checkpoint().ships;
        let wreck = rows.iter().find(|s| s.ship_id == first.0).expect("the wreck is saved");
        assert_eq!(wreck.account, None);
        assert_eq!(rows.iter().find(|s| s.ship_id == successor.0).unwrap().account.as_deref(), Some("acct-1"));

        let mut restarted = shard();
        assert!(restarted.adopt(server.checkpoint()).is_empty());
        drop(server);
        assert_eq!(restarted.by_account["acct-1"], successor);
        assert_inert(&restarted, CraftId(first.0));
        let again = ClientId(3);
        wire.client_says(again, hello("j2"));
        restarted.tick(&mut wire).await.unwrap();
        assert_eq!(welcomed(wire.take(again)), Some(successor));
        wire.client_says(again, act(first, Order::Burn { beta: [1.0e-6, 0.0, 0.0] }));
        restarted.tick(&mut wire).await.unwrap();
        let said = wire.take(again);
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { ship_id, reason: Refusal::NotYours } if *ship_id == first)), "{said:?}");
    }

    /// Four ships collapse hours apart; every wreck comes back from one restart, and each is swept
    /// on the tick its own light passes the watcher.
    #[tokio::test]
    async fn a_cascade_of_wrecks_restores_and_sweeps_one_by_one() {
        let Some((mut server, mut wire)) = scene() else { return };
        server.disconnected(OWNER);
        let near = near_the_star().unwrap();
        let star = a_star().unwrap().position_ly * LIGHT_US_PER_LY;
        let wrecks: Vec<ShipId> = (0..3).map(|k| ShipId(10 + k)).collect();
        for (k, id) in wrecks.iter().enumerate() {
            let mut craft = still(*id, star + (near - star) * (1.0 + 0.01 * (k + 1) as f64));
            server.fit_new(&mut craft);
            server.fleet.insert(craft);
        }
        server.next_ship = 20;
        let wrecks: Vec<ShipId> = std::iter::once(DYING).chain(wrecks).collect();
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
            if wrecks.iter().all(|id| server.ship(*id).is_some_and(|c| c.ended_s().is_some())) {
                break;
            }
        }
        let watcher = server.ship(WATCHING).unwrap().position_at(0.0);
        let arrives: Vec<i64> = wrecks
            .iter()
            .map(|id| {
                let wreck = server.ship(*id).expect("premise: none swept before the restart");
                let end_t = wreck.ended_s().expect("premise: all collapsed") * 1.0e6;
                (end_t + wreck.position_at(end_t).distance(watcher)).ceil() as i64
            })
            .collect();
        let mut distinct = arrives.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), wrecks.len(), "premise: they collapse apart");
        assert!(server.take_destroyed().is_empty());

        let mut restarted = restart(&server);
        drop(server);
        for id in &wrecks {
            assert_inert(&restarted, CraftId(id.0));
        }
        let mut wire = Loopback::new();
        let mut swept: Vec<(i64, i64)> = Vec::new();
        let mut last_t = restarted.now_t();
        for _ in 0..100 {
            restarted.tick(&mut wire).await.unwrap();
            let (from_t, now_t) = (last_t, restarted.now_t());
            last_t = now_t;
            for id in restarted.take_destroyed() {
                let k = wrecks.iter().position(|w| w.0 == id).expect("only wrecks are swept");
                assert!(arrives[k] > from_t && arrives[k] <= now_t, "{id} swept at {now_t}, its light arrives {}", arrives[k]);
                swept.push((id, now_t));
            }
            if swept.len() == wrecks.len() {
                break;
            }
        }
        assert_eq!(swept.len(), wrecks.len(), "{swept:?}");
        assert!(collapse_events(&restarted).is_empty(), "a wreck collapsed again");
    }

    /// Six ships stacked at one point, as the spawn point stacks them, together take no more than
    /// one in contact. Conversion stores what pays the drain, so each settles where the heat it keeps
    /// of that half, `1 − η`, is what it lacks: summed per source, storage would fill and they
    /// would heat each other without bound.
    #[tokio::test]
    async fn ships_stacked_at_one_point_heat_each_other_only_so_far() {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(1_000.0);
        for k in 0..6 {
            let mut craft = still(ShipId(k + 1), DVec3::ZERO);
            server.fit_new(&mut craft);
            server.fleet.insert(craft);
        }
        let idle_j = server.balance().field_tau_s * server.ship(ShipId(1)).unwrap().fitting().unwrap().hull().capacities.drain_w;
        let mut wire = Loopback::new();
        let ten_tau_s = 10.0 * server.balance().field_tau_s;
        while (server.now_t() as f64) * 1.0e-6 < ten_tau_s {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(collapse_events(&server).is_empty(), "the stack collapsed");
        let now_s = server.now_t() as f64 * 1.0e-6;
        for k in 0..6 {
            let craft = server.ship(ShipId(k + 1)).unwrap();
            let heat_j = craft.fitting().unwrap().heat_j_at(&craft.motion, now_s);
            let kept = CONTACT_FRACTION * (1.0 - server.balance().conversion_efficiency);
            let want_j = idle_j / (1.0 - kept);
            assert!((heat_j - want_j).abs() < 1.0e-3 * want_j, "ship {}: {} of idle", k + 1, heat_j / idle_j);
        }
    }
}
