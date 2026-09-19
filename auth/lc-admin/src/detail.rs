//! What a user page shows beyond the account row: where the account can be signed in from.
//!
//! Administrative reads, which is why they are here rather than in `lc_identity::store`. The
//! broker's store holds what the broker itself needs on a request path; these two queries
//! exist only to be looked at by a person.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// One way of signing in to the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub provider: String,
    /// What the provider calls them. Google's `sub`, or the address for a password account.
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub created_at: DateTime<Utc>,
}

/// What a desktop client is holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// What a revocation list shows a person: "Ada's laptop", not a hex string.
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

impl Grant {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
    }
}

pub async fn links_for(pool: &PgPool, account_id: Uuid) -> sqlx::Result<Vec<Link>> {
    let rows: Vec<(String, String, Option<String>, bool, DateTime<Utc>)> = sqlx::query_as(
        "select provider, subject, email, email_verified, created_at \
         from links where account_id = $1 order by created_at, provider",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Link {
            provider: r.0,
            subject: r.1,
            email: r.2,
            email_verified: r.3,
            created_at: r.4,
        })
        .collect())
}

/// Device grants, live and lapsed, newest first.
///
/// **The digest is not selected.** It is the longest-lived credential in the system, and a
/// page that put it on screen would be a page that puts it in a screenshot. What an
/// administrator needs is the label and the dates.
pub async fn grants_for(pool: &PgPool, account_id: Uuid) -> sqlx::Result<Vec<Grant>> {
    let rows: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>, DateTime<Utc>)> = sqlx::query_as(
        "select label, created_at, last_used, expires_at \
         from device_grants where account_id = $1 order by created_at desc",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Grant {
            label: r.0,
            created_at: r.1,
            last_used: r.2,
            expires_at: r.3,
        })
        .collect())
}
