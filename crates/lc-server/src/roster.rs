//! Filing where craft are into what every craft knows: seen by detection, heard by a landing,
//! stated by the craft itself where it shares its location. Runs for every craft, flown or not.
//! See `lightcone/docs/22-provenance.md` §Where craft are.

use std::collections::{HashMap, HashSet};

use glam::DVec3;
use lc_proto::{Chat, FactionId, Outbound, Payload, Scope, ShipId, Stated};
use lc_spacetime::{LIGHT_MICROSECOND_M, Worldline};
use lc_world::craft::CraftId;
use lc_world::knowledge::{Bearing, Subject};
use lc_world::flight::JULIAN_YEAR_S;
use lc_world::knowledge::trajectory::{Place, Trajectory};
use lc_world::knowledge::whereabouts::{Fix, Kind, Sighting, Witnessed};
use lc_world::motion::LIGHT_US_PER_LY;
use lc_world::system::{LocalSystem, M_PER_LY};

use crate::chase;
use crate::instruments::witness;
use crate::journal::Journal;
use crate::server::Server;
use crate::world::Event;

/// Game seconds between passes over every pair of craft.
const PASS_S: f64 = 60.0;
/// A craft kept in sight is filed again this often, so its stay's span keeps up with it. One that
/// keeps to its trajectory costs its file only the time.
const REFILE_S: f64 = 6.0 * 3600.0;
/// Places gathered toward a fit are at least this far apart in game seconds, so five of them span
/// enough of an arc to bend.
const PLACE_GAP_S: f64 = 600.0;
/// Places held toward a fit; past this every other one is dropped, so the arc they span doubles
/// until it is long enough to say what it is.
const MOST_PLACES: usize = 16;
/// Full sightings since the last fit that the next fit may fold; older ones stay as they are.
const MOST_UNFOLDED: usize = 32;
/// Places on the current trajectory kept to refit over when a sighting falls off it. A run longer
/// than this is not folded into the refit, and stands as it is.
const MOST_RUN: usize = 64;
/// Off a fitted trajectory by more than this, or three sigma, is off it, meters: what the
/// light-cone solve and a body's ephemeris may disagree by.
const FIT_FLOOR_M: f64 = 1e3;
/// How well a detection places a craft across the line of sight, radians: about a metre-class
/// aperture's diffraction. Range comes from the light-cone solve, as a contact's does.
const SEEN_SIGMA_RAD: f64 = 1e-6;
/// Before a trajectory is fitted, a craft that strays less than this from where its last filing
/// reckoned it on a straight line is not filed again, meters: orbital curvature alone would refile
/// it every pass.
const STRAY_FLOOR_M: f64 = 1e7;
/// How well a receiver tells which way a transmission came from, radians.
const HEARD_SIGMA_RAD: f64 = 1e-2;

/// What was last filed of each craft by each observer, to file only what is news.
#[derive(Default)]
pub struct Roster {
    filed: HashMap<(CraftId, CraftId), Filed>,
    next_t: i64,
    /// When the last pass ran: a collapse's light is looked for over everything since, so a
    /// spike between two passes is not missed.
    last_t: Option<i64>,
    /// Who each craft saw on the last pass: a collapse is known for whose only by a craft that
    /// had it in sight until its light went out.
    in_sight: HashSet<(CraftId, CraftId)>,
    collapses_seen: HashSet<(CraftId, CraftId)>,
}

impl Roster {
    /// Let go of what was filed about wrecks that have been swept.
    pub(crate) fn forget(&mut self, gone: &[i64]) {
        let gone = |id: &CraftId| gone.contains(&id.0);
        self.filed.retain(|(_, quarry), _| !gone(quarry));
        self.in_sight.retain(|(_, quarry)| !gone(quarry));
        self.collapses_seen.retain(|(_, wreck)| !gone(wreck));
    }
}

#[derive(Clone)]
struct Filed {
    /// When it was last filed, in full or carried on.
    emitted_s: f64,
    system: Option<lc_world::sky::StarId>,
    /// Where it was last filed in full, and how it was moving.
    position_ly: DVec3,
    beta: DVec3,
    /// Fitted from what was seen, and kept while the craft holds to it.
    trajectory: Option<Trajectory>,
    /// Places toward a fit, oldest first, at least [`PLACE_GAP_S`] apart.
    places: Vec<Place>,
    /// Full sightings filed since the last fit, which the next one folds.
    unfolded: Vec<Place>,
    /// Places on `trajectory`: those it was fitted from, and each filing on it since.
    run: Vec<Place>,
}

/// What a pass makes of one craft seen by another.
enum Pass {
    /// Nothing new to file.
    Quiet,
    /// Only the roster's own state moved: a place gathered toward a fit.
    Gathered(Filed),
    /// A sighting to file, and the state after it.
    File(Filed, Option<Trajectory>, Vec<f64>),
}

impl Filed {
    fn first(system: Option<lc_world::sky::StarId>, place: Place, beta: DVec3) -> Self {
        Filed {
            emitted_s: place.at_s,
            system,
            position_ly: place.position_ly,
            beta,
            trajectory: None,
            places: vec![place],
            unfolded: vec![place],
            run: Vec::new(),
        }
    }

    /// Fitted from `run`, which ends at `place`: the sighting to file, folding the rest.
    fn fitted(&self, trajectory: Trajectory, mut run: Vec<Place>, place: Place) -> Pass {
        if run.len() > MOST_RUN {
            run.drain(..run.len() - MOST_RUN);
        }
        let folded = run.iter().map(|p| p.at_s).filter(|&t| t < place.at_s).collect();
        let after = Filed {
            emitted_s: place.at_s,
            trajectory: Some(trajectory),
            places: Vec::new(),
            unfolded: Vec::new(),
            run,
            ..self.clone()
        };
        Pass::File(after, Some(trajectory), folded)
    }

    /// Off the straight line from where it was last filed in full.
    fn strayed(&self, place: Place, sigma_ly: f64) -> bool {
        let reckoned = self.position_ly + self.beta * ((place.at_s - self.emitted_s) / JULIAN_YEAR_S);
        let floor_ly = STRAY_FLOOR_M / LIGHT_MICROSECOND_M / LIGHT_US_PER_LY;
        reckoned.distance(place.position_ly) > (3.0 * sigma_ly).max(floor_ly)
    }

    /// Fold this pass's sight of the craft in. `system` is `None` between systems and in one that
    /// is not loaded, where only a line can be fitted.
    fn pass(&self, system: Option<&LocalSystem>, place: Place, beta: DVec3, sigma_ly: f64) -> Pass {
        let tolerance_ly = (3.0 * sigma_ly).max(FIT_FLOOR_M / M_PER_LY);
        let refile = place.at_s - self.emitted_s >= REFILE_S;
        if let Some(trajectory) = self.trajectory {
            let on = trajectory.place_at(system, place.at_s).is_some_and(|at| at.distance(place.position_ly) <= tolerance_ly);
            let mut run = self.run.clone();
            run.push(place);
            return match (on, refile) {
                (true, false) => Pass::Quiet,
                (true, true) => {
                    run.drain(..run.len().saturating_sub(MOST_RUN));
                    Pass::File(Filed { emitted_s: place.at_s, run, ..self.clone() }, Some(trajectory), Vec::new())
                }
                // Off a fit from a short arc need not be a burn: the whole run says which.
                (false, _) => match Trajectory::fit(system, &run, tolerance_ly, Some(&trajectory)) {
                    Some(refitted) => self.fitted(refitted, run, place),
                    None => Pass::File(Filed::first(self.system, place, beta), None, Vec::new()),
                },
            };
        }
        let mut next = self.clone();
        let gathered = self.places.last().is_none_or(|last| place.at_s - last.at_s >= PLACE_GAP_S);
        if gathered {
            next.places.push(place);
        }
        let mut points: Vec<Place> = next.places.iter().chain(&next.unfolded).copied().chain([place]).collect();
        points.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
        points.dedup_by(|a, b| a.at_s.to_bits() == b.at_s.to_bits());
        if let Some(fitted) = Trajectory::fit(system, &points, tolerance_ly, None) {
            return self.fitted(fitted, points, place);
        }
        if next.places.len() > MOST_PLACES {
            let latest = next.places.len() - 1;
            next.places = next.places.iter().enumerate().filter(|(k, _)| (latest - k).is_multiple_of(2)).map(|(_, p)| *p).collect();
        }
        if refile || self.strayed(place, sigma_ly) {
            next.emitted_s = place.at_s;
            next.position_ly = place.position_ly;
            next.beta = beta;
            next.unfolded.push(place);
            if next.unfolded.len() > MOST_UNFOLDED {
                next.unfolded.remove(0);
            }
            return Pass::File(next, None, Vec::new());
        }
        if gathered { Pass::Gathered(next) } else { Pass::Quiet }
    }
}

/// Where a stated place goes as the receiver holds it: under the key it came under, kept to
/// itself when it was said to it alone, and open when it was said to everyone.
fn scope_of(to: Option<i64>, under: Option<FactionId>) -> Scope {
    match (under, to) {
        (Some(key), _) => Scope::Faction(key),
        (None, Some(_)) => Scope::Own,
        (None, None) => Scope::Open,
    }
}

impl<J: Journal> Server<J> {
    /// Share `ship`'s location in `chat` from now on, or stop. False when `from` may not.
    pub(crate) fn set_sharing(&mut self, from: lc_proto::ClientId, ship: ShipId, chat: Chat, on: bool) -> bool {
        let id = CraftId(ship.0);
        let order = lc_proto::Order::ShareLocation { chat, on };
        if !self.may(from, crate::ability::Act::of(&order), Some(id)) || chat == Chat::With(ship) {
            return false;
        }
        let set = self.sharing.entry(id).or_default();
        match on {
            true => set.insert(chat),
            false => set.remove(&chat),
        };
        if set.is_empty() {
            self.sharing.remove(&id);
        }
        true
    }

    pub(crate) fn tell_sharing(&self, wire: &mut impl crate::transport::Transport, client: lc_proto::ClientId, ship: ShipId) {
        let chats = self.sharing.get(&CraftId(ship.0)).map(|set| set.iter().copied().collect()).unwrap_or_default();
        wire.send(client, Outbound::SharingLocation { ship_id: ship, chats });
    }

    /// Where `id` says it is on a transmission at `at`, if it shares that in the chat the
    /// transmission belongs to. A broadcast belongs to none.
    pub(crate) fn stated_by(&self, id: CraftId, to: Option<ShipId>, under: Option<FactionId>, at: i64) -> Option<Stated> {
        let chat = match (under, to) {
            (Some(key), _) => Chat::Faction(key),
            (None, Some(ship)) => Chat::With(ship),
            (None, None) => return None,
        };
        if !self.sharing.get(&id).is_some_and(|set| set.contains(&chat)) {
            return None;
        }
        let craft = self.fleet.get(id)?;
        Some(Stated {
            at_ly: (craft.position_at(at as f64) / LIGHT_US_PER_LY).to_array(),
            beta: craft.worldline().velocity_at(at as f64).to_array(),
        })
    }

    /// File, for every craft, every craft it now detects that is news to it: first sight, a new
    /// system, a trajectory fitted, leaving the trajectory, or long enough since the last filing.
    pub(crate) fn watch_crafts(&mut self) {
        if self.now_t < self.roster.next_t {
            return;
        }
        self.roster.next_t = self.now_t + (PASS_S * 1.0e6) as i64;
        let since_t = self.roster.last_t.replace(self.now_t).unwrap_or(self.now_t - (PASS_S * 1.0e6) as i64);
        self.watch_collapses(since_t);
        let ids: Vec<CraftId> = self.fleet.iter().map(|c| c.id).collect();
        let seeable = self.seeable();
        let mut news = Vec::new();
        let mut in_sight = HashSet::new();
        for &observer in &ids {
            let Some(here_us) = self.fleet.get(observer).map(|c| c.position_at(self.now_t as f64)) else { continue };
            let here = here_us / LIGHT_US_PER_LY;
            // A wreck too, whose hull is seen until the light of its end arrives.
            let quarries: Vec<CraftId> = seeable.from(here_us, self.now_t as f64).map(|c| c.id).collect();
            for &quarry in &quarries {
                let Some(seen) = chase::sighting(seeable, &self.balance, self.sees_everything, observer, ShipId(quarry.0), self.now_t)
                else {
                    continue;
                };
                in_sight.insert((observer, quarry));
                let sigma_ly = SEEN_SIGMA_RAD * here.distance(seen.position_ly);
                let system = self.world.star_containing(seen.position_ly);
                let local = system.and_then(|id| self.world.loaded_system(id)).map(|s| s.as_ref());
                let place = Place { at_s: seen.emitted_s, position_ly: seen.position_ly };
                let pass = match self.roster.filed.get(&(observer, quarry)) {
                    Some(last) if last.system == system => last.pass(local, place, seen.beta, sigma_ly),
                    _ => Pass::File(Filed::first(system, place, seen.beta), None, Vec::new()),
                };
                news.push((observer, quarry, pass, place, seen.beta, sigma_ly));
            }
        }
        self.roster.in_sight = in_sight;
        let now_s = self.now_t as f64 * 1.0e-6;
        for (observer, quarry, pass, place, beta, sigma_ly) in news {
            let (filed, trajectory, thinned_s) = match pass {
                Pass::Quiet => continue,
                Pass::Gathered(filed) => {
                    self.roster.filed.insert((observer, quarry), filed);
                    continue;
                }
                Pass::File(filed, trajectory, thinned_s) => (filed, trajectory, thinned_s),
            };
            let sighting = Sighting {
                kind: Kind::Seen,
                emitted_s: place.at_s,
                system: filed.system,
                fix: Fix::Place { position_ly: place.position_ly, beta, sigma_ly },
                witnesses: vec![Witnessed { witness: witness(observer), observed_s: now_s, scope: Scope::Open, lineage: Vec::new() }],
                trajectory,
                thinned_s,
            };
            self.roster.filed.insert((observer, quarry), filed);
            self.aboard(observer).knowledge.spotted(Subject::Craft(quarry.0), sighting);
        }
    }

    /// File, for every craft, each collapse whose light it has just seen: the spike if it landed
    /// since `since_t`, or the afterglow over that span. Once per collapse, as a collapse and
    /// nobody's: only a craft that had the ship in sight until then knows whose it was.
    fn watch_collapses(&mut self, since_t: i64) {
        let exposure_s = (self.now_t - since_t) as f64 * 1.0e-6;
        if exposure_s <= 0.0 {
            return;
        }
        let mut news = Vec::new();
        for wreck in self.brights.wrecks() {
            let (afterglow, wreck) = (&wreck.afterglow, wreck.craft.id);
            for observer in self.fleet.iter() {
                if self.roster.collapses_seen.contains(&(observer.id, wreck)) {
                    continue;
                }
                let here = observer.position_at(self.now_t as f64);
                let delay_s = afterglow.position().distance(here) * 1.0e-6;
                let left_s = self.now_t as f64 * 1.0e-6 - delay_s;
                if left_s < afterglow.at_s {
                    continue;
                }
                let flux = afterglow.mean_flux(left_s - exposure_s, left_s, delay_s * 1.0e6 * LIGHT_MICROSECOND_M);
                if self.sees_everything || lc_world::glow::detected_over(&observer.sensor, &flux, exposure_s) {
                    let collapse = Subject::collapse(afterglow.at_s, afterglow.from);
                    news.push((observer.id, wreck, collapse, afterglow.position() / LIGHT_US_PER_LY, afterglow.at_s, here / LIGHT_US_PER_LY));
                }
            }
        }
        let now_s = self.now_t as f64 * 1.0e-6;
        for (observer, wreck, collapse, position_ly, at_s, here) in news {
            self.roster.collapses_seen.insert((observer, wreck));
            let system = self.world.star_containing(position_ly);
            let sighting = Sighting {
                kind: Kind::Collapse,
                emitted_s: at_s,
                system,
                fix: Fix::Place { position_ly, beta: DVec3::ZERO, sigma_ly: SEEN_SIGMA_RAD * here.distance(position_ly) },
                witnesses: vec![Witnessed { witness: witness(observer), observed_s: now_s, scope: Scope::Open, lineage: Vec::new() }],
                trajectory: None,
                thinned_s: Vec::new(),
            };
            let whose = self.roster.in_sight.contains(&(observer, wreck));
            let knowledge = &mut self.aboard(observer).knowledge;
            knowledge.spotted(collapse, sighting.clone());
            if whose {
                knowledge.spotted(Subject::Craft(wreck.0), sighting);
                knowledge.write(collapse, lc_proto::Writing::Was(Some(ShipId(wreck.0))), Scope::Open, now_s);
            }
        }
    }

    /// File what a landing says of where its sender was: the place it stated, if the observer
    /// can read it, and otherwise a bearing, when the sender is out of sight.
    pub(crate) fn heard_place(&mut self, event: &Event, observer: CraftId, arrive_t: i64, strength: f32) {
        let sender = CraftId(event.source.0);
        if observer == sender {
            return;
        }
        let addressing = match (&event.payload, event.payload.addressing()) {
            (_, Some(addressing)) => addressing,
            (Payload::Drive(_), None) => lc_proto::Addressing { to: None, sealed: false, under: None },
            _ => return,
        };
        let readable = addressing.readable_by(observer.0, |key| self.factions.holds(observer, key, arrive_t));
        let stated = event.payload.stated().copied();
        let Some(here) = self.fleet.get(observer).map(|c| c.position_at(arrive_t as f64) / LIGHT_US_PER_LY) else { return };
        let emitted_s = event.t as f64 * 1.0e-6;
        let observed_s = arrive_t as f64 * 1.0e-6;
        let held = |scope| vec![Witnessed { witness: witness(observer), observed_s, scope, lineage: Vec::new() }];
        let sighting = match stated.filter(|_| readable) {
            Some(stated) => {
                let position_ly = DVec3::from_array(stated.at_ly);
                Sighting {
                    kind: Kind::Stated,
                    emitted_s,
                    system: self.world.star_containing(position_ly),
                    fix: Fix::Place { position_ly, beta: DVec3::from_array(stated.beta), sigma_ly: 0.0 },
                    witnesses: held(scope_of(addressing.to, addressing.under)),
                    trajectory: None,
                    thinned_s: Vec::new(),
                }
            }
            None => {
                if self.roster.filed.contains_key(&(observer, sender))
                    && chase::sighting(self.seeable(), &self.balance, self.sees_everything, observer, ShipId(sender.0), arrive_t).is_some()
                {
                    return;
                }
                let from_ly = event.at / LIGHT_US_PER_LY;
                // Which system, only when it is the observer's own: a bearing alone cannot tell a
                // near shell from the far ones behind it.
                let mine = self.fleet.get(observer).and_then(|c| c.system.as_ref().map(|s| s.star));
                let system = self.world.star_containing(from_ly).filter(|star| Some(*star) == mine);
                let bearing = Bearing { observer_ly: here, toward: (from_ly - here).normalize_or_zero(), sigma_rad: HEARD_SIGMA_RAD };
                let flux = f64::from(strength) / (LIGHT_MICROSECOND_M * LIGHT_MICROSECOND_M);
                Sighting { kind: Kind::Heard, emitted_s, system, fix: Fix::Toward { bearing, flux }, witnesses: held(Scope::Open), trajectory: None, thinned_s: Vec::new() }
            }
        };
        self.aboard(observer).knowledge.spotted(Subject::Craft(sender.0), sighting);
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{Aim, ClientId, Inbound, Intent, Order, Outbound, Secrecy};
    use lc_world::knowledge::Subject;

    use super::*;
    use crate::journal::Memory;
    use crate::server::TICK_US;
    use crate::transport::Loopback;

    const TWO_LIGHT_HOURS: f64 = 7_200.0 * 1_000_000.0;
    const ADA: ClientId = ClientId(1);
    const BRY: ClientId = ClientId(2);
    const CY: ClientId = ClientId(3);

    /// Ada at the origin; Bry two light-hours out, and Cy close beside Bry.
    fn three() -> (Server<Memory>, Loopback) {
        let mut server = Server::new(Memory::default(), 0, 1);
        let far = DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0);
        server.admit(ADA, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(BRY, crate::world::still(ShipId(2), far), 0.0);
        server.admit(CY, crate::world::still(ShipId(3), far + DVec3::new(0.0, 1.0e8, 0.0)), 0.0);
        (server, Loopback::new())
    }

    fn tell(server: &Server<Memory>, wire: &mut Loopback, client: ClientId, order: Order) {
        let ship_id = ShipId(client.0 as i64);
        wire.client_says(client, Inbound::Act(Intent { ship_id, order, issued_at_client_t: server.now_t() }));
    }

    async fn run_for(server: &mut Server<Memory>, wire: &mut Loopback, us: f64) {
        let until = server.now_t() + us as i64 + TICK_US;
        while server.now_t() < until {
            server.tick(wire).await.unwrap();
        }
    }

    fn sightings_of(server: &Server<Memory>, by: i64, of: i64, kind: Kind) -> Vec<Sighting> {
        server
            .knowledge_of(ShipId(by))
            .and_then(|k| k.file(Subject::Craft(of)))
            .map(|f| f.whereabouts().sightings().filter(|s| s.kind == kind).cloned().collect())
            .unwrap_or_default()
    }

    /// 11h's done-when, its last part: a location shared in a faction chat places the sender for
    /// every holder of the key and for nobody else.
    #[tokio::test]
    async fn a_location_shared_in_a_faction_chat_places_the_sender_for_every_holder_and_nobody_else() {
        let (mut server, mut wire) = three();
        tell(&server, &mut wire, BRY, Order::OfferKey { to: Some(ShipId(1)), aim: Aim::Omni });
        tell(&server, &mut wire, ADA, Order::Found { name: "Wake".into(), key: FactionId(0), replaces: None });
        server.tick(&mut wire).await.unwrap();
        let key = wire
            .take(ADA)
            .into_iter()
            .find_map(|m| match m {
                Outbound::Accepted { order: Order::Found { key, .. }, .. } => Some(key),
                _ => None,
            })
            .expect("founding was refused");
        run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
        tell(&server, &mut wire, ADA, Order::GiveKey { key, to: Some(ShipId(2)), aim: Aim::Omni, secrecy: Secrecy::Sealed });
        run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
        assert!(server.factions.holds(CraftId(2), key, server.now_t()));

        tell(&server, &mut wire, ADA, Order::ShareLocation { chat: Chat::Faction(key), on: true });
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(ADA).iter().any(|m| matches!(m, Outbound::SharingLocation { chats, .. } if chats == &vec![Chat::Faction(key)])));
        let say = Order::Say { to: None, aim: Aim::Omni, secrecy: Secrecy::Faction(key), body: "here".into(), idem: 1 };
        tell(&server, &mut wire, ADA, say);
        run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;

        let bry = sightings_of(&server, 2, 1, Kind::Stated);
        assert_eq!(bry.len(), 1, "Bry holds the key");
        assert_eq!(bry[0].witnesses[0].scope, Scope::Faction(key));
        let Fix::Place { position_ly, .. } = bry[0].fix else { panic!("a statement is a place") };
        assert!(position_ly.length() < 1e-12, "where Ada said it was: {position_ly}");
        assert!(sightings_of(&server, 3, 1, Kind::Stated).is_empty(), "Cy does not");
    }

    /// A transmission from a craft too far off to see is still a direction it came from.
    #[tokio::test]
    async fn a_craft_too_far_to_see_is_heard_by_what_it_sends() {
        let (mut server, mut wire) = three();
        assert!(chase::sighting(server.seeable(), &server.balance, false, CraftId(2), ShipId(1), TWO_LIGHT_HOURS as i64).is_none());
        let say = Order::Say { to: None, aim: Aim::Omni, secrecy: Secrecy::Open, body: "hello".into(), idem: 1 };
        tell(&server, &mut wire, ADA, say);
        run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
        let heard = sightings_of(&server, 2, 1, Kind::Heard);
        assert_eq!(heard.len(), 1);
        let Fix::Toward { bearing, flux } = heard[0].fix else { panic!("a bearing") };
        assert!(bearing.toward.dot(-DVec3::X) > 0.999 && flux > 0.0, "{bearing:?}");
        assert!(sightings_of(&server, 2, 1, Kind::Stated).is_empty(), "nothing shared, nothing stated");
    }

    /// Every craft keeps the roster, flown or not, and files a craft again only when it is news.
    #[tokio::test]
    async fn a_craft_in_sight_is_seen_once_and_again_only_when_it_is_news() {
        let (mut server, mut wire) = three();
        server.see_everything(true);
        run_for(&mut server, &mut wire, 10.0 * PASS_S * 1.0e6).await;
        assert_eq!(sightings_of(&server, 2, 3, Kind::Seen).len(), 1, "Cy, held still beside Bry");
        assert_eq!(sightings_of(&server, 1, 2, Kind::Seen).len(), 1, "Bry, once its light reached Ada");
        let stays = server.knowledge_of(ShipId(2)).unwrap().file(Subject::Craft(3)).unwrap().whereabouts().stays().len();
        assert_eq!(stays, 1);
    }

    /// Whether Bry's beam, aimed at where Cy's report last put Ada, lands on Ada: Cy beside Ada
    /// sees it, Bry two light-hours off never does. `moves` has Ada burn away once Cy has seen it.
    async fn beamed_at_last_known(moves: bool) -> bool {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        server.admit(ADA, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
        server.admit(BRY, crate::world::still(ShipId(2), DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0)), 0.0);
        server.admit(CY, crate::world::still(ShipId(3), DVec3::new(0.0, 1.0e6, 0.0)), 0.0);
        run_for(&mut server, &mut wire, 2.0 * PASS_S * 1.0e6).await;
        assert_eq!(sightings_of(&server, 3, 1, Kind::Seen).len(), 1, "Cy sees Ada");
        let report = Order::SendReport { to: None, aim: Aim::Omni, secrecy: Secrecy::Open, idem: 1 };
        tell(&server, &mut wire, CY, report);
        if moves {
            tell(&server, &mut wire, ADA, Order::Burn { beta: [0.0, 0.0, 0.01] });
        }
        run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;

        let bry = server.knowledge_of(ShipId(2)).unwrap();
        assert!(bry.file(Subject::Craft(1)).unwrap().whereabouts().sightings().all(|s| s.kind != Kind::Seen || s.witnesses[0].witness != witness(CraftId(2))));
        let (_, last) = bry.file(Subject::Craft(1)).unwrap().whereabouts().last_known().expect("Cy's report placed Ada");
        assert_eq!(last.kind, Kind::Seen);
        let hops: Vec<_> = last.witnesses[0].lineage.iter().map(|h| (h.from, h.to)).collect();
        assert_eq!((last.witnesses[0].witness, hops), (witness(CraftId(3)), vec![(witness(CraftId(3)), witness(CraftId(2)))]));
        let here = server.fleet.get(CraftId(2)).unwrap().position_at(server.now_t() as f64) / LIGHT_US_PER_LY;
        let toward = last.aim_from(here, server.now_t() as f64 * 1.0e-6).unwrap();
        let say = Order::Say { to: Some(ShipId(1)), aim: Aim::Bearing(toward.to_array()), secrecy: Secrecy::Open, body: "there?".into(), idem: 2 };
        wire.take(ADA);
        tell(&server, &mut wire, BRY, say);
        run_for(&mut server, &mut wire, 1.5 * TWO_LIGHT_HOURS).await;
        wire.take(ADA).iter().any(|m| match m {
            Outbound::Sightings(list) => list.iter().any(|c| {
                let s = c.get();
                s.kind == lc_proto::kind::MESSAGE && s.source_id == 2
            }),
            _ => false,
        })
    }

    /// 11h's done-when, its first two parts: a member who has never detected a craft learns where
    /// it was from an ally's report, witness and hop, and a message beamed there lands if the
    /// craft has stayed and misses if it has moved.
    #[tokio::test]
    async fn a_beam_at_a_place_learned_from_an_ally_lands_if_the_craft_stayed_and_misses_if_it_moved() {
        assert!(beamed_at_last_known(false).await, "it stayed");
        assert!(!beamed_at_last_known(true).await, "it moved");
    }
}
