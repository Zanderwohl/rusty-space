//! Clear, Black and Auto on the authority. See `lightcone/docs/30-the-field.md` §Clear and Black.
//!
//! Scheduled as a collapse is: nothing is kept but the account, whose next Auto switch is a closed
//! form re-solved each tick, so it runs with no client connected and a restart restores it. A
//! completed switch is taken out of the account here and only here, which is what makes each flip
//! exactly one event, stamped when and where it completed and seen by everyone at light delay.

use lc_proto::{FieldMode, Refusal, ShadeChange};
use lc_world::craft::{Craft, CraftId};
use lc_world::field::Mode;
use lc_world::fitting::Setting;

use super::{auto_by, collapse_by};
use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Set Black and kept there, as tests of anything but the modes want.
#[cfg(test)]
pub(crate) fn hold_black(craft: &mut Craft) {
    let Some(mut fitting) = craft.fitting().cloned() else { return };
    fitting.set_posture(lc_world::fitting::Posture::BLACK);
    craft.fit(Some(fitting));
}

impl<J: Journal> Server<J> {
    /// Take every switch done by now and begin every Auto switch due by now, in order, stopping at
    /// a collapse that comes first.
    pub(crate) fn keep_field_modes(
        &mut self,
        after_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let ids: Vec<CraftId> =
            self.fleet.iter().filter(|c| c.ended_s().is_none() && c.fitting().is_some()).map(|c| c.id).collect();
        for id in ids {
            while self.keep_field_mode(id, now_s, after_t, events, deliveries) {
                self.tell_fitted(wire, id);
            }
        }
    }

    /// One step of [`Server::keep_field_modes`]: whether it changed anything.
    fn keep_field_mode(
        &mut self,
        id: CraftId,
        now_s: f64,
        after_t: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> bool {
        let Some(craft) = self.fleet.get(id) else { return false };
        let Some(fitting) = craft.fitting() else { return false };
        if let Some(switch) = fitting.posture().switch {
            if switch.done_s > now_s || collapse_by(craft, switch.done_s).is_some() {
                return false;
            }
            return self.flip(id, now_s, after_t, events, deliveries);
        }
        let Some((at_s, to)) = auto_by(craft, now_s) else { return false };
        if collapse_by(craft, at_s).is_some() {
            return false;
        }
        self.fleet.get_mut(id).is_some_and(|craft| craft.begin_switch(to, at_s).is_ok())
    }

    /// Take a switch done by `by_s` out of `id`'s account and state it. Stamped up, and never into
    /// the tick before, where cursors already stand.
    fn flip(&mut self, id: CraftId, by_s: f64, after_t: i64, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) -> bool {
        let Some(craft) = self.fleet.get_mut(id) else { return false };
        let Some(done_s) = craft.fitting().and_then(|f| f.posture().switch).map(|s| s.done_s).filter(|&t| t <= by_s) else {
            return false;
        };
        let Some(switch) = craft.take_flip(done_s) else { return false };
        let clear_absorptivity = craft.fitting().map_or(0.0, |f| f.balance().clear_absorptivity);
        // What changes is the starlight it reflects.
        let power_w = craft.starlight_w_at(done_s) * (1.0 - clear_absorptivity);
        let payload = serde_json::to_string(&ShadeChange { shade: switch.to.into() }).unwrap_or_else(|_| "{}".into());
        // The clamp moves the stamp only on the order path, for a switch shorter than a tick.
        let at_t = ((done_s * 1.0e6).ceil() as i64).clamp(after_t + 1, self.now_t());
        self.emit(id, lc_proto::kind::SHADE, power_w, payload, at_t, events, deliveries);
        true
    }

    /// `Order::FieldMode` at `at`, or at the settlement if that is later, which is returned. Refused
    /// while a switch runs, and for Auto thresholds without both gaps open.
    pub(crate) fn order_field_mode(
        &mut self,
        id: CraftId,
        mode: FieldMode,
        at: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<i64, Refusal> {
        let setting = Setting::from(mode);
        if let Setting::Auto(thresholds) = setting
            && !thresholds.is_valid()
        {
            return Err(Refusal::Impossible);
        }
        let since_s = self.fleet.get(id).ok_or(Refusal::NotYours)?.fitting().ok_or(Refusal::Impossible)?.since_s();
        // An account cannot settle backward: a lagging order lands where it has been settled to,
        // which may be past a flip already taken, and must be judged there.
        let at = if since_s > at as f64 * 1.0e-6 { ((since_s * 1.0e6).ceil() as i64).min(self.now_t()) } else { at };
        let at_s = at as f64 * 1.0e-6;
        // Only when a switch is shorter than a tick: every other is taken before orders are read.
        self.flip(id, at_s, at - 1, events, deliveries);
        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
        craft.set_field(setting, at_s).map_err(|_| Refusal::Switching)?;
        Ok(at)
    }

    /// The console's `field`, through the order's own checks.
    pub(crate) fn field_command(
        &mut self,
        id: CraftId,
        mode: &str,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<String, String> {
        let balance = self.balance();
        let mode = match mode {
            "clear" => FieldMode::Clear,
            "black" => FieldMode::Black,
            _ => Setting::Auto(lc_world::fitting::Thresholds::of(&balance)).into(),
        };
        self.order_field_mode(id, mode, self.now_t(), events, deliveries).map_err(|why| format!("refused: {why:?}"))?;
        self.tell_fitted(wire, id);
        let craft = self.fleet.get(id).ok_or("no such ship")?;
        let posture = craft.fitting().ok_or("no field")?.posture();
        let set = match mode {
            FieldMode::Clear => "Clear".to_string(),
            FieldMode::Black => "Black".to_string(),
            FieldMode::Auto { clear_above, black_below, refill_below } => {
                format!("Auto (Clear above {clear_above}, Black below {black_below} under {refill_below} full)")
            }
        };
        Ok(match posture.switch {
            Some(switch) => format!("{}: {set}, {:?} in {:.1} days", craft.designation(), switch.to, (switch.done_s - self.now_t() as f64 * 1.0e-6) / 86_400.0),
            None => format!("{}: {set}, {:?}", craft.designation(), posture.shade),
        })
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, Order, Outbound, Shade, ShipId, Sighting};
    use lc_world::fitting::{Account, Fitting, Posture, Switch};
    use lc_world::motion::LIGHT_US_PER_LY;

    use super::*;
    use crate::journal::Memory;
    use crate::server::TICK_US;
    use crate::server::course_tests::{a_star, a_system};
    use crate::transport::Loopback;
    use crate::world::{World, still};

    const OWNER: ClientId = ClientId(1);
    const WATCHER: ClientId = ClientId(2);
    const SHIP: ShipId = ShipId(1);
    const WATCHING: ShipId = ShipId(2);
    const DAY_S: f64 = 86_400.0;
    const AU_US: f64 = lc_world::system::UNIT_M / 299.792_458;
    const APART_US: f64 = 3.0 * DAY_S * 1.0e6;

    /// Light-microseconds, where the sample star gives what the Sun gives at `d_au`.
    fn sunlike(d_au: f64) -> Option<DVec3> {
        let star = a_star()?;
        let sun_w = lc_world::solar::SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * lc_world::system::UNIT_M.powi(2);
        let scaled_au = d_au * (a_system()?.star_luminosity_w() / sun_w).sqrt();
        Some(star.position_ly * LIGHT_US_PER_LY + DVec3::X * scaled_au * AU_US)
    }

    fn server() -> Option<Server<Memory>> {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(vec![a_star()?]));
        server.next_ship = 3;
        Some(server)
    }

    fn rate_for_tick_s(tick_s: f64) -> f64 {
        tick_s * 1.0e6 / TICK_US as f64
    }

    fn act(order: Order) -> Inbound {
        Inbound::Act(Intent { ship_id: SHIP, order, issued_at_client_t: i64::MAX })
    }

    /// What each flip stated, in order.
    fn flips(server: &Server<Memory>) -> Vec<(i64, Shade)> {
        server
            .journal()
            .events
            .iter()
            .filter(|e| e.kind == lc_proto::kind::SHADE && e.source == SHIP)
            .map(|e| (e.t, serde_json::from_str::<ShadeChange>(&e.payload).unwrap().shade))
            .collect()
    }

    fn with_account(craft: &mut Craft, change: impl FnOnce(Account) -> Account) {
        let fitting = craft.fitting().unwrap().clone();
        craft.fit(Some(Fitting::from_account(&change(fitting.account()), *fitting.balance())));
    }

    fn to_us(s: f64) -> i64 {
        (s * 1.0e6).ceil() as i64
    }

    #[tokio::test]
    async fn a_new_ship_starts_in_auto() {
        let Some(mut server) = server() else { return };
        let ship = server.spawn(None);
        let posture = *server.ship(ship).unwrap().fitting().unwrap().posture();
        assert_eq!(posture, Posture::new_ship(&server.balance()));
        assert!(matches!(posture.setting, Setting::Auto(t) if t == lc_world::fitting::Thresholds::of(&server.balance())));
    }

    /// A new ship in empty space, full and Clear, with a watcher three light-days off. Hours a tick.
    fn apart() -> Option<(Server<Memory>, Loopback)> {
        let mut server = server()?;
        server.set_rate(rate_for_tick_s(3_600.0));
        let mut ship = still(SHIP, DVec3::ZERO);
        server.fit_new(&mut ship);
        server.admit(OWNER, ship, 0.0);
        server.admit(WATCHER, still(WATCHING, DVec3::Y * APART_US), 0.0);
        Some((server, Loopback::new()))
    }

    /// Ordered Black, it absorbs as Clear for a day; then the flip is one event, where the ship was
    /// when it completed, and a ship three light-days off hears of it three days later.
    #[tokio::test]
    async fn a_switch_changes_nothing_for_a_day_and_its_flip_arrives_at_light_delay() {
        let Some((mut server, mut wire)) = apart() else { return };
        server.tick(&mut wire).await.unwrap();
        wire.client_says(OWNER, act(Order::FieldMode { mode: FieldMode::Black }));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(OWNER);
        let Some(at_t) = said.iter().find_map(|m| match m {
            Outbound::Accepted { at_t, .. } => Some(*at_t),
            _ => None,
        }) else {
            panic!("{said:?}")
        };
        let done_s = at_t as f64 * 1.0e-6 + DAY_S;
        let fitted = said.iter().rev().find_map(|m| match m {
            Outbound::Fitted { field: Some(field), .. } => Some(*field),
            _ => None,
        });
        assert_eq!(fitted.map(|f| (f.mode, f.shade, f.switch)), Some((FieldMode::Black, Shade::Clear, Some(lc_proto::Switch { to: Shade::Black, done_s }))));

        let mut heard: Vec<(i64, Sighting)> = Vec::new();
        while (server.now_t() as f64) < done_s * 1.0e6 + APART_US + DAY_S * 1.0e6 {
            let now_s = server.now_t() as f64 * 1.0e-6;
            if now_s < done_s {
                assert_eq!(server.ship(SHIP).unwrap().fitting().unwrap().shade_at(now_s), lc_world::field::Mode::Clear);
                assert!(flips(&server).is_empty(), "flipped early");
            }
            server.tick(&mut wire).await.unwrap();
            for message in wire.take(WATCHER) {
                if let Outbound::Sightings(list) = message {
                    heard.extend(list.into_iter().map(|s| s.get().clone()).filter(|s| s.kind == lc_proto::kind::SHADE).map(|s| (server.now_t(), s)));
                }
            }
        }
        let flipped_t = to_us(done_s);
        assert_eq!(flips(&server), vec![(flipped_t, Shade::Black)]);
        let event = server.journal().events.iter().find(|e| e.kind == lc_proto::kind::SHADE).unwrap();
        assert!(event.at.distance(server.ship(SHIP).unwrap().position_at(flipped_t as f64)) < 1.0);
        let [(told_t, sighting)] = &heard[..] else { panic!("{heard:?}") };
        assert_eq!((sighting.emitted_t, sighting.arrive_t), (flipped_t, flipped_t + APART_US as i64));
        assert!(*told_t >= sighting.arrive_t);
        let posture = *server.ship(SHIP).unwrap().fitting().unwrap().posture();
        assert_eq!((posture.shade, posture.switch), (lc_world::field::Mode::Black, None), "taken once");
    }

    #[tokio::test]
    async fn a_second_order_while_one_switches_is_refused() {
        let Some((mut server, mut wire)) = apart() else { return };
        server.tick(&mut wire).await.unwrap();
        wire.client_says(OWNER, act(Order::FieldMode { mode: FieldMode::Black }));
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(OWNER);
        let running = *server.ship(SHIP).unwrap().fitting().unwrap().posture();
        let equal = FieldMode::Auto { clear_above: 0.4, black_below: 0.4, refill_below: 0.95 };
        for (mode, reason) in [(FieldMode::Clear, Refusal::Switching), (equal, Refusal::Impossible)] {
            wire.client_says(OWNER, act(Order::FieldMode { mode }));
            server.tick(&mut wire).await.unwrap();
            let said = wire.take(OWNER);
            assert!(said.iter().any(|m| matches!(m, Outbound::Refused { reason: r, .. } if *r == reason)), "{mode:?}: {said:?}");
        }
        assert_eq!(*server.ship(SHIP).unwrap().fitting().unwrap().posture(), running);
    }

    /// Clear at a tenth of an AU with half its store: Auto goes Black at once, fills, and turns
    /// Clear when full, where it stays. No client is connected.
    fn unwatched(rate: f64) -> Option<(Server<Memory>, f64, f64)> {
        let mut server = server()?;
        server.set_rate(rate);
        let mut ship = still(SHIP, sunlike(0.1)?);
        server.fit_new(&mut ship);
        let capacity_j = ship.fitting()?.hull().capacities.storage_j;
        with_account(&mut ship, |a| Account { stored_j: 0.8 * capacity_j, ..a });
        ship.enter(a_system(), 0.0);
        let fitting = ship.fitting()?.clone();
        server.fleet.insert(ship);
        // By hand: Clear for the day the first switch takes, then Black until full.
        let b = *fitting.balance();
        let (arriving_w, caps) = (fitting.starlight_w(), fitting.hull().capacities);
        let stored_w = |absorptivity: f64| b.conversion_efficiency * (absorptivity * arriving_w).min(caps.aperture_w);
        let at_day_j = 0.8 * capacity_j + (stored_w(b.clear_absorptivity) - caps.drain_w) * b.field_switch_s;
        let fill_s = b.field_switch_s + (capacity_j - at_day_j) / (stored_w(1.0) - caps.drain_w);
        Some((server, b.field_switch_s, fill_s))
    }

    #[tokio::test]
    async fn auto_switches_at_the_predicted_instants_stepped_or_in_one_leap_with_no_client() {
        let Some((mut stepped, switch_s, fill_s)) = unwatched(rate_for_tick_s(3_600.0)) else { return };
        let end_s = fill_s + switch_s + 20.0 * DAY_S;
        assert!(stepped.ship(SHIP).unwrap().fitting().unwrap().starlight_w() > 0.0, "premise: in starlight");
        let mut wire = Loopback::new();
        while (stepped.now_t() as f64) < end_s * 1.0e6 {
            stepped.tick(&mut wire).await.unwrap();
        }
        let Some((mut leap, ..)) = unwatched(rate_for_tick_s(end_s + DAY_S)) else { return };
        leap.tick(&mut wire).await.unwrap();
        assert!(leap.now_t() as f64 >= end_s * 1.0e6, "premise: one tick");

        let predicted = [(to_us(switch_s), Shade::Black), (to_us(fill_s + switch_s), Shade::Clear)];
        for (server, name) in [(&stepped, "stepped"), (&leap, "leap")] {
            let got = flips(server);
            assert_eq!(got.len(), 2, "{name}: {got:?}");
            for ((t, shade), (want_t, want)) in got.iter().zip(predicted) {
                assert_eq!(*shade, want, "{name}");
                assert!((t - want_t).abs() <= 1, "{name}: {t} {want_t}");
            }
            let fitting = server.ship(SHIP).unwrap().fitting().unwrap();
            assert_eq!((fitting.posture().shade, fitting.posture().switch), (lc_world::field::Mode::Clear, None), "{name}");
        }
    }

    /// Black in Auto, cooling well short of `clear_above`: a burst past it begins the switch at the
    /// instant it lands.
    #[tokio::test]
    async fn a_burst_past_a_threshold_starts_the_switch_at_once() {
        let Some(mut server) = server() else { return };
        server.set_rate(rate_for_tick_s(3_600.0));
        let mut ship = still(SHIP, DVec3::ZERO);
        server.fit_new(&mut ship);
        let max_j = ship.fitting().unwrap().field().heat_max_j();
        let capacity_j = ship.fitting().unwrap().hull().capacities.storage_j;
        with_account(&mut ship, |a| Account { stored_j: 0.5 * capacity_j, heat_j: 0.45 * max_j, posture: Posture { shade: lc_world::field::Mode::Black, ..a.posture }, ..a });
        server.fleet.insert(ship);
        let mut wire = Loopback::new();
        for _ in 0..3 {
            server.tick(&mut wire).await.unwrap();
        }
        assert_eq!(server.ship(SHIP).unwrap().fitting().unwrap().posture().switch, None, "premise: cooling");

        let landed_s = server.now_t() as f64 * 1.0e-6 - 1_000.0;
        let craft = server.fleet.get_mut(CraftId(SHIP.0)).unwrap();
        craft.settle(landed_s);
        let absorptivity = craft.fitting().unwrap().absorptivity_at(landed_s);
        with_account(craft, |a| Account { heat_j: a.heat_j + lc_world::field::Burst::Arriving(0.1 * max_j).heat_j(absorptivity), ..a });
        server.tick(&mut wire).await.unwrap();
        let switch = server.ship(SHIP).unwrap().fitting().unwrap().posture().switch;
        let balance = server.balance();
        assert_eq!(switch, Some(Switch { to: lc_world::field::Mode::Clear, done_s: landed_s + balance.field_switch_s }));
    }

    /// The mode, the shade and a switch under way come back from a checkpoint, and the flip fires
    /// when it would have.
    #[tokio::test]
    async fn mode_shade_and_switch_survive_a_restart() {
        let Some((mut server, mut wire)) = apart() else { return };
        server.tick(&mut wire).await.unwrap();
        wire.client_says(OWNER, act(Order::FieldMode { mode: FieldMode::Black }));
        server.tick(&mut wire).await.unwrap();
        server.tick(&mut wire).await.unwrap();
        let posture = *server.ship(SHIP).unwrap().fitting().unwrap().posture();
        let done_s = posture.switch.expect("premise: switching").done_s;
        let checkpoint = server.checkpoint();
        drop(server);

        let Some(mut restarted) = self::server() else { return };
        restarted.set_rate(rate_for_tick_s(3_600.0));
        assert!(restarted.adopt(checkpoint).is_empty());
        assert_eq!(*restarted.ship(SHIP).unwrap().fitting().unwrap().posture(), posture);
        let mut wire = Loopback::new();
        while (restarted.now_t() as f64) < (done_s + DAY_S) * 1.0e6 {
            restarted.tick(&mut wire).await.unwrap();
        }
        assert_eq!(flips(&restarted), vec![(to_us(done_s), Shade::Black)]);
    }

    /// An order stamped before a flip taken in the same tick is judged, and stamped, where the
    /// account stands: after the flip.
    #[tokio::test]
    async fn a_lagging_order_lands_after_the_flip_its_tick_took() {
        let Some((mut server, mut wire)) = apart() else { return };
        server.tick(&mut wire).await.unwrap();
        wire.client_says(OWNER, act(Order::FieldMode { mode: FieldMode::Black }));
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(OWNER);
        let done_s = server.ship(SHIP).unwrap().fitting().unwrap().posture().switch.unwrap().done_s;
        while server.now_t() + 3_600_000_000 < to_us(done_s) {
            server.tick(&mut wire).await.unwrap();
        }
        let _ = wire.take(OWNER);
        assert!(server.ship(SHIP).unwrap().fitting().unwrap().posture().switch.is_some(), "premise: not yet taken");
        wire.client_says(OWNER, Inbound::Act(Intent { ship_id: SHIP, order: Order::FieldMode { mode: FieldMode::Clear }, issued_at_client_t: 0 }));
        server.tick(&mut wire).await.unwrap();
        assert_eq!(flips(&server), vec![(to_us(done_s), Shade::Black)], "premise: the flip was taken this tick");
        let said = wire.take(OWNER);
        let Some(at_t) = said.iter().find_map(|m| match m {
            Outbound::Accepted { at_t, .. } => Some(*at_t),
            _ => None,
        }) else {
            panic!("{said:?}")
        };
        assert!(at_t >= to_us(done_s), "{at_t} before the flip at {}", to_us(done_s));
        let switch = server.ship(SHIP).unwrap().fitting().unwrap().posture().switch.unwrap();
        assert_eq!(switch.to, lc_world::field::Mode::Clear);
        assert!((switch.done_s - (at_t as f64 * 1.0e-6 + DAY_S)).abs() < 1.0e-6, "{switch:?} {at_t}");
    }

    #[tokio::test]
    async fn the_console_orders_it_as_the_wire_does() {
        let Some((mut server, mut wire)) = apart() else { return };
        server.directing(true);
        let mut answers = Vec::new();
        for (seq, line) in [(1, "field black"), (2, "field clear")] {
            wire.client_says(OWNER, Inbound::Command { seq, line: line.into() });
            server.tick(&mut wire).await.unwrap();
            answers.extend(wire.take(OWNER).into_iter().filter_map(|m| match m {
                Outbound::Answered { ok, text, .. } => Some((ok, text)),
                _ => None,
            }));
        }
        let [(true, began), (false, refused)] = &answers[..] else { panic!("{answers:?}") };
        assert!(began.contains("Black in 1.0 days"), "{began}");
        assert!(refused.contains("Switching"), "{refused}");
        let switch = server.ship(SHIP).unwrap().fitting().unwrap().posture().switch;
        assert_eq!(switch.map(|s| s.to), Some(lc_world::field::Mode::Black));
    }
}
