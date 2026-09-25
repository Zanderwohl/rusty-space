//! Presets: forms an account has kept under a name.
//!
//! Per account and not per craft, like `reading`. [`Preset::form`] is bytes the server encodes;
//! see `0013_presets.sql`. See `lightcone/docs/29-ship-form.md` §Your own presets.

use tokio_postgres::{Client, Error, GenericClient, Row};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub account: String,
    pub name: String,
    pub form: Vec<u8>,
}

fn preset(row: &Row) -> Preset {
    Preset { account: row.get(0), name: row.get(1), form: row.get(2) }
}

/// Write presets, replacing any an account already had under the same name.
pub async fn save(client: &impl GenericClient, presets: &[Preset]) -> Result<u64, Error> {
    if presets.is_empty() {
        return Ok(0);
    }
    let accounts: Vec<&str> = presets.iter().map(|p| p.account.as_str()).collect();
    let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    let forms: Vec<&[u8]> = presets.iter().map(|p| p.form.as_slice()).collect();
    client
        .execute(
            "INSERT INTO presets (account, name, form)
             SELECT * FROM unnest($1::text[], $2::text[], $3::bytea[])
             ON CONFLICT (account, name) DO UPDATE SET form = excluded.form",
            &[&accounts, &names, &forms],
        )
        .await
}

pub async fn delete(client: &impl GenericClient, account: &str, name: &str) -> Result<u64, Error> {
    client.execute("DELETE FROM presets WHERE account = $1 AND name = $2", &[&account, &name]).await
}

/// Every preset, by account and then name. Read whole at boot, as bookmarks are.
pub async fn load(client: &Client) -> Result<Vec<Preset>, Error> {
    let rows = client.query("SELECT account, name, form FROM presets ORDER BY account, name", &[]).await?;
    Ok(rows.iter().map(preset).collect())
}

/// One account's presets, by name.
pub async fn list(client: &impl GenericClient, account: &str) -> Result<Vec<Preset>, Error> {
    let rows = client
        .query("SELECT account, name, form FROM presets WHERE account = $1 ORDER BY name", &[&account])
        .await?;
    Ok(rows.iter().map(preset).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skipped where there is no database; see `migrate`'s tests.
    async fn store() -> Option<Client> {
        let client = crate::connect().await.ok()?;
        crate::migrate::apply(&client).await.expect("the schema applies");
        Some(client)
    }

    fn kept(account: &str, name: &str, form: &[u8]) -> Preset {
        Preset { account: account.into(), name: name.into(), form: form.to_vec() }
    }

    async fn clear(client: &Client, accounts: &[&str]) {
        for account in accounts {
            client.execute("DELETE FROM presets WHERE account = $1", &[account]).await.unwrap();
        }
    }

    #[tokio::test]
    async fn an_account_lists_only_its_own_presets() {
        let Some(client) = store().await else { return };
        let (alice, bob) = ("presets-test-alice", "presets-test-bob");
        clear(&client, &[alice, bob]).await;

        save(&client, &[kept(alice, "Plate", b"a1"), kept(bob, "Plate", b"b1"), kept(bob, "Ring", b"b2")])
            .await
            .unwrap();
        assert_eq!(list(&client, alice).await.unwrap(), vec![kept(alice, "Plate", b"a1")]);
        assert_eq!(list(&client, bob).await.unwrap(), vec![kept(bob, "Plate", b"b1"), kept(bob, "Ring", b"b2")]);

        let all: Vec<_> =
            load(&client).await.unwrap().into_iter().filter(|p| p.account == alice || p.account == bob).collect();
        assert_eq!(all.len(), 3);
        clear(&client, &[alice, bob]).await;
    }

    #[tokio::test]
    async fn saving_a_name_again_replaces_it_and_deleting_touches_only_that_one() {
        let Some(client) = store().await else { return };
        let (alice, bob) = ("presets-test-carol", "presets-test-dave");
        clear(&client, &[alice, bob]).await;

        save(&client, &[kept(alice, "Plate", b"old"), kept(bob, "Plate", b"theirs")]).await.unwrap();
        save(&client, &[kept(alice, "Plate", b"new")]).await.unwrap();
        assert_eq!(list(&client, alice).await.unwrap(), vec![kept(alice, "Plate", b"new")]);

        assert_eq!(delete(&client, alice, "Plate").await.unwrap(), 1);
        assert!(list(&client, alice).await.unwrap().is_empty());
        assert_eq!(list(&client, bob).await.unwrap(), vec![kept(bob, "Plate", b"theirs")], "only alice's went");
        clear(&client, &[alice, bob]).await;
    }
}
