use glam::DVec3;
use lc_proto::{Aim, Apertures, ClientId, Inbound, Intent, Lead, Order, Outbound, Refusal, ShipId, Spectrum};
use lc_spacetime::LIGHT_MICROSECOND_M;
use lc_world::craft::{Craft, CraftId};
use lc_world::fitting::{Account, Balance, Fitting, Posture, Setting};
use lc_world::form::Form;
use lc_world::motion::LIGHT_US_PER_LY;

use super::*;
use crate::field::hold_black;
use crate::journal::Memory;
use crate::transport::Loopback;
use crate::world::still;

const EMITTING: ClientId = ClientId(1);
const EMITTER: ShipId = ShipId(1);
const US_PER_M: f64 = 1.0 / LIGHT_MICROSECOND_M;
const LIGHT_HOUR_US: f64 = 3_600.0e6;
const LIGHT_SECOND_US: f64 = 1.0e6;
const HOUR_S: f64 = 3_600.0;

/// The plate with one of its pair turned to fire fore: rated alike at both ends.
fn two_ended() -> Form {
    lc_world::form::presets::turned_fore(lc_world::form::presets::Builtin::Plate.form(), 1)
}

/// Full, Black and at rest `at` light-microseconds.
fn ship(id: i64, at: DVec3, form: Form) -> Craft {
    let mut craft = still(ShipId(id), at);
    craft.fit(Some(Fitting::full(form, Balance::DEFAULT, 0.0)));
    hold_black(&mut craft);
    craft
}

fn account(craft: &mut Craft, change: impl FnOnce(&mut Account)) {
    let fitting = craft.fitting().unwrap().clone();
    let mut account = fitting.account();
    change(&mut account);
    craft.fit(Some(Fitting::from_account(&account, *fitting.balance())));
}

fn emit(aim: Aim, apertures: Apertures, power_w: f64, wavelength_m: f64, spread_rad: f64, duration_s: f64) -> Inbound {
    leading(aim, apertures, power_w, wavelength_m, spread_rad, duration_s, Lead::Coasting)
}

fn leading(aim: Aim, apertures: Apertures, power_w: f64, wavelength_m: f64, spread_rad: f64, duration_s: f64, lead: Lead) -> Inbound {
    let order = Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s, lead };
    Inbound::Act(Intent { ship_id: EMITTER, order, issued_at_client_t: i64::MAX })
}

fn along(axis: DVec3) -> Aim {
    Aim::Bearing(axis.to_array())
}

/// Every beam a client was told of: `(beam, power_w, arrive_t)`.
fn illuminated(said: &[Outbound]) -> Vec<(i64, f64, i64)> {
    said.iter()
        .filter_map(|m| match m {
            Outbound::Illuminated { beam, power_w, arrive_t, .. } => Some((*beam, *power_w, *arrive_t)),
            _ => None,
        })
        .collect()
}

fn now_s(server: &Server<Memory>) -> f64 {
    server.now_t() as f64 * 1.0e-6
}

fn heat_j(server: &Server<Memory>, id: i64) -> f64 {
    let craft = server.ship(ShipId(id)).unwrap();
    craft.fitting().unwrap().heat_j_at(&craft.motion, now_s(server))
}

fn stored_j(server: &Server<Memory>, id: i64) -> f64 {
    let craft = server.ship(ShipId(id)).unwrap();
    craft.fitting().unwrap().stored_j_at(&craft.motion, now_s(server))
}

fn lit_w(server: &Server<Memory>, id: i64) -> f64 {
    server.ship(ShipId(id)).unwrap().fitting().unwrap().lit_w()
}

/// The emitting events `id` has written, oldest first: `(t, emission)`.
fn emissions_of(server: &Server<Memory>, id: ShipId) -> Vec<(i64, Emitted)> {
    server.journal().events.iter().filter(|e| e.kind == KIND_EMIT && e.source == id).map(|e| (e.t, emitted(&e.payload).unwrap())).collect()
}

async fn until<J: Journal>(server: &mut Server<J>, wire: &mut Loopback, t: f64) {
    while (server.now_t() as f64) < t {
        server.tick(wire).await.unwrap();
    }
}

/// A light-hour off, a craft inside the cone is fed from the instant its light arrives and not
/// before, until the light of its going out lands, and sees the emitter's glare meanwhile. One
/// just outside is told nothing and fed nothing.
#[tokio::test]
async fn a_craft_in_the_cone_is_fed_at_its_light_delay_and_one_just_outside_is_not() {
    let (half, power_w, duration_s) = (0.01, 1.0e17, 3.0 * HOUR_S);
    let inside = DVec3::new(LIGHT_HOUR_US, 0.9 * half * LIGHT_HOUR_US, 0.0);
    let outside = DVec3::new(LIGHT_HOUR_US, 1.1 * half * LIGHT_HOUR_US, 0.0);
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, inside, Form::starting()), 0.0);
    server.admit(ClientId(3), ship(3, outside, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, power_w, 1.0e-6, half, duration_s));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));

    let (lit_t, lit) = emissions_of(&server, EMITTER).into_iter().find(|(_, e)| e.axis[0] > 0.0).expect("lit this tick");
    let arrive_t = (lit_t as f64 + inside.length()).ceil() as i64;
    let out_t = lit_t + (duration_s * 1.0e6) as i64;
    let (mut told, mut glared, mut stray) = (Vec::new(), Vec::new(), Vec::new());
    while (server.now_t() as f64) < out_t as f64 + inside.length() + 2.0 * crate::server::TICK_US as f64 {
        server.tick(&mut wire).await.unwrap();
        let now = server.now_t();
        let said = wire.take(ClientId(2));
        told.extend(illuminated(&said).into_iter().map(|beam| (now, beam)));
        for message in said {
            if let Outbound::Present(list) = message {
                glared.extend(list.iter().filter(|p| p.get().ship_id == EMITTER).filter_map(|p| p.get().glare).map(|g| (now, g)));
            }
        }
        if now < arrive_t {
            assert_eq!(lit_w(&server, 2), 0.0, "fed before its light arrived");
        }
        let said = wire.take(ClientId(3));
        stray.extend(illuminated(&said).into_iter().map(|(beam, ..)| beam));
        for message in said {
            if let Outbound::Present(list) = message {
                stray.extend(list.iter().filter(|p| p.get().glare.is_some()).map(|p| p.get().ship_id.0));
            }
        }
        assert_eq!(lit_w(&server, 3), 0.0, "fed from outside the cone");
    }
    let [(on_told_t, (beam, on_w, on_t)), (_, (_, off_w, off_t))] = told[..] else { panic!("{told:?}") };
    assert_eq!((beam, on_t), (lit.beam, arrive_t));
    assert!(on_told_t >= arrive_t);
    let receiver = server.ship(ShipId(2)).unwrap();
    let shadow_m2 = crate::field::shadow_toward_m2(receiver, -inside, arrive_t as f64 * 1.0e-6);
    let want_w = power_w * lc_world::emit::received_fraction(half, shadow_m2, inside.length() * LIGHT_MICROSECOND_M);
    assert!((on_w - want_w).abs() < 1.0e-9 * want_w && want_w > 0.0, "{on_w} {want_w}");
    assert_eq!((off_w, off_t), (0.0, (out_t as f64 + inside.length()).ceil() as i64));
    assert_eq!(lit_w(&server, 2), 0.0, "still fed after it went out");
    assert!(stray.is_empty(), "{stray:?}");

    let flux = lc_world::emit::flux_w_m2(power_w, half, inside.length() * LIGHT_MICROSECOND_M);
    assert!(!glared.is_empty() && glared.iter().all(|(t, _)| *t >= arrive_t && *t < off_t));
    for (_, glare) in glared {
        assert_eq!(glare.spectrum, Spectrum::Line { wavelength_m: 1.0e-6 });
        assert!((glare.flux_w_m2 - flux).abs() < 1.0e-9 * flux, "{} {flux}", glare.flux_w_m2);
    }
}

/// Aimed at a craft a light-hour off from its sighting, the beam lands on it if it holds still,
/// and misses it if it moves off after the beam left.
#[tokio::test]
async fn an_aimed_beam_misses_a_target_that_maneuvered_after_it_left() {
    for maneuvers in [false, true] {
        let target = DVec3::X * LIGHT_HOUR_US;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
        server.admit(ClientId(2), ship(2, target, Form::starting()), 0.0);
        let mut wire = Loopback::new();
        until(&mut server, &mut wire, 1.5 * LIGHT_HOUR_US).await;
        wire.client_says(EMITTING, emit(Aim::Ship(ShipId(2)), Apertures::Both, 1.0e17, 1.0e-9, 1.0e-7, 3.0 * HOUR_S));
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
        let lit_t = emissions_of(&server, EMITTER)[0].0;
        server.tick(&mut wire).await.unwrap();
        if maneuvers {
            let now_t = server.now_t();
            let craft = server.fleet.get_mut(CraftId(2)).unwrap();
            let here = craft.position_at(now_t as f64) / LIGHT_US_PER_LY;
            craft.drift_from(here, DVec3::Y * 1.0e-3, now_t as f64 * 1.0e-6);
        }
        assert!(server.now_t() < lit_t + target.length() as i64, "premise: it moved before the light arrived");
        until(&mut server, &mut wire, lit_t as f64 + target.length() + 1.0e9).await;
        let told = illuminated(&wire.take(ClientId(2)));
        assert_eq!(told.is_empty(), maneuvers, "maneuvered {maneuvers}: {told:?}");
        assert_eq!(lit_w(&server, 2) > 0.0, !maneuvers);
    }
}

/// A light-minute off and burning at 5 g across the line of sight, a target is hit by a beam led
/// along its burn and missed by one led as though it coasted, by `½ a (2d/c)²`, about 350 km,
/// against a spot a few kilometers across.
#[tokio::test]
async fn a_beam_led_along_a_steady_burn_lands_and_one_led_coasting_misses() {
    const LIGHT_MINUTE_US: f64 = 60.0e6;
    for lead in [Lead::Burning, Lead::Coasting] {
        let target = DVec3::X * LIGHT_MINUTE_US;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
        let mut burning = ship(2, target, Form::starting());
        let from_ly = target / LIGHT_US_PER_LY;
        let boost = lc_world::emit::Boost::plan(from_ly, DVec3::ZERO, 0.0, DVec3::Y, DVec3::Y, 5.0, 1.0e6, 0.01, DVec3::Y, 0.05);
        assert_eq!(boost.turn_s(), 0.0, "premise: lit from the start");
        burning.boost(boost, 0.0);
        server.admit(ClientId(2), burning, 0.0);
        let mut wire = Loopback::new();
        until(&mut server, &mut wire, 1.5 * LIGHT_MINUTE_US).await;
        wire.client_says(EMITTING, leading(Aim::Ship(ShipId(2)), Apertures::Both, 1.0e17, 1.0e-9, 3.0e-8, HOUR_S, lead));
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })), "{lead:?}");
        let lit_t = emissions_of(&server, EMITTER)[0].0;
        until(&mut server, &mut wire, lit_t as f64 + 1.2 * LIGHT_MINUTE_US).await;
        let told = illuminated(&wire.take(ClientId(2)));
        assert_eq!(!told.is_empty(), lead == Lead::Burning, "{lead:?}: {told:?}");
    }
}

/// Fore and aft at equal power is no push: the emitter is where a twin that did nothing is.
#[tokio::test]
async fn a_balanced_emit_leaves_the_worldline_alone() {
    let mut server = Server::new(Memory::default(), 0, 1);
    let emitter = ship(1, DVec3::ZERO, two_ended());
    let twin = emitter.clone();
    server.admit(EMITTING, emitter, 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e19, 1.0e-6, 0.01, HOUR_S));
    until(&mut server, &mut wire, 2.0 * HOUR_S * 1.0e6).await;
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    assert_eq!(emissions_of(&server, EMITTER).len(), 4, "premise: both ends lit and went out");
    let craft = server.ship(EMITTER).unwrap();
    assert_eq!(craft.motion.motive, twin.motion.motive);
    for t in [0.0, 0.5 * HOUR_S * 1.0e6, 2.0 * HOUR_S * 1.0e6] {
        assert_eq!(craft.position_at(t), twin.position_at(t), "at {t}");
    }
}

/// From one end alone the emit is a burn along `-aim`, and the rapidity is what the rocket law
/// gives for the energy it spent, heat and storage together.
#[tokio::test]
async fn recoil_points_away_from_the_beam() {
    let (power_w, duration_s) = (1.0e19, 600.0);
    let mut server = Server::new(Memory::default(), 0, 1);
    let emitter = ship(1, DVec3::ZERO, Form::starting());
    let twin = emitter.clone();
    server.admit(EMITTING, emitter, 0.0);
    let mut wire = Loopback::new();
    let aim = DVec3::new(-1.0, 0.2, 0.0).normalize();
    wire.client_says(EMITTING, emit(along(aim), Apertures::Aft, power_w, 1.0e-6, 0.01, duration_s));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    while emissions_of(&server, EMITTER).is_empty() {
        server.tick(&mut wire).await.unwrap();
    }
    let lit_t = emissions_of(&server, EMITTER)[0].0;
    let mass_kg = twin.mass_kg_at(lit_t as f64 * 1.0e-6);
    until(&mut server, &mut wire, lit_t as f64 + duration_s * 1.0e6 + 1.0).await;

    let craft = server.ship(EMITTER).unwrap();
    let beta = craft.motion.beta;
    assert!((beta.normalize() + aim).length() < 1.0e-9, "{beta} against {aim}");
    let mut twin = twin;
    twin.settle(now_s(&server));
    let held = |c: &Craft| c.fitting().unwrap().heat_j_at(&c.motion, now_s(&server)) + c.fitting().unwrap().stored_j_at(&c.motion, now_s(&server));
    let spent_j = held(&twin) - held(craft);
    let rapidity = beta.length().atanh();
    let want_j = lc_world::cost::energy_j(mass_kg, rapidity, Balance::DEFAULT.drive_efficiency);
    assert!((spent_j - want_j).abs() < 2.0e-3 * want_j, "spent {spent_j}, the rocket law {want_j}");
    assert!((want_j / (power_w * duration_s) - 1.0).abs() < 1.0e-3, "premise: the power it was asked for");
}

/// A refit lights nothing, and an emit is refused until it is done.
#[tokio::test]
async fn an_emit_during_a_refit_is_refused() {
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    let mut shrunk = Form::starting();
    shrunk.parts.iter_mut().find(|p| p.id == lc_world::form::PartId(2)).unwrap().volume_m3 *= 3.0 / 5.0;
    wire.client_says(EMITTING, Inbound::Act(Intent { ship_id: EMITTER, order: Order::Refit { target: (&shrunk).into() }, issued_at_client_t: i64::MAX }));
    server.tick(&mut wire).await.unwrap();
    assert!(server.ship(EMITTER).unwrap().is_refitting(now_s(&server)), "premise: refitting");
    wire.client_says(EMITTING, emit(along(-DVec3::X), Apertures::Aft, 1.0e15, 1.0e-6, 0.01, 60.0));
    server.tick(&mut wire).await.unwrap();
    let said = wire.take(EMITTING);
    assert!(said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::Refitting, .. })), "{said:?}");
    assert!(emissions_of(&server, EMITTER).is_empty() && !server.emissions.is_emitting(CraftId(1)));
}

/// Asked for no spread at all, a beam gets its aperture's diffraction floor, and says so.
#[tokio::test]
async fn a_spread_under_the_diffraction_floor_is_widened_to_it() {
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(-DVec3::X), Apertures::Aft, 1.0e15, 1.0e-6, 0.0, 60.0));
    server.tick(&mut wire).await.unwrap();
    let face = lc_world::form::capacity::ends(&Form::starting(), &Balance::DEFAULT).unwrap().1;
    let floor = 1.0e-6 / face.diameter_m / 2.0;
    let said = wire.take(EMITTING);
    let spread = said.iter().find_map(|m| match m {
        Outbound::Accepted { order: Order::Emit { spread_rad, .. }, .. } => Some(*spread_rad),
        _ => None,
    });
    assert_eq!(spread, Some(floor), "{said:?}");
    assert_eq!(emissions_of(&server, EMITTER)[0].1.half_angle_rad, floor);
    wire.client_says(EMITTING, emit(along(-DVec3::X), Apertures::Both, 1.0e15, 1.0e-6, 0.0, 60.0));
    wire.client_says(EMITTING, emit(along(-DVec3::X), Apertures::Aft, 1.0e30, 1.0e-6, 0.0, 60.0));
    server.tick(&mut wire).await.unwrap();
    let said = wire.take(EMITTING);
    let refused = said.iter().filter(|m| matches!(m, Outbound::Refused { reason: Refusal::UnderWay, .. })).count();
    assert_eq!(refused, 2, "a second emit while the first burns: {said:?}");
}

/// Both ends asked of a ship with engines aft only, or more than an end is rated for, is refused.
#[tokio::test]
async fn an_emit_past_the_apertures_is_refused() {
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    let rating_w = lc_world::form::capacity::ends(&Form::starting(), &Balance::DEFAULT).unwrap().1.rating_w;
    for (apertures, power_w, reason) in [
        (Apertures::Both, 1.0e15, Refusal::NoAperture),
        (Apertures::Fore, 1.0e15, Refusal::NoAperture),
        (Apertures::Aft, 1.01 * rating_w, Refusal::OverRating),
    ] {
        wire.client_says(EMITTING, emit(along(-DVec3::X), apertures, power_w, 1.0e-6, 0.01, 60.0));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(EMITTING);
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { reason: r, .. } if *r == reason)), "{apertures:?}: {said:?}");
    }
    assert!(emissions_of(&server, EMITTER).is_empty());
}

/// A dump from a hot ship comes out of its heat: storage ends where a twin's does, the heat is
/// lower by what was emitted, and nothing is left committed.
#[tokio::test]
async fn the_emitters_heat_is_drawn_first() {
    let (power_w, duration_s) = (1.0e19, 1_800.0);
    let mut server = Server::new(Memory::default(), 0, 1);
    let mut emitter = ship(1, DVec3::ZERO, two_ended());
    let heat_max_j = emitter.fitting().unwrap().field().heat_max_j();
    account(&mut emitter, |a| a.heat_j = 0.5 * heat_max_j);
    assert!(2.0 * power_w * duration_s < 0.25 * heat_max_j, "premise: heat covers it");
    let twin = emitter.clone();
    server.admit(EMITTING, emitter, 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, power_w, 1.0e-6, 0.01, duration_s));
    server.tick(&mut wire).await.unwrap();
    let lit_s = emissions_of(&server, EMITTER)[0].0 as f64 * 1.0e-6;
    until(&mut server, &mut wire, (lit_s + 3.0 * duration_s) * 1.0e6).await;

    let mut twin = twin;
    twin.settle(now_s(&server));
    let twin_heat_j = twin.fitting().unwrap().heat_j_at(&twin.motion, now_s(&server));
    let twin_stored_j = twin.fitting().unwrap().stored_j_at(&twin.motion, now_s(&server));
    assert!((stored_j(&server, 1) - twin_stored_j).abs() < 1.0e-12 * twin_stored_j, "storage paid for it");
    let tau_s = Balance::DEFAULT.field_tau_s;
    let emitted_j = 2.0 * power_w * duration_s;
    let drop_j = twin_heat_j - heat_j(&server, 1);
    assert!((drop_j - emitted_j).abs() < 2.0 * (3.0 * duration_s / tau_s) * emitted_j, "{drop_j} {emitted_j}");
    let craft = server.ship(EMITTER).unwrap();
    assert_eq!(craft.fitting().unwrap().committed_j_at(&craft.motion, now_s(&server)), 0.0);
}

/// Between two receivers alike but for their shade, beamed alike with storage full, the Clear one
/// heats by `clear_absorptivity` of what the Black one does.
#[tokio::test]
async fn a_clear_receiver_takes_thirty_percent() {
    let mut rose = Vec::new();
    for setting in [Setting::Black, Setting::Clear] {
        let at = DVec3::X * LIGHT_SECOND_US;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
        let mut receiver = ship(2, at, Form::starting());
        let mut fitting = receiver.fitting().unwrap().clone();
        let shade = if setting == Setting::Clear { lc_world::field::Mode::Clear } else { lc_world::field::Mode::Black };
        fitting.set_posture(Posture { setting, shade, switch: None });
        receiver.fit(Some(fitting));
        let twin = receiver.clone();
        server.fleet.insert(receiver);
        let mut wire = Loopback::new();
        wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e18, 1.0e-9, 0.0, 3.0 * HOUR_S));
        until(&mut server, &mut wire, 2.0 * HOUR_S * 1.0e6).await;
        assert!(lit_w(&server, 2) > 0.9e18, "premise: fed nearly all of it");
        let mut twin = twin;
        twin.settle(now_s(&server));
        let fitting = twin.fitting().unwrap();
        let held_j = fitting.heat_j_at(&twin.motion, now_s(&server)) + fitting.stored_j_at(&twin.motion, now_s(&server));
        rose.push(heat_j(&server, 2) + stored_j(&server, 2) - held_j);
    }
    // Heat and storage together: conversion refills the room the drain made, alike in both.
    let ratio = rose[1] / rose[0];
    assert!((ratio - Balance::DEFAULT.clear_absorptivity).abs() < 1.0e-4, "{ratio}: {rose:?}");
}

/// Beamed under its rating into storage with room, a receiver keeps `η` of it and heats by the
/// rest. The same energy as a burst is all heat.
#[tokio::test]
async fn a_beam_into_room_adds_only_the_conversion_loss_and_a_burst_adds_all_of_it() {
    let (power_w, duration_s) = (1.0e18, HOUR_S);
    let at = DVec3::X * LIGHT_SECOND_US;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    let mut receiver = ship(2, at, Form::starting());
    let capacity_j = receiver.fitting().unwrap().hull().capacities.storage_j;
    account(&mut receiver, |a| a.stored_j = 0.1 * capacity_j);
    assert!(power_w < receiver.fitting().unwrap().hull().capacities.aperture_w, "premise: under its rating");
    let mut burst_to = ship(3, DVec3::Y * LIGHT_SECOND_US, Form::starting());
    account(&mut burst_to, |a| a.stored_j = 0.1 * capacity_j);
    let twin = receiver.clone();
    server.fleet.insert(receiver);
    server.fleet.insert(burst_to);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, power_w, 1.0e-9, 0.0, duration_s));
    server.tick(&mut wire).await.unwrap();
    let lit_t = emissions_of(&server, EMITTER)[0].0;
    let (energy_j, burst_t) = (power_w * duration_s, lit_t + (duration_s * 1.0e6) as i64);
    let pulse = Emitted { burst_j: energy_j, power_w: 0.0, axis: DVec3::Y.to_array(), ..emissions_of(&server, EMITTER)[0].1 };
    server.emissions.landings.push(Landing { observer: CraftId(3), arrive_t: burst_t + LIGHT_SECOND_US as i64, source: EMITTER, emitted: pulse, said_t: burst_t });
    until(&mut server, &mut wire, burst_t as f64 + 2.0 * LIGHT_SECOND_US + 1.0).await;
    assert_eq!(lit_w(&server, 2), 0.0, "premise: it went out");

    let mut twin = twin;
    twin.settle(now_s(&server));
    let (twin_heat_j, twin_stored_j) = (twin.fitting().unwrap().heat_j_at(&twin.motion, now_s(&server)), twin.fitting().unwrap().stored_j_at(&twin.motion, now_s(&server)));
    let eta = Balance::DEFAULT.conversion_efficiency;
    let decay = 2.0 * (now_s(&server) - lit_t as f64 * 1.0e-6) / Balance::DEFAULT.field_tau_s;
    let beamed_rose_j = heat_j(&server, 2) - twin_heat_j;
    assert!((beamed_rose_j / ((1.0 - eta) * energy_j) - 1.0).abs() < decay, "beamed heat {beamed_rose_j}");
    assert!(((stored_j(&server, 2) - twin_stored_j) / (eta * energy_j) - 1.0).abs() < 1.0e-6, "stored");
    let burst_rose_j = heat_j(&server, 3) - twin_heat_j;
    assert!((burst_rose_j / energy_j - 1.0).abs() < decay, "burst heat {burst_rose_j}");
    assert_eq!(stored_j(&server, 3), twin_stored_j, "a burst converted into storage");
}

/// A transmission is charged its power for a second, from storage, whatever heat there is. Loud
/// enough that a full store can tell: a message's megawatt-second is under its rounding.
#[tokio::test]
async fn radio_is_charged_from_storage() {
    let mut server = Server::new(Memory::default(), 0, 1);
    let emitter = ship(1, DVec3::ZERO, Form::starting());
    let twin = emitter.clone();
    server.admit(EMITTING, emitter, 0.0);
    let mut wire = Loopback::new();
    let power_w = 1.0e20;
    wire.client_says(EMITTING, Inbound::Act(Intent { ship_id: EMITTER, order: Order::Transmit { power_w }, issued_at_client_t: i64::MAX }));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    let mut twin = twin;
    twin.settle(now_s(&server));
    let paid_j = twin.fitting().unwrap().stored_j_at(&twin.motion, now_s(&server)) - stored_j(&server, 1);
    let want_j = power_w * crate::radio::TRANSMISSION_S;
    assert!((paid_j - want_j).abs() < 1.0e-3 * want_j, "{paid_j} {want_j}");
}

/// The shard as it comes back from `server`'s checkpoint and journal.
#[cfg(feature = "storage")]
async fn restart(server: &Server<Memory>, rate: f64, resume: bool) -> Server<Memory> {
    let journal = Memory { events: server.journal().events.clone(), deliveries: server.journal().deliveries.clone(), ..Default::default() };
    let mut restarted = Server::new(journal, 0, 2);
    restarted.set_rate(rate);
    assert!(restarted.adopt(server.checkpoint()).is_empty());
    if resume {
        restarted.resume_landings().await.unwrap();
    }
    restarted
}

/// Restarted with a collapse's spike and a beam both in flight, the shard lands each at its
/// arrival: the spike kills a neighbor inside its lethal radius, and the beam feeds its target. A
/// shard that did not rebuild landings from the journal loses the spike.
#[cfg(feature = "storage")]
#[tokio::test]
async fn a_spike_and_a_beam_in_flight_across_a_restart_still_land() {
    // Ticks of 44 µs, so both lights are still on their way when the first tick ends.
    const RATE: f64 = 1.0e-7;
    let hot = 1.0 - 1.0e-6;
    let mut dying = ship(10, DVec3::ZERO, Form::starting());
    let heat_max_j = dying.fitting().unwrap().field().heat_max_j();
    account(&mut dying, |a| a.heat_j = heat_max_j);
    let fitting = dying.fitting().unwrap();
    let spike_j = Balance::DEFAULT.collapse_spike_fraction * fitting.field().released_j(fitting.hull().capacities.storage_j);
    let probe = ship(11, DVec3::Y, Form::starting());
    let headroom_j = (1.0 - hot) * heat_max_j;
    let shadow_m2 = crate::field::shadow_toward_m2(&probe, -DVec3::Y, 0.0);
    let lethal_us = lc_world::field::lethal_radius_m(1.0, spike_j, shadow_m2, headroom_j) * US_PER_M;
    let victim_at = DVec3::Y * 0.5 * lethal_us;
    let mut victim = ship(11, victim_at, Form::starting());
    account(&mut victim, |a| a.heat_j = hot * heat_max_j);
    let (emitter_at, target_at) = (DVec3::new(0.0, -3.0e4, 0.0), DVec3::new(300.0, -3.0e4, 0.0));

    for resume in [true, false] {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(RATE);
        server.fleet.insert(dying.clone());
        server.fleet.insert(victim.clone());
        server.admit(EMITTING, ship(1, emitter_at, two_ended()), 0.0);
        server.fleet.insert(ship(2, target_at, Form::starting()));
        let mut wire = Loopback::new();
        wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e17, 1.0e-9, 0.0, 1.0));
        server.tick(&mut wire).await.unwrap();
        let died_t = server.journal().events.iter().find(|e| e.kind == KIND_COLLAPSE && e.source == ShipId(10)).expect("premise: it collapsed").t;
        let lit_t = emissions_of(&server, EMITTER)[0].0;
        let (spike_t, beam_t) = ((died_t as f64 + victim_at.length()).ceil() as i64, (lit_t as f64 + 300.0).ceil() as i64);
        assert!(spike_t > server.now_t() && beam_t > server.now_t(), "premise: both still in flight");

        let mut restarted = restart(&server, RATE, resume).await;
        drop(server);
        let target = restarted.ship(ShipId(2)).unwrap().clone();
        restarted.admit(ClientId(2), target, 0.0);
        let mut told = Vec::new();
        while restarted.now_t() < spike_t.max(beam_t) + 100 {
            restarted.tick(&mut wire).await.unwrap();
            told.extend(illuminated(&wire.take(ClientId(2))));
        }
        let victim_died = restarted.journal().events.iter().find(|e| e.kind == KIND_COLLAPSE && e.source == ShipId(11)).map(|e| e.t);
        if resume {
            assert_eq!(victim_died, Some(spike_t), "the spike landed off its arrival");
        } else {
            assert_eq!(victim_died, None, "premise: only the journal brings the spike back");
        }
        // A beam near its emitter is followed from what the emitter said, which its checkpoint
        // keeps, whether or not the journal is read.
        assert!(matches!(told[..], [(_, w, t)] if w > 0.0 && t == beam_t), "resumed {resume}: {told:?}");
        assert!(lit_w(&restarted, 2) > 0.0);
    }
}

/// A beam that has landed stays on across a restart, and goes out when the light of its end
/// arrives: what a craft has lit and what lands on it come back with the checkpoint.
#[cfg(feature = "storage")]
#[tokio::test]
async fn a_beam_landing_across_a_restart_stays_on_until_its_end_arrives() {
    let at = DVec3::X * LIGHT_SECOND_US;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, at, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e17, 1.0e-9, 0.0, 3.0 * HOUR_S));
    server.tick(&mut wire).await.unwrap();
    server.tick(&mut wire).await.unwrap();
    let beamed_w = lit_w(&server, 2);
    assert!(beamed_w > 0.0, "premise: landed before the restart");
    let (lit_t, lit) = emissions_of(&server, EMITTER).into_iter().find(|(_, e)| e.axis[0] > 0.0).unwrap();
    assert!(matches!(illuminated(&wire.take(ClientId(2)))[..], [(beam, _, _)] if beam == lit.beam));

    let mut restarted = restart(&server, 1.0, true).await;
    drop(server);
    let target = restarted.ship(ShipId(2)).unwrap().clone();
    restarted.admit(ClientId(2), target, 0.0);
    restarted.tick(&mut wire).await.unwrap();
    assert_eq!(lit_w(&restarted, 2), beamed_w, "the restart put it out");
    let out_t = lit_t + (3.0 * HOUR_S * 1.0e6) as i64;
    until(&mut restarted, &mut wire, out_t as f64 + at.length() + 1.0).await;
    let told = illuminated(&wire.take(ClientId(2)));
    assert_eq!(told, vec![(lit.beam, 0.0, (out_t as f64 + at.length()).ceil() as i64)], "{told:?}");
    assert_eq!(lit_w(&restarted, 2), 0.0);
}

/// Cutting the drive puts out whatever is lit, well before its end: the receiver is told when the
/// light of the cut arrives, and the draw stops there.
#[tokio::test]
async fn cutting_the_drive_puts_a_beam_out_early() {
    let at = DVec3::X * LIGHT_SECOND_US;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, at, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e17, 1.0e-9, 0.0, 10.0 * HOUR_S));
    server.tick(&mut wire).await.unwrap();
    server.tick(&mut wire).await.unwrap();
    assert!(lit_w(&server, 2) > 0.0, "premise: landed");
    wire.client_says(EMITTING, Inbound::Act(Intent { ship_id: EMITTER, order: Order::CutDrive, issued_at_client_t: i64::MAX }));
    server.tick(&mut wire).await.unwrap();
    let cut_t = server.now_t();
    server.tick(&mut wire).await.unwrap();
    let told = illuminated(&wire.take(ClientId(2)));
    assert!(matches!(told[..], [(_, on, _), (_, 0.0, off)] if on > 0.0 && off == (cut_t as f64 + at.length()).ceil() as i64), "{told:?}");
    assert_eq!(lit_w(&server, 2), 0.0);
    let craft = server.ship(EMITTER).unwrap();
    assert!(craft.fitting().unwrap().lit().is_empty() && !server.emissions.is_emitting(CraftId(1)));
    assert_eq!(craft.fitting().unwrap().committed_j_at(&craft.motion, now_s(&server)), 0.0);
}

/// Through the store a shard really comes back from: its position rounded to the grid, and no
/// power stored, a beam in flight still lands where and when it would have.
#[cfg(feature = "storage")]
#[tokio::test]
async fn a_beam_in_flight_comes_back_through_the_store() {
    use crate::journal::Postgres;
    const EMITTER_ID: i64 = 30_100;
    const TARGET_ID: i64 = 30_101;
    let Ok(journal) = Postgres::open().await else { return };
    for (table, column) in [("deliveries", "observer_id"), ("events", "source_id")] {
        let sql = format!("DELETE FROM {table} WHERE {column} IN ($1, $2)");
        journal.client().execute(sql.as_str(), &[&EMITTER_ID, &TARGET_ID]).await.unwrap();
    }
    let target_at = DVec3::new(LIGHT_HOUR_US, 0.3, 0.7);
    let mut server = Server::new(journal, 0, 11);
    server.admit(EMITTING, ship(EMITTER_ID, DVec3::new(0.4, 0.0, 0.0), two_ended()), 0.0);
    server.fleet.insert(ship(TARGET_ID, target_at, Form::starting()));
    let mut wire = Loopback::new();
    let order = Order::Emit { aim: along(DVec3::X), apertures: Apertures::Both, power_w: 1.0e17, wavelength_m: 1.0e-9, spread_rad: 1.0e-3, duration_s: 3.0 * HOUR_S, lead: Lead::Coasting };
    wire.client_says(EMITTING, Inbound::Act(Intent { ship_id: ShipId(EMITTER_ID), order, issued_at_client_t: i64::MAX }));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    let checkpoint = server.checkpoint();
    let lit_t = server.now_t();
    let arrive_t = (lit_t as f64 + (target_at - DVec3::new(0.4, 0.0, 0.0)).length()).ceil() as i64;
    drop(server);

    let Ok(journal) = Postgres::open().await else { return };
    let mut restarted = Server::new(journal, 0, 12);
    assert!(restarted.adopt(checkpoint).is_empty());
    restarted.resume_landings().await.unwrap();
    let target = restarted.ship(ShipId(TARGET_ID)).unwrap().clone();
    restarted.admit(ClientId(2), target, 0.0);
    until(&mut restarted, &mut wire, arrive_t as f64 + 1.0).await;
    let told = illuminated(&wire.take(ClientId(2)));
    assert!(matches!(told[..], [(_, w, t)] if w > 0.0 && t == arrive_t), "{told:?}, due at {arrive_t}");
}

/// One rating bounds the drive and what is emitted: while a balanced emit is lit, nothing lights
/// the drive, and a standing intercept is dropped rather than left to re-plan a burn.
#[tokio::test]
async fn nothing_lights_the_drive_while_a_balanced_emit_runs() {
    use lc_proto::{Approach, Closeness};
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.fleet.insert(ship(2, DVec3::Y * LIGHT_SECOND_US, Form::starting()));
    server.pursuits.insert(CraftId(1), crate::chase::Pursuit {
        quarry: ShipId(2),
        closeness: lc_world::pursuit::Closeness::Company,
        approach: Approach::Courteous,
        last_plan_t: i64::MIN,
        last_seen: None,
    });
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, 1.0e17, 1.0e-6, 0.01, HOUR_S));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    assert!(server.pursuits.is_empty(), "the intercept outlived the emit");
    let before = server.ship(EMITTER).unwrap().clone();
    let act = |order| Inbound::Act(Intent { ship_id: EMITTER, order, issued_at_client_t: i64::MAX });
    wire.client_says(EMITTING, act(Order::Burn { beta: [1.0e-5, 0.0, 0.0] }));
    wire.client_says(EMITTING, act(Order::Intercept { ship_id: ShipId(2), closeness: Closeness::Company, approach: Approach::Direct }));
    server.tick(&mut wire).await.unwrap();
    let said = wire.take(EMITTING);
    let refused = said.iter().filter(|m| matches!(m, Outbound::Refused { reason: Refusal::UnderWay, .. })).count();
    assert_eq!(refused, 2, "{said:?}");
    let craft = server.ship(EMITTER).unwrap();
    assert_eq!(craft.motion.motive, before.motion.motive);
    assert_eq!(craft.fitting().unwrap().lit(), before.fitting().unwrap().lit());
    assert!(server.pursuits.is_empty());
}

/// What `observer` was told of the emitter each tick: `(arrive_t, emitted_t, fore, aft, drive)`.
async fn watch(server: &mut Server<Memory>, wire: &mut Loopback, observer: ClientId, until_t: f64) -> Vec<(i64, i64, f64, f64, f64)> {
    let mut seen = Vec::new();
    while (server.now_t() as f64) < until_t {
        server.tick(wire).await.unwrap();
        for message in wire.take(observer) {
            if let Outbound::Present(list) = message {
                let of = list.iter().map(|p| p.get()).filter(|p| p.ship_id == EMITTER);
                seen.extend(of.map(|p| (p.arrive_t, p.emitted_t, p.emit_fore_w, p.emit_aft_w, p.drive_w)));
            }
        }
    }
    seen
}

/// A light-hour off, a balanced emit is stated at both ends from when its light arrives until the
/// light of its going out does, long after the emitter has put it out.
#[tokio::test]
async fn a_balanced_emit_is_stated_at_both_ends_as_its_light_left() {
    let (power_w, duration_s) = (1.0e17, 3.0 * HOUR_S);
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, DVec3::Y * LIGHT_HOUR_US, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, power_w, 1.0e-6, 0.01, duration_s));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    let (lit_t, _) = emissions_of(&server, EMITTER)[0];
    let out_t = lit_t + (duration_s * 1.0e6) as i64;

    let seen = watch(&mut server, &mut wire, ClientId(2), out_t as f64 + 2.0 * LIGHT_HOUR_US).await;
    let lit = |t: i64| lit_t <= t && t < out_t;
    for &(_, emitted_t, fore, aft, drive) in &seen {
        let want = if lit(emitted_t) { power_w } else { 0.0 };
        assert_eq!((fore, aft, drive), (want, want, 0.0), "left at {emitted_t}");
    }
    assert!(seen.iter().any(|s| !lit(s.1)) && seen.iter().any(|s| lit(s.1)), "premise: seen dark and lit");
    assert!(seen.iter().any(|s| lit(s.1) && s.0 > out_t), "premise: seen lit after it went out");
}

/// An emit flown as a burn from the bow is stated at the bow alone, at the rocket law's throttle,
/// and is not the drive.
#[tokio::test]
async fn a_burn_emit_is_stated_at_the_end_it_leaves() {
    let (power_w, duration_s) = (1.0e17, 3.0 * HOUR_S);
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, DVec3::Y * LIGHT_SECOND_US, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Fore, power_w, 1.0e-6, 0.01, duration_s));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));

    let seen = watch(&mut server, &mut wire, ClientId(2), 12.0 * HOUR_S * 1.0e6).await;
    let lit: Vec<_> = seen.iter().filter(|s| s.2 > 0.0).collect();
    assert!(!lit.is_empty(), "premise: seen lit");
    for &&(_, emitted_t, fore, aft, drive) in &lit {
        assert!((fore / power_w - 1.0).abs() < 0.01, "{fore} W at {emitted_t}");
        assert_eq!((aft, drive), (0.0, 0.0));
    }
    assert!(seen.last().is_some_and(|s| s.2 == 0.0), "it went out");
}

/// Aimed at a craft 30 km off with the spread left at its floor, as the emit window sends it, the
/// beam lands and stays on until it goes out, though both are moving. Its nanoradian is micrometers wide there, finer than
/// a position light-years from the origin keeps, so only a cone that reaches the receiver's shadow,
/// and not merely its middle, can say it did.
#[tokio::test]
async fn a_diffraction_limited_beam_at_a_craft_lands_on_it_until_it_goes_out() {
    let here = DVec3::new(4.2, 0.3, 0.0) * LIGHT_US_PER_LY;
    let at = here + DVec3::new(3.0, 1.0, 0.5).normalize() * 3.0e4 * US_PER_M;
    // Short enough that its recoil, along the led axis, does not walk it off the receiver.
    let duration_s = 4.0 * crate::server::TICK_US as f64 * 1.0e-6;
    // Both on one orbit's velocity, so the aim is led.
    let moving = |id: i64, at: DVec3| {
        let mut craft = crate::world::coasting(ShipId(id), at, DVec3::new(-0.6, 0.8, 0.0) * 1.0e-4, 0);
        craft.fit(ship(id, at, Form::starting()).fitting().cloned());
        hold_black(&mut craft);
        craft
    };
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, moving(1, here), 0.0);
    server.admit(ClientId(2), moving(2, at), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(Aim::Ship(ShipId(2)), Apertures::Aft, 1.0e17, 5.51e-7, 0.0, duration_s));
    server.tick(&mut wire).await.unwrap();
    let floor = wire.take(EMITTING).iter().find_map(|m| match m {
        Outbound::Accepted { order: Order::Emit { spread_rad, .. }, .. } => Some(*spread_rad),
        _ => None,
    });
    assert!(floor.is_some_and(|s| s < 1.0e-8), "premise: a nanoradian, {floor:?}");
    let mut told = Vec::new();
    for _ in 0..8 {
        server.tick(&mut wire).await.unwrap();
        told.extend(illuminated(&wire.take(ClientId(2))));
    }
    let [.., (_, off_w, _)] = told[..] else { panic!("never lit: {told:?}") };
    assert_eq!(off_w, 0.0, "it went out: {told:?}");
    assert!(told[..told.len() - 1].iter().all(|(_, w, _)| *w > 0.0), "went dark while lit: {told:?}");
}

/// Every `Presence` of the emitter the observer was told while `apertures` were lit, by the spread
/// it stated, and each DRIVE event the emitter wrote.
async fn stated_emit(apertures: Apertures, power_w: f64, spread_rad: f64) -> (Vec<f64>, Vec<lc_proto::DriveChange>) {
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    server.admit(ClientId(2), ship(2, DVec3::Y * LIGHT_SECOND_US, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), apertures, power_w, 1.0e-6, spread_rad, 3.0 * HOUR_S));
    let mut spreads = Vec::new();
    while (server.now_t() as f64) < 12.0 * HOUR_S * 1.0e6 {
        server.tick(&mut wire).await.unwrap();
        for message in wire.take(ClientId(2)) {
            if let Outbound::Present(list) = message {
                let of = list.iter().map(|p| p.get()).filter(|p| p.ship_id == EMITTER && p.emit_fore_w > 0.0);
                spreads.extend(of.map(|p| p.emit_spread_rad));
            }
        }
    }
    let stated = server
        .journal()
        .events
        .iter()
        .filter(|e| e.kind == crate::server::KIND_DRIVE && e.source == EMITTER)
        .map(|e| serde_json::from_str(&e.payload).unwrap())
        .collect();
    (spreads, stated)
}

/// A burn is an emit and an emit a burn: a one-ended emit is stated as a drive is, with a DRIVE
/// event where it lights and one where it goes out, each saying the end it leaves and its
/// spread, and every `Presence` of it while lit states that spread.
#[tokio::test]
async fn a_burn_emit_is_stated_as_a_burn_is() {
    let (power_w, spread_rad) = (1.0e17, 0.02);
    let (spreads, stated) = stated_emit(Apertures::Fore, power_w, spread_rad).await;
    assert!(!spreads.is_empty() && spreads.iter().all(|s| *s == spread_rad), "{spreads:?}");
    let [lit, out] = stated[..] else { panic!("{stated:?}") };
    assert!((lit.emit_fore_w / power_w - 1.0).abs() < 0.01, "{lit:?}");
    assert_eq!((lit.emit_aft_w, lit.power_w, lit.emit_spread_rad), (0.0, 0.0, spread_rad));
    assert_eq!((out.emit_fore_w, out.emit_aft_w), (0.0, 0.0));
}

/// A balanced emit moves nothing, so no change of motive marks it: it is stated where it lights and
/// goes out all the same, from both ends.
#[tokio::test]
async fn a_balanced_emit_is_stated_as_a_burn_is() {
    let (power_w, spread_rad) = (1.0e17, 0.02);
    let (spreads, stated) = stated_emit(Apertures::Both, power_w, spread_rad).await;
    assert!(!spreads.is_empty() && spreads.iter().all(|s| *s == spread_rad), "{spreads:?}");
    let [lit, out] = stated[..] else { panic!("{stated:?}") };
    assert_eq!((lit.emit_fore_w, lit.emit_aft_w, lit.emit_spread_rad), (power_w, power_w, spread_rad));
    assert_eq!((out.emit_fore_w, out.emit_aft_w), (0.0, 0.0));
}

mod drives;
