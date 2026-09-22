//! What craft know: a file per subject, and the photometric log beside it.
//!
//! Subjects and files are opaque bytes here; the server encodes both. See `sql/0010_knowledge.sql` and
//! `lightcone/docs/24-standing-instruments.md`.

use tokio_postgres::{Client, Error, GenericClient};

/// One craft's file about one subject.
#[derive(Clone, Debug, PartialEq)]
pub struct Filed {
    pub ship_id: i64,
    pub subject: Vec<u8>,
    pub format: i32,
    pub file: Vec<u8>,
    pub saved_t: i64,
}

/// One photometric sample in a craft's log.
#[derive(Clone, Debug, PartialEq)]
pub struct LogRow {
    pub ship_id: i64,
    pub subject: Vec<u8>,
    pub witness: i64,
    pub band: i16,
    pub observed_s: f64,
    pub learned_t: i64,
    pub deficit: f64,
    pub sigma: f64,
}

/// Samples of one series read and thrown away: everything observed at or before `through_s`.
#[derive(Clone, Debug, PartialEq)]
pub struct Discarded {
    pub ship_id: i64,
    pub subject: Vec<u8>,
    pub witness: i64,
    pub band: i16,
    pub through_s: f64,
}

/// Write files, replacing whatever each craft had for each subject.
pub async fn save_files(client: &impl GenericClient, files: &[Filed]) -> Result<u64, Error> {
    if files.is_empty() {
        return Ok(0);
    }
    let ships: Vec<i64> = files.iter().map(|f| f.ship_id).collect();
    let subjects: Vec<Vec<u8>> = files.iter().map(|f| f.subject.clone()).collect();
    let formats: Vec<i32> = files.iter().map(|f| f.format).collect();
    let bodies: Vec<Vec<u8>> = files.iter().map(|f| f.file.clone()).collect();
    let saved: Vec<i64> = files.iter().map(|f| f.saved_t).collect();
    client
        .execute(
            "INSERT INTO lc_knowledge (ship_id, subject, format, file, saved_t)
             SELECT * FROM unnest($1::bigint[], $2::bytea[], $3::int[], $4::bytea[], $5::bigint[])
             ON CONFLICT (ship_id, subject) DO UPDATE SET
                 format  = excluded.format,
                 file    = excluded.file,
                 saved_t = excluded.saved_t",
            &[&ships, &subjects, &formats, &bodies, &saved],
        )
        .await
}

/// Every file, grouped by craft.
pub async fn load_files(client: &Client) -> Result<Vec<Filed>, Error> {
    let rows = client
        .query(
            "SELECT ship_id, subject, format, file, saved_t FROM lc_knowledge ORDER BY ship_id, subject",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| Filed {
            ship_id: row.get(0),
            subject: row.get(1),
            format: row.get(2),
            file: row.get(3),
            saved_t: row.get(4),
        })
        .collect())
}

/// Append samples. A duplicate is skipped rather than refused, so a retried checkpoint does not
/// fail on what it wrote the first time.
///
/// The partitions for their learned times must exist: see [`crate::store::ensure_partitions`].
pub async fn save_samples(client: &impl GenericClient, rows: &[LogRow]) -> Result<u64, Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let ships: Vec<i64> = rows.iter().map(|r| r.ship_id).collect();
    let subjects: Vec<Vec<u8>> = rows.iter().map(|r| r.subject.clone()).collect();
    let witnesses: Vec<i64> = rows.iter().map(|r| r.witness).collect();
    let bands: Vec<i16> = rows.iter().map(|r| r.band).collect();
    let observed: Vec<f64> = rows.iter().map(|r| r.observed_s).collect();
    let learned: Vec<i64> = rows.iter().map(|r| r.learned_t).collect();
    let deficits: Vec<f64> = rows.iter().map(|r| r.deficit).collect();
    let sigmas: Vec<f64> = rows.iter().map(|r| r.sigma).collect();
    client
        .execute(
            "INSERT INTO lc_samples (ship_id, subject, witness, band, observed_s, learned_t, deficit, sigma)
             SELECT * FROM unnest($1::bigint[], $2::bytea[], $3::bigint[], $4::smallint[],
                                  $5::float8[], $6::bigint[], $7::float8[], $8::float8[])
             ON CONFLICT DO NOTHING",
            &[&ships, &subjects, &witnesses, &bands, &observed, &learned, &deficits, &sigmas],
        )
        .await
}

/// Delete samples that have been read into conclusions.
pub async fn delete_samples(client: &impl GenericClient, discarded: &[Discarded]) -> Result<u64, Error> {
    if discarded.is_empty() {
        return Ok(0);
    }
    let ships: Vec<i64> = discarded.iter().map(|d| d.ship_id).collect();
    let subjects: Vec<Vec<u8>> = discarded.iter().map(|d| d.subject.clone()).collect();
    let witnesses: Vec<i64> = discarded.iter().map(|d| d.witness).collect();
    let bands: Vec<i16> = discarded.iter().map(|d| d.band).collect();
    let through: Vec<f64> = discarded.iter().map(|d| d.through_s).collect();
    client
        .execute(
            "DELETE FROM lc_samples s
             USING unnest($1::bigint[], $2::bytea[], $3::bigint[], $4::smallint[], $5::float8[])
                 AS d(ship_id, subject, witness, band, through_s)
             WHERE s.ship_id = d.ship_id AND s.subject = d.subject AND s.witness = d.witness
               AND s.band = d.band AND s.observed_s <= d.through_s",
            &[&ships, &subjects, &witnesses, &bands, &through],
        )
        .await
}

/// Every sample, in the order each series was taken.
pub async fn load_samples(client: &Client) -> Result<Vec<LogRow>, Error> {
    let rows = client
        .query(
            "SELECT ship_id, subject, witness, band, observed_s, learned_t, deficit, sigma
             FROM lc_samples ORDER BY ship_id, subject, witness, band, observed_s",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| LogRow {
            ship_id: row.get(0),
            subject: row.get(1),
            witness: row.get(2),
            band: row.get(3),
            observed_s: row.get(4),
            learned_t: row.get(5),
            deficit: row.get(6),
            sigma: row.get(7),
        })
        .collect())
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

    /// Each test owns a band of craft identifiers and clears it first, so the suite runs in
    /// parallel against one database and twice in a row against the same one.
    async fn clear(client: &Client, band: i64) {
        let to = band + 999;
        client.execute("DELETE FROM lc_knowledge WHERE ship_id BETWEEN $1 AND $2", &[&band, &to]).await.unwrap();
        client.execute("DELETE FROM lc_samples WHERE ship_id BETWEEN $1 AND $2", &[&band, &to]).await.unwrap();
    }

    fn filed(ship_id: i64, subject: &[u8], file: &[u8]) -> Filed {
        Filed { ship_id, subject: subject.to_vec(), format: 1, file: file.to_vec(), saved_t: 5 }
    }

    #[tokio::test]
    async fn a_file_written_twice_is_the_second_one() {
        let Some(client) = store().await else { return };
        let band = 8_000_000;
        clear(&client, band).await;
        save_files(&client, &[filed(band, b"star", b"first"), filed(band, b"planet", b"p")]).await.unwrap();
        save_files(&client, &[filed(band, b"star", b"second")]).await.unwrap();
        let read: Vec<Filed> =
            load_files(&client).await.unwrap().into_iter().filter(|f| f.ship_id == band).collect();
        assert_eq!(read, vec![filed(band, b"planet", b"p"), filed(band, b"star", b"second")]);
    }

    /// Samples come back bit-exact, in series order, and a sample written twice is one sample.
    #[tokio::test]
    async fn samples_come_back_to_the_bit_in_order_and_once() {
        let Some(client) = store().await else { return };
        let band = 8_001_000;
        clear(&client, band).await;
        let now: i64 = 3_000_000_000_000;
        crate::store::ensure_partitions(&client, 0, now + 1).await.unwrap();
        let row = |observed_s: f64, deficit: f64| LogRow {
            ship_id: band,
            subject: b"star".to_vec(),
            // The charting office's id, which does not fit a signed column as itself.
            witness: u64::MAX as i64,
            band: 1,
            observed_s,
            learned_t: now,
            deficit,
            sigma: 1.0e-5,
        };
        let written = vec![row(20.000_000_1, -1.8149592025296526e-22), row(10.0, 0.1)];
        assert_eq!(save_samples(&client, &written).await.unwrap(), 2);
        assert_eq!(save_samples(&client, &written).await.unwrap(), 0, "the same samples again");
        let read: Vec<LogRow> =
            load_samples(&client).await.unwrap().into_iter().filter(|r| r.ship_id == band).collect();
        assert_eq!(read, vec![row(10.0, 0.1), row(20.000_000_1, -1.8149592025296526e-22)]);
        assert_eq!(read[0].witness as u64, u64::MAX);

        let discarded = Discarded { ship_id: band, subject: b"star".to_vec(), witness: u64::MAX as i64, band: 1, through_s: 10.0 };
        assert_eq!(delete_samples(&client, &[discarded]).await.unwrap(), 1);
        let left: Vec<LogRow> =
            load_samples(&client).await.unwrap().into_iter().filter(|r| r.ship_id == band).collect();
        assert_eq!(left, vec![row(20.000_000_1, -1.8149592025296526e-22)]);
    }

    #[tokio::test]
    async fn saving_nothing_is_not_an_error() {
        let Some(client) = store().await else { return };
        assert_eq!(save_files(&client, &[]).await.unwrap(), 0);
        assert_eq!(save_samples(&client, &[]).await.unwrap(), 0);
        assert_eq!(delete_samples(&client, &[]).await.unwrap(), 0);
    }
}
