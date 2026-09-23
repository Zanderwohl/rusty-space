//! What the systems index is showing, as it travels in the URL.
//!
//! [`crate::listing`]'s idea, deliberately not its type: one struct with the union of both
//! indexes' parameters would put `?standing=` on a page with no standing to filter by. They
//! share only the `data-listing` contract with `ts/params.ts`.
//!
//! **The paging is not this service's**: the systems live on the shard, and this turns one
//! query string into the one the shard is asked.

use serde::Deserialize;

/// One per column, as in [`crate::listing`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Name,
    Ships,
    Objects,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Name, Sort::Ships, Sort::Objects];

    pub fn slug(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Ships => "ships",
            Sort::Objects => "objects",
        }
    }

    pub fn heading(self) -> &'static str {
        match self {
            Sort::Name => "System",
            Sort::Ships => "Ships",
            Sort::Objects => "Objects",
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
}

pub const PER_PAGE: [u32; 3] = [25, 50, 100];
pub const DEFAULT_PER: u32 = 25;

/// Every field a string; see [`crate::listing::Params`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Params {
    pub q: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub page: Option<String>,
    pub per: Option<String>,
}

/// The query string once it has been made sense of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub q: String,
    pub sort: Sort,
    pub dir: Dir,
    pub page: u32,
    pub per: u32,
}

impl Default for Listing {
    fn default() -> Self {
        Listing {
            q: String::new(),
            sort: Sort::default(),
            dir: Dir::default(),
            page: 1,
            per: DEFAULT_PER,
        }
    }
}

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

    pub fn query_string(&self) -> String {
        let fallback = Listing::default();
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if !self.q.is_empty() {
            pairs.push(("q", self.q.clone()));
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

    /// **Not** the same string: the console counts in pages, the shard in rows, and the page
    /// size is the interface's choice.
    pub fn shard_query(&self) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("q", &self.q)
            .append_pair("sort", self.sort.slug())
            .append_pair("dir", self.dir.slug())
            .append_pair("offset", &self.offset().to_string())
            .append_pair("limit", &self.per.to_string())
            .finish()
    }

    pub fn offset(&self) -> u64 {
        u64::from(self.page - 1) * u64::from(self.per)
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
        self.url(crate::routes::SYSTEMS)
    }

    pub fn partial_url(&self) -> String {
        self.url(crate::routes::SYSTEM_ROWS)
    }

    pub fn at_page(&self, page: u32) -> Listing {
        Listing {
            page: page.max(1),
            ..self.clone()
        }
    }

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

    /// At least one.
    pub fn pages(&self, total: u64) -> u32 {
        let per = u64::from(self.per);
        (total.div_ceil(per).max(1)) as u32
    }

    pub fn defaults_json() -> serde_json::Value {
        let fallback = Listing::default();
        serde_json::json!({
            "page": crate::routes::SYSTEMS,
            "partial": crate::routes::SYSTEM_ROWS,
            "target": "system-index",
            "order": ["q", "sort", "dir", "per", "page"],
            "defaults": {
                "q": "",
                "sort": fallback.sort.slug(),
                "dir": fallback.dir.slug(),
                "per": fallback.per.to_string(),
                "page": fallback.page.to_string(),
            },
        })
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
        assert_eq!(plain.page_url(), "/systems");
        assert_eq!(plain.partial_url(), "/systems/rows");
    }

    #[test]
    fn a_listing_survives_its_own_url() {
        let listing = Listing {
            q: "centauri".into(),
            sort: Sort::Ships,
            dir: Dir::Desc,
            page: 3,
            per: 50,
        };
        assert_eq!(parsed(&listing.query_string()), listing);
        assert_eq!(
            listing.query_string(),
            "q=centauri&sort=ships&dir=desc&per=50&page=3",
        );
    }

    /// **The console counts pages and the shard counts rows.** Getting this wrong is an index
    /// that shows page one forever, or one that skips the first twenty-five systems.
    #[test]
    fn the_shard_is_asked_in_rows_not_pages() {
        let first = Listing::default();
        assert_eq!(first.offset(), 0);
        assert!(first.shard_query().contains("offset=0"));
        assert!(first.shard_query().contains("limit=25"));

        let third = first.at_page(3);
        assert_eq!(third.offset(), 50);
        assert!(
            third.shard_query().contains("offset=50"),
            "{}",
            third.shard_query()
        );

        let bigger = Listing {
            per: 100,
            ..first.at_page(2)
        };
        assert_eq!(bigger.offset(), 100);
        assert!(bigger.shard_query().contains("limit=100"));
    }

    #[test]
    fn nonsense_falls_back_rather_than_failing() {
        for query in [
            "sort=nonsense",
            "dir=sideways",
            "page=-4",
            "page=x",
            "per=9999",
        ] {
            let listing = parsed(query);
            assert!(listing.page >= 1, "{query}");
            assert!(PER_PAGE.contains(&listing.per), "{query}");
        }
        assert_eq!(parsed("sort=nonsense").sort, Sort::Name);
        assert_eq!(parsed("page=-4").page, 1);
        assert_eq!(parsed("per=9999").per, DEFAULT_PER);
        assert_eq!(parsed("q=++sol++").q, "sol");
    }

    #[test]
    fn the_pager_counts_whole_pages_and_never_zero_of_them() {
        let listing = Listing::default();
        assert_eq!(listing.pages(0), 1);
        assert_eq!(listing.pages(1), 1);
        assert_eq!(listing.pages(25), 1);
        assert_eq!(listing.pages(26), 2);
        assert_eq!(listing.pages(7973), 319);
    }

    #[test]
    fn clicking_a_heading_flips_only_the_column_already_sorted_on() {
        let listing = Listing::default().at_page(4);
        let flipped = listing.sorted_by(Sort::Name);
        assert_eq!(flipped.dir, Dir::Desc);
        assert_eq!(flipped.page, 1, "a reorder kept the old page");

        let other = flipped.sorted_by(Sort::Ships);
        assert_eq!(other.sort, Sort::Ships);
        assert_eq!(other.dir, Dir::Asc);
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
        let all_defaults: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                defaults
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap())),
            )
            .finish();
        assert_eq!(parsed(&all_defaults).query_string(), "");
        assert_eq!(parsed(&all_defaults), Listing::default());
        // And it targets its own region, not the user index's.
        assert_eq!(json["target"], "system-index");
    }
}
