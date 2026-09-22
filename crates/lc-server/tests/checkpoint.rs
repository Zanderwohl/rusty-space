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
    let row = save(&craft, Some("acct-checkpoint"), None, Default::default(), 900_000_000);
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

/// Saving the same craft again replaces it, because a checkpoint is written over and over.
#[tokio::test]
async fn the_newest_checkpoint_is_the_one_that_comes_back() {
    let Some(client) = store().await else { return };
    let band = 7_101_000;
    clear(&client, band).await;

    let mut craft = under_way(band);
    save_ships(&client, &[save(&craft, Some("acct-again"), None, Default::default(), 1)]).await.unwrap();
    craft.motion.clock_s = 999_999.0;
    save_ships(&client, &[save(&craft, Some("acct-again"), None, Default::default(), 2)]).await.unwrap();

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
