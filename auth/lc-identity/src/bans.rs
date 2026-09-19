//! Bans: what one is, and what a banned sign-in is told.
//!
//! **Bans are served concurrently.** An account can hold several at once and every one of them
//! runs on its own clock, so the state of an account is a list rather than a date. What the
//! sign-in path asks for is the summary — [`Sanction`] — and what the summary says is the
//! furthest expiry among the bans in force, because that is the one that has to elapse before
//! anything changes for the person waiting.
//!
//! An administrator cannot be banned. That rule is in [`crate::ability`] with the others, not
//! here: this module is what a ban *is*, and that module is who may issue one.

use chrono::{DateTime, Duration, Months, Utc};
use uuid::Uuid;

use crate::store::{Store, StoreError};

/// Why an account was banned.
///
/// A closed set stored by name. The text is shown to the banned account, so each variant is
/// written to be read by the person it is about — which is also why the case notes are a
/// separate, private field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    Cheating,
    Harassment,
    Spam,
    Exploit,
    Fraud,
    Evasion,
    Compromised,
    Other,
}

impl Reason {
    pub const ALL: [Reason; 8] = [
        Reason::Cheating,
        Reason::Harassment,
        Reason::Spam,
        Reason::Exploit,
        Reason::Fraud,
        Reason::Evasion,
        Reason::Compromised,
        Reason::Other,
    ];

    /// The stored name, and the one a URL carries.
    pub fn slug(self) -> &'static str {
        match self {
            Reason::Cheating => "cheating",
            Reason::Harassment => "harassment",
            Reason::Spam => "spam",
            Reason::Exploit => "exploit",
            Reason::Fraud => "fraud",
            Reason::Evasion => "evasion",
            Reason::Compromised => "compromised",
            Reason::Other => "other",
        }
    }

    /// What the banned account is told.
    pub fn said(self) -> &'static str {
        match self {
            Reason::Cheating => "Cheating",
            Reason::Harassment => "Harassing other players",
            Reason::Spam => "Spam",
            Reason::Exploit => "Abusing a bug",
            Reason::Fraud => "A payment dispute",
            Reason::Evasion => "Evading a ban",
            // Not punitive. An account locked because somebody else got into it, which reads
            // as a ban to whoever is holding the stolen password and as an explanation to the
            // owner when they get it back.
            Reason::Compromised => "This account was locked after a security problem",
            Reason::Other => "A breach of the rules",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Reason> {
        Reason::ALL.into_iter().find(|r| r.slug() == slug)
    }

    /// Anything unrecognised in the column reads as [`Reason::Other`].
    ///
    /// A row written by a future version, or by hand. Failing to render a user page because
    /// one ban has a name this build does not know is worse than showing it vaguely.
    pub fn from_stored(stored: &str) -> Reason {
        Reason::from_slug(stored).unwrap_or(Reason::Other)
    }
}

/// How long a ban runs, as the form offers it.
///
/// A closed list rather than a free-text duration: an administrator picking "2 months" from a
/// menu cannot typo it into two minutes, and the stored value is an absolute instant either
/// way, so nothing downstream has to parse this back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Term {
    Hours3,
    Hours12,
    Day,
    Days3,
    Week,
    Weeks2,
    Month,
    Months2,
    Months6,
    Year,
    Forever,
}

impl Term {
    pub const ALL: [Term; 11] = [
        Term::Hours3,
        Term::Hours12,
        Term::Day,
        Term::Days3,
        Term::Week,
        Term::Weeks2,
        Term::Month,
        Term::Months2,
        Term::Months6,
        Term::Year,
        Term::Forever,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Term::Hours3 => "3h",
            Term::Hours12 => "12h",
            Term::Day => "1d",
            Term::Days3 => "3d",
            Term::Week => "1w",
            Term::Weeks2 => "2w",
            Term::Month => "1mo",
            Term::Months2 => "2mo",
            Term::Months6 => "6mo",
            Term::Year => "1y",
            Term::Forever => "forever",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Term::Hours3 => "3 hours",
            Term::Hours12 => "12 hours",
            Term::Day => "1 day",
            Term::Days3 => "3 days",
            Term::Week => "1 week",
            Term::Weeks2 => "2 weeks",
            Term::Month => "1 month",
            Term::Months2 => "2 months",
            Term::Months6 => "6 months",
            Term::Year => "1 year",
            Term::Forever => "Permanent",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Term> {
        Term::ALL.into_iter().find(|t| t.slug() == slug)
    }

    /// When a ban issued `now` for this term runs out. `None` is permanent.
    pub fn until(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let months = |n: u32, approx_days: i64| {
            // Calendar months, so "2 months" lands on the same day of the month. The fallback
            // is unreachable with any real clock — `checked_add_months` only fails near the
            // end of representable time — and exists so that an overflow can never be mistaken
            // for `None`, which would silently turn a two-month ban into a permanent one.
            now.checked_add_months(Months::new(n))
                .unwrap_or_else(|| now + Duration::days(approx_days))
        };
        Some(match self {
            Term::Hours3 => now + Duration::hours(3),
            Term::Hours12 => now + Duration::hours(12),
            Term::Day => now + Duration::days(1),
            Term::Days3 => now + Duration::days(3),
            Term::Week => now + Duration::weeks(1),
            Term::Weeks2 => now + Duration::weeks(2),
            Term::Month => months(1, 30),
            Term::Months2 => months(2, 61),
            Term::Months6 => months(6, 183),
            Term::Year => months(12, 365),
            Term::Forever => return None,
        })
    }
}

/// One ban, as a user page shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ban {
    pub id: Uuid,
    pub account_id: Uuid,
    pub reason: Reason,
    /// **Private.** Never rendered on anything a banned account can reach.
    pub notes: String,
    pub issued_by: Option<Uuid>,
    /// Resolved for display. `None` where the issuing account has since been deleted.
    pub issued_by_name: Option<String>,
    pub issued_at: DateTime<Utc>,
    /// `None` is permanent.
    pub expires_at: Option<DateTime<Utc>>,
    pub lifted_at: Option<DateTime<Utc>>,
    pub lifted_by: Option<Uuid>,
    pub lifted_by_name: Option<String>,
    pub lift_notes: String,
}

impl Ban {
    /// Whether this one is stopping anybody from signing in at `now`.
    pub fn in_force(&self, now: DateTime<Utc>) -> bool {
        self.lifted_at.is_none() && self.expires_at.is_none_or(|end| end > now)
    }

    /// Lifted, expired, or running.
    pub fn state(&self, now: DateTime<Utc>) -> State {
        if self.lifted_at.is_some() {
            State::Lifted
        } else if self.expires_at.is_some_and(|end| end <= now) {
            State::Expired
        } else {
            State::InForce
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    InForce,
    Expired,
    Lifted,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::InForce => "In force",
            State::Expired => "Expired",
            State::Lifted => "Lifted",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            State::InForce => "in-force",
            State::Expired => "expired",
            State::Lifted => "lifted",
        }
    }
}

/// What a refused sign-in is told: how many bans are in force, and when the last of them ends.
///
/// The *furthest* expiry, not the nearest. Telling somebody their ban ends in three hours when
/// a two-month one is also running would be a lie they discover three hours later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sanction {
    pub count: i64,
    /// `None` when at least one ban in force is permanent.
    pub until: Option<DateTime<Utc>>,
}

/// A ban about to be issued.
#[derive(Clone, Debug)]
pub struct Issue {
    pub account_id: Uuid,
    pub reason: Reason,
    pub notes: String,
    pub by: Uuid,
    pub until: Option<DateTime<Utc>>,
}

impl Store {
    /// What is stopping this account signing in, if anything.
    ///
    /// The one ban query on the sign-in path, and the only one that has to be quick. It is a
    /// single aggregate over the partial index rather than a list the caller folds, because
    /// the caller does not want the list and fetching one would put a ban's private notes on a
    /// code path that has no business holding them.
    pub async fn sanction(
        &self,
        account_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<Sanction>, StoreError> {
        let (count, permanent, until) = match self {
            Store::Memory(m) => {
                let tables = m.0.lock().unwrap();
                let live: Vec<&Ban> = tables
                    .bans
                    .iter()
                    .filter(|b| b.account_id == account_id && b.in_force(now))
                    .collect();
                (
                    live.len() as i64,
                    live.iter().any(|b| b.expires_at.is_none()),
                    live.iter().filter_map(|b| b.expires_at).max(),
                )
            }
            Store::Postgres(pool) => {
                // `max` skips nulls, so a permanent ban has to be asked about separately —
                // otherwise a permanent ban alongside a three-hour one would report three
                // hours, which is the exact mistake this summary exists to prevent.
                let row: (i64, Option<bool>, Option<DateTime<Utc>>) = sqlx::query_as(
                    "select count(*), bool_or(expires_at is null), max(expires_at) \
                     from bans \
                     where account_id = $1 and lifted_at is null \
                       and (expires_at is null or expires_at > $2)",
                )
                .bind(account_id)
                .bind(now)
                .fetch_one(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                (row.0, row.1.unwrap_or(false), row.2)
            }
        };
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(Sanction {
            count,
            until: if permanent { None } else { until },
        }))
    }

    /// Every ban an account has ever had, newest first. The user page.
    pub async fn bans_for(&self, account_id: Uuid) -> Result<Vec<Ban>, StoreError> {
        match self {
            Store::Memory(m) => {
                let tables = m.0.lock().unwrap();
                let mut bans: Vec<Ban> = tables
                    .bans
                    .iter()
                    .filter(|b| b.account_id == account_id)
                    .cloned()
                    .collect();
                bans.sort_by(|a, b| b.issued_at.cmp(&a.issued_at));
                Ok(bans)
            }
            Store::Postgres(pool) => {
                type Row = (
                    Uuid,
                    Uuid,
                    String,
                    String,
                    Option<Uuid>,
                    Option<String>,
                    DateTime<Utc>,
                    Option<DateTime<Utc>>,
                    Option<DateTime<Utc>>,
                    Option<Uuid>,
                    Option<String>,
                    String,
                );
                let rows: Vec<Row> = sqlx::query_as(
                    "select b.id, b.account_id, b.reason, b.notes, \
                            b.issued_by, issuer.display_name, b.issued_at, b.expires_at, \
                            b.lifted_at, b.lifted_by, lifter.display_name, b.lift_notes \
                     from bans b \
                     left join accounts issuer on issuer.id = b.issued_by \
                     left join accounts lifter on lifter.id = b.lifted_by \
                     where b.account_id = $1 \
                     order by b.issued_at desc, b.id",
                )
                .bind(account_id)
                .fetch_all(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(rows
                    .into_iter()
                    .map(|r| Ban {
                        id: r.0,
                        account_id: r.1,
                        reason: Reason::from_stored(&r.2),
                        notes: r.3,
                        issued_by: r.4,
                        issued_by_name: r.5,
                        issued_at: r.6,
                        expires_at: r.7,
                        lifted_at: r.8,
                        lifted_by: r.9,
                        lifted_by_name: r.10,
                        lift_notes: r.11,
                    })
                    .collect())
            }
        }
    }

    /// Write a ban. Returns its id.
    ///
    /// Nothing is checked here. [`crate::ability::may_ban`] is the check, and it runs against
    /// the subject's level read in the same request.
    pub async fn issue_ban(
        &self,
        issue: &Issue,
        now: DateTime<Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::new_v4();
        match self {
            Store::Memory(m) => {
                m.0.lock().unwrap().bans.push(Ban {
                    id,
                    account_id: issue.account_id,
                    reason: issue.reason,
                    notes: issue.notes.clone(),
                    issued_by: Some(issue.by),
                    issued_by_name: None,
                    issued_at: now,
                    expires_at: issue.until,
                    lifted_at: None,
                    lifted_by: None,
                    lifted_by_name: None,
                    lift_notes: String::new(),
                });
            }
            Store::Postgres(pool) => {
                sqlx::query(
                    "insert into bans (id, account_id, reason, notes, issued_by, issued_at, expires_at) \
                     values ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(id)
                .bind(issue.account_id)
                .bind(issue.reason.slug())
                .bind(&issue.notes)
                .bind(issue.by)
                .bind(now)
                .bind(issue.until)
                .execute(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            }
        }
        Ok(id)
    }

    /// Lift one. `false` when it was already lifted, or is not this account's.
    ///
    /// The account id is a parameter rather than something the caller trusts the ban to carry,
    /// so a ban id guessed from another account cannot be lifted through a user page.
    pub async fn lift_ban(
        &self,
        id: Uuid,
        account_id: Uuid,
        by: Uuid,
        notes: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        match self {
            Store::Memory(m) => {
                let mut tables = m.0.lock().unwrap();
                let Some(ban) = tables
                    .bans
                    .iter_mut()
                    .find(|b| b.id == id && b.account_id == account_id && b.lifted_at.is_none())
                else {
                    return Ok(false);
                };
                ban.lifted_at = Some(now);
                ban.lifted_by = Some(by);
                ban.lift_notes = notes.to_owned();
                Ok(true)
            }
            Store::Postgres(pool) => {
                let done = sqlx::query(
                    "update bans set lifted_at = $4, lifted_by = $3, lift_notes = $5 \
                     where id = $1 and account_id = $2 and lifted_at is null",
                )
                .bind(id)
                .bind(account_id)
                .bind(by)
                .bind(now)
                .bind(notes)
                .execute(pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
                Ok(done.rows_affected() == 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Uuid {
        Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap()
    }

    fn admin() -> Uuid {
        Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap()
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    async fn ban(store: &Store, term: Term) -> Uuid {
        store
            .issue_ban(
                &Issue {
                    account_id: account(),
                    reason: Reason::Spam,
                    notes: "case notes".into(),
                    by: admin(),
                    until: term.until(now()),
                },
                now(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_clear_account_has_no_sanction() {
        let store = Store::memory();
        assert_eq!(store.sanction(account(), now()).await.unwrap(), None);
    }

    /// The headline: several bans at once, and the one reported is the furthest out.
    #[tokio::test]
    async fn concurrent_bans_report_the_longest() {
        let store = Store::memory();
        ban(&store, Term::Hours3).await;
        ban(&store, Term::Months2).await;
        let sanction = store.sanction(account(), now()).await.unwrap().unwrap();
        assert_eq!(sanction.count, 2);
        assert_eq!(sanction.until, Term::Months2.until(now()));
        assert_ne!(
            sanction.until,
            Term::Hours3.until(now()),
            "the nearest expiry was reported",
        );
    }

    /// And a permanent one alongside a short one is permanent, which `max(expires_at)` alone
    /// would get exactly backwards.
    #[tokio::test]
    async fn a_permanent_ban_beats_a_dated_one() {
        let store = Store::memory();
        ban(&store, Term::Hours3).await;
        ban(&store, Term::Forever).await;
        let sanction = store.sanction(account(), now()).await.unwrap().unwrap();
        assert_eq!(sanction.count, 2);
        assert_eq!(sanction.until, None, "a permanent ban reported an end");
    }

    #[tokio::test]
    async fn a_ban_stops_mattering_when_it_runs_out() {
        let store = Store::memory();
        ban(&store, Term::Hours3).await;
        let later = now() + Duration::hours(4);
        assert!(store.sanction(account(), now()).await.unwrap().is_some());
        assert_eq!(store.sanction(account(), later).await.unwrap(), None);
        // The row stays, as history.
        assert_eq!(store.bans_for(account()).await.unwrap().len(), 1);
        assert_eq!(
            store.bans_for(account()).await.unwrap()[0].state(later),
            State::Expired,
        );
    }

    #[tokio::test]
    async fn lifting_ends_a_ban_and_keeps_the_row() {
        let store = Store::memory();
        let id = ban(&store, Term::Forever).await;
        assert!(
            store
                .lift_ban(id, account(), admin(), "appealed", now())
                .await
                .unwrap()
        );
        assert_eq!(store.sanction(account(), now()).await.unwrap(), None);

        let held = store.bans_for(account()).await.unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].state(now()), State::Lifted);
        assert_eq!(held[0].lift_notes, "appealed");
        assert_eq!(held[0].lifted_by, Some(admin()));

        // Twice is not an error, and is not a second lift.
        assert!(
            !store
                .lift_ban(id, account(), admin(), "again", now())
                .await
                .unwrap()
        );
        assert_eq!(store.bans_for(account()).await.unwrap()[0].lift_notes, "appealed");
    }

    /// A ban id belonging to somebody else is not liftable through this account's page.
    #[tokio::test]
    async fn a_ban_is_lifted_only_through_the_account_that_holds_it() {
        let store = Store::memory();
        let id = ban(&store, Term::Week).await;
        let somebody_else = Uuid::new_v4();
        assert!(
            !store
                .lift_ban(id, somebody_else, admin(), "", now())
                .await
                .unwrap()
        );
        assert!(store.sanction(account(), now()).await.unwrap().is_some());
    }

    #[test]
    fn a_term_lands_where_it_says_it_does() {
        let start = now();
        assert_eq!(Term::Hours3.until(start), Some(start + Duration::hours(3)));
        assert_eq!(Term::Week.until(start), Some(start + Duration::days(7)));
        // Calendar months, so this is the same day of a later month rather than 60 days.
        assert_eq!(
            Term::Months2.until(start).unwrap().to_rfc3339(),
            "2026-03-01T00:00:00+00:00",
        );
        assert_eq!(Term::Forever.until(start), None);
        // Every term but the last is finite and in the future, which is the property the
        // fallback in `until` exists to keep true.
        for term in Term::ALL {
            match term.until(start) {
                Some(end) => assert!(end > start, "{} did not move the clock", term.label()),
                None => assert_eq!(term, Term::Forever),
            }
        }
    }

    #[test]
    fn slugs_round_trip() {
        for reason in Reason::ALL {
            assert_eq!(Reason::from_slug(reason.slug()), Some(reason));
            assert_eq!(Reason::from_stored(reason.slug()), reason);
        }
        for term in Term::ALL {
            assert_eq!(Term::from_slug(term.slug()), Some(term));
        }
        // A name from a future version renders rather than failing the page.
        assert_eq!(Reason::from_stored("something-new"), Reason::Other);
        assert_eq!(Reason::from_slug("something-new"), None);
        assert_eq!(Term::from_slug("3 hours"), None);
    }
}
