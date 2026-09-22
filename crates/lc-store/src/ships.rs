//! The checkpoint: craft, and the clock they are stamped against.
//!
//! Distinct from the rest of this crate in what it is *for*. Events are the record of what
//! happened and a worldline can be reconstructed from them; this is the answer as of a moment,
//! written so a shard that restarts does not have to replay the history of the world to find
//! out where anything is.
//!
//! No opinion here about what a craft is: [`Ship::state`] is bytes and the server is the only
//! thing that knows how to read them. Bytes rather than JSON because `serde_json` does not
//! round-trip every f64, and a checkpoint that moves a coordinate by one place every restart is
//! not a checkpoint. See the column comment in `0004_ships.sql`.

use tokio_postgres::{Client, Error, GenericClient};

/// One craft, saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Ship {
    pub ship_id: i64,
    /// The account it belongs to. `None` for a craft with no pilot.
    pub account: Option<String>,
    /// Coordinate microseconds [`Ship::state`] was taken at. See the column comment in
    /// `0004_ships.sql`: this is read back, not merely recorded.
    pub saved_t: i64,
    pub state: Vec<u8>,
    pub format: i32,
}

/// A shard's clock and identifier counter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shard {
    pub now_t: i64,
    pub next_ship: i64,
}

/// Write every craft, replacing what is there.
///
/// One statement whatever the count, and an upsert rather than a delete and re-insert: a
/// checkpoint that briefly has no ships in it is a checkpoint a crash can land inside.
pub async fn save_ships(client: &impl GenericClient, ships: &[Ship]) -> Result<u64, Error> {
    if ships.is_empty() {
        return Ok(0);
    }
    let ids: Vec<i64> = ships.iter().map(|s| s.ship_id).collect();
    let accounts: Vec<Option<String>> = ships.iter().map(|s| s.account.clone()).collect();
    let saved: Vec<i64> = ships.iter().map(|s| s.saved_t).collect();
    let states: Vec<Vec<u8>> = ships.iter().map(|s| s.state.clone()).collect();
    let formats: Vec<i32> = ships.iter().map(|s| s.format).collect();
    client
        .execute(
            "INSERT INTO ships (ship_id, account, saved_t, state, format)
             SELECT * FROM unnest($1::bigint[], $2::text[], $3::bigint[], $4::bytea[], $5::int[])
             ON CONFLICT (ship_id) DO UPDATE SET
                 account = excluded.account,
                 saved_t = excluded.saved_t,
                 state   = excluded.state,
                 format  = excluded.format",
            &[&ids, &accounts, &saved, &states, &formats],
        )
        .await
}

/// Every craft, oldest identifier first so a load is deterministic.
pub async fn load_ships(client: &Client) -> Result<Vec<Ship>, Error> {
    let rows = client
        .query("SELECT ship_id, account, saved_t, state, format FROM ships ORDER BY ship_id", &[])
        .await?;
    Ok(rows
        .iter()
        .map(|row| Ship {
            ship_id: row.get(0),
            account: row.get(1),
            saved_t: row.get(2),
            state: row.get(3),
            format: row.get(4),
        })
        .collect())
}

/// A query rather than `load_ships` filtered in Rust: reading every ship in the world to
/// answer about one person works until the world has ships in it.
pub async fn ship_for_account(client: &Client, account: &str) -> Result<Option<Ship>, Error> {
    let row = client
        .query_opt(
            "SELECT ship_id, account, saved_t, state, format FROM ships WHERE account = $1",
            &[&account],
        )
        .await?;
    Ok(row.map(|row| Ship {
        ship_id: row.get(0),
        account: row.get(1),
        saved_t: row.get(2),
        state: row.get(3),
        format: row.get(4),
    }))
}

/// Forget a craft. Nothing calls this yet; a ship that exists goes on existing.
pub async fn delete_ship(client: &Client, ship_id: i64) -> Result<u64, Error> {
    client.execute("DELETE FROM ships WHERE ship_id = $1", &[&ship_id]).await
}

pub async fn save_shard(client: &impl GenericClient, shard_id: i64, shard: Shard) -> Result<(), Error> {
    client
        .execute(
            "INSERT INTO shard_state (shard_id, now_t, next_ship) VALUES ($1, $2, $3)
             ON CONFLICT (shard_id) DO UPDATE SET
                 now_t = excluded.now_t, next_ship = excluded.next_ship",
            &[&shard_id, &shard.now_t, &shard.next_ship],
        )
        .await?;
    Ok(())
}

/// `None` for a shard that has never saved, which is the first boot of a new one.
pub async fn load_shard(client: &Client, shard_id: i64) -> Result<Option<Shard>, Error> {
    let row = client
        .query_opt("SELECT now_t, next_ship FROM shard_state WHERE shard_id = $1", &[&shard_id])
        .await?;
    Ok(row.map(|row| Shard { now_t: row.get(0), next_ship: row.get(1) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    /// Each test owns a band of identifiers and clears it first, so the suite runs in parallel
    /// against one database and twice in a row against the same one.
    async fn clear(client: &Client, band: i64) {
        client
            .execute("DELETE FROM ships WHERE ship_id BETWEEN $1 AND $2", &[&band, &(band + 999)])
            .await
            .unwrap();
        client.execute("DELETE FROM shard_state WHERE shard_id = $1", &[&band]).await.unwrap();
    }

    fn ship(id: i64, account: Option<&str>, state: &str) -> Ship {
        Ship {
            ship_id: id,
            account: account.map(Into::into),
            saved_t: 1_234_567,
            state: state.as_bytes().to_vec(),
            format: 1,
        }
    }

    #[tokio::test]
    async fn ships_written_come_back_as_they_went_in() {
        let Some(client) = store().await else { return };
        let band = 7_000_000;
        clear(&client, band).await;

        let written = vec![
            ship(band, Some("acct-1"), r#"{"motive":"Holding"}"#),
            ship(band + 1, None, r#"{"motive":"Drifting"}"#),
        ];
        assert_eq!(save_ships(&client, &written).await.unwrap(), 2);

        let read: Vec<Ship> = load_ships(&client)
            .await
            .unwrap()
            .into_iter()
            .filter(|s| (band..band + 1000).contains(&s.ship_id))
            .collect();
        assert_eq!(read, written);
    }

    /// A checkpoint is written over and over for the life of a shard, so the second write of a
    /// ship has to replace the first rather than fail on the key.
    #[tokio::test]
    async fn saving_a_ship_twice_replaces_it() {
        let Some(client) = store().await else { return };
        let band = 7_001_000;
        clear(&client, band).await;

        save_ships(&client, &[ship(band, Some("acct-2"), "first")]).await.unwrap();
        let mut again = ship(band, Some("acct-2"), "second");
        again.saved_t = 9_999;
        save_ships(&client, &[again.clone()]).await.unwrap();

        let read: Vec<Ship> = load_ships(&client)
            .await
            .unwrap()
            .into_iter()
            .filter(|s| s.ship_id == band)
            .collect();
        assert_eq!(read, vec![again], "the second checkpoint did not replace the first");
    }

    /// The clock outlives the process, or every saved ship reads as one whose crossing has not
    /// begun.
    #[tokio::test]
    async fn a_shard_that_has_never_saved_has_no_clock_and_one_that_has_keeps_it() {
        let Some(client) = store().await else { return };
        let band = 7_002_000;
        clear(&client, band).await;

        assert_eq!(load_shard(&client, band).await.unwrap(), None, "nothing saved yet");
        let state = Shard { now_t: 8_766_000_000, next_ship: 42 };
        save_shard(&client, band, state).await.unwrap();
        assert_eq!(load_shard(&client, band).await.unwrap(), Some(state));

        let later = Shard { now_t: 9_000_000_000, next_ship: 43 };
        save_shard(&client, band, later).await.unwrap();
        assert_eq!(load_shard(&client, band).await.unwrap(), Some(later), "it did not advance");
    }

    #[tokio::test]
    async fn saving_nothing_is_not_an_error() {
        let Some(client) = store().await else { return };
        assert_eq!(save_ships(&client, &[]).await.unwrap(), 0);
    }
}
