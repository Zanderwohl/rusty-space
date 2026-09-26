//! What a ship's energy allows: the budget an order is flown within, refits, and the grant.
//!
//! The account itself is `lc_world::fitting`, and a craft keeps it: every change of motive
//! settles it and commits the new plan. What lives here is the authority's half — refusing what
//! a ship cannot pay for, and telling its owner what the account now says.

use std::collections::HashSet;

use lc_proto::{ClientId, FormFault, Outbound, Refusal, ShipId, Shortfall};
use lc_world::craft::{Craft, CraftId};
use lc_world::fitting::{Balance, Fitting};
use lc_world::flight::Drive;
use lc_world::form::{Form, rules};
use lc_world::motion::{Change, Event as Change_, ShipId as MotionId};
use lc_world::refit::rounds;

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
        fitting.balance().drive_efficiency,
    );
    if cost > craft.free_j_at(at_s) { Err(Refusal::NoEnergy) } else { Ok(()) }
}

/// Why a round toward a valid form cannot begin, naming the part where there is one.
fn unplanned(why: rounds::Refusal) -> Refusal {
    match why {
        rounds::Refusal::Form(fault) => Refusal::Form(fault.into()),
        rounds::Refusal::Mind(part) => Refusal::Form(FormFault::OtherMind(part.into())),
        rounds::Refusal::NoDrones { part } => Refusal::Short(Shortfall::NoDrones(part.into())),
        rounds::Refusal::Energy { .. } => Refusal::Short(Shortfall::Energy),
    }
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
                fitting.set_balance(balance);
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
        craft.fit(Some(Fitting::full(lc_world::form::Form::starting(), self.balance, now_s)));
    }

    /// Begin a round toward `target`. Flying and refitting exclude each other, and one round runs
    /// at a time.
    ///
    /// Every rule of 29 §Placement rules is checked before the planner runs, and the first fault
    /// is the refusal: the editor marks all of them from the same `rules::check`. Its grid, tens
    /// of milliseconds, is thrown away; the target is measured again as the round finishes.
    pub(crate) fn refit(&mut self, id: CraftId, target: &lc_proto::Form, at_s: f64) -> Result<(), Refusal> {
        let pursuing = self.pursuits.contains_key(&id);
        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
        let balance = *craft.fitting().ok_or(Refusal::Impossible)?.balance();
        if pursuing || craft.motion.is_under_way() {
            return Err(Refusal::UnderWay);
        }
        if craft.is_refitting(at_s) {
            return Err(Refusal::Refitting);
        }
        let target = Form::from(target);
        if let Err(faults) = rules::check(&target, &balance) {
            return Err(faults.first().map_or(Refusal::Impossible, |fault| Refusal::Form((*fault).into())));
        }
        craft.begin_refit(target, at_s).map_err(unplanned)?;
        self.refitting.insert(id, 0);
        Ok(())
    }

    /// Tell a craft's owner what its account now says.
    pub(crate) fn tell_fitted(&self, wire: &mut impl Transport, id: CraftId) {
        let Some(craft) = self.fleet.get(id) else { return };
        let Some(fitting) = craft.fitting() else { return };
        let Some(owner) = self.owners.get(&id).copied() else { return };
        let (hull, fitting) = (fitting.into(), fitting.into());
        wire.send(owner, Outbound::Fitted { ship_id: ShipId(id.0), fitting, hull, field: None });
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

    /// Once a tick, after the pursuits: refit steps that finished, and chases that can no longer
    /// be paid for. A plan committed when it was ordered cannot run dry; an escort keeping station
    /// on a quarry that goes on burning can, and is broken off when it does.
    pub(crate) fn keep_accounts(&mut self, wire: &mut impl Transport) {
        let now_s = self.now_t as f64 * 1.0e-6;
        // A finished step changes the form, and with it everything `Fitted` states.
        let stepped: Vec<(CraftId, Option<usize>)> = self
            .refitting
            .iter()
            .filter_map(|(id, told)| {
                let running = self.fleet.get(*id).filter(|craft| craft.is_refitting(now_s));
                let finished = running.and_then(|craft| craft.fitting()?.refit()).map(|plan| plan.at(now_s).finished);
                (finished != Some(*told)).then_some((*id, finished))
            })
            .collect();
        for (id, finished) in stepped {
            match finished {
                Some(finished) => self.refitting.insert(id, finished),
                None => self.refitting.remove(&id),
            };
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
    use crate::server::TICK_US;

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
        craft.fit(Some(Fitting::from_account(&empty, *fitting.balance())));
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

    /// The starting form with more engine on its bell.
    fn more_engine() -> Form {
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(2)).unwrap().volume_m3 *= 1.4;
        target
    }

    fn refit_to(target: &Form) -> Inbound {
        act(Order::Refit { target: target.into() })
    }

    /// Accepted, it runs as a round: the owner is told the round and the hull, again as each step
    /// finishes, and last with the target as the ship.
    #[tokio::test]
    async fn a_refit_runs_as_a_round_and_says_each_step() {
        let (mut server, mut wire, from, ship) = fitted_server(false);
        // Hours a tick, where a step takes days.
        server.set_rate(60.0);
        server.tick(&mut wire).await.unwrap();
        let _ = replies(&mut wire);
        let mut target = more_engine();
        target.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(1)).unwrap().volume_m3 *= 1.1;
        wire.client_says(from, refit_to(&target));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Accepted { order: Order::Refit { .. }, .. })), "{said:?}");
        let Some(Outbound::Fitted { fitting, hull, .. }) = said.iter().rev().find(|m| matches!(m, Outbound::Fitted { .. }))
        else {
            panic!("{said:?}")
        };
        let round = fitting.refit.as_ref().expect("the round is on the wire");
        assert_eq!(round.target, lc_proto::Form::from(&target));
        assert_eq!(hull.form, lc_proto::Form::from(&Form::starting()), "nothing is built yet");

        let plan = server.ship(ship).unwrap().fitting().unwrap().refit().unwrap().clone();
        assert!(plan.steps().len() >= 2, "premise: {:?}", plan.steps());
        let mut told = 0;
        while server.ship(ship).unwrap().is_refitting(server.now_t() as f64 * 1.0e-6) {
            server.tick(&mut wire).await.unwrap();
            told += replies(&mut wire).iter().filter(|m| matches!(m, Outbound::Fitted { .. })).count();
        }
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert_eq!(told, plan.steps().len(), "one Fitted a step");
        assert!(!said.iter().any(|m| matches!(m, Outbound::Fitted { .. })), "told again: {said:?}");
        let craft = server.ship(ship).unwrap();
        assert_eq!(craft.fitting().unwrap().form(), &target);
        assert!(craft.fitting().unwrap().refit().is_none());
    }

    /// A target that breaks a placement rule is refused naming the part, and one the planner
    /// cannot run says why; neither begins anything.
    #[tokio::test]
    async fn an_invalid_target_is_refused_by_name() {
        use lc_proto::form::PartId;
        let (mut server, mut wire, from, ship) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        let before = server.ship(ship).unwrap().fitting().cloned();

        let mut turned = Form::starting();
        let engine = turned.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(2)).unwrap();
        engine.placement.as_mut().unwrap().tilt = glam::DVec2::new(0.0, 1.0);
        let mut tiny = Form::starting();
        tiny.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(4)).unwrap().volume_m3 = 10.0;
        let mut other_mind = Form::starting();
        for part in &mut other_mind.parts {
            if part.id == lc_world::form::PartId(0) {
                part.id = lc_world::form::PartId(40);
            }
            if let Some(placement) = part.placement.as_mut().filter(|p| p.parent == lc_world::form::PartId(0)) {
                placement.parent = lc_world::form::PartId(40);
            }
        }
        let mut huge = Form::starting();
        for part in huge.parts.iter_mut().filter(|p| p.placement.is_some()) {
            part.volume_m3 *= 50.0;
        }
        let cases = [
            (turned, Refusal::Form(FormFault::EngineOffAxis(PartId(2)))),
            (tiny, Refusal::Form(FormFault::TooSmall(PartId(4)))),
            (other_mind, Refusal::Form(FormFault::OtherMind(PartId(40)))),
            (huge, Refusal::Short(Shortfall::Energy)),
        ];
        for (target, reason) in cases {
            wire.client_says(from, refit_to(&target));
            server.tick(&mut wire).await.unwrap();
            let said = replies(&mut wire);
            let refused: Vec<_> = said.iter().filter_map(|m| match m {
                Outbound::Refused { reason, .. } => Some(*reason),
                _ => None,
            }).collect();
            assert_eq!(refused, vec![reason], "{said:?}");
        }
        let after = server.ship(ship).unwrap().fitting().unwrap();
        assert_eq!(after.form(), before.as_ref().unwrap().form());
        assert!(after.refit().is_none() && server.refitting.is_empty());
    }

    /// Under way is flying a course. A ship that has only burned is adrift and may refit.
    #[tokio::test]
    async fn a_ship_under_way_cannot_refit() {
        use crate::server::course_tests::{a_system, orbitable};
        let Some(system) = a_system() else { return };
        let Some(body) = orbitable(&system) else { return };
        let (mut server, mut wire, from, ship) = fitted_server(false);
        server.fleet.get_mut(CraftId(1)).unwrap().enter(Some(system), 0.0);
        server.tick(&mut wire).await.unwrap();
        let course = lc_proto::Course::Orbit { body, altitude_radii: 2.0, plane: lc_proto::Plane::Equatorial };
        wire.client_says(from, act(Order::SetCourse { course, accel_g: 1.0, max_beta: 0.1 }));
        server.tick(&mut wire).await.unwrap();
        assert!(server.ship(ship).unwrap().motion.is_under_way(), "premise: {:?}", replies(&mut wire));
        wire.client_says(from, refit_to(&more_engine()));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::UnderWay, .. })), "{said:?}");
    }

    #[tokio::test]
    async fn flying_is_refused_while_refitting_and_a_cancel_frees_it() {
        let (mut server, mut wire, from, _) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        wire.client_says(from, refit_to(&more_engine()));
        server.tick(&mut wire).await.unwrap();
        wire.client_says(from, refit_to(&more_engine()));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(
            said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::Refitting, .. })),
            "a second round began over the first: {said:?}"
        );

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
        assert!(server.refitting.is_empty());

        wire.client_says(from, act(Order::Burn { beta: [1.0e-5, 0.0, 0.0] }));
        server.tick(&mut wire).await.unwrap();
        let said = replies(&mut wire);
        assert!(said.iter().any(|m| matches!(m, Outbound::Accepted { .. })), "{said:?}");
    }

    /// Each order the wire has before the shard can do it is refused as such, and does nothing.
    #[tokio::test]
    async fn an_order_not_built_yet_is_refused_as_not_built() {
        use lc_proto::{Aim, Apertures, Approach, Closeness, FieldMode};
        let (mut server, mut wire, from, ship) = fitted_server(false);
        server.tick(&mut wire).await.unwrap();
        let before = server.ship(ship).unwrap().fitting().cloned();
        let unbuilt = [
            act(Order::FieldMode { mode: FieldMode::Clear }),
            act(Order::Emit {
                aim: Aim::Omni,
                apertures: Apertures::Aft,
                power_w: 1.0e15,
                wavelength_m: 1.0e-6,
                spread_rad: 0.01,
                duration_s: 60.0,
            }),
        ];
        for message in unbuilt {
            wire.client_says(from, message.clone());
            server.tick(&mut wire).await.unwrap();
            let said = replies(&mut wire);
            assert!(
                said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::NotBuilt, .. })),
                "{message:?}: {said:?}"
            );
            assert!(!said.iter().any(|m| matches!(m, Outbound::Accepted { .. })), "{message:?}: {said:?}");
        }
        assert_eq!(server.ship(ship).unwrap().fitting().cloned(), before);
        assert!(server.pursuits.is_empty());

        // Both approaches are built, and reach the sighting check.
        for approach in [Approach::Courteous, Approach::Direct] {
            wire.client_says(
                from,
                act(Order::Intercept { ship_id: ShipId(2), closeness: Closeness::Company, approach }),
            );
            server.tick(&mut wire).await.unwrap();
            let said = replies(&mut wire);
            assert!(
                said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::NotInSight, .. })),
                "{approach:?}: {said:?}"
            );
        }
    }

    /// **Another ship's new form arrives with its light.** Two ships ten light-hours apart: the
    /// watcher goes on seeing the old form for ten hours after the refit's step ends, and at every
    /// tick sees exactly the form the step had left when the light it is shown left, and the round
    /// only while that light left during it.
    #[tokio::test]
    async fn another_ship_sees_a_refit_only_when_its_light_arrives() {
        const APART_US: f64 = 10.0 * 3_600.0 * 1.0e6;
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let (actor, watcher) = (ClientId(1), ClientId(2));
        for (client, at) in [(actor, DVec3::ZERO), (watcher, DVec3::new(APART_US, 0.0, 0.0))] {
            let mut craft = crate::world::still(ShipId(client.0 as i64), at);
            server.fit_new(&mut craft);
            server.admit(client, craft, 0.0);
        }
        server.tick(&mut wire).await.unwrap();
        // A small growth: one step, hours long, so the light delay is most of what is measured.
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(2)).unwrap().volume_m3 *= 1.01;
        wire.client_says(actor, refit_to(&target));
        server.tick(&mut wire).await.unwrap();
        let plan = server.ship(ShipId(1)).unwrap().fitting().unwrap().refit().expect("it began").clone();
        let built_s = plan.round().start_s + plan.duration_s();
        let arrives_s = built_s + APART_US * 1.0e-6;
        assert!(arrives_s - built_s > 20.0 * TICK_US as f64 * 1.0e-6, "the delay is not worth testing");

        let (start, target) = (lc_proto::Form::from(&Form::starting()), lc_proto::Form::from(&target));
        let round = lc_proto::Round::from(plan.round());
        let (mut old_after_build, mut new_seen_at, mut building_seen) = (0, None, 0);
        while new_seen_at.is_none() && (server.now_t() as f64) < (arrives_s + 3_600.0) * 1.0e6 {
            server.tick(&mut wire).await.unwrap();
            let now_s = server.now_t() as f64 * 1.0e-6;
            let seen = wire.take(watcher);
            let contact = seen.iter().rev().find_map(|m| match m {
                Outbound::Present(list) => list.iter().map(|c| c.get()).find(|p| p.ship_id == ShipId(1)).cloned(),
                _ => None,
            });
            let contact = contact.expect("the other ship is in sight");
            let emitted_s = contact.emitted_t as f64 * 1.0e-6;
            let expected = if emitted_s < built_s { &start } else { &target };
            assert_eq!(&contact.form, expected, "at {now_s}, light from {emitted_s}, built at {built_s}");
            let running = (plan.round().start_s..built_s).contains(&emitted_s);
            assert_eq!(contact.refit.as_ref(), running.then_some(&round), "at {now_s}, light from {emitted_s}");
            building_seen += running as usize;
            if contact.form == start && now_s > built_s {
                old_after_build += 1;
            }
            if contact.form == target {
                new_seen_at = Some(now_s);
            }
        }
        let seen_at = new_seen_at.expect("the new form was never seen");
        assert!(seen_at >= arrives_s && seen_at < arrives_s + TICK_US as f64 * 1.0e-6, "{seen_at} vs {arrives_s}");
        assert!(old_after_build > 20, "premise: the old form was seen after the build, {old_after_build} times");
        assert!(building_seen > 20, "premise: the round was seen under way, {building_seen} times");
    }

    /// **A cancel arrives with its light too.** The watcher goes on seeing the round under way for
    /// the ten hours the cancel's light takes, then the round gone and the form it left.
    #[tokio::test]
    async fn another_ship_sees_a_cancel_only_when_its_light_arrives() {
        const APART_US: f64 = 10.0 * 3_600.0 * 1.0e6;
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let (actor, watcher) = (ClientId(1), ClientId(2));
        for (client, at) in [(actor, DVec3::ZERO), (watcher, DVec3::new(APART_US, 0.0, 0.0))] {
            let mut craft = crate::world::still(ShipId(client.0 as i64), at);
            server.fit_new(&mut craft);
            server.admit(client, craft, 0.0);
        }
        server.tick(&mut wire).await.unwrap();
        wire.client_says(actor, refit_to(&more_engine()));
        server.tick(&mut wire).await.unwrap();
        let plan = server.ship(ShipId(1)).unwrap().fitting().unwrap().refit().expect("it began").clone();
        let first = plan.steps()[0];
        while (server.now_t() as f64) * 1.0e-6 < plan.round().start_s + first.begins_s + 0.5 * first.duration_s {
            server.tick(&mut wire).await.unwrap();
        }
        wire.client_says(actor, act(Order::CancelRefit));
        server.tick(&mut wire).await.unwrap();
        let canceled_s = server.now_t() as f64 * 1.0e-6;
        let left = lc_proto::Form::from(server.ship(ShipId(1)).unwrap().fitting().unwrap().form());
        let arrives_s = canceled_s + APART_US * 1.0e-6;
        assert!(canceled_s < plan.round().start_s + first.ends_s(), "premise: canceled mid-step");

        let round = lc_proto::Round::from(plan.round());
        let (mut under_way_after_cancel, mut gone_at) = (0, None);
        while gone_at.is_none() && (server.now_t() as f64) < (arrives_s + 3_600.0) * 1.0e6 {
            server.tick(&mut wire).await.unwrap();
            let now_s = server.now_t() as f64 * 1.0e-6;
            let contact = wire.take(watcher).iter().rev().find_map(|m| match m {
                Outbound::Present(list) => list.iter().map(|c| c.get()).find(|p| p.ship_id == ShipId(1)).cloned(),
                _ => None,
            });
            let contact = contact.expect("the other ship is in sight");
            let emitted_s = contact.emitted_t as f64 * 1.0e-6;
            let running = (plan.round().start_s..canceled_s).contains(&emitted_s);
            assert_eq!(contact.refit.as_ref(), running.then_some(&round), "at {now_s}, light from {emitted_s}");
            if running && now_s > canceled_s {
                under_way_after_cancel += 1;
            }
            if emitted_s >= canceled_s {
                assert_eq!(contact.form, left, "the form the cancel left");
                gone_at = Some(now_s);
            }
        }
        let gone_at = gone_at.expect("the cancel was never seen");
        assert!(gone_at >= arrives_s && gone_at < arrives_s + TICK_US as f64 * 1.0e-6, "{gone_at} vs {arrives_s}");
        assert!(under_way_after_cancel > 20, "premise: seen under way after the cancel {under_way_after_cancel} times");
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
            craft.fit(Some(Fitting::from_account(&empty, *fitting.balance())));
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
