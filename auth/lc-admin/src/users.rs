//! The user index query.
//!
//! **Paging happens in the database**: `limit`/`offset` plus a `count(*) over ()` window for
//! the total, in one statement. Slicing the table in Rust would work today and get gradually
//! slower rather than failing.

use chrono::{DateTime, Utc};
use lc_identity::level::Level;
use sqlx::PgPool;
use uuid::Uuid;

use crate::listing::{Listing, Rank, Standing};

/// One line of the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: Uuid,
    pub display_name: String,
    pub level: Level,
    /// One, to recognise the account by. The user page enumerates them.
    pub email: Option<String>,
    /// How many bans are in force right now.
    pub in_force: i64,
    pub permanent: bool,
    /// The **furthest** expiry. `None` with `permanent` is a ban that does not end.
    pub until: Option<DateTime<Utc>>,
}

impl Row {
    pub fn is_banned(&self) -> bool {
        self.in_force > 0
    }
}

/// One page of the index, and what it took to get here.
#[derive(Clone, Debug)]
pub struct Page {
    pub rows: Vec<Row>,
    /// Across every page, not the length of `rows`.
    pub total: i64,
    pub listing: Listing,
}

impl Page {
    /// At least one, or an empty result has no page to be on.
    pub fn pages(&self) -> u32 {
        let per = i64::from(self.listing.per);
        (((self.total + per - 1) / per).max(1)) as u32
    }

    pub fn first(&self) -> i64 {
        if self.rows.is_empty() {
            0
        } else {
            self.listing.offset() + 1
        }
    }

    pub fn last(&self) -> i64 {
        self.listing.offset() + self.rows.len() as i64
    }
}

type IndexRow = (
    Uuid,
    String,
    i32,
    Option<String>,
    i64,
    bool,
    Option<DateTime<Utc>>,
    i64,
);

/// Escaped, or a search for `100%` matches every account. The backslash is doubled first or
/// the escaping is itself escapable.
fn contains(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// **One static statement** whatever the filters: each is a bound parameter that is null or a
/// value, and only `order by` is assembled — from enums, never from request text.
///
/// The `($n is null or ...)` shape costs a plan the planner cannot specialise per filter,
/// which is the right trade until the `ilike` search needs a trigram index anyway.
pub async fn page(pool: &PgPool, listing: &Listing, now: DateTime<Utc>) -> sqlx::Result<Page> {
    let search = (!listing.q.is_empty()).then(|| contains(&listing.q));
    let exactly = match listing.rank {
        Rank::Exactly(level) => Some(level.as_i32()),
        _ => None,
    };
    let administrators = matches!(listing.rank, Rank::Administrators);
    let banned = match listing.standing {
        Standing::Any => None,
        Standing::Banned => Some(true),
        Standing::Clear => Some(false),
    };

    let sql = format!(
        "select a.id, \
                a.display_name, \
                a.permission, \
                (select min(l.email) from links l \
                  where l.account_id = a.id and l.email is not null) as email, \
                live.in_force, \
                live.permanent, \
                live.until, \
                count(*) over () as total \
         from accounts a \
         left join lateral ( \
             select count(*) as in_force, \
                    coalesce(bool_or(b.expires_at is null), false) as permanent, \
                    max(b.expires_at) as until \
             from bans b \
             where b.account_id = a.id \
               and b.lifted_at is null \
               and (b.expires_at is null or b.expires_at > $1) \
         ) live on true \
         where ($2::text is null \
                 or a.display_name ilike $2 escape '\\' \
                 or exists (select 1 from links l \
                             where l.account_id = a.id and l.email ilike $2 escape '\\')) \
           and ($3::int is null or a.permission = $3) \
           and (not $4::bool or a.permission > 0) \
           and ($5::bool is null or (live.in_force > 0) = $5) \
         {} \
         limit $6 offset $7",
        listing.order_by(),
    );

    let rows: Vec<IndexRow> = sqlx::query_as(&sql)
        .bind(now)
        .bind(search)
        .bind(exactly)
        .bind(administrators)
        .bind(banned)
        .bind(i64::from(listing.per))
        .bind(listing.offset())
        .fetch_all(pool)
        .await?;

    // The window comes back *with* the rows, so an empty page carries no total. Paging past
    // the end is the ordinary way here.
    let total = rows.first().map_or(0, |r| r.7);
    Ok(Page {
        rows: rows
            .into_iter()
            .map(|r| Row {
                id: r.0,
                display_name: r.1,
                level: Level::from_stored(r.2),
                email: r.3,
                in_force: r.4,
                permanent: r.5,
                until: r.6,
            })
            .collect(),
        total,
        listing: listing.clone(),
    })
}

/// For the one case where "nothing matches" and "you paged past the end" differ.
pub async fn total_matching(
    pool: &PgPool,
    listing: &Listing,
    now: DateTime<Utc>,
) -> sqlx::Result<i64> {
    let search = (!listing.q.is_empty()).then(|| contains(&listing.q));
    let exactly = match listing.rank {
        Rank::Exactly(level) => Some(level.as_i32()),
        _ => None,
    };
    let administrators = matches!(listing.rank, Rank::Administrators);
    let banned = match listing.standing {
        Standing::Any => None,
        Standing::Banned => Some(true),
        Standing::Clear => Some(false),
    };
    let (total,): (i64,) = sqlx::query_as(
        "select count(*) from accounts a \
         left join lateral ( \
             select count(*) as in_force from bans b \
             where b.account_id = a.id and b.lifted_at is null \
               and (b.expires_at is null or b.expires_at > $1) \
         ) live on true \
         where ($2::text is null \
                 or a.display_name ilike $2 escape '\\' \
                 or exists (select 1 from links l \
                             where l.account_id = a.id and l.email ilike $2 escape '\\')) \
           and ($3::int is null or a.permission = $3) \
           and (not $4::bool or a.permission > 0) \
           and ($5::bool is null or (live.in_force > 0) = $5)",
    )
    .bind(now)
    .bind(search)
    .bind(exactly)
    .bind(administrators)
    .bind(banned)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::DEFAULT_PER;

    fn page_of(total: i64, rows: usize, listing: Listing) -> Page {
        Page {
            rows: (0..rows)
                .map(|n| Row {
                    id: Uuid::new_v4(),
                    display_name: format!("account {n}"),
                    level: Level::PLAYER,
                    email: None,
                    in_force: 0,
                    permanent: false,
                    until: None,
                })
                .collect(),
            total,
            listing,
        }
    }

    #[test]
    fn a_wildcard_in_a_search_is_searched_for_rather_than_obeyed() {
        assert_eq!(contains("100%"), "%100\\%%");
        assert_eq!(contains("a_b"), "%a\\_b%");
        assert_eq!(contains("ada"), "%ada%");
        // The escape character itself, or the escaping would be escapable.
        assert_eq!(contains("a\\%b"), "%a\\\\\\%b%");
    }

    #[test]
    fn the_pager_counts_whole_pages_and_never_zero_of_them() {
        let listing = Listing::default();
        assert_eq!(page_of(0, 0, listing.clone()).pages(), 1);
        assert_eq!(page_of(1, 1, listing.clone()).pages(), 1);
        assert_eq!(
            page_of(i64::from(DEFAULT_PER), 25, listing.clone()).pages(),
            1
        );
        assert_eq!(
            page_of(i64::from(DEFAULT_PER) + 1, 25, listing.clone()).pages(),
            2,
        );
        assert_eq!(page_of(312, 25, listing).pages(), 13);
    }

    #[test]
    fn the_range_shown_counts_from_the_offset() {
        let listing = Listing::default();
        let second = page_of(312, 25, listing.at_page(2));
        assert_eq!(second.first(), 26);
        assert_eq!(second.last(), 50);
        // An empty page has no first row rather than a first row of one.
        assert_eq!(page_of(0, 0, listing).first(), 0);
    }
}
