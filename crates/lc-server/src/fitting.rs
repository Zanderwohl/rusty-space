//! What a ship's energy allows: the budget an order is flown within, refits, and the grant.
//!
//! The account itself is `lc_world::fitting`, and a craft keeps it: every change of motive
//! settles it and commits the new plan. What lives here is the authority's half — refusing what
//! a ship cannot pay for, and telling its owner what the account now says.

use std::collections::HashSet;

use lc_proto::{ClientId, Outbound, Refusal, ShipId};
use lc_world::craft::{Craft, CraftId};
use lc_world::fitting::{Balance, Fitting, Loadout};
use lc_world::flight::Drive;
use lc_world::motion::{Change, Event as Change_, ShipId as MotionId};

use crate::journal::Journal;
use crate::server::{Server, refusal_for};
use crate::transport::Transport;

/// The slowest cap a course is lowered to before it is refused, as a fraction of `c`. About
/// three kilometers a second: slower than this is not a flight anyone ordered.
pub const SLOWEST_BETA: f64 = 1.0e-5;

/// Halvings of the cap, fixed rather than to a tolerance so a replay lands on the same one.
const CAP_HALVINGS: usize = 40;

/// The fastest version of a change this craft can pay for at `at_s`.
///
/// `drive` is already clamped to what was asked for and what the engines give. If its cap is
/// unaffordable the cap is bisected down — cost only rises with the cap — and a course that is
/// unaffordable even at [`SLOWEST_BETA`] is refused.
pub fn within_budget(
    craft: &Craft,
    at_s: f64,
    drive: Drive,
    change: impl Fn(Drive) -> Change,
) -> Result<Drive, Refusal> {
    let free = craft.free_j_at(at_s);
    let cost = |drive: Drive| {
        let event = Change_ { ship: MotionId(craft.id.0), at_t: at_s, change: change(drive) };
        craft.cost_of(&event).map_err(refusal_for)
    };
    if cost(drive)? <= free {
        return Ok(drive);
    }
    let with = |beta: f64| Drive { max_beta: beta, ..drive };
    if cost(with(SLOWEST_BETA))? > free {
        return Err(Refusal::NoEnergy);
    }
    let (mut lo, mut hi) = (SLOWEST_BETA, drive.max_beta);
    for _ in 0..CAP_HALVINGS {
        let mid = 0.5 * (lo + hi);
        if cost(with(mid))? <= free {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(with(lo))
}

/// A burn that changes velocity at once, if the ship can pay for it.
pub fn afford_burn(craft: &Craft, at_s: f64, to_beta: glam::DVec3) -> Result<(), Refusal> {
    let Some(fitting) = craft.fitting() else { return Ok(()) };
    let from = lc_world::motion::state_at(&craft.motion, craft.system.as_deref(), at_s)
        .map_or(craft.motion.beta, |(_, beta)| beta);
    let rapidity = lc_world::cost::rapidity_between(from, to_beta);
    let cost = lc_world::cost::energy_j(
        craft.mass_kg_at(at_s),
        rapidity,
        fitting.balance.drive_efficiency,
    );
    if cost > craft.free_j_at(at_s) { Err(Refusal::NoEnergy) } else { Ok(()) }
}

/// Whether a craft that is flying a plan can still finish it.
pub fn can_pay_for_its_plan(craft: &Craft, now_s: f64) -> bool {
    let Some(fitting) = craft.fitting() else { return true };
    let stored = fitting.stored_j_at(&craft.motion, now_s);
    stored > 0.0 && stored >= fitting.committed_j_at(&craft.motion, now_s)
}

impl<J: Journal> Server<J> {
    /// The tunables every ship this shard fits is read under.
    pub fn set_balance(&mut self, balance: Balance) {
        self.balance = balance;
        for craft in self.fleet.iter_mut() {
            if let Some(mut fitting) = craft.fitting().cloned() {
                fitting.balance = balance;
                craft.fit(Some(fitting));
            }
        }
    }

    pub fn balance(&self) -> Balance {
        self.balance
    }

    /// What a new player's ship is given.
    pub(crate) fn fit_new(&self, craft: &mut Craft) {
        let now_s = self.now_t as f64 * 1.0e-6;
        craft.fit(Some(Fitting::full(Loadout::STARTING, self.balance, now_s)));
    }

    /// Tell a craft's owner what its account now says.
    pub(crate) fn tell_fitted(&self, wire: &mut impl Transport, id: CraftId) {
        let Some(craft) = self.fleet.get(id) else { return };
        let Some(fitting) = craft.fitting() else { return };
        let Some(owner) = self.owners.get(&id).copied() else { return };
        wire.send(owner, Outbound::Fitted { ship_id: ShipId(id.0), fitting: fitting.into() });
    }

    /// Begin a refit, or say why not.
    pub(crate) fn refit(&mut self, id: CraftId, target: Loadout, at_s: f64) -> Result<(), Refusal> {
        let pursuing = self.pursuits.contains_key(&id);
        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
        if craft.fitting().is_none() {
            return Err(Refusal::Impossible);
        }
        if pursuing || craft.motion.is_under_way() {
            return Err(Refusal::UnderWay);
        }
        craft.begin_refit(target, at_s).map_err(|s| Refusal::Short(s.into()))?;
        self.refitting.insert(id);
        Ok(())
    }

    /// **Development only.** Put energy into the asking client's ship, if it may develop.
    pub fn granted(&mut self, from: ClientId, joules: f64, wire: &mut impl Transport) {
        let ship = self.owned_by(from);
        let done = self.may(from, crate::ability::Act::GrantEnergy, ship)
            && joules.is_finite()
            && joules > 0.0
            && self.grant(from, joules).is_some();
        match ship {
            Some(id) if done => self.tell_fitted(wire, id),
            // The answer `Stage` gives, for the reason it gives it.
            _ => wire.send(from, Outbound::Refused { ship_id: ShipId(0), reason: Refusal::Impossible }),
        }
    }

    /// No permission check; the caller makes it. `None` without a fitted ship.
    pub(crate) fn grant(&mut self, from: ClientId, joules: f64) -> Option<CraftId> {
        let now_s = self.now_t as f64 * 1.0e-6;
        let ship = self.owned_by(from)?;
        let craft = self.fleet.get_mut(ship)?;
        let fitted = craft.fitting().is_some();
        craft.grant(joules, now_s);
        fitted.then_some(ship)
    }

    /// Once a tick, after the pursuits: refits that finished, and chases that can no longer be
    /// paid for. A plan committed when it was ordered cannot run dry; an escort keeping station
    /// on a quarry that goes on burning can, and is broken off when it does.
    pub(crate) fn keep_accounts(&mut self, wire: &mut impl Transport) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let finished: Vec<CraftId> = self
            .refitting
            .iter()
            .copied()
            .filter(|id| self.fleet.get(*id).is_none_or(|craft| !craft.is_refitting(now_s)))
            .collect();
        for id in finished {
            self.refitting.remove(&id);
            self.tell_fitted(wire, id);
        }

        let broke: HashSet<CraftId> = self
            .pursuits
            .keys()
            .copied()
            .filter(|id| self.fleet.get(*id).is_some_and(|craft| !can_pay_for_its_plan(craft, now_s)))
            .collect();
        for id in broke {
            self.pursuits.remove(&id);
            let Some(craft) = self.fleet.get_mut(id) else { continue };
            let cut = Change_ { ship: MotionId(id.0), at_t: now_s, change: Change::CutDrive };
            if craft.apply(&cut).is_ok() {
                self.tell_flying(wire, id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use glam::DVec3;
    use lc_proto::{Inbound, Intent, Order};
    use lc_world::craft::Kind;

    fn fitted_server(directs: bool) -> (Server<Memory>, Loopback, ClientId, ShipId) {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.directing(directs);
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        server.fit_new(&mut craft);
        server.admit(ClientId(1), craft, 0.0);
        (server, Loopback::new(), ClientId(1), ShipId(1))
    }

    fn act(order: Order) -> Inbound {
        Inbound::Act(Intent { ship_id: ShipId(1), order, issued_at_client_t: i64::MAX })
    }

    fn free(server: &Server<Memory>) -> f64 {
        let now_s = server.now_t() as f64 * 1.0e-6;
        server.ship(ShipId(1)).unwrap().free_j_at(now_s)
    }

    fn drain(server: &mut Server<Memory>) {
        let now_s = server.now_t() as f64 * 1.0e-6;
        let craft = server.fleet.get_mut(CraftId(1)).unwrap();
        craft.settle(now_s);
        let fitting = craft.fitting().unwrap().clone();
        let empty = lc_world::fitting::Account { stored_j: 0.0, ..fitting.account() };
        craft.fit(Some(Fitting::from_account(&empty, fitting.balance)));
    }

    fn replies(wire: &mut Loopback) -> Vec<Outbound> {
        wire.take(ClientId(1))
    }

    #[tokio::test]
    async fn a_crossing_the_ship_cannot_pay_for_at_full_speed_is_flown_slower() {
        let (mut server, mut wire, _, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        let before = free(&server);
        let craft = server.ship(ShipId(1)).unwrap();
        let drive = craft.rated_drive(server.now_t() as f64 * 1.0e-6);
        let at_s = server.now_t() as f64 * 1.0e-6;
        // Twenty light-years: at 0.999c this is far more than the starting ship holds.
        let far = DVec3::X * 20.0;
        let flown = within_budget(craft, at_s, drive, |drive| Change::Cross { to_ly: far, drive })
            .expect("a slower crossing is affordable");
        assert!(flown.max_beta < drive.max_beta && flown.max_beta > 0.3, "{}", flown.max_beta);
        let event = Change_ {
            ship: MotionId(1),
            at_t: at_s,
            change: Change::Cross { to_ly: far, drive: flown },
        };
        let cost = craft.cost_of(&event).unwrap();
        assert!(cost <= before && cost > 0.99 * before, "it did not spend what it had: {cost} of {before}");
    }

    #[tokio::test]
    async fn an_empty_ship_cannot_burn() {
        let (mut server, mut wire, from, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        drain(&mut server);
        wire.client_says(from, act(Order::Burn { beta: [1.0e-4, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::NoEnergy, .. })),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_refit_is_refused_under_way_and_flying_is_refused_while_refitting() {
        let (mut server, mut wire, from, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        let target = lc_proto::Loadout { storage: 6, drones: 2, living: 1, engines: 6, slots: 20, data: 1 };
        wire.client_says(from, act(Order::Refit { target }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Accepted { .. })), "{said:?}");
        assert!(said.iter().any(|m| matches!(m, Outbound::Fitted { .. })), "{said:?}");

        wire.client_says(from, act(Order::Burn { beta: [1.0e-5, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::Refitting, .. })),
            "{said:?}"
        );

        wire.client_says(from, act(Order::CancelRefit));
        server.tick(&mut wire).await.unwrap();
        let _ = replies(&mut wire);
        let now_s = server.now_t() as f64 * 1.0e-6;
        assert!(!server.ship(ShipId(1)).unwrap().is_refitting(now_s));

        wire.client_says(from, act(Order::Burn { beta: [1.0e-5, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let _ = replies(&mut wire);
        let now_s = server.now_t() as f64 * 1.0e-6;
        let craft = server.fleet.get_mut(CraftId(1)).unwrap();
        // A drift is not under way, so hold it in a crossing instead.
        craft.apply(&Change_ {
            ship: MotionId(1),
            at_t: now_s,
            change: Change::Cross { to_ly: DVec3::X * 0.01, drive: lc_world::flight::Drive::DEFAULT },
        })
        .unwrap();
        wire.client_says(from, act(Order::Refit { target }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::UnderWay, .. })),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_refit_that_cannot_be_done_says_why() {
        let (mut server, mut wire, from, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        // Forty engines and the slots for them: more than thirty stored module-energies pay for.
        let target = lc_proto::Loadout { storage: 6, drones: 2, living: 1, engines: 40, slots: 60, data: 1 };
        wire.client_says(from, act(Order::Refit { target }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(
            said.iter().any(|m| matches!(
                m,
                Outbound::Refused { reason: Refusal::Short(lc_proto::Shortfall::Energy), .. }
            )),
            "{said:?}"
        );
    }

    #[tokio::test]
    async fn a_grant_is_refused_by_a_shard_that_is_not_directing() {
        let (mut server, mut wire, from, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        drain(&mut server);
        wire.client_says(from, Inbound::Grant { joules: 1.0e26 });
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { .. })), "{said:?}");
        assert_eq!(free(&server), 0.0);
    }

    /// **Granting is levelled.** Every administrative level may do it on a real shard —
    /// `DEBUG` included, which is the tier named for exactly this. A player may not, and
    /// neither may a ticket minted before the broker carried a level at all. Signed in by
    /// ticket rather than admitted, because the level is the ticket's to carry — see
    /// `crate::ability`.
    #[tokio::test]
    async fn only_an_administrators_ticket_may_grant_on_a_shard() {
        let broker = crate::testing::Broker::new([3u8; 32]);
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = crate::ticket::Trusted::new("shard-1");
        trusted.learn(&broker.jwks());
        server.trust(trusted);
        let mut wire = Loopback::new();
        use crate::ability::Level;
        let (player, admin, old, debug) = (ClientId(1), ClientId(2), ClientId(3), ClientId(4));
        let hello = |ticket: String| Inbound::Hello { protocol: lc_proto::PROTOCOL_VERSION, ticket };
        wire.client_says(player, hello(broker.mint_with("acct-player", "shard-1", "j1", 0)));
        wire.client_says(
            admin,
            hello(broker.mint_with("acct-admin", "shard-1", "j2", Level::ADMIN.as_i32())),
        );
        // No `perm` claim at all, as a broker from before levels minted one.
        wire.client_says(old, hello(broker.mint("acct-old", "shard-1", 60, "j3")));
        // The junior administrative level, which may develop like the others. It is here so
        // the loop below covers all three rather than only the top two.
        wire.client_says(
            debug,
            hello(broker.mint_with("acct-debug", "shard-1", "j4", Level::DEBUG.as_i32())),
        );
        server.tick(&mut wire).await.unwrap();
        for who in [player, admin, old, debug] {
            let _ = wire.take(who);
        }

        let stored = |server: &Server<Memory>, who: ClientId| {
            let id = server.owned_by(who).unwrap();
            let craft = server.fleet.get(id).unwrap();
            let now_s = server.now_t() as f64 * 1.0e-6;
            craft.fitting().unwrap().stored_j_at(&craft.motion, now_s)
        };
        for who in [player, admin, old, debug] {
            let id = server.owned_by(who).unwrap();
            let craft = server.fleet.get_mut(id).unwrap();
            let fitting = craft.fitting().unwrap().clone();
            let empty = lc_world::fitting::Account { stored_j: 0.0, ..fitting.account() };
            craft.fit(Some(Fitting::from_account(&empty, fitting.balance)));
            wire.client_says(who, Inbound::Grant { joules: 1.0e26 });
        }
        server.tick(&mut wire).await.unwrap();

        for (who, level) in [(admin, Level::ADMIN), (debug, Level::DEBUG)] {
            assert!(
                stored(&server, who) > 0.9e26,
                "{} was not granted anything",
                level.name(),
            );
            assert!(wire.take(who).iter().any(|m| matches!(m, Outbound::Fitted { .. })));
        }
        // A player, and a ticket from a broker that had no levels yet.
        for who in [player, old] {
            assert_eq!(stored(&server, who), 0.0, "{who:?} was granted energy");
            assert!(wire.take(who).iter().any(|m| matches!(m, Outbound::Refused { .. })));
        }
    }

    #[tokio::test]
    async fn a_grant_fills_a_directing_servers_ship_and_says_so() {
        let (mut server, mut wire, from, _) = fitted_server(true);
        server.tick(&mut wire).await.unwrap();
        drain(&mut server);
        wire.client_says(from, Inbound::Grant { joules: 1.0e26 });
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Fitted { .. })), "{said:?}");
        assert!(free(&server) > 0.9e26, "{}", free(&server));
    }
}
