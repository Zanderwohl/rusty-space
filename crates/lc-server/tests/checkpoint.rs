//! A craft through the real store and back.
//!
//! The unit tests either side of this one are complete on their own: [`lc_server::persist`]
//! turns a craft into bytes and back without a database, and `lc_store::ships` writes bytes and
//! reads them without knowing what they are. This is the join — and the join is where a column
//! type, an array cast or a lossy encoding would hide, because neither half can see it.
//!
//! Skipped with no database, the way every other store test is.

use glam::DVec3;
use lc_server::persist::{SAVE_FORMAT, load, save};
use lc_store::ships::{Shard, load_shard, load_ships, save_shard, save_ships};
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::fitting::{Balance, Fitting};
use lc_world::form::Form;
use lc_world::flight::{Cruise, Drive};
use lc_world::motion::Motive;
use tokio_postgres::Client;

async fn store() -> Option<Client> {
    let client = lc_store::connect().await.ok()?;
    lc_store::migrate::apply(&client).await.expect("the schema applies");
    Some(client)
}

async fn clear(client: &Client, band: i64) {
    client
        .execute("DELETE FROM ships WHERE ship_id BETWEEN $1 AND $2", &[&band, &(band + 999)])
        .await
        .unwrap();
}

/// A ship part-way through a crossing, which is the state with the most to lose.
fn under_way(id: i64) -> Craft {
    let mut craft = Craft::at(CraftId(id), Kind::Ship, DVec3::new(4.200079062537049, 0.0, 0.0));
    craft.name = Some("Ada".into());
    // The coordinate that found `serde_json`'s rounding. Kept here on purpose: this is the path
    // that has to carry it intact.
    let target = DVec3::new(4.200000033670821, -1.8149592025296526e-22, 0.0);
    let cruise = Cruise::plan_from(
        craft.motion.position_ly,
        DVec3::new(0.0, 0.002, 0.0),
        target,
        craft.motion.attitude,
        876.6000009999999,
        Drive::DEFAULT,
    );
    craft.motion.beta = DVec3::new(0.0, 0.002, 0.0);
    craft.motion.clock_s = 280_274.137_379_621_1;
    craft.motion.resume_crossing(cruise, None, 12.5);
    craft
}

#[tokio::test]
async fn a_craft_written_to_the_store_comes_back_bit_for_bit() {
    let Some(client) = store().await else { return };
    let band = 7_100_000;
    clear(&client, band).await;

    let craft = under_way(band);
    let row = save(&craft, Some("acct-checkpoint"), None, None, Default::default(), 900_000_000);
    assert_eq!(row.format, SAVE_FORMAT);
    save_ships(&client, &[row]).await.unwrap();

    let read = load_ships(&client)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.ship_id == band)
        .expect("the row came back");
    let back = load(&read, None).expect("it reads");

    assert_eq!(back.id, craft.id);
    assert_eq!(back.name, craft.name);
    assert_eq!(back.motion.clock_s, craft.motion.clock_s, "the crew aged");
    assert_eq!(back.motion.beta, craft.motion.beta);
    assert_eq!(
        back.motion.position_ly.to_array().map(f64::to_bits),
        craft.motion.position_ly.to_array().map(f64::to_bits),
        "a coordinate moved in the round trip",
    );
    // The crossing itself, re-planned from its recipe at the far end and identical to the bit.
    assert_eq!(back.motion.motive, craft.motion.motive, "it came back on a different flight");
    match (&back.motion.motive, &craft.motion.motive) {
        (Motive::Crossing(a), Motive::Crossing(b)) => {
            assert_eq!(a.to_ly.y.to_bits(), b.to_ly.y.to_bits(), "the target moved");
            assert_eq!(a.initial_beta(), b.initial_beta());
        }
        _ => unreachable!("a crossing"),
    }
}

/// A fitted ship's row holds its form and account, and both come back whole.
#[tokio::test]
async fn a_fitted_ships_account_comes_back_through_the_store() {
    let Some(client) = store().await else { return };
    let band = 7_103_000;
    clear(&client, band).await;

    let mut craft = Craft::at(CraftId(band), Kind::Ship, DVec3::ZERO);
    craft.fit(Some(Fitting::full(Form::starting(), Balance::DEFAULT, 0.0)));
    craft.drain(4.0e25, 30.0);
    let row = save(&craft, Some("acct-fitted"), None, None, Default::default(), 60_000_000);
    save_ships(&client, &[row]).await.unwrap();

    let read = load_ships(&client).await.unwrap().into_iter().find(|s| s.ship_id == band).unwrap();
    let back = load(&read, None).expect("it reads");
    assert_eq!(back.fitting(), craft.fitting());
    assert_eq!(back.fitting().unwrap().form(), &Form::starting());
    assert_eq!(back.length_m, craft.length_m);
}

/// A round ordered on one shard, checkpointed partway through the real store, runs on in the
/// next: the same plan, its owner told of the steps left, and the ship ending in the target.
#[tokio::test]
async fn a_refit_round_survives_the_store_and_a_restart() {
    use lc_proto::{ClientId, Inbound, Intent, Order, Outbound, ShipId};
    use lc_server::journal::Memory;
    use lc_server::server::Server;
    use lc_server::transport::Loopback;
    use lc_world::form::PartId;

    let Some(client) = store().await else { return };
    let band = 7_104_000;
    clear(&client, band).await;

    let ship = ShipId(band);
    let mut old = Server::new(Memory::default(), 0, 1);
    // Hours a tick, where a step takes days.
    old.set_rate(60.0);
    let mut craft = lc_server::world::still(ship, DVec3::ZERO);
    craft.fit(Some(Fitting::full(Form::starting(), old.balance(), 0.0)));
    old.admit(ClientId(1), craft, 0.0);
    let mut target = Form::starting();
    target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 1.4;
    target.parts.iter_mut().find(|p| p.id == PartId(1)).unwrap().volume_m3 *= 1.1;
    let mut wire = Loopback::new();
    let refit = Order::Refit { target: (&target).into() };
    wire.client_says(ClientId(1), Inbound::Act(Intent { ship_id: ship, order: refit, issued_at_client_t: i64::MAX }));
    old.tick(&mut wire).await.unwrap();
    let plan = old.ship(ship).unwrap().fitting().unwrap().refit().expect("the round began").clone();
    assert!(plan.steps().len() >= 2, "premise: {:?}", plan.steps());
    let first_s = plan.round().start_s + plan.steps()[0].ends_s();
    while (old.now_t() as f64) < first_s * 1.0e6 {
        old.tick(&mut wire).await.unwrap();
    }
    let now_s = old.now_t() as f64 * 1.0e-6;
    assert!(old.ship(ship).unwrap().is_refitting(now_s), "premise: it is partway");
    assert_ne!(old.ship(ship).unwrap().fitting().unwrap().form(), &Form::starting(), "premise: a step is done");

    let checkpoint = old.checkpoint();
    lc_store::store::ensure_partitions(&client, 0, checkpoint.now_t + 1).await.unwrap();
    save_ships(&client, &checkpoint.ships).await.unwrap();
    let ships: Vec<_> = load_ships(&client).await.unwrap().into_iter().filter(|s| s.ship_id == band).collect();
    let mut new = Server::new(Memory::default(), 0, 1);
    new.set_rate(60.0);
    assert!(new.adopt(lc_server::persist::Checkpoint { now_t: checkpoint.now_t, next_ship: checkpoint.next_ship, ships }).is_empty());
    let back = new.ship(ship).unwrap();
    assert!(back.is_refitting(now_s), "the round was dropped");
    assert_eq!(back.fitting().unwrap().refit().unwrap().steps(), plan.steps(), "and it plans the same");
    assert_eq!(back.fitting().unwrap().form(), old.ship(ship).unwrap().fitting().unwrap().form());

    let mut told = 0;
    while new.ship(ship).unwrap().is_refitting(new.now_t() as f64 * 1.0e-6) {
        new.tick(&mut wire).await.unwrap();
        told += wire.take(ClientId(1)).iter().filter(|m| matches!(m, Outbound::Fitted { .. })).count();
    }
    assert_eq!(new.ship(ship).unwrap().fitting().unwrap().form(), &target);
    assert!(told >= 1, "nobody was told the round finished");
}

/// Saving the same craft again replaces it, because a checkpoint is written over and over.
#[tokio::test]
async fn the_newest_checkpoint_is_the_one_that_comes_back() {
    let Some(client) = store().await else { return };
    let band = 7_101_000;
    clear(&client, band).await;

    let mut craft = under_way(band);
    save_ships(&client, &[save(&craft, Some("acct-again"), None, None, Default::default(), 1)]).await.unwrap();
    craft.motion.clock_s = 999_999.0;
    save_ships(&client, &[save(&craft, Some("acct-again"), None, None, Default::default(), 2)]).await.unwrap();

    let read = load_ships(&client).await.unwrap().into_iter().find(|s| s.ship_id == band).unwrap();
    assert_eq!(read.saved_t, 2);
    assert_eq!(load(&read, None).unwrap().motion.clock_s, 999_999.0);
}

/// The shard's own row: a clock that outlives the process and a counter that does not reissue.
#[tokio::test]
async fn a_shards_clock_and_counter_come_back() {
    let Some(client) = store().await else { return };
    let shard_id = 7_102_000;
    client.execute("DELETE FROM shard_state WHERE shard_id = $1", &[&shard_id]).await.unwrap();

    assert_eq!(load_shard(&client, shard_id).await.unwrap(), None);
    let state = Shard { now_t: 317_767_500_000, next_ship: 5 };
    save_shard(&client, shard_id, state).await.unwrap();
    assert_eq!(load_shard(&client, shard_id).await.unwrap(), Some(state));
}

/// What a craft knows, through the real store and back into a new shard: its files, its log,
/// and its telescope's duty with the ship. The map after is the map before.
#[tokio::test]
async fn what_a_craft_knows_survives_the_store_and_a_restart() {
    use lc_proto::{ClientId, Inbound, Intent, Order, ShipId};
    use lc_server::journal::Memory;
    use lc_server::server::Server;
    use lc_server::transport::Loopback;
    use lc_server::world::World;
    use lc_world::sky::{AuthoredStars, StarId, StarProvider};

    let Some(client) = store().await else { return };
    let band = 7_200_000;
    clear(&client, band).await;
    let to = band + 999;
    client.execute("DELETE FROM lc_knowledge WHERE ship_id BETWEEN $1 AND $2", &[&band, &to]).await.unwrap();
    client.execute("DELETE FROM lc_samples WHERE ship_id BETWEEN $1 AND $2", &[&band, &to]).await.unwrap();

    let template = AuthoredStars::sample().stars()[1].clone();
    let sky: Vec<_> = [DVec3::ZERO, DVec3::X * 30.0]
        .into_iter()
        .enumerate()
        .map(|(k, at)| {
            let mut star = template.clone();
            star.id = StarId::synthesize("store-knowledge", k as u64);
            star.position_ly = at;
            star
        })
        .collect();
    let shard = || {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(sky.clone()));
        server
    };

    let ship = ShipId(band);
    let mut old = shard();
    let mut wire = Loopback::new();
    // An AU from the home star, so the charts have somewhere to have been taken from.
    old.admit(ClientId(1), lc_server::world::still(ship, DVec3::X * 499.0e6), 0.0);
    wire.client_says(
        ClientId(1),
        Inbound::Act(Intent {
            ship_id: ship,
            order: Order::SetDuty { duty: lc_proto::Duty::Stare { star: sky[0].id.get() }, integration_s: 1.0e4 },
            issued_at_client_t: i64::MAX,
        }),
    );
    for _ in 0..200 {
        old.tick(&mut wire).await.unwrap();
    }

    let checkpoint = old.checkpoint();
    let remembered = old.take_knowledge();
    assert!(!remembered.samples.is_empty(), "a stare leaves a log");
    lc_store::store::ensure_partitions(&client, 0, checkpoint.now_t + 1).await.unwrap();
    save_ships(&client, &checkpoint.ships).await.unwrap();
    lc_store::knowledge::save_files(&client, &remembered.files).await.unwrap();
    lc_store::knowledge::save_samples(&client, &remembered.samples).await.unwrap();

    let ships: Vec<_> = load_ships(&client).await.unwrap().into_iter().filter(|s| s.ship_id == band).collect();
    let files: Vec<_> =
        lc_store::knowledge::load_files(&client).await.unwrap().into_iter().filter(|f| f.ship_id == band).collect();
    let samples: Vec<_> =
        lc_store::knowledge::load_samples(&client).await.unwrap().into_iter().filter(|r| r.ship_id == band).collect();
    let mut new = shard();
    assert!(new.adopt(lc_server::persist::Checkpoint { now_t: checkpoint.now_t, next_ship: checkpoint.next_ship, ships }).is_empty());
    assert!(new.adopt_knowledge(&files, &samples).is_empty());

    assert_eq!(new.knowledge_of(ship), old.knowledge_of(ship), "the map after is the map before");
    assert_eq!(new.duty_of(ship), old.duty_of(ship), "and the telescope is still on its star");
}

/// Presets through a checkpoint and into a new shard, deletions included, one account apart from
/// another.
#[tokio::test]
async fn presets_survive_the_store_and_a_restart() {
    use lc_proto::Form;
    use lc_server::presets::{Presets, rows};

    let Some(client) = store().await else { return };
    let (alice, bob) = ("acct-checkpoint-presets-a", "acct-checkpoint-presets-b");
    for account in [alice, bob] {
        client.execute("DELETE FROM presets WHERE account = $1", &[&account]).await.unwrap();
    }
    let checkpoint = |shard: &mut Presets| {
        let changes = shard.take_dirty();
        let client = &client;
        async move {
            let (saved, deleted) = rows(&changes);
            lc_store::presets::save(client, &saved).await.unwrap();
            for (account, name) in &deleted {
                lc_store::presets::delete(client, account, name).await.unwrap();
            }
        }
    };

    let mut old = Presets::default();
    let plate = Form { parts: vec![awkward_part()] };
    old.save(alice, "Plate".into(), plate.clone()).unwrap();
    old.save(alice, "Ring".into(), Form::default()).unwrap();
    old.save(bob, "Plate".into(), Form::default()).unwrap();
    checkpoint(&mut old).await;
    old.delete(alice, "Ring");
    checkpoint(&mut old).await;

    let saved: Vec<_> = lc_store::presets::load(&client)
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.account == alice || p.account == bob)
        .collect();
    let mut new = Presets::default();
    assert!(new.adopt(saved).is_empty());
    assert_eq!(new.for_account(alice), old.for_account(alice));
    assert_eq!(new.for_account(alice).len(), 1, "the deletion was written too");
    assert_eq!(new.for_account(bob), old.for_account(bob));
    assert_eq!(new.for_account(alice)[0].form, plate, "bit for bit");
}

/// A part with awkward floats, so a lossy encoding would show.
fn awkward_part() -> lc_proto::form::Part {
    use lc_proto::form::{Kind, Part, PartId, Primitive};
    Part {
        id: PartId(7),
        kind: Kind::Mind,
        primitive: Primitive::Ellipsoid { axes: [4.200000033670821, 1.8149592025296526e-22, 0.1] },
        volume_m3: 876.6000009999999,
        placement: None,
    }
}
