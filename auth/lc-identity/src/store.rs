//! Where accounts live.
//!
//! An enum rather than a trait object because the set is closed: this runs against Postgres,
//! and against memory in a test. The same reasoning `lc_server::world::Path` used before it
//! grew a third case.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::providers::Provider;

/// An account as anything outside the broker sees it: an opaque id and a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: Uuid,
    pub display_name: String,
}

/// One way of signing in to one account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub provider: Provider,
    pub subject: String,
    pub account_id: Uuid,
    pub email: Option<String>,
    pub email_verified: bool,
}

/// An address as it is stored and compared: trimmed and lowercased.
///
/// Comparing addresses that have not been through this is how one account becomes two, and —
/// where a verified address aligns accounts — how two become one that should not have.
pub fn normalise_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[derive(Debug)]
pub enum StoreError {
    /// A verified address that already belongs to someone else. The database says so, through
    /// `links_one_account_per_verified_email`, and this is that refusal surfacing.
    EmailTaken,
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::EmailTaken => f.write_str("that address already belongs to an account"),
            StoreError::Backend(why) => write!(f, "identity store: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Clone, Default)]
struct Tables {
    accounts: HashMap<Uuid, Account>,
    links: HashMap<(Provider, String), Link>,
    secrets: HashMap<(Provider, String), String>,
    codes: HashMap<Vec<u8>, (Uuid, String, chrono::DateTime<chrono::Utc>)>,
    grants: HashMap<Vec<u8>, (Uuid, chrono::DateTime<chrono::Utc>)>,
    flows: HashMap<Vec<u8>, Flow>,
}

/// Accounts in memory, for tests. Every rule the schema enforces is enforced here too, or a
/// test proves something the database would not allow.
#[derive(Clone, Default)]
pub struct Memory(Arc<Mutex<Tables>>);

#[derive(Clone)]
pub enum Store {
    Memory(Memory),
    Postgres(sqlx::PgPool),
}

impl Store {
    pub fn memory() -> Self {
        Store::Memory(Memory::default())
    }

    pub async fn link(
        &self,
        provider: Provider,
        subject: &str,
    ) -> Result<Option<Link>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m
                .0
                .lock()
                .unwrap()
                .links
                .get(&(provider, subject.into()))
                .cloned()),
            Store::Postgres(pool) => {
                let row: Option<(Uuid, Option<String>, bool)> = sqlx::query_as(
                    "select account_id, email, email_verified from links \
                     where provider = $1 and subject = $2",
                )
                .bind(provider.name())
                .bind(subject)
                .fetch_optional(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(row.map(|(account_id, email, email_verified)| Link {
                    provider,
                    subject: subject.to_owned(),
                    account_id,
                    email,
                    email_verified,
                }))
            }
        }
    }

    /// The account a **verified** address already belongs to, if any.
    ///
    /// Only verified, always. An unverified address matching is not evidence of anything: a
    /// password provider hands them out to whoever types one, and aligning on that would let
    /// anyone claim anyone's Google account. See `lightcone/docs/16-identity.md`.
    pub async fn account_for_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let email = normalise_email(email);
        match self {
            Store::Memory(m) => Ok(m
                .0
                .lock()
                .unwrap()
                .links
                .values()
                .find(|l| l.email_verified && l.email.as_deref() == Some(email.as_str()))
                .map(|l| l.account_id)),
            Store::Postgres(pool) => {
                let row: Option<(Uuid,)> = sqlx::query_as(
                    "select account_id from links where email = $1 and email_verified",
                )
                .bind(&email)
                .fetch_optional(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(row.map(|(id,)| id))
            }
        }
    }

    pub async fn account(&self, id: Uuid) -> Result<Option<Account>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m.0.lock().unwrap().accounts.get(&id).cloned()),
            Store::Postgres(pool) => {
                let row: Option<(Uuid, String)> =
                    sqlx::query_as("select id, display_name from accounts where id = $1")
                        .bind(id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(row.map(|(id, display_name)| Account { id, display_name }))
            }
        }
    }

    /// Attach a way of signing in to an account, creating the account if `account_id` is none.
    pub async fn add_link(
        &self,
        account_id: Option<Uuid>,
        display_name: &str,
        link: Link,
    ) -> Result<Account, StoreError> {
        let email = link.email.as_deref().map(normalise_email);
        if link.email_verified
            && let Some(email) = &email
            && let Some(owner) = self.account_for_verified_email(email).await?
            && Some(owner) != account_id
        {
            return Err(StoreError::EmailTaken);
        }
        let id = account_id.unwrap_or_else(Uuid::new_v4);
        let account = Account {
            id,
            display_name: display_name.to_owned(),
        };
        let link = Link {
            account_id: id,
            email,
            ..link
        };

        match self {
            Store::Memory(m) => {
                let mut tables = m.0.lock().unwrap();
                tables.accounts.entry(id).or_insert_with(|| account.clone());
                tables
                    .links
                    .insert((link.provider, link.subject.clone()), link);
                Ok(tables.accounts[&id].clone())
            }
            Store::Postgres(pool) => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                sqlx::query(
                    "insert into accounts (id, display_name) values ($1, $2) \
                     on conflict (id) do nothing",
                )
                .bind(id)
                .bind(display_name)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                sqlx::query(
                    "insert into links (provider, subject, account_id, email, email_verified) \
                     values ($1, $2, $3, $4, $5)",
                )
                .bind(link.provider.name())
                .bind(&link.subject)
                .bind(id)
                .bind(&link.email)
                .bind(link.email_verified)
                .execute(&mut *tx)
                .await
                .map_err(taken_or_backend)?;
                tx.commit()
                    .await
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                self.account(id)
                    .await?
                    .ok_or_else(|| StoreError::Backend("account vanished".into()))
            }
        }
    }

    pub async fn secret(
        &self,
        provider: Provider,
        subject: &str,
    ) -> Result<Option<String>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m
                .0
                .lock()
                .unwrap()
                .secrets
                .get(&(provider, subject.into()))
                .cloned()),
            Store::Postgres(pool) => {
                let row: Option<(String,)> =
                    sqlx::query_as("select phc from secrets where provider = $1 and subject = $2")
                        .bind(provider.name())
                        .bind(subject)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(row.map(|(phc,)| phc))
            }
        }
    }

    pub async fn set_secret(
        &self,
        provider: Provider,
        subject: &str,
        phc: &str,
    ) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.0.lock()
                    .unwrap()
                    .secrets
                    .insert((provider, subject.into()), phc.to_owned());
                Ok(())
            }
            Store::Postgres(pool) => sqlx::query(
                "insert into secrets (provider, subject, phc) values ($1, $2, $3) \
                     on conflict (provider, subject) do update \
                     set phc = excluded.phc, updated_at = now()",
            )
            .bind(provider.name())
            .bind(subject)
            .bind(phc)
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| StoreError::Backend(e.to_string())),
        }
    }
}

impl Store {
    /// Write down a sign-in in flight. `digest` is the hash of the code, never the code.
    pub async fn put_code(
        &self,
        digest: &[u8],
        account_id: Uuid,
        return_to: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.0.lock().unwrap().codes.insert(
                    digest.to_vec(),
                    (account_id, return_to.to_owned(), expires_at),
                );
                Ok(())
            }
            Store::Postgres(pool) => sqlx::query(
                "insert into signin_codes (digest, account_id, return_to, expires_at) \
                 values ($1, $2, $3, $4)",
            )
            .bind(digest)
            .bind(account_id)
            .bind(return_to)
            .bind(expires_at)
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| StoreError::Backend(e.to_string())),
        }
    }

    /// Spend a code. **Deleted on read, whether or not it was still valid**, so a code is worth
    /// one attempt however that attempt goes.
    ///
    /// `None` for a code that never existed, has already been spent, has expired, or was issued
    /// for somewhere other than `return_to`. The caller cannot tell those apart, and does not
    /// need to: every one of them is "no".
    pub async fn take_code(
        &self,
        digest: &[u8],
        return_to: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Uuid>, StoreError> {
        let taken = match self {
            Store::Memory(m) => m.0.lock().unwrap().codes.remove(digest),
            Store::Postgres(pool) => sqlx::query_as(
                "delete from signin_codes where digest = $1 \
                 returning account_id, return_to, expires_at",
            )
            .bind(digest)
            .fetch_optional(pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?,
        };
        Ok(taken
            .filter(|(_, issued_for, expires_at)| issued_for == return_to && *expires_at > now)
            .map(|(account_id, _, _)| account_id))
    }
}

impl Store {
    /// Record a device grant. `digest` is the hash of it, never the grant.
    pub async fn put_grant(
        &self,
        digest: &[u8],
        account_id: Uuid,
        label: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.0.lock()
                    .unwrap()
                    .grants
                    .insert(digest.to_vec(), (account_id, expires_at));
                Ok(())
            }
            Store::Postgres(pool) => sqlx::query(
                "insert into device_grants (digest, account_id, label, expires_at) \
                 values ($1, $2, $3, $4)",
            )
            .bind(digest)
            .bind(account_id)
            .bind(label)
            .bind(expires_at)
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| StoreError::Backend(e.to_string())),
        }
    }

    /// Who a grant belongs to, if it is still good.
    ///
    /// **Not** spent on use, unlike a code: this is what a client holds for months, and the
    /// whole point is that it works every launch. Revocation is deleting the row.
    pub async fn grant_holder(
        &self,
        digest: &[u8],
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Uuid>, StoreError> {
        match self {
            Store::Memory(m) => Ok(m
                .0
                .lock()
                .unwrap()
                .grants
                .get(digest)
                .filter(|(_, expires_at)| *expires_at > now)
                .map(|(id, _)| *id)),
            Store::Postgres(pool) => {
                // Touched on the way past, so a revocation list can show what is actually in
                // use rather than everything ever issued.
                let row: Option<(Uuid,)> = sqlx::query_as(
                    "update device_grants set last_used = now() \
                     where digest = $1 and expires_at > $2 returning account_id",
                )
                .bind(digest)
                .bind(now)
                .fetch_optional(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(row.map(|(id,)| id))
            }
        }
    }

    pub async fn revoke_grant(&self, digest: &[u8]) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.0.lock().unwrap().grants.remove(digest);
                Ok(())
            }
            Store::Postgres(pool) => sqlx::query("delete from device_grants where digest = $1")
                .bind(digest)
                .execute(pool)
                .await
                .map(|_| ())
                .map_err(|e| StoreError::Backend(e.to_string())),
        }
    }
}

/// The unique index on verified addresses, surfacing as the refusal it is rather than as a
/// generic database error.
fn taken_or_backend(error: sqlx::Error) -> StoreError {
    let unique_violation = error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| code == "23505");
    if unique_violation {
        StoreError::EmailTaken
    } else {
        StoreError::Backend(error.to_string())
    }
}

/// A dance with an upstream provider, waiting for the browser to come back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub provider: Provider,
    pub verifier: String,
    pub return_to: String,
    pub downstream_state: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl Store {
    pub async fn put_flow(&self, digest: &[u8], flow: &Flow) -> Result<(), StoreError> {
        match self {
            Store::Memory(m) => {
                m.0.lock()
                    .unwrap()
                    .flows
                    .insert(digest.to_vec(), flow.clone());
                Ok(())
            }
            Store::Postgres(pool) => sqlx::query(
                "insert into upstream_flows \
                 (digest, provider, verifier, return_to, downstream_state, expires_at) \
                 values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(digest)
            .bind(flow.provider.name())
            .bind(&flow.verifier)
            .bind(&flow.return_to)
            .bind(&flow.downstream_state)
            .bind(flow.expires_at)
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| StoreError::Backend(e.to_string())),
        }
    }

    /// Spend a dance. Deleted on read whether or not it was still valid, like a sign-in code:
    /// one `state` is worth one callback, so a replayed callback finds nothing.
    pub async fn take_flow(
        &self,
        digest: &[u8],
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Flow>, StoreError> {
        let taken = match self {
            Store::Memory(m) => m.0.lock().unwrap().flows.remove(digest),
            Store::Postgres(pool) => {
                let row: Option<(
                    String,
                    String,
                    String,
                    String,
                    chrono::DateTime<chrono::Utc>,
                )> = sqlx::query_as(
                    "delete from upstream_flows where digest = $1 returning \
                         provider, verifier, return_to, downstream_state, expires_at",
                )
                .bind(digest)
                .fetch_optional(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                row.and_then(
                    |(provider, verifier, return_to, downstream_state, expires_at)| {
                        Some(Flow {
                            provider: Provider::parse(&provider)?,
                            verifier,
                            return_to,
                            downstream_state,
                            expires_at,
                        })
                    },
                )
            }
        };
        Ok(taken.filter(|flow| flow.expires_at > now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(provider: Provider, subject: &str, email: &str, verified: bool) -> Link {
        Link {
            provider,
            subject: subject.into(),
            account_id: Uuid::nil(),
            email: Some(email.into()),
            email_verified: verified,
        }
    }

    #[tokio::test]
    async fn a_link_finds_the_account_it_was_made_for() {
        let store = Store::memory();
        let made = store
            .add_link(
                None,
                "Ada",
                link(
                    Provider::Password,
                    "ada@example.test",
                    "ada@example.test",
                    false,
                ),
            )
            .await
            .unwrap();
        let found = store
            .link(Provider::Password, "ada@example.test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.account_id, made.id);
        assert!(
            store
                .link(Provider::Google, "ada@example.test")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Addresses are compared normalised, or one account quietly becomes two.
    #[test]
    fn addresses_are_compared_the_same_way_they_are_stored() {
        assert_eq!(normalise_email("  Ada@Example.Test "), "ada@example.test");
        assert_eq!(normalise_email("ada@example.test"), "ada@example.test");
    }

    /// The takeover this design exists to prevent. A password account's address is never
    /// verified, so it must not collide with — or claim — a verified one.
    #[tokio::test]
    async fn an_unverified_address_neither_aligns_nor_blocks() {
        let store = Store::memory();
        let google = store
            .add_link(
                None,
                "Ada",
                link(Provider::Google, "g-1", "ada@example.test", true),
            )
            .await
            .unwrap();

        // Someone registers a password account against the same address.
        let password = store
            .add_link(
                None,
                "Not Ada",
                link(
                    Provider::Password,
                    "ada@example.test",
                    "Ada@Example.Test",
                    false,
                ),
            )
            .await
            .expect("an unverified address is allowed to coexist");

        assert_ne!(
            password.id, google.id,
            "an unverified address aligned an account"
        );
        assert_eq!(
            store
                .account_for_verified_email("ada@example.test")
                .await
                .unwrap(),
            Some(google.id),
            "and the verified one still owns the address",
        );
    }

    /// Two *verified* addresses that match are the one case that aligns, and the second one
    /// must attach to the existing account rather than make a rival.
    #[tokio::test]
    async fn two_verified_addresses_may_not_be_two_accounts() {
        let store = Store::memory();
        let first = store
            .add_link(
                None,
                "Ada",
                link(Provider::Google, "g-1", "ada@example.test", true),
            )
            .await
            .unwrap();

        // Claiming it for a *new* account is refused: the address is spoken for.
        let rival = store
            .add_link(
                None,
                "Ada",
                link(Provider::Discord, "d-1", "ada@example.test", true),
            )
            .await;
        assert!(
            matches!(rival, Err(StoreError::EmailTaken)),
            "a verified address was claimed twice"
        );

        // Attaching it to the account that already holds it is the aligned case, and works.
        store
            .add_link(
                Some(first.id),
                "Ada",
                link(Provider::Discord, "d-1", "ada@example.test", true),
            )
            .await
            .expect("aligning onto the owning account");
        assert_eq!(
            store
                .link(Provider::Discord, "d-1")
                .await
                .unwrap()
                .unwrap()
                .account_id,
            first.id,
        );
    }

    #[tokio::test]
    async fn a_secret_belongs_to_a_link_and_can_be_replaced() {
        let store = Store::memory();
        store
            .add_link(
                None,
                "Ada",
                link(
                    Provider::Password,
                    "ada@example.test",
                    "ada@example.test",
                    false,
                ),
            )
            .await
            .unwrap();
        assert!(
            store
                .secret(Provider::Password, "ada@example.test")
                .await
                .unwrap()
                .is_none()
        );

        store
            .set_secret(Provider::Password, "ada@example.test", "$argon2id$first")
            .await
            .unwrap();
        store
            .set_secret(Provider::Password, "ada@example.test", "$argon2id$second")
            .await
            .unwrap();
        assert_eq!(
            store
                .secret(Provider::Password, "ada@example.test")
                .await
                .unwrap()
                .as_deref(),
            Some("$argon2id$second"),
        );
    }
}

#[cfg(test)]
mod code_tests {
    use super::*;
    use chrono::{Duration, Utc};

    async fn an_account(store: &Store) -> Uuid {
        store
            .add_link(
                None,
                "Ada",
                Link {
                    provider: Provider::Password,
                    subject: "ada@example.test".into(),
                    account_id: Uuid::nil(),
                    email: None,
                    email_verified: false,
                },
            )
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn a_code_is_worth_exactly_one_exchange() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();
        let here = "https://lightcone.example/auth/return";
        store
            .put_code(b"digest", id, here, now + Duration::seconds(60))
            .await
            .unwrap();

        assert_eq!(
            store.take_code(b"digest", here, now).await.unwrap(),
            Some(id)
        );
        assert_eq!(
            store.take_code(b"digest", here, now).await.unwrap(),
            None,
            "replayed"
        );
    }

    /// Bound to the destination it was issued for, so a code leaked from one allowlisted site
    /// cannot be redeemed by another.
    #[tokio::test]
    async fn a_code_may_only_be_spent_where_it_was_issued() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();
        store
            .put_code(
                b"d",
                id,
                "https://lightcone.example/auth/return",
                now + Duration::seconds(60),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .take_code(b"d", "https://other.example/return", now)
                .await
                .unwrap(),
            None
        );
    }

    /// And spent anyway. A wrong destination burns the code rather than leaving it for a
    /// second guess.
    #[tokio::test]
    async fn a_refused_code_is_still_spent() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();
        let here = "https://lightcone.example/auth/return";
        store
            .put_code(b"d", id, here, now + Duration::seconds(60))
            .await
            .unwrap();
        assert_eq!(
            store
                .take_code(b"d", "https://other.example/return", now)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store.take_code(b"d", here, now).await.unwrap(),
            None,
            "it survived a miss"
        );
    }

    #[tokio::test]
    async fn an_expired_code_is_no_code() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();
        let here = "https://lightcone.example/auth/return";
        store
            .put_code(b"d", id, here, now - Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(store.take_code(b"d", here, now).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_code_nobody_issued_is_nothing() {
        let store = Store::memory();
        assert_eq!(
            store
                .take_code(
                    b"invented",
                    "https://lightcone.example/auth/return",
                    Utc::now()
                )
                .await
                .unwrap(),
            None,
        );
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;
    use chrono::{Duration, Utc};

    async fn an_account(store: &Store) -> Uuid {
        store
            .add_link(
                None,
                "Ada",
                Link {
                    provider: Provider::Password,
                    subject: "ada@example.test".into(),
                    account_id: Uuid::nil(),
                    email: None,
                    email_verified: false,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// Unlike a code, a grant is **not** spent on use. It is what a desktop client holds for
    /// months, and a sign-in that only worked once would be a sign-in every launch.
    #[tokio::test]
    async fn a_grant_works_every_time() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();
        store
            .put_grant(b"digest", id, "Ada's laptop", now + Duration::days(90))
            .await
            .unwrap();

        for _ in 0..3 {
            assert_eq!(store.grant_holder(b"digest", now).await.unwrap(), Some(id));
        }
    }

    #[tokio::test]
    async fn a_revoked_or_expired_grant_is_nobody() {
        let store = Store::memory();
        let id = an_account(&store).await;
        let now = Utc::now();

        store
            .put_grant(b"expired", id, "old laptop", now - Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(store.grant_holder(b"expired", now).await.unwrap(), None);

        store
            .put_grant(b"live", id, "laptop", now + Duration::days(90))
            .await
            .unwrap();
        assert_eq!(store.grant_holder(b"live", now).await.unwrap(), Some(id));
        store.revoke_grant(b"live").await.unwrap();
        assert_eq!(store.grant_holder(b"live", now).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_grant_nobody_issued_is_nothing() {
        let store = Store::memory();
        assert_eq!(
            store.grant_holder(b"invented", Utc::now()).await.unwrap(),
            None
        );
    }
}
