//! Filing where craft are into what every craft knows: seen by detection, heard by a landing,
//! stated by the craft itself where it shares its location. Runs for every craft, flown or not.
//! See `lightcone/docs/22-provenance.md` §Where craft are.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::{Chat, FactionId, Outbound, Reported, Scope, ShipId, Spoken, Stated};
use lc_spacetime::{LIGHT_MICROSECOND_M, Worldline};
use lc_world::craft::CraftId;
use lc_world::flight::JULIAN_YEAR_S;
use lc_world::knowledge::Bearing;
use lc_world::knowledge::whereabouts::{Fix, Kind, Sighting, Witnessed};
use lc_world::motion::LIGHT_US_PER_LY;

use crate::chase;
use crate::instruments::witness;
use crate::journal::Journal;
use crate::server::Server;
use crate::world::Event;

/// Game seconds between passes over every pair of craft.
const PASS_S: f64 = 60.0;
/// A craft kept in sight is filed again this often, so its stay's span keeps up with it.
const REFILE_S: f64 = 6.0 * 3600.0;
/// How well a detection places a craft across the line of sight, radians: about a metre-class
/// aperture's diffraction. Range comes from the light-cone solve, as a contact's does.
const SEEN_SIGMA_RAD: f64 = 1e-6;
/// Below this a craft that strays from where its last filing reckoned it is taken to be in orbit,
/// not maneuvering, meters: orbital curvature alone would refile every pass.
const STRAY_FLOOR_M: f64 = 1e7;
/// How well a receiver tells which way a transmission came from, radians.
const HEARD_SIGMA_RAD: f64 = 1e-2;

/// What was last filed of each craft by each observer, to file only what is news.
#[derive(Default)]
pub struct Roster {
    filed: HashMap<(CraftId, CraftId), Filed>,
    next_t: i64,
}

#[derive(Clone, Copy)]
struct Filed {
    emitted_s: f64,
    position_ly: DVec3,
    beta: DVec3,
    system: Option<lc_world::sky::StarId>,
}

impl Filed {
    fn strayed(&self, emitted_s: f64, position_ly: DVec3, sigma_ly: f64) -> bool {
        let reckoned = self.position_ly + self.beta * ((emitted_s - self.emitted_s) / JULIAN_YEAR_S);
        let floor_ly = STRAY_FLOOR_M / LIGHT_MICROSECOND_M / LIGHT_US_PER_LY;
        reckoned.distance(position_ly) > (3.0 * sigma_ly).max(floor_ly)
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
    /// system, a course that has changed, or long enough since the last filing.
    pub(crate) fn watch_crafts(&mut self) {
        if self.now_t < self.roster.next_t {
            return;
        }
        self.roster.next_t = self.now_t + (PASS_S * 1.0e6) as i64;
        let ids: Vec<CraftId> = self.fleet.iter().filter(|c| c.ended_s().is_none()).map(|c| c.id).collect();
        let mut news = Vec::new();
        for &observer in &ids {
            let Some(here) = self.fleet.get(observer).map(|c| c.position_at(self.now_t as f64) / LIGHT_US_PER_LY) else {
                continue;
            };
            for &quarry in &ids {
                let Some(seen) = chase::sighting(&self.fleet, &self.balance, self.sees_everything, observer, ShipId(quarry.0), self.now_t)
                else {
                    continue;
                };
                let sigma_ly = SEEN_SIGMA_RAD * here.distance(seen.position_ly);
                let system = self.world.star_containing(seen.position_ly);
                let due = match self.roster.filed.get(&(observer, quarry)) {
                    None => true,
                    Some(last) => {
                        last.system != system
                            || seen.emitted_s - last.emitted_s >= REFILE_S
                            || last.strayed(seen.emitted_s, seen.position_ly, sigma_ly)
                    }
                };
                if due {
                    let filed = Filed { emitted_s: seen.emitted_s, position_ly: seen.position_ly, beta: seen.beta, system };
                    news.push((observer, quarry, filed, sigma_ly));
                }
            }
        }
        let now_s = self.now_t as f64 * 1.0e-6;
        for (observer, quarry, filed, sigma_ly) in news {
            self.roster.filed.insert((observer, quarry), filed);
            let sighting = Sighting {
                kind: Kind::Seen,
                emitted_s: filed.emitted_s,
                system: filed.system,
                fix: Fix::Place { position_ly: filed.position_ly, beta: filed.beta, sigma_ly },
                witnesses: vec![Witnessed { witness: witness(observer), observed_s: now_s, scope: Scope::Open, lineage: Vec::new() }],
            };
            self.aboard(observer).knowledge.spotted(quarry.0, sighting);
        }
    }

    /// File what a landing says of where its sender was: the place it stated, if the observer
    /// can read it, and otherwise a bearing, when the sender is out of sight.
    pub(crate) fn heard_place(&mut self, event: &Event, observer: CraftId, arrive_t: i64, strength: f32) {
        let sender = CraftId(event.source.0);
        if observer == sender {
            return;
        }
        let (to, sealed, under, stated) = match event.kind {
            lc_proto::kind::MESSAGE | lc_proto::kind::KEY => match serde_json::from_str::<Spoken>(&event.payload) {
                Ok(s) => (s.to, s.sealed, s.under, s.stated),
                Err(_) => return,
            },
            lc_proto::kind::REPORT => match serde_json::from_str::<Reported>(&event.payload) {
                Ok(r) => (r.to, r.sealed, r.under, r.stated),
                Err(_) => return,
            },
            lc_proto::kind::DRIVE => (None, false, None, None),
            _ => return,
        };
        let readable = !(sealed && to != Some(observer.0)) && under.is_none_or(|key| self.factions.holds(observer, key, arrive_t));
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
                    witnesses: held(scope_of(to, under)),
                }
            }
            None => {
                if self.roster.filed.contains_key(&(observer, sender))
                    && chase::sighting(&self.fleet, &self.balance, self.sees_everything, observer, ShipId(sender.0), arrive_t).is_some()
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
                Sighting { kind: Kind::Heard, emitted_s, system, fix: Fix::Toward { bearing, flux }, witnesses: held(Scope::Open) }
            }
        };
        self.aboard(observer).knowledge.spotted(sender.0, sighting);
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
        assert!(chase::sighting(&server.fleet, &server.balance, false, CraftId(2), ShipId(1), TWO_LIGHT_HOURS as i64).is_none());
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
}
