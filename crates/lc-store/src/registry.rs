//! Every ship ever launched, and who launched it. See `0023_ship_registry.sql`.
//!
//! The types are here without the `postgres` feature, for a shard that records and never saves.

#[cfg(feature = "postgres")]
pub use db::{get, list, save};

/// What the shard knows about a ship at a launch, a boot or a collapse.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub ship_id: i64,
    pub account: Option<String>,
    pub name: Option<String>,
    /// Coordinate microseconds of its collapse.
    pub ended_t: Option<i64>,
}

/// One row, as an administrator reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub ship_id: i64,
    pub account: Option<String>,
    pub name: Option<String>,
    /// Unix seconds.
    pub launched_at: i64,
    pub ended_t: Option<i64>,
    /// Unix seconds.
    pub ended_at: Option<i64>,
}

/// Which records to list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Matches the name, case-insensitively, or the account exactly.
    pub search: Option<String>,
    pub account: Option<String>,
    /// `Some(true)` for ships still flying, `Some(false)` for ended ones.
    pub flying: Option<bool>,
    pub limit: i64,
    pub offset: i64,
}

#[cfg(feature = "postgres")]
mod db {
    use tokio_postgres::{Client, Error, GenericClient, Row};

    use super::{Entry, Query, Record};

    /// Write each entry, keeping what an entry does not say: a wreck's checkpoint row has lost its
    /// account, and an entry written then must not erase who flew it.
    pub async fn save(client: &impl GenericClient, entries: &[Entry]) -> Result<u64, Error> {
        if entries.is_empty() {
            return Ok(0);
        }
        let ids: Vec<i64> = entries.iter().map(|e| e.ship_id).collect();
        let accounts: Vec<Option<&str>> = entries.iter().map(|e| e.account.as_deref()).collect();
        let names: Vec<Option<&str>> = entries.iter().map(|e| e.name.as_deref()).collect();
        let ended: Vec<Option<i64>> = entries.iter().map(|e| e.ended_t).collect();
        client
            .execute(
                "INSERT INTO ship_registry (ship_id, account, name, ended_t, ended_at)
                 SELECT id, account, name, ended_t, CASE WHEN ended_t IS NULL THEN NULL ELSE now() END
                   FROM unnest($1::bigint[], $2::text[], $3::text[], $4::bigint[]) AS e (id, account, name, ended_t)
                 ON CONFLICT (ship_id) DO UPDATE SET
                     account  = COALESCE(excluded.account, ship_registry.account),
                     name     = COALESCE(excluded.name, ship_registry.name),
                     ended_t  = COALESCE(ship_registry.ended_t, excluded.ended_t),
                     ended_at = COALESCE(ship_registry.ended_at, excluded.ended_at)",
                &[&ids, &accounts, &names, &ended],
            )
            .await
    }

    const COLUMNS: &str = "ship_id, account, name, extract(epoch FROM launched_at)::bigint, ended_t, \
                           extract(epoch FROM ended_at)::bigint";

    fn record(row: &Row) -> Record {
        Record {
            ship_id: row.get(0),
            account: row.get(1),
            name: row.get(2),
            launched_at: row.get(3),
            ended_t: row.get(4),
            ended_at: row.get(5),
        }
    }

    pub async fn get(client: &Client, ship_id: i64) -> Result<Option<Record>, Error> {
        let sql = format!("SELECT {COLUMNS} FROM ship_registry WHERE ship_id = $1");
        Ok(client.query_opt(&sql, &[&ship_id]).await?.as_ref().map(record))
    }

    /// Newest first, and how many match in all.
    pub async fn list(client: &Client, query: &Query) -> Result<(Vec<Record>, i64), Error> {
        let pattern = query.search.as_deref().map(|s| format!("%{}%", escape_like(s)));
        let sql = format!(
            "SELECT {COLUMNS}, count(*) OVER () FROM ship_registry
              WHERE ($1::text IS NULL OR name ILIKE $1 ESCAPE '\\' OR account = $2)
                AND ($3::text IS NULL OR account = $3)
                AND ($4::bool IS NULL OR (ended_t IS NULL) = $4)
              ORDER BY ship_id DESC
              LIMIT $5 OFFSET $6"
        );
        let rows = client
            .query(&sql, &[&pattern, &query.search, &query.account, &query.flying, &query.limit, &query.offset])
            .await?;
        let total = rows.first().map_or(0, |row| row.get(6));
        Ok((rows.iter().map(record).collect(), total))
    }

    fn escape_like(text: &str) -> String {
        text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use tokio_postgres::Client;

    use super::*;
    use crate::migrate;

    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    async fn clear(client: &Client, band: i64) {
        client
            .execute("DELETE FROM ship_registry WHERE ship_id BETWEEN $1 AND $2", &[&band, &(band + 999)])
            .await
            .unwrap();
    }

    fn entry(id: i64, account: Option<&str>, name: Option<&str>, ended_t: Option<i64>) -> Entry {
        Entry { ship_id: id, account: account.map(Into::into), name: name.map(Into::into), ended_t }
    }

    /// A collapse written with no account keeps the pilot a launch wrote, and the first end stands.
    #[tokio::test]
    async fn an_end_keeps_who_flew_it() {
        let Some(client) = store().await else { return };
        let band = 7_060_000;
        clear(&client, band).await;
        let account = format!("pilot-{band}");
        save(&client, &[entry(band, Some(&account), Some("Canterbury"), None)]).await.unwrap();
        save(&client, &[entry(band, None, None, Some(5))]).await.unwrap();
        save(&client, &[entry(band, None, None, Some(9))]).await.unwrap();
        let got = get(&client, band).await.unwrap().unwrap();
        assert_eq!((got.account.as_deref(), got.name.as_deref(), got.ended_t), (Some(account.as_str()), Some("Canterbury"), Some(5)));
        assert!(got.ended_at.is_some() && got.launched_at > 0);
    }

    #[tokio::test]
    async fn a_listing_filters_by_account_and_by_whether_it_flies() {
        let Some(client) = store().await else { return };
        let band = 7_061_000;
        clear(&client, band).await;
        let account = format!("lister-{band}");
        save(&client, &[
            entry(band, Some(&account), Some("Knight"), Some(1)),
            entry(band + 1, Some(&account), Some("Donnager_100%"), None),
            entry(band + 2, Some("someone-else"), Some("Knight"), None),
        ])
        .await
        .unwrap();
        let query = |flying| Query { account: Some(account.clone()), flying, limit: 10, ..Query::default() };
        let (all, total) = list(&client, &query(None)).await.unwrap();
        assert_eq!((all.iter().map(|r| r.ship_id).collect::<Vec<_>>(), total), (vec![band + 1, band], 2));
        let (flying, _) = list(&client, &query(Some(true))).await.unwrap();
        assert_eq!(flying.iter().map(|r| r.ship_id).collect::<Vec<_>>(), vec![band + 1]);
        let named = Query { search: Some("ger_100%".into()), limit: 10, ..Query::default() };
        let (found, _) = list(&client, &named).await.unwrap();
        assert_eq!(found.iter().map(|r| r.ship_id).collect::<Vec<_>>(), vec![band + 1], "% and _ are literal");
    }
}
