use lc_world::courtesy::cooking_distance_m;
use lc_world::craft::Kind;
use lc_world::emit::received_fraction;
use lc_world::flight::{Drive, G0};
use lc_world::motion::{Change, Event as Order_, ShipId as MotionId};
use lc_world::system::M_PER_LY;

use super::*;
use crate::server::TICK_US;

/// A rate whose ticks are about `tick_s` coordinate seconds.
fn ticking(tick_s: f64) -> f64 {
    tick_s * 1.0e6 / TICK_US as f64
}

/// Told at `at_s` to cross `far_m` along +x at five g, from rest at the origin facing +x: its
/// exhaust goes along -x from the instant it is told. A crossing stops a standoff short.
fn crossing(mut craft: Craft, at_s: f64, far_m: f64) -> Craft {
    let drive = Drive { accel_g: 5.0, ..craft.turning(craft.kind.drive()) };
    let to_ly = DVec3::X * (lc_world::flight::STANDOFF_LY + far_m / M_PER_LY);
    let order = Order_ { ship: MotionId(craft.id.0), at_t: at_s, change: Change::Cross { to_ly, drive } };
    craft.apply(&order).unwrap();
    craft
}

/// Five kilometers and unfitted, as a scene's cast is.
fn anvil(id: i64) -> Craft {
    let mut craft = Craft::at(CraftId(id), Kind::Ship, DVec3::ZERO);
    craft.length_m = 5_000.0;
    craft
}

fn anvil_w() -> f64 {
    Drive::exhaust_w(anvil(1).mass_kg(), 5.0)
}

/// Every lit drive's statements `id` has written, oldest first.
fn drive_said(server: &Server<Memory>, id: ShipId) -> Vec<(i64, Emitted)> {
    emissions_of(server, id).into_iter().filter(|(_, e)| matches!(e.spectrum, Spectrum::Blackbody { .. })).collect()
}

/// Inside the cooking distance behind a five-kilometer ship lighting at five g, a full, Black ship
/// five seconds of that light short of its limit collapses once the light arrives. The same ship as far
/// off beside the burn is told nothing and fed nothing, and never collapses.
#[tokio::test]
async fn a_full_ship_behind_a_burn_walks_to_collapse_and_one_beside_does_not() {
    let b = Balance::DEFAULT;
    let cooking_m = cooking_distance_m(&b, anvil_w(), b.drive_spread_rad, 1.0);
    // Nose on to the burn, where it is four times its rated load.
    let probe = ship(2, -DVec3::X, Form::starting());
    let (shadow_m2, rated_w) = (crate::field::shadow_toward_m2(&probe, DVec3::X, 0.0), probe.fitting().unwrap().field().rated_load_w());
    let d_m = (anvil_w() * shadow_m2 / (lc_world::signal::cone_solid_angle_sr(b.drive_spread_rad) * 4.0 * rated_w)).sqrt();
    assert!(d_m < cooking_m, "premise: inside the cooking distance, {d_m} {cooking_m}");
    let d_us = d_m * US_PER_M;
    for (at, behind) in [(-DVec3::X * d_us, true), (DVec3::Y * d_us, false)] {
        let mut receiver = ship(2, at, Form::starting());
        let first_w = anvil_w() * received_fraction(b.drive_spread_rad, shadow_m2, d_m);
        let heat_max_j = receiver.fitting().unwrap().field().heat_max_j();
        account(&mut receiver, |a| a.heat_j = heat_max_j - 5.0 * first_w);
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(ticking(1.0));
        server.fleet.insert(crossing(anvil(1), 1.0, 1.0e10));
        server.admit(ClientId(2), receiver, 0.0);
        let mut wire = Loopback::new();
        let mut told = Vec::new();
        let mut died = None;
        while server.now_t() < 60_000_000 && died.is_none() {
            server.tick(&mut wire).await.unwrap();
            told.extend(illuminated(&wire.take(ClientId(2))));
            died = server.journal().events.iter().find(|e| e.kind == KIND_COLLAPSE && e.source == ShipId(2)).map(|e| e.t);
            if !behind {
                assert_eq!(server.emissions.beamed_w(CraftId(2)), 0.0, "fed from beside the burn");
            }
        }
        let (lit_t, _) = drive_said(&server, ShipId(1))[0];
        if behind {
            let died = died.expect("it survived the burn");
            let arrive_t = (lit_t as f64 + d_us).ceil() as i64;
            assert!(matches!(told[..], [(_, w, t), ..] if t == arrive_t && (w / first_w - 1.0).abs() < 1.0e-3), "{told:?}");
            assert!(died > arrive_t && died < arrive_t + 30_000_000, "collapsed at {died}, lit from {arrive_t}");
        } else {
            assert!(told.is_empty() && died.is_none(), "{told:?} {died:?}");
        }
    }
}

/// A light-year off in the cone, an observer is handed the drive's glare at the flux its statement
/// puts there, in the face's blackbody, long after anyone near has stopped following it.
#[tokio::test]
async fn an_observer_a_light_year_off_is_handed_the_drives_glare() {
    let b = Balance::DEFAULT;
    let year_us = lc_world::flight::JULIAN_YEAR_S * 1.0e6;
    let at = -DVec3::X * year_us;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.fleet.insert(crossing(anvil(1), 1.0, 1.0e12));
    server.admit(ClientId(2), ship(2, at, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    server.set_rate(ticking(1.0e6));
    until(&mut server, &mut wire, year_us - 2.0e12).await;
    server.set_rate(ticking(1.0e4));
    let mut glare = None;
    while glare.is_none() && (server.now_t() as f64) < year_us + 1.0e12 {
        server.tick(&mut wire).await.unwrap();
        glare = server.emissions.glare(CraftId(2), ShipId(1));
    }
    let glare = glare.expect("no glare");
    let (_, said) = drive_said(&server, ShipId(1))[0];
    let distance_m = at.distance(DVec3::from_array(said.from)) * LIGHT_MICROSECOND_M;
    assert!(distance_m > 1.0e3 * cooking_distance_m(&b, said.power_w, said.half_angle_rad, 1.0), "premise: out of reach");
    let flux = lc_world::emit::flux_w_m2(said.power_w, said.half_angle_rad, distance_m);
    assert!((glare.flux_w_m2 / flux - 1.0).abs() < 1.0e-9, "{} {flux}", glare.flux_w_m2);
    assert_eq!(glare.spectrum, said.spectrum);
    assert!(matches!(glare.spectrum, Spectrum::Blackbody { temperature_k } if temperature_k > 1.0e4));
    assert_eq!(said.half_angle_rad, b.drive_spread_rad);
}

/// What a burning ship's `Presence` states is what its drive's emission is lit at: `m a c`, at the
/// mass it had as the light left, through the same `exhaust` E4 lights it from.
#[tokio::test]
async fn a_presence_states_what_the_drive_is_lit_at() {
    let b = Balance::DEFAULT;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(1.0));
    server.fleet.insert(crossing(anvil(1), 1.0, 1.0e10));
    server.admit(ClientId(2), ship(2, DVec3::Y * 1.0e3, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    let mut stated = Vec::new();
    while server.now_t() < 20_000_000 {
        server.tick(&mut wire).await.unwrap();
        for said in wire.take(ClientId(2)) {
            if let Outbound::Present(list) = said {
                stated.extend(list.iter().map(|p| p.get()).filter(|p| p.ship_id == ShipId(1) && p.drive_w > 0.0).map(|p| (p.emitted_t, p.drive_w)));
            }
        }
    }
    assert!(stated.len() > 3, "premise: seen burning, {stated:?}");
    let craft = server.fleet.get(CraftId(1)).unwrap();
    let lit_w = |t: i64| {
        let s = t as f64 * 1.0e-6;
        let jets = lc_world::emit::exhaust(craft, &b, s);
        assert!(matches!(jets[..], [lc_world::emit::Exhaust { jet: Jet::Drive, .. }]), "{jets:?}");
        assert!((jets[0].power_w / Drive::exhaust_w(craft.mass_kg_at(s), 5.0) - 1.0).abs() < 1.0e-12);
        jets[0].power_w
    };
    for (t, drive_w) in &stated {
        assert!((drive_w / lit_w(*t) - 1.0).abs() < 1.0e-9, "stated {drive_w} at {t}, lit at {}", lit_w(*t));
    }
    let (lit_t, said) = drive_said(&server, ShipId(1))[0];
    assert!((said.power_w / lit_w(lit_t) - 1.0).abs() < 1.0e-9, "{} at {lit_t}", said.power_w);
}

/// A craft drifting across a beam whose light has long been passing is fed from the microsecond it
/// enters the cone until the one it leaves it, and at no tick outside.
#[tokio::test]
async fn a_craft_flying_across_a_beam_is_fed_only_while_inside_it() {
    let (half, power_w) = (0.01, 1.0e17);
    let (d, beta) = (1_000.0, 1.0e-5);
    let start = DVec3::new(d, -3.0 * half * d, 0.0);
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(0.5));
    server.admit(EMITTING, ship(1, DVec3::ZERO, two_ended()), 0.0);
    let mut crosser = crate::world::coasting(ShipId(2), start, DVec3::Y * beta, 0);
    crosser.fit(ship(2, start, Form::starting()).fitting().cloned());
    hold_black(&mut crosser);
    let track = crosser.clone();
    server.admit(ClientId(2), crosser, 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(DVec3::X), Apertures::Both, power_w, 1.0e-6, half, 10.0 * HOUR_S));
    server.tick(&mut wire).await.unwrap();
    let (lit_t, lit) = emissions_of(&server, EMITTER).into_iter().find(|(_, e)| e.axis[0] > 0.0).expect("lit");
    let reach_m = lc_world::emit::distance_at_flux_m(power_w, half, lc_world::courtesy::cooking_flux_w_m2(&Balance::DEFAULT) * 1.0e-6);
    assert!(d * LIGHT_MICROSECOND_M < reach_m, "premise: within reach");

    // Found here by walking the cone test on its own, not by the shard's search.
    let cone = lc_world::signal::Beam::along(DVec3::X, half);
    let inside = |t: i64| cone.covers(track.position_at(t as f64));
    let edge = |from: i64, want: bool| {
        let mut b = from;
        while inside(b) != want {
            b += 1_000;
        }
        let mut a = b - 1_000;
        while b - a > 1 {
            let m = (a + b) / 2;
            if inside(m) == want { b = m } else { a = m }
        }
        b
    };
    let in_t = edge(0, true);
    let out_t = edge(in_t, false);
    assert!(in_t > lit_t + 10_000, "premise: it enters long after the light arrives, {in_t} {lit_t}");

    let mut told = Vec::new();
    while server.now_t() < out_t + 2_000_000 {
        server.tick(&mut wire).await.unwrap();
        told.extend(illuminated(&wire.take(ClientId(2))));
        let now = server.now_t();
        assert_eq!(lit_w(&server, 2) > 0.0, (in_t..out_t).contains(&now), "at {now}: {in_t} {out_t} {told:?}");
    }
    // Its shadow toward the emitter changes on the way through, and each step of it is said.
    let [(beam, _, on_t), .., (_, off_w, off_t)] = told[..] else { panic!("{told:?}") };
    assert_eq!((beam, on_t, off_w, off_t), (lit.beam, in_t, 0.0, out_t));
    assert!(told[..told.len() - 1].iter().all(|(b, w, t)| *b == lit.beam && *w > 0.0 && (in_t..out_t).contains(t)), "{told:?}");
}

/// A light-hour behind a fitted ship burning for days, a receiver is told the drive's power again
/// each time it has fallen a step as the ship lightens, each at the instant that statement's light
/// arrives, and the power it is told is what arrives of it.
#[tokio::test]
async fn a_receiver_behind_a_burn_is_told_its_power_falling_at_the_retarded_times() {
    let b = Balance::DEFAULT;
    let at = -DVec3::X * LIGHT_HOUR_US;
    let emitter = crossing(ship(1, DVec3::ZERO, Form::starting()), 1.0, 8.0e12);
    let first_w = Drive::exhaust_w(emitter.mass_kg_at(1.0), 5.0);
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(3_600.0));
    server.fleet.insert(emitter);
    server.admit(ClientId(2), ship(2, at, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    let mut told = Vec::new();
    until(&mut server, &mut wire, 4.0 * 86_400.0e6).await;
    told.extend(illuminated(&wire.take(ClientId(2))));

    let said = drive_said(&server, ShipId(1));
    let burning: Vec<&(i64, Emitted)> = said.iter().take_while(|(_, e)| e.power_w > 0.0).collect();
    assert!(burning.len() >= 3, "premise: it lightened by steps, {} said", burning.len());
    assert!((burning[0].1.power_w / first_w - 1.0).abs() < 1.0e-6, "lit at F c");
    let receiver = server.ship(ShipId(2)).unwrap();
    for (k, (t, said)) in burning.iter().enumerate() {
        let from = DVec3::from_array(said.from);
        let arrive_t = (*t as f64 + at.distance(from)).ceil() as i64;
        let shadow_m2 = crate::field::shadow_toward_m2(receiver, from - at, arrive_t as f64 * 1.0e-6);
        let want_w = said.power_w * received_fraction(said.half_angle_rad, shadow_m2, at.distance(from) * LIGHT_MICROSECOND_M);
        let got = told.iter().find(|(_, _, t)| *t == arrive_t);
        assert!(got.is_some_and(|(_, w, _)| (w / want_w - 1.0).abs() < 1.0e-9), "statement {k}: {got:?}, want {want_w} at {arrive_t}");
        if k > 0 {
            let step = said.power_w / burning[k - 1].1.power_w;
            assert!(step < 0.99 && step > 0.985, "stepped by {step}");
        }
    }
    assert!(told.windows(2).all(|w| w[1].2 > w[0].2), "{told:?}");
    assert!(b.drive_spread_rad == burning[0].1.half_angle_rad);
}

/// An escort's leg on thrusters beside a burning quarry is two emissions: the main drive carrying
/// the quarry's acceleration at `drive_spread_rad`, and the thrusters the closing at
/// `rcs_spread_rad`.
#[tokio::test]
async fn a_thruster_leg_is_two_emissions_at_their_two_spreads() {
    let b = Balance::DEFAULT;
    let craft = thruster_leg(anvil(1));
    let mass_kg = craft.mass_kg();
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(10.0));
    server.fleet.insert(craft);
    let mut wire = Loopback::new();
    for _ in 0..3 {
        server.tick(&mut wire).await.unwrap();
    }
    let lit = &server.emissions.emitting[&CraftId(1)];
    let jet = |kind| lit.iter().find(|l| l.jet == Some(kind)).unwrap_or_else(|| panic!("no {kind:?} in {lit:?}"));
    let (main, rcs) = (jet(Jet::Drive), jet(Jet::Thrusters));
    assert_eq!((main.half_angle_rad, rcs.half_angle_rad), (b.drive_spread_rad, b.rcs_spread_rad));
    assert!((main.power_w / Drive::exhaust_w(mass_kg, 1.0) - 1.0).abs() < 1.0e-6, "{}", main.power_w);
    assert!((rcs.power_w / Drive::exhaust_w(mass_kg, b.rcs_accel_g) - 1.0).abs() < 1.0e-6, "{}", rcs.power_w);
    assert!(DVec3::from_array(main.axis).angle_between(-DVec3::X) < 1.0e-3, "the main drive carries the quarry's burn");
    assert!(DVec3::from_array(rcs.axis).dot(DVec3::Y) > 0.9, "the thrusters close along -y, exhausting along +y");
    let stated_w = lc_world::emit::drive_w(server.fleet.get(CraftId(1)).unwrap(), &b, server.now_t() as f64 * 1.0e-6);
    assert!((stated_w / main.power_w - 1.0).abs() < 1.0e-6, "a thruster leg states its main drive alone, {stated_w}");
    let beams: Vec<i64> = emissions_of(&server, ShipId(1)).iter().map(|(_, e)| e.beam).collect();
    assert!(beams.contains(&main.beam.unwrap()) && beams.contains(&rcs.beam.unwrap()) && main.beam != rcs.beam);
}

/// A fitted ship lightening on a thruster leg: the thrusters turning over and going out are E4's
/// instants, and none of them is a change of the main drive `Presence` states.
#[tokio::test]
async fn the_thrusters_changing_is_no_drive_event() {
    let b = Balance::DEFAULT;
    let craft = thruster_leg(ship(1, DVec3::ZERO, Form::starting()));
    let end_s = 3_000.0;
    let found = lc_world::ignition::transitions(&craft, &b, 0.0, end_s);
    assert!(found.iter().any(|t| t.power_w > 0.0 && t.was_w > 0.0 && t.power_w != t.was_w), "premise: {found:?}");
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(100.0));
    server.fleet.insert(craft);
    let mut wire = Loopback::new();
    until(&mut server, &mut wire, end_s * 1.0e6).await;
    let drives: Vec<i64> =
        server.journal().events.iter().filter(|e| e.kind == crate::server::KIND_DRIVE && e.source == ShipId(1)).map(|e| e.t).collect();
    assert!(drives.is_empty(), "the main drive never changed, yet it was said to at {drives:?}");
}

/// `craft` on an escort's leg on its thrusters, 50 km off a quarry burning at one g along +x,
/// closing to 10 km along -y.
fn thruster_leg(mut craft: Craft) -> Craft {
    use lc_world::escort::{Burning, Station};
    use lc_world::flight::C_M_S;
    let b = Balance::DEFAULT;
    let quarry = Burning { position_ly: DVec3::ZERO, beta: DVec3::ZERO, accel: DVec3::X * (G0 / C_M_S), since_t: 0.0 };
    let thrusters = Drive { accel_g: b.rcs_accel_g, slew_rate_rad_s: 1.0, ..craft.kind.drive() };
    let km_ly = 1.0e3 / M_PER_LY;
    let station = Station {
        from_ly: DVec3::Y * 50.0 * km_ly,
        beta0: DVec3::ZERO,
        to_ly: DVec3::Y * 10.0 * km_ly,
        start_s: 0.0,
        drive: thrusters,
        quarry,
        target: lc_world::motion::ShipId(9),
    };
    craft.motion.motive = lc_world::motion::Motive::Escort(station.solve(DVec3::X));
    craft
}

/// The receiver behind a burn, settled across a hundred seconds of it in one tick and in a hundred:
/// the same instants are restated, so the two end in the same place.
#[tokio::test]
async fn one_leap_and_every_tick_agree() {
    let b = Balance::DEFAULT;
    let cooking_m = cooking_distance_m(&b, anvil_w(), b.drive_spread_rad, 1.0);
    let at = -DVec3::X * 0.5 * cooking_m * US_PER_M;
    let mut ends = Vec::new();
    for tick_s in [100.0, 1.0] {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(ticking(tick_s));
        server.fleet.insert(crossing(anvil(1), 1.0, 1.0e10));
        server.admit(ClientId(2), ship(2, at, Form::starting()), 0.0);
        let mut wire = Loopback::new();
        until(&mut server, &mut wire, 100.0e6).await;
        let told = illuminated(&wire.take(ClientId(2)));
        let mut receiver = server.ship(ShipId(2)).unwrap().clone();
        receiver.settle(101.0);
        let fitting = receiver.fitting().unwrap();
        ends.push((fitting.heat_j_at(&receiver.motion, 101.0), fitting.lit_w(), told));
    }
    let (leap, ticks) = (&ends[0], &ends[1]);
    assert!(leap.2.len() > 20, "premise: restated as it went, {} times", leap.2.len());
    assert_eq!(leap.2, ticks.2, "told differently");
    assert!((leap.1 / ticks.1 - 1.0).abs() < 1.0e-9, "{} {}", leap.1, ticks.1);
    assert!((leap.0 / ticks.0 - 1.0).abs() < 1.0e-9, "{} {}", leap.0, ticks.0);
}

/// Restarted while a burn's light is on its way to a receiver, the shard lands it at its arrival
/// once, and goes on stating the same drive rather than lighting it again. Near the burn the light
/// is followed from what the drive said; beyond its reach only the journal's deliveries bring it
/// back, and a shard that did not read them feeds nothing.
#[tokio::test]
async fn a_burn_in_flight_across_a_restart_still_lands() {
    const RATE: f64 = 1.0e-7;
    let b = Balance::DEFAULT;
    let reach_us = 1.0e3 * cooking_distance_m(&b, anvil_w(), b.drive_spread_rad, 1.0) * US_PER_M;
    for (d_us, resume) in [(3_000.0, true), (1.0e6, true), (1.0e6, false)] {
        assert_eq!(d_us > reach_us, d_us > 1.0e4, "premise: {d_us} µs against a reach of {reach_us}");
        let at = -DVec3::X * d_us;
        let mut server = Server::new(Memory::default(), 0, 1);
        server.set_rate(RATE);
        server.fleet.insert(crossing(anvil(1), 1.0e-4, 1.0e10));
        server.fleet.insert(ship(2, at, Form::starting()));
        let mut wire = Loopback::new();
        while drive_said(&server, ShipId(1)).is_empty() {
            assert!(server.now_t() < 1_000, "premise: the drive lit");
            server.tick(&mut wire).await.unwrap();
        }
        let (lit_t, lit) = drive_said(&server, ShipId(1))[0];
        let arrive_t = (lit_t as f64 + at.length()).ceil() as i64;
        assert!(server.now_t() < arrive_t, "premise: in flight");

        let mut restarted = restart(&server, 1.0e-4, resume).await;
        drop(server);
        let receiver = restarted.ship(ShipId(2)).unwrap().clone();
        restarted.admit(ClientId(2), receiver, 0.0);
        until(&mut restarted, &mut wire, arrive_t as f64 + 1.0).await;
        let told = illuminated(&wire.take(ClientId(2)));
        if d_us > reach_us && !resume {
            assert!(told.is_empty() && lit_w(&restarted, 2) == 0.0, "{told:?}");
            continue;
        }
        assert!(matches!(told[..], [(beam, w, t)] if beam == lit.beam && w > 0.0 && t == arrive_t), "{d_us} µs: {told:?}");
        until(&mut restarted, &mut wire, 5.0e6).await;
        let beams: Vec<i64> = drive_said(&restarted, ShipId(1)).iter().map(|(_, e)| e.beam).collect();
        assert!(beams.iter().all(|b| *b == lit.beam), "lit again after the restart: {beams:?}");
        assert!(lit_w(&restarted, 2) > 0.0);
    }
}

/// A ship that collapses partway through a tick in which its drive was stated as lighting later
/// never lit it: the lighting is withdrawn from what the tick writes, sends and keeps.
#[tokio::test]
async fn a_drive_stated_after_its_ship_collapsed_is_never_sent() {
    let b = Balance::DEFAULT;
    let probe = ship(2, -DVec3::X, Form::starting());
    let (shadow_m2, rated_w) = (crate::field::shadow_toward_m2(&probe, DVec3::X, 0.0), probe.fitting().unwrap().field().rated_load_w());
    let d_m = (anvil_w() * shadow_m2 / (lc_world::signal::cone_solid_angle_sr(b.drive_spread_rad) * 4.0 * rated_w)).sqrt();
    // Facing away from where it is told to go, so it lights only once it has come about.
    let mut victim = ship(2, -DVec3::X * d_m * US_PER_M, Form::starting());
    victim.motion.attitude = -DVec3::X;
    let heat_max_j = victim.fitting().unwrap().field().heat_max_j();
    let first_w = anvil_w() * received_fraction(b.drive_spread_rad, shadow_m2, d_m);
    account(&mut victim, |a| a.heat_j = heat_max_j - 0.5 * first_w);
    let victim = crossing(victim, 0.5, 1.0e10);
    let lights_s = match &victim.motion.motive {
        lc_world::motion::Motive::Crossing(cruise) => cruise.phase_changes_s()[0],
        other => panic!("{other:?}"),
    };
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(1.2 * lights_s));
    server.fleet.insert(crossing(anvil(1), 1.0, 1.0e10));
    server.fleet.insert(victim);
    let mut wire = Loopback::new();
    server.tick(&mut wire).await.unwrap();

    let died_t = server.journal().events.iter().find(|e| e.kind == KIND_COLLAPSE && e.source == ShipId(2)).expect("premise: it collapsed").t;
    assert!((died_t as f64) < lights_s * 1.0e6 && lights_s * 1.0e6 < server.now_t() as f64, "premise: it would have lit later that tick");
    assert!(drive_said(&server, ShipId(2)).is_empty(), "a wreck's drive went out on the journal");
    let written: Vec<i64> = server.journal().events.iter().map(|e| e.id).collect();
    assert!(server.journal().deliveries.iter().all(|d| written.contains(&d.event)), "a delivery of an event never written");
    assert!(!server.emissions.said.contains_key(&CraftId(2)) && server.emissions.landings.iter().all(|l| l.source != ShipId(2)));
}

/// An emit flown as a burn throttles as the drive does, and is said again as the ship lightens.
#[tokio::test]
async fn an_emit_flown_as_a_burn_is_said_again_as_the_ship_lightens() {
    let power_w = 3.0e19;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.set_rate(ticking(3_600.0));
    server.admit(EMITTING, ship(1, DVec3::ZERO, Form::starting()), 0.0);
    let mut wire = Loopback::new();
    wire.client_says(EMITTING, emit(along(-DVec3::X), Apertures::Aft, power_w, 1.0e-6, 0.01, 10.0 * 86_400.0));
    server.tick(&mut wire).await.unwrap();
    assert!(wire.take(EMITTING).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
    until(&mut server, &mut wire, 10.5 * 86_400.0e6).await;
    let said: Vec<f64> = emissions_of(&server, EMITTER).iter().map(|(_, e)| e.power_w).take_while(|w| *w > 0.0).collect();
    assert!(said.len() >= 3, "{said:?}");
    assert!((said[0] / power_w - 1.0).abs() < 1.0e-6, "{said:?}");
    assert!(said.windows(2).all(|w| w[1] / w[0] < 0.99 && w[1] / w[0] > 0.985), "{said:?}");
}
