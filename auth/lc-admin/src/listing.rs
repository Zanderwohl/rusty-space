//! What the user index is showing, as it travels in the URL.
//!
//! **The URL is the state** — nothing about the index lives in a cookie, a session or the DOM.
//! One type serves the page handler, the partial handler and, through
//! [`Listing::defaults_json`], `ts/params.ts`, so the three cannot disagree about whether
//! `?page=1` is worth writing down.
//!
//! **Parsing never fails.** A hand-edited `?sort=nonsense` falls back rather than answering
//! 400, and admits nothing: every value here is an enum and the SQL is built from the enum.

use lc_identity::level::Level;
use serde::Deserialize;

/// Which column the index is ordered by.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
/// One per column. Sorting is by clicking a heading, so an ordering without one is a feature
/// with no way in — which is why `created` left with the Joined column.
pub enum Sort {
    #[default]
    Name,
    Level,
    Status,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Name, Sort::Level, Sort::Status];

    pub fn slug(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Level => "level",
            Sort::Status => "status",
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Sort::Name => "Account",
            Sort::Level => "Level",
            Sort::Status => "Standing",
        }
    }

    /// **Never interpolated from a request**: the text comes from this match alone, which is
    /// what makes a concatenated `order by` safe.
    fn column(self) -> &'static str {
        match self {
            Sort::Name => "lower(a.display_name)",
            // 0 is a player and belongs at the bottom of a seniority sort, not the top.
            Sort::Level => "nullif(a.permission, 0)",

            Sort::Status => "(live.in_force > 0)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dir {
    #[default]
    Asc,
    Desc,
}

impl Dir {
    pub fn slug(self) -> &'static str {
        match self {
            Dir::Asc => "asc",
            Dir::Desc => "desc",
        }
    }

    pub fn flipped(self) -> Dir {
        match self {
            Dir::Asc => Dir::Desc,
            Dir::Desc => Dir::Asc,
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Dir::Asc => "asc",
            Dir::Desc => "desc",
        }
    }
}

/// Whether the index is narrowed to accounts with a ban in force.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Standing {
    #[default]
    Any,
    Banned,
    Clear,
}

impl Standing {
    pub const ALL: [Standing; 3] = [Standing::Any, Standing::Banned, Standing::Clear];

    pub fn slug(self) -> &'static str {
        match self {
            Standing::Any => "any",
            Standing::Banned => "banned",
            Standing::Clear => "clear",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Standing::Any => "Any standing",
            Standing::Banned => "Banned",
            Standing::Clear => "Not banned",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Rank {
    #[default]
    Any,
    /// The filter wanted most often, and not expressible as one level.
    Administrators,
    Exactly(Level),
}

impl Rank {
    pub fn all() -> Vec<Rank> {
        let mut all = vec![Rank::Any, Rank::Administrators];
        all.extend(Level::ALL.map(Rank::Exactly));
        all
    }

    pub fn slug(self) -> &'static str {
        match self {
            Rank::Any => "any",
            Rank::Administrators => "admins",
            Rank::Exactly(level) => level.slug(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rank::Any => "Any level",
            Rank::Administrators => "Any administrator",
            Rank::Exactly(level) => level.name(),
        }
    }

    fn from_slug(slug: &str) -> Option<Rank> {
        match slug {
            "any" => Some(Rank::Any),
            "admins" => Some(Rank::Administrators),
            other => Level::from_slug(other).map(Rank::Exactly),
        }
    }
}

/// Closed, so `limit` is never handed a number from a URL: unbounded is a query that reads
/// every account.
pub const PER_PAGE: [u32; 3] = [25, 50, 100];
pub const DEFAULT_PER: u32 = 25;

/// **Every field a string**, including the numbers: a `page: Option<u32>` makes `?page=-4` an
/// axum rejection — a bare 400 — before the fallback above can run.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Params {
    pub q: Option<String>,
    pub level: Option<String>,
    pub standing: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub page: Option<String>,
    pub per: Option<String>,
}

/// The query string once it has been made sense of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub q: String,
    pub rank: Rank,
    pub standing: Standing,
    pub sort: Sort,
    pub dir: Dir,
    /// One-based, as the interface counts.
    pub page: u32,
    pub per: u32,
}

impl Default for Listing {
    fn default() -> Self {
        Listing {
            q: String::new(),
            rank: Rank::default(),
            standing: Standing::default(),
            sort: Sort::default(),
            dir: Dir::default(),
            page: 1,
            per: DEFAULT_PER,
        }
    }
}

/// A `like` against two columns, so a megabyte of it is work on request. Truncated, not
/// refused.
const MAX_SEARCH: usize = 100;

impl Listing {
    pub fn from_params(params: Params) -> Listing {
        let fallback = Listing::default();
        Listing {
            q: params
                .q
                .unwrap_or_default()
                .trim()
                .chars()
                .take(MAX_SEARCH)
                .collect(),
            rank: params
                .level
                .as_deref()
                .and_then(Rank::from_slug)
                .unwrap_or(fallback.rank),
            standing: params
                .standing
                .as_deref()
                .and_then(|s| Standing::ALL.into_iter().find(|v| v.slug() == s))
                .unwrap_or(fallback.standing),
            sort: params
                .sort
                .as_deref()
                .and_then(|s| Sort::ALL.into_iter().find(|v| v.slug() == s))
                .unwrap_or(fallback.sort),
            dir: match params.dir.as_deref() {
                Some("desc") => Dir::Desc,
                Some("asc") => Dir::Asc,
                _ => fallback.dir,
            },
            page: params
                .page
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1),
            per: params
                .per
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|p| PER_PAGE.contains(p))
                .unwrap_or(fallback.per),
        }
    }

    /// Every parameter that differs from its default, in a fixed order — so two ways to the
    /// same view give the same URL, and the common view is `/users`.
    pub fn query_string(&self) -> String {
        let fallback = Listing::default();
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if !self.q.is_empty() {
            pairs.push(("q", self.q.clone()));
        }
        if self.rank != fallback.rank {
            pairs.push(("level", self.rank.slug().to_owned()));
        }
        if self.standing != fallback.standing {
            pairs.push(("standing", self.standing.slug().to_owned()));
        }
        if self.sort != fallback.sort {
            pairs.push(("sort", self.sort.slug().to_owned()));
        }
        if self.dir != fallback.dir {
            pairs.push(("dir", self.dir.slug().to_owned()));
        }
        if self.per != fallback.per {
            pairs.push(("per", self.per.to_string()));
        }

        if self.page != fallback.page {
            pairs.push(("page", self.page.to_string()));
        }
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish()
    }

    fn url(&self, path: &str) -> String {
        let query = self.query_string();
        if query.is_empty() {
            path.to_owned()
        } else {
            format!("{path}?{query}")
        }
    }

    pub fn page_url(&self) -> String {
        self.url(crate::routes::USERS)
    }

    pub fn partial_url(&self) -> String {
        self.url(crate::routes::USER_ROWS)
    }

    pub fn at_page(&self, page: u32) -> Listing {
        Listing {
            page: page.max(1),
            ..self.clone()
        }
    }

    /// Clicking the current column flips it, another sorts afresh. Either way back to page
    /// one: page 7 of a list just reordered is a slice nobody asked for.
    pub fn sorted_by(&self, sort: Sort) -> Listing {
        let dir = if self.sort == sort {
            self.dir.flipped()
        } else {
            Dir::Asc
        };
        Listing {
            sort,
            dir,
            page: 1,
            ..self.clone()
        }
    }

    /// Rendered into the page so `ts/params.ts` canonicalizes against this same table.
    pub fn defaults_json() -> serde_json::Value {
        let fallback = Listing::default();
        serde_json::json!({
            "page": crate::routes::USERS,
            "partial": crate::routes::USER_ROWS,
            "target": "user-index",

            "order": ["q", "level", "standing", "sort", "dir", "per", "page"],
            "defaults": {
                "q": "",
                "level": fallback.rank.slug(),
                "standing": fallback.standing.slug(),
                "sort": fallback.sort.slug(),
                "dir": fallback.dir.slug(),
                "per": fallback.per.to_string(),
                "page": fallback.page.to_string(),
            },
        })
    }

    /// The trailing `a.id` is load-bearing: without a total order, two accounts created in
    /// the same second can swap places between the query for page 1 and the query for page 2.
    pub(crate) fn order_by(&self) -> String {
        let nulls = match self.dir {
            Dir::Asc => "nulls last",
            Dir::Desc => "nulls first",
        };
        format!(
            "order by {} {} {nulls}, a.id asc",
            self.sort.column(),
            self.dir.sql(),
        )
    }

    pub(crate) fn offset(&self) -> i64 {
        i64::from(self.page - 1) * i64::from(self.per)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(query: &str) -> Listing {
        let params: Params = serde_urlencoded::from_str(query).expect("a query string");
        Listing::from_params(params)
    }

    #[test]
    fn the_default_view_has_an_empty_query_string() {
        let plain = Listing::default();
        assert_eq!(plain.query_string(), "");
        assert_eq!(plain.page_url(), "/users");
        assert_eq!(plain.partial_url(), "/users/rows");
    }

    /// The round trip the address bar depends on.
    #[test]
    fn a_listing_survives_its_own_url() {
        let listing = Listing {
            q: "ada".into(),
            rank: Rank::Exactly(Level::DEBUG),
            standing: Standing::Banned,
            sort: Sort::Status,
            dir: Dir::Desc,
            page: 3,
            per: 50,
        };
        assert_eq!(parsed(&listing.query_string()), listing);
        // And the order is fixed, so two routes to the same view give the same string.
        assert_eq!(
            listing.query_string(),
            "q=ada&level=debug&standing=banned&sort=status&dir=desc&per=50&page=3",
        );
    }

    /// Nothing a person can type into the address bar is an error.
    #[test]
    fn nonsense_falls_back_rather_than_failing() {
        for query in [
            "sort=nonsense",
            "dir=sideways",
            "level=god",
            "standing=maybe",
            "page=0",
            // Negative and non-numeric, which would be an extractor rejection rather than a
            // fallback if the fields were typed as numbers. See `Params`.
            "page=-4",
            "page=nine",
            "page=99999999999999999999",
            "per=1000000",
            "per=26",
            "per=lots",
            "q=",
        ] {
            let listing = parsed(query);
            assert!(listing.page >= 1, "{query} produced page {}", listing.page);
            assert!(
                PER_PAGE.contains(&listing.per),
                "{query} produced per {}",
                listing.per,
            );
        }
        assert_eq!(parsed("sort=nonsense").sort, Sort::Name);
        assert_eq!(parsed("page=0").page, 1);
        assert_eq!(parsed("page=-4").page, 1);
        assert_eq!(parsed("page=nine").page, 1);
        // An unbounded page size is the one that would actually hurt.
        assert_eq!(parsed("per=1000000").per, DEFAULT_PER);
    }

    #[test]
    fn a_search_term_is_trimmed_and_bounded() {
        assert_eq!(parsed("q=++ada++").q, "ada");
        let long = "x".repeat(MAX_SEARCH * 10);
        assert_eq!(parsed(&format!("q={long}")).q.len(), MAX_SEARCH);
    }

    #[test]
    fn clicking_a_heading_flips_only_the_column_already_sorted_on() {
        let listing = Listing {
            page: 4,
            ..Listing::default()
        };
        assert_eq!(listing.sort, Sort::Name);

        let flipped = listing.sorted_by(Sort::Name);
        assert_eq!(flipped.dir, Dir::Desc);
        assert_eq!(flipped.page, 1, "a reorder kept the old page");

        let other = flipped.sorted_by(Sort::Level);
        assert_eq!(other.sort, Sort::Level);
        assert_eq!(
            other.dir,
            Dir::Asc,
            "a fresh column did not start ascending"
        );
        // Flipping twice comes back.
        assert_eq!(other.sorted_by(Sort::Level).dir, Dir::Desc);
    }

    /// Every paged query needs a unique tie-break or page 2 repeats rows from page 1.
    #[test]
    fn every_ordering_is_total() {
        for sort in Sort::ALL {
            for dir in [Dir::Asc, Dir::Desc] {
                let order = Listing {
                    sort,
                    dir,
                    ..Listing::default()
                }
                .order_by();
                assert!(
                    order.ends_with("a.id asc"),
                    "{sort:?} {dir:?} has no tie-break: {order}",
                );
                assert!(order.contains(sort.column()), "{order}");
            }
        }
    }

    #[test]
    fn offsets_count_from_the_first_page() {
        let first = Listing::default();
        assert_eq!(first.offset(), 0);
        assert_eq!(first.at_page(2).offset(), i64::from(DEFAULT_PER));
        assert_eq!(first.at_page(0).page, 1, "page zero was admitted");
    }

    /// The browser canonicalizes against this, so it has to name every parameter
    /// `query_string` knows about and agree about their order.
    #[test]
    fn the_defaults_the_browser_reads_are_the_defaults_here() {
        let json = Listing::defaults_json();
        let order: Vec<String> = serde_json::from_value(json["order"].clone()).unwrap();
        let defaults = json["defaults"].as_object().unwrap();
        assert_eq!(order.len(), defaults.len());
        for key in &order {
            assert!(defaults.contains_key(key), "{key} has no default");
        }

        // Every default, set explicitly, must still produce the empty query string — which is
        // the property the browser relies on when it drops them.
        let all_defaults: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                defaults
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap())),
            )
            .finish();
        assert_eq!(parsed(&all_defaults).query_string(), "");
        assert_eq!(parsed(&all_defaults), Listing::default());
    }
}
