//! Bookmarks: which account is where in which book.
//!
//! The one table here that no rule depends on. See `lightcone/docs/21-library.md`.

use tokio_postgres::{Client, Error, GenericClient};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub account: String,
    pub book: String,
    pub spine: i32,
    pub char_offset: i32,
    pub location: i32,
    pub locations: i32,
}

/// Write bookmarks, replacing whatever those accounts had for those books.
///
/// `read_at` moves to now on every write, which is what makes "recently read" mean recently
/// *read* rather than recently started.
pub async fn save(client: &impl GenericClient, marks: &[Bookmark]) -> Result<u64, Error> {
    if marks.is_empty() {
        return Ok(0);
    }
    let statement = client
        .prepare(
            "INSERT INTO reading (account, book, spine, char_offset, location, locations, read_at)
             VALUES ($1, $2, $3, $4, $5, $6, now())
             ON CONFLICT (account, book) DO UPDATE SET
                 spine = EXCLUDED.spine,
                 char_offset = EXCLUDED.char_offset,
                 location = EXCLUDED.location,
                 locations = EXCLUDED.locations,
                 read_at = EXCLUDED.read_at",
        )
        .await?;
    let mut written = 0;
    for mark in marks {
        written += client
            .execute(&statement, &[
                &mark.account,
                &mark.book,
                &mark.spine,
                &mark.char_offset,
                &mark.location,
                &mark.locations,
            ])
            .await?;
    }
    Ok(written)
}

/// Every bookmark, most recently read first.
///
/// Read whole at boot, the way ships are: a shard's tick loop has nowhere to await a query, and
/// a row per account per book is small enough that the alternative buys nothing.
pub async fn load(client: &Client) -> Result<Vec<Bookmark>, Error> {
    let rows = client
        .query(
            "SELECT account, book, spine, char_offset, location, locations
             FROM reading ORDER BY account, read_at DESC",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| Bookmark {
            account: row.get(0),
            book: row.get(1),
            spine: row.get(2),
            char_offset: row.get(3),
            location: row.get(4),
            locations: row.get(5),
        })
        .collect())
}

/// Forget an account's place in one book.
pub async fn forget(client: &Client, account: &str, book: &str) -> Result<u64, Error> {
    client.execute("DELETE FROM reading WHERE account = $1 AND book = $2", &[&account, &book]).await
}
