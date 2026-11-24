//! An account and a ship are separate things: an account may have none, and gets one by launching.
//!
//! A signed-in account with no ship is held here, told [`Outbound::Unlaunched`], and hears
//! nothing of the world until it sends [`Inbound::Launch`](lc_proto::Inbound::Launch). A
//! collapse puts its owner back here: the wreck is nobody's, and nothing the ship knew passes on.

use glam::DVec3;
use lc_proto::{ClientId, LaunchRefusal, Outbound, ShipId};
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::navigation::Course;

use crate::journal::Journal;
use crate::server::{Connected, Server};
use crate::transport::Transport;

/// Where a new ship starts, when the shard holds the real solar system.
const LAUNCH_STAR: &str = lc_world::system::SOL;
const LAUNCH_COURSE: &str = "orbit:Luna:high";

/// A signed-in connection with no ship.
#[derive(Clone, Debug)]
pub struct Unlaunched {
    pub account: String,
    pub permission: crate::ability::Level,
}

impl<J: Journal> Server<J> {
    /// Hold `from` with no ship and tell it so.
    pub(crate) fn ground(&mut self, from: ClientId, account: String, permission: crate::ability::Level) {
        self.clients.remove(&from);
        self.unlaunched.insert(from, Unlaunched { account, permission });
    }

    pub(crate) fn launch_ship(&mut self, from: ClientId, typed: &str, wire: &mut impl Transport) {
        if self.clients.contains_key(&from) {
            wire.send(from, Outbound::LaunchRefused(LaunchRefusal::Flying));
            return;
        }
        // Not signed in: nobody to refuse.
        if !self.unlaunched.contains_key(&from) {
            return;
        }
        // Another connection for the same account launched first: this one takes the ship over,
        // as signing in again would, rather than being refused until it reconnects.
        if let Some(Unlaunched { account, permission }) = self.unlaunched.get(&from).cloned()
            && let Some(&ship) = self.by_account.get(&account)
        {
            self.embark(from, ship, account.clone(), permission);
            let name = self.ship(ship).map(Craft::designation).unwrap_or_default();
            self.welcome(from, ship, name, &account, wire);
            return;
        }
        let Some(name) = lc_proto::ship_name(typed).map(str::to_owned) else {
            wire.send(from, Outbound::LaunchRefused(LaunchRefusal::Name));
            return;
        };
        let Some(Unlaunched { account, permission }) = self.unlaunched.remove(&from) else { return };
        let ship = self.spawn(Some(name.clone()));
        self.register(ship, &account, None);
        self.embark(from, ship, account.clone(), permission);
        self.welcome(from, ship, name, &account, wire);
    }

    /// Record `ship` in `ship_registry` at the next checkpoint.
    pub(crate) fn register(&mut self, ship: ShipId, account: &str, ended_t: Option<i64>) {
        let name = self.ship(ship).or_else(|| self.wreck(ship).map(|w| &w.craft)).and_then(|craft| craft.name.clone());
        let account = (!account.is_empty()).then(|| account.to_owned());
        self.registered.push(lc_store::registry::Entry { ship_id: ship.0, account, name, ended_t });
    }

    /// Registry entries since this was last called.
    pub fn take_registered(&mut self) -> Vec<lc_store::registry::Entry> {
        std::mem::take(&mut self.registered)
    }

    /// Hand back what [`Server::take_registered`] took, because writing them failed.
    pub fn untake_registered(&mut self, entries: Vec<lc_store::registry::Entry>) {
        self.registered.extend(entries);
    }

    /// Bind `from` to `ship` as its pilot.
    pub(crate) fn embark(&mut self, from: ClientId, ship: ShipId, account: String, permission: crate::ability::Level) {
        self.unlaunched.remove(&from);
        if !account.is_empty() {
            self.by_account.insert(account.clone(), ship);
        }
        // A reconnection replaces the old connection's claim on the craft rather than sharing
        // it: two sockets acting for one ship is two clients predicting different futures.
        self.clients.retain(|_, state| state.ship != ship);
        self.clients.insert(from, Connected::new(ship, account, permission));
        self.claim_if_unnamed(CraftId(ship.0));
        // Being welcomed is not the same fact as owning the craft, and `act` checks the second.
        self.owners.insert(CraftId(ship.0), from);
    }

    /// A new starting ship, holding a high orbit of Luna when the shard has Sol, and otherwise
    /// at [`crate::world::World::start`].
    pub(crate) fn spawn(&mut self, name: Option<String>) -> ShipId {
        let ship = ShipId(self.next_ship);
        self.next_ship += 1;
        let at = self.world.start().unwrap_or(DVec3::ZERO);
        let mut craft = Craft::at(CraftId(ship.0), Kind::Ship, at);
        craft.name = name;
        self.put_in_orbit(&mut craft);
        self.fit_new(&mut craft);
        self.fleet.insert(craft);
        ship
    }

    fn put_in_orbit(&mut self, craft: &mut Craft) -> Option<()> {
        let now_s = self.now_t as f64 * 1.0e-6;
        let star = self.world.star_named(LAUNCH_STAR)?;
        let system = self.world.system_at(star, now_s)?;
        let from = craft.motion.position_ly;
        let waypoint = Course::parse(LAUNCH_COURSE)?.resolve(&system, from, now_s)?.nearest_to(from, &system, now_s);
        craft.motion.position_ly = waypoint.place_at(&system, now_s)?;
        craft.motion.begin_holding(waypoint);
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use lc_proto::{Inbound, Outbound};
    use lc_world::motion::Motive;
    use lc_world::navigation::{Anchor, Waypoint};

    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use crate::world::World;

    const PILOT: ClientId = ClientId(1);

    /// A signed-in account with no ship, held where a ticketed sign-in would leave it.
    fn grounded(world: World) -> (Server<Memory>, Loopback) {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(world);
        server.ground(PILOT, "acct-1".into(), crate::ability::Level::from_claim(0));
        (server, Loopback::new())
    }

    #[tokio::test]
    async fn a_launch_in_sol_holds_a_high_orbit_of_luna() {
        let Some(mut sol) = crate::server::course_tests::a_star() else { return };
        sol.provenance.name = Some(lc_world::system::SOL.to_string());
        let (mut server, mut wire) = grounded(World::new(vec![sol]));
        wire.client_says(PILOT, Inbound::Launch { name: " Tycho Station ".into() });
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(PILOT);
        let Some(Outbound::Welcome { ship_id, name, .. }) = said.first() else { panic!("{said:?}") };
        assert_eq!(name, "Tycho Station", "trimmed");
        assert_eq!(server.by_account["acct-1"], *ship_id);
        let craft = server.ship(*ship_id).unwrap();
        let Motive::Holding(Waypoint::Orbit(orbit)) = &craft.motion.motive else { panic!("{:?}", craft.motion.motive) };
        assert_eq!(orbit.about, Anchor::Body("Luna".into()));
        assert!(server.owned_by(PILOT).is_some_and(|id| id.0 == ship_id.0));
    }

    #[tokio::test]
    async fn a_name_too_long_is_refused_and_launches_nothing() {
        let (mut server, mut wire) = grounded(World::default());
        wire.client_says(PILOT, Inbound::Launch { name: "x".repeat(lc_proto::NAME_LIMIT + 1) });
        server.tick(&mut wire).await.unwrap();
        assert_eq!(wire.take(PILOT), vec![Outbound::LaunchRefused(LaunchRefusal::Name)]);
        assert!(server.fleet.is_empty() && server.unlaunched.contains_key(&PILOT));
    }

    /// Two connections grounded for one account: the second to launch joins the first's ship.
    #[tokio::test]
    async fn a_second_connection_joins_the_ship_the_first_launched() {
        let (mut server, mut wire) = grounded(World::default());
        let other = ClientId(2);
        server.ground(other, "acct-1".into(), crate::ability::Level::from_claim(0));
        wire.client_says(PILOT, Inbound::Launch { name: "Tachi".into() });
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(PILOT);
        wire.client_says(other, Inbound::Launch { name: "Rocinante".into() });
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(other);
        let Some(Outbound::Welcome { ship_id, name, .. }) = said.first() else { panic!("{said:?}") };
        assert_eq!((name.as_str(), server.fleet.len()), ("Tachi", 1), "joined, not launched");
        assert!(server.owned_by(other).is_some_and(|id| id.0 == ship_id.0) && server.owned_by(PILOT).is_none());
    }

    /// A launch from a connection that never signed in is not answered: there is nobody to refuse.
    #[tokio::test]
    async fn a_launch_before_signing_in_is_ignored() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        wire.client_says(PILOT, Inbound::Launch { name: "Stowaway".into() });
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(PILOT).is_empty());
        assert!(server.fleet.is_empty());
    }
}
