use glam::DVec3;
use lc_proto::{ClientId, Inbound, Outbound, Presence, ShipId, Sighting, kind};
use lc_world::craft::{Craft, CraftId, Kind as Hull};
use lc_world::navigation::Waypoint;
use lc_world::sky::{AuthoredStars, CatalogStar, StarProvider};

use super::*;
use crate::journal::Memory;
use crate::transport::Loopback;
use crate::world::World;

const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;

fn stars() -> Vec<CatalogStar> {
    AuthoredStars::sample().stars().to_vec()
}

/// Two ships ten astronomical units apart in the first star's system — about eleven ticks of
/// light — with the first one's owner at `level`.
fn shard(level: Level) -> (Server<Memory>, Loopback) {
    let stars = stars();
    let home = stars[0].position_ly;
    let mut server = Server::new(Memory::default(), 0, 1);
    server.admit(ClientId(1), Craft::at(CraftId(1), Hull::Ship, home + DVec3::X * 5.0 * AU_LY), 0.0);
    server.admit(ClientId(2), Craft::at(CraftId(2), Hull::Ship, home + DVec3::X * 15.0 * AU_LY), 0.0);
    server.load_world(World::new(stars));
    if let Some(client) = server.clients.get_mut(&ClientId(1)) {
        client.permission = level;
    }
    (server, Loopback::new())
}

async fn ask(server: &mut Server<Memory>, wire: &mut Loopback, seq: u32, line: &str) -> (bool, String) {
    wire.client_says(ClientId(1), Inbound::Command { seq, line: line.into() });
    server.tick(wire).await.unwrap();
    let said = wire.take(ClientId(1));
    said.into_iter()
        .find_map(|m| match m {
            Outbound::Answered { seq: s, ok, text } if s == seq => Some((ok, text)),
            _ => None,
        })
        .expect("an answer")
}

fn presences(said: &[Outbound]) -> Vec<Presence> {
    said.iter()
        .filter_map(|m| match m {
            Outbound::Present(list) => Some(list.iter().map(|c| c.clone().into_inner())),
            _ => None,
        })
        .flatten()
        .collect()
}

fn sightings(said: &[Outbound]) -> Vec<Sighting> {
    said.iter()
        .filter_map(|m| match m {
            Outbound::Sightings(list) => Some(list.iter().map(|c| c.clone().into_inner())),
            _ => None,
        })
        .flatten()
        .collect()
}

/// A player is told the same thing about a command above them as about one that does not
/// exist, and `help` does not list it.
#[tokio::test]
async fn a_command_above_the_asker_is_indistinguishable_from_none() {
    let (mut server, mut wire) = shard(Level::PLAYER);
    let (ok, above) = ask(&mut server, &mut wire, 1, "teleport 1").await;
    assert!(!ok);
    let (_, nothing) = ask(&mut server, &mut wire, 2, "teleprot 1").await;
    assert_eq!(above.replace("teleport", "X"), nothing.replace("teleprot", "X"));

    let (_, explained) = ask(&mut server, &mut wire, 3, "help teleport").await;
    assert!(explained.starts_with("no command"), "{explained}");
    let (ok, list) = ask(&mut server, &mut wire, 4, "help").await;
    assert!(ok);
    assert!(list.contains("help") && !list.contains("teleport"), "{list}");
}

#[tokio::test]
async fn an_answer_names_the_line_it_answers_and_a_mistake_says_what_it_was() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    wire.client_says(ClientId(1), Inbound::Command { seq: 7, line: "help".into() });
    wire.client_says(ClientId(1), Inbound::Command { seq: 8, line: "teleport x:\"1".into() });
    server.tick(&mut wire).await.unwrap();
    let answers: Vec<(u32, bool, String)> = wire
        .take(ClientId(1))
        .into_iter()
        .filter_map(|m| match m {
            Outbound::Answered { seq, ok, text } => Some((seq, ok, text)),
            _ => None,
        })
        .collect();
    assert_eq!(answers.len(), 2, "{answers:?}");
    assert_eq!((answers[0].0, answers[0].1), (7, true));
    assert_eq!((answers[1].0, answers[1].1), (8, false));
    assert!(answers[1].2.contains("never closed"), "{}", answers[1].2);
    assert!(!wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Answered { .. })));
}

/// Level 3 moves and fills its own ship only; levels 1 and 2 any ship.
#[tokio::test]
async fn debug_acts_on_its_own_ship_and_admins_on_anyone_s() {
    let far = stars()[2].id.get();
    for line in [
        format!("teleport {far} ship:2"),
        "energize ship:2".into(),
        "refit-finish ship:2".into(),
        "refit-magic ship:2".into(),
        "drain ship:2".into(),
    ] {
        let (mut server, mut wire) = shard(Level::DEBUG);
        let (ok, why) = ask(&mut server, &mut wire, 1, &line).await;
        assert!(!ok && why.contains("no argument 'ship'"), "{line}: {why}");
    }

    let (mut server, mut wire) = shard(Level::DEBUG);
    let (ok, why) = ask(&mut server, &mut wire, 1, &format!("teleport {far}")).await;
    assert!(ok, "{why}");
    assert_eq!(server.fleet.get(CraftId(1)).unwrap().system.as_ref().map(|s| s.star.get()), Some(far));

    for level in [Level::ADMIN, Level::SUPERADMIN] {
        let (mut server, mut wire) = shard(level);
        let (ok, why) = ask(&mut server, &mut wire, 1, &format!("teleport {far} ship:2")).await;
        assert!(ok, "{why}");
        let moved = server.fleet.get(CraftId(2)).unwrap();
        assert_eq!(moved.system.as_ref().map(|s| s.star.get()), Some(far));
        assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Flying { .. })));
        assert!(server.fleet.get(CraftId(1)).unwrap().motion.position_ly.x < 5.0);
    }
}

/// Ship 2 fitted, with nothing stored.
fn emptied(server: &mut Server<Memory>) {
    use lc_world::fitting::Fitting;
    let balance = server.balance();
    let mut account = Fitting::full(lc_world::form::Form::starting(), balance, 0.0).account();
    account.stored_j = 0.0;
    server.fleet.get_mut(CraftId(2)).unwrap().fit(Some(Fitting::from_account(&account, balance)));
}

fn held(server: &Server<Memory>) -> (f64, f64) {
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get(CraftId(2)).unwrap();
    let fitting = craft.fitting().unwrap();
    let me = fitting.balance().module_energy_j();
    (fitting.stored_j_at(&craft.motion, now_s) / me, fitting.capacity_j_at(now_s) / me)
}

#[tokio::test]
async fn energize_adds_what_is_asked_and_never_more_than_fits() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    let (ok, why) = ask(&mut server, &mut wire, 1, "energize 2 ship:2").await;
    assert!(ok, "{why}");
    let (stored, capacity) = held(&server);
    assert!((stored - 2.0).abs() < 1e-3, "{stored} of {capacity}: {why}");
    assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Fitted { .. })));

    let (ok, why) = ask(&mut server, &mut wire, 2, "energize amount:1e6 ship:2").await;
    assert!(ok, "{why}");
    let (stored, capacity) = held(&server);
    assert!((stored - capacity).abs() < 1e-6, "an overcharge left {stored} of {capacity}");
}

/// Ship 2's refit, begun from the console as the order would, is finished by an admin: the
/// target form at once, and its owner told.
#[tokio::test]
async fn refit_finish_completes_a_refit_under_way_and_only_one() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    let (ok, why) = ask(&mut server, &mut wire, 1, "refit-finish ship:2").await;
    assert!(!ok && why.contains("not refitting"), "{why}");

    let (ok, why) = ask(&mut server, &mut wire, 2, "energize ship:2").await;
    assert!(ok, "{why}");
    let (ok, why) = ask(&mut server, &mut wire, 3, "refit default 1.05 ship:2").await;
    assert!(ok && why.contains("steps"), "{why}");
    let now_s = server.now_t() as f64 * 1.0e-6;
    assert!(server.fleet.get(CraftId(2)).unwrap().is_refitting(now_s));
    assert!(server.refitting.contains_key(&CraftId(2)), "its owner would not be told of its steps");
    wire.take(ClientId(2));

    let (ok, why) = ask(&mut server, &mut wire, 4, "refit-finish ship:2").await;
    assert!(ok, "{why}");
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get(CraftId(2)).unwrap();
    assert!(!craft.is_refitting(now_s));
    assert_eq!(craft.fitting().unwrap().form(), &lc_world::form::presets::named("default", 1.05).unwrap());
    assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Fitted { .. })));
}

/// `refit-magic` takes a form, drops the round under way, and is refused for a form that breaks
/// a placement rule, naming the part.
#[tokio::test]
async fn refit_magic_rebuilds_as_a_form_at_once() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    let (ok, why) = ask(&mut server, &mut wire, 1, "energize ship:2").await;
    assert!(ok, "{why}");
    let (ok, why) = ask(&mut server, &mut wire, 2, "refit default 1.05 ship:2").await;
    assert!(ok, "{why}");
    wire.take(ClientId(2));

    let (ok, why) = ask(&mut server, &mut wire, 3, "refit-magic cluster ship:2").await;
    assert!(ok, "{why}");
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get(CraftId(2)).unwrap();
    assert!(!craft.is_refitting(now_s) && !server.refitting.contains_key(&CraftId(2)));
    let cluster = lc_world::form::presets::Builtin::Cluster.form();
    assert_eq!(craft.fitting().unwrap().form(), &cluster);
    assert_eq!(craft.length_m, craft.fitting().unwrap().hull().extent_m);
    assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Fitted { .. })));

    // Far too small to hold the least drone a ship may keep.
    let (ok, why) = ask(&mut server, &mut wire, 4, "refit-magic default 0.05 ship:2").await;
    assert!(!ok && why.contains("drone"), "{why}");
    assert_eq!(server.fleet.get(CraftId(2)).unwrap().fitting().unwrap().form(), &cluster);
    let (ok, why) = ask(&mut server, &mut wire, 5, "refit-magic hexagon ship:2").await;
    assert!(!ok, "{why}");
}

#[tokio::test]
async fn drain_takes_what_is_asked_and_never_below_empty() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    assert!(ask(&mut server, &mut wire, 1, "energize 5 ship:2").await.0);
    let (ok, why) = ask(&mut server, &mut wire, 2, "drain 2 ship:2").await;
    assert!(ok, "{why}");
    let (stored, _) = held(&server);
    assert!((stored - 3.0).abs() < 1e-3, "{stored}: {why}");
    assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Fitted { .. })));

    let (ok, why) = ask(&mut server, &mut wire, 3, "drain amount:1e6 ship:2").await;
    assert!(ok, "{why}");
    assert!(held(&server).0 < 1e-3, "an overdraw left {}", held(&server).0);
}

#[tokio::test]
async fn drain_with_no_amount_empties_the_ship() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    assert!(ask(&mut server, &mut wire, 1, "energize ship:2").await.0);
    assert!(held(&server).0 > 1.0, "premise: something to drain");
    let (ok, why) = ask(&mut server, &mut wire, 2, "drain ship:2").await;
    assert!(ok, "{why}");
    assert!(held(&server).0 < 1e-3, "{why}");
}

/// Charting the system the ship is in, and one it is not, leaves bodies it can place; between
/// the stars it has to be told which.
#[tokio::test]
async fn chart_hands_a_craft_a_system_it_can_place() {
    let far = stars()[2].id;
    let (mut server, mut wire) = shard(Level::DEBUG);
    let (ok, why) = ask(&mut server, &mut wire, 1, &format!("chart star:{}", far.get())).await;
    assert!(ok, "{why}");
    let now_s = server.now_t() as f64 * 1.0e-6;
    let bodies = server.aboard(CraftId(1)).knowledge.bodies_of(far, now_s);
    assert!(!bodies.is_empty(), "{why}");
    assert!(
        bodies.iter().all(|b| matches!(b.position_now, lc_world::knowledge::body::Placed::Known { .. })),
        "a charted body was not placed",
    );

    let home = stars()[0].id;
    let (ok, why) = ask(&mut server, &mut wire, 2, "chart").await;
    assert!(ok && why.contains(&format!("{:#x}", home.get())), "{why}");

    let (mut server, mut wire) = shard(Level::PLAYER);
    let (ok, why) = ask(&mut server, &mut wire, 1, "chart").await;
    assert!(!ok && why.starts_with("no command"), "{why}");
}

fn gap_m(server: &Server<Memory>) -> f64 {
    let t = server.now_t() as f64;
    let at = |id| server.fleet.get(CraftId(id)).unwrap().position_at(t);
    at(1).distance(at(2)) * lc_spacetime::LIGHT_MICROSECOND_M
}

/// Beside a ship holding an orbit is on that orbit, a standoff ahead, and stays there.
#[tokio::test]
async fn beside_a_ship_on_station_is_on_its_orbit() {
    let far = stars()[2].id.get();
    let (mut server, mut wire) = shard(Level::ADMIN);
    assert!(ask(&mut server, &mut wire, 1, &format!("teleport {far} altitude:5 ship:2")).await.0);
    let (ok, why) = ask(&mut server, &mut wire, 2, "teleport beside:2").await;
    assert!(ok, "{why}");

    let (me, them) = (server.fleet.get(CraftId(1)).unwrap(), server.fleet.get(CraftId(2)).unwrap());
    let standoff = lc_world::pursuit::standoff_m(me.length_m, them.length_m);
    let (lc_world::motion::Motive::Holding(Waypoint::Orbit(mine)), lc_world::motion::Motive::Holding(Waypoint::Orbit(theirs))) =
        (&me.motion.motive, &them.motion.motive)
    else {
        panic!("not both holding orbits: {why}");
    };
    assert_eq!((mine.radius_m, mine.pole), (theirs.radius_m, theirs.pole));
    for _ in 0..5 {
        let gap = gap_m(&server);
        assert!((gap - standoff).abs() < 0.01 * standoff, "{gap} m apart, wanted {standoff}");
        server.tick(&mut wire).await.unwrap();
    }
}

/// Beside a ship that is not on station is a standoff to one side, moving as it moves.
#[tokio::test]
async fn beside_a_drifting_ship_moves_with_it() {
    let (mut server, mut wire) = shard(Level::DEBUG);
    let (ok, why) = ask(&mut server, &mut wire, 1, "teleport beside:2").await;
    assert!(ok, "{why}");
    let (me, them) = (server.fleet.get(CraftId(1)).unwrap(), server.fleet.get(CraftId(2)).unwrap());
    let standoff = lc_world::pursuit::standoff_m(me.length_m, them.length_m);
    let gap = gap_m(&server);
    assert!((gap - standoff).abs() < 0.01 * standoff, "{gap} m apart, wanted {standoff}");

    for (line, why) in [
        ("teleport beside:1", "itself"),
        ("teleport 1 beside:2", "not both"),
        ("teleport", "is required"),
        ("teleport beside:2 star:1", "goes with a target"),
    ] {
        let (ok, said) = ask(&mut server, &mut wire, 9, line).await;
        assert!(!ok && said.contains(why), "{line}: {said}");
    }
}

#[tokio::test]
async fn energize_with_no_amount_fills_the_ship() {
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    let (ok, why) = ask(&mut server, &mut wire, 1, "energize ship:2").await;
    assert!(ok, "{why}");
    let (stored, capacity) = held(&server);
    assert!(capacity > 0.0 && (stored - capacity).abs() < 1e-6, "{stored} of {capacity}");
}

/// By name, every match, in any case.
#[tokio::test]
async fn who_is_names_a_ship_by_id_or_finds_it_by_name() {
    let (mut server, mut wire) = shard(Level::DEBUG);
    server.fleet.get_mut(CraftId(2)).unwrap().name = Some("Rosa Luxemburg".into());
    let (ok, said) = ask(&mut server, &mut wire, 1, "who-is id:2").await;
    assert!(ok, "{said}");
    assert_eq!(said, "ship 2\n  name: Rosa Luxemburg");
    let (ok, said) = ask(&mut server, &mut wire, 2, "who-is name:\"rosa LUXEMBURG\"").await;
    assert_eq!((ok, said.as_str()), (true, "ship 2\n  name: Rosa Luxemburg"));
    // Positionally, the first is the id.
    assert!(ask(&mut server, &mut wire, 3, "who-is 2").await.0);

    server.fleet.get_mut(CraftId(1)).unwrap().name = Some("Rosa Luxemburg".into());
    let (_, said) = ask(&mut server, &mut wire, 4, "who-is name:\"Rosa Luxemburg\"").await;
    assert_eq!(said, "ship 1\n  name: Rosa Luxemburg\nship 2\n  name: Rosa Luxemburg");

    for (line, why) in [
        ("who-is id:99", "no ship 99"),
        ("who-is name:nobody", "no ship is called"),
        ("who-is", "is required"),
        ("who-is id:1 name:x", "not both"),
    ] {
        let (ok, said) = ask(&mut server, &mut wire, 9, line).await;
        assert!(!ok && said.contains(why), "{line}: {said}");
    }
    let (mut server, mut wire) = shard(Level::PLAYER);
    assert!(!ask(&mut server, &mut wire, 1, "who-is id:2").await.0);
}

/// `where` prints ids and a teleport takes them: a body of the system the ship is in, found by
/// its id alone because the system is loaded.
#[tokio::test]
async fn where_names_bodies_a_teleport_can_reach() {
    let far = stars()[2].id.get();
    let (mut server, mut wire) = shard(Level::ADMIN);
    let (ok, _) = ask(&mut server, &mut wire, 1, &format!("teleport {far}")).await;
    assert!(ok);
    let (ok, listed) = ask(&mut server, &mut wire, 2, "where show:all").await;
    assert!(ok, "{listed}");
    assert!(listed.starts_with(&format!("star {far:#x}")), "{listed}");
    let body = listed
        .lines()
        .skip(1)
        .find_map(|line| line.split_whitespace().find(|w| w.starts_with("0x")))
        .expect("a body")
        .trim_end_matches(',');
    let (ok, why) = ask(&mut server, &mut wire, 3, &format!("teleport {body} altitude:5")).await;
    assert!(ok, "{why}");
    assert!(why.contains("body"), "{why}");
    // And never by the generator's name for it.
    assert!(!listed.contains("Authored"), "{listed}");
}

/// A ship ten AU away sees the jumper where it was until the jump's light arrives, then nothing.
/// Counting contacts by the system a craft is in now loses it on the next tick instead.
#[tokio::test]
async fn a_ship_that_jumps_is_seen_to_go_only_when_the_light_of_it_arrives() {
    let far = stars()[2].id.get();
    let (mut server, mut wire) = shard(Level::ADMIN);
    // Settle into the system, so both are contacts of each other.
    server.tick(&mut wire).await.unwrap();
    server.tick(&mut wire).await.unwrap();
    let was = server.fleet.get(CraftId(1)).unwrap().motion.position_ly;
    assert!(presences(&wire.take(ClientId(2))).iter().any(|p| p.ship_id == ShipId(1)), "premise: a contact");

    let (ok, why) = ask(&mut server, &mut wire, 1, &format!("teleport {far}")).await;
    assert!(ok, "{why}");
    let jumped_t = server.now_t();
    let delay_us = (10.0 * AU_LY * lc_world::motion::LIGHT_US_PER_LY) as i64;

    let mut vanished_at = None;
    let mut lost_at = None;
    for _ in 0..30 {
        let said = wire.take(ClientId(2));
        let now = server.now_t();
        if let Some(seen) = presences(&said).into_iter().find(|p| p.ship_id == ShipId(1)) {
            assert!(lost_at.is_none(), "the contact came back");
            let at = DVec3::from_array(seen.at_ly);
            assert!(
                at.distance(was) < AU_LY,
                "seen {} AU from where it was at {now} (jumped {jumped_t}), emitted {}",
                at.distance(was) / AU_LY,
                seen.emitted_t,
            );
        } else if lost_at.is_none() {
            lost_at = Some(now);
        }
        if let Some(gone) = sightings(&said).into_iter().find(|s| s.kind == kind::VANISH) {
            vanished_at = Some(gone.arrive_t);
        }
        server.tick(&mut wire).await.unwrap();
    }
    let vanished_at = vanished_at.expect("the vanishing was never seen");
    assert!(vanished_at >= jumped_t + delay_us - 1, "seen to vanish early, {vanished_at}");
    let lost_at = lost_at.expect("the contact was never lost");
    assert!(lost_at >= jumped_t + delay_us, "lost the contact before the light could say so, {lost_at}");
    assert!(lost_at <= vanished_at + 2 * crate::server::TICK_US, "kept the contact long after it went");
}

/// The table is one rule with `crate::ability`: whoever may grant or stage by the old messages
/// may by command, on every kind of shard.
#[test]
fn development_commands_agree_with_the_ability_table() {
    use crate::ability::{Act, Asking, Directing, Standing, allows};
    for (name, act) in [("energize", Act::GrantEnergy), ("stage", Act::Stage)] {
        for level in Level::ALL {
            for directing in [Directing(true), Directing(false)] {
                let by_command = find(name, crate::ability::commanding(level, directing)).is_some();
                let by_table = allows(act, Asking { level, standing: Standing::Flies }, directing);
                assert_eq!(by_command, by_table, "{name} for {} (directing {})", level.name(), directing.0);
            }
        }
    }
}

#[test]
fn every_scene_can_be_named_and_every_name_is_a_scene() {
    let all: Vec<&str> = lc_world::scenario::Scenario::ALL.iter().map(|s| s.name).collect();
    assert_eq!(SCENES, all.as_slice());
}

/// Every default is something its own argument would accept from the least senior asker who
/// can see it, or the command cannot be run without typing it.
#[test]
fn every_default_binds_for_whoever_can_see_it() {
    for spec in COMMANDS {
        let line = spec.args.iter().filter(|a| matches!(a.need, Need::Required)).fold(
            spec.name.to_string(),
            |line, arg| match arg.kind {
                Kind::Word(words) => format!("{line} {}", words[0]),
                Kind::Number(limits) => format!("{line} {}", limits[0].min),
                Kind::Id | Kind::Count(_) | Kind::Text => format!("{line} 1"),
            },
        );
        let parsed = parse(&line).unwrap();
        bind(spec, &parsed, spec.level).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
}
