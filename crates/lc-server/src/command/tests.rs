use glam::DVec3;
use lc_proto::{ClientId, Inbound, Outbound, Presence, ShipId, Sighting, kind};
use lc_world::craft::{Craft, CraftId, Kind as Hull};
use lc_world::sky::{AuthoredStars, CatalogueStar, StarProvider};

use super::*;
use crate::journal::Memory;
use crate::transport::Loopback;
use crate::world::World;

const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;

fn stars() -> Vec<CatalogueStar> {
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
    // And nobody else hears either.
    assert!(!wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Answered { .. })));
}

/// Level 3 moves and fills its own ship only; levels 1 and 2 any ship.
#[tokio::test]
async fn debug_acts_on_its_own_ship_and_admins_on_anyone_s() {
    let far = stars()[2].id.get();
    for line in [format!("teleport {far} ship:2"), "energize ship:2".into(), "finish-refit ship:2".into()] {
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
        // Its owner is told what it is doing now; the asker's own ship stayed where it was.
        assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Flying { .. })));
        assert!(server.fleet.get(CraftId(1)).unwrap().motion.position_ly.x < 5.0);
    }
}

/// Ship 2 fitted and empty, and what it holds and can hold, in ME.
fn emptied(server: &mut Server<Memory>) {
    use lc_world::fitting::{Fitting, Loadout};
    let balance = server.balance();
    let mut account = Fitting::full(Loadout::STARTING, balance, 0.0).account();
    account.stored_j = 0.0;
    server.fleet.get_mut(CraftId(2)).unwrap().fit(Some(Fitting::from_account(&account, balance)));
}

fn held(server: &Server<Memory>) -> (f64, f64) {
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get(CraftId(2)).unwrap();
    let fitting = craft.fitting().unwrap();
    let me = fitting.balance.module_energy_j();
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

/// Ship 2's refit is finished by an admin: the target loadout at once, and its owner told.
#[tokio::test]
async fn finish_refit_completes_a_refit_under_way_and_only_one() {
    use lc_world::fitting::Loadout;
    let (mut server, mut wire) = shard(Level::ADMIN);
    emptied(&mut server);
    let (ok, why) = ask(&mut server, &mut wire, 1, "finish-refit ship:2").await;
    assert!(!ok && why.contains("not refitting"), "{why}");

    let (ok, why) = ask(&mut server, &mut wire, 2, "energize ship:2").await;
    assert!(ok, "{why}");
    let target = Loadout { engines: 7, ..Loadout::STARTING };
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get_mut(CraftId(2)).unwrap();
    craft.begin_refit(target, now_s).expect("the refit plans");
    server.refitting.insert(CraftId(2));
    wire.take(ClientId(2));

    let (ok, why) = ask(&mut server, &mut wire, 3, "finish-refit ship:2").await;
    assert!(ok, "{why}");
    let now_s = server.now_t() as f64 * 1.0e-6;
    let craft = server.fleet.get(CraftId(2)).unwrap();
    assert!(!craft.is_refitting(now_s));
    assert_eq!(craft.fitting().unwrap().loadout, target);
    assert!(wire.take(ClientId(2)).iter().any(|m| matches!(m, Outbound::Fitted { .. })));
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

/// **The light-delay test.** A ship ten AU off the one that jumps goes on seeing it where it
/// was until the light of the jump arrives, sees the vanishing at that moment and not before,
/// and then sees nothing: the landing is light-years away and its light is years out.
///
/// Checked against the mechanism: with the old rule — a contact is whoever shares your system
/// *now* — the second ship lost the contact on the very next tick.
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
                Kind::Id | Kind::Text => format!("{line} 1"),
            },
        );
        let parsed = parse(&line).unwrap();
        bind(spec, &parsed, spec.level).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
}
