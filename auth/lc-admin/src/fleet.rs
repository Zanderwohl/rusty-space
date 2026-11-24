//! What the ships index is showing, as it travels in the URL.
//!
//! [`crate::catalog`]'s shape: the ships live on the shard, which pages them, newest first.

use serde::Deserialize;

/// Which ships, by whether they still fly.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Flying {
    #[default]
    Any,
    Flying,
    Ended,
}

impl Flying {
    pub const ALL: [Flying; 3] = [Flying::Any, Flying::Flying, Flying::Ended];

    pub fn slug(self) -> &'static str {
        match self {
            Flying::Any => "any",
            Flying::Flying => "flying",
            Flying::Ended => "ended",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Flying::Any => "Any",
            Flying::Flying => "Flying",
            Flying::Ended => "Collapsed",
        }
    }
}

pub use crate::catalog::{DEFAULT_PER, PER_PAGE};

/// Every field a string; see [`crate::listing::Params`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Params {
    pub q: Option<String>,
    pub state: Option<String>,
    pub page: Option<String>,
    pub per: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub q: String,
    pub state: Flying,
    pub page: u32,
    pub per: u32,
}

impl Default for Listing {
    fn default() -> Self {
        Listing { q: String::new(), state: Flying::default(), page: 1, per: DEFAULT_PER }
    }
}

const MAX_SEARCH: usize = 100;

impl Listing {
    pub fn from_params(params: Params) -> Listing {
        Listing {
            q: params.q.unwrap_or_default().trim().chars().take(MAX_SEARCH).collect(),
            state: params
                .state
                .as_deref()
                .and_then(|s| Flying::ALL.into_iter().find(|v| v.slug() == s))
                .unwrap_or_default(),
            page: params.page.and_then(|p| p.parse::<u32>().ok()).unwrap_or(1).max(1),
            per: params
                .per
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|p| PER_PAGE.contains(p))
                .unwrap_or(DEFAULT_PER),
        }
    }

    pub fn query_string(&self) -> String {
        let fallback = Listing::default();
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if !self.q.is_empty() {
            pairs.push(("q", self.q.clone()));
        }
        if self.state != fallback.state {
            pairs.push(("state", self.state.slug().to_owned()));
        }
        if self.per != fallback.per {
            pairs.push(("per", self.per.to_string()));
        }
        if self.page != fallback.page {
            pairs.push(("page", self.page.to_string()));
        }
        url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
    }

    /// In rows, as the shard counts.
    pub fn shard_query(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("q", &self.q);
        if self.state != Flying::Any {
            query.append_pair("state", self.state.slug());
        }
        query
            .append_pair("offset", &self.offset().to_string())
            .append_pair("limit", &self.per.to_string())
            .finish()
    }

    pub fn offset(&self) -> u64 {
        u64::from(self.page - 1) * u64::from(self.per)
    }

    fn url(&self, path: &str) -> String {
        let query = self.query_string();
        if query.is_empty() { path.to_owned() } else { format!("{path}?{query}") }
    }

    pub fn page_url(&self) -> String {
        self.url(crate::routes::SHIPS)
    }

    pub fn partial_url(&self) -> String {
        self.url(crate::routes::SHIP_ROWS)
    }

    pub fn at_page(&self, page: u32) -> Listing {
        Listing { page: page.max(1), ..self.clone() }
    }

    /// At least one.
    pub fn pages(&self, total: u64) -> u32 {
        total.div_ceil(u64::from(self.per)).max(1) as u32
    }

    pub fn defaults_json() -> serde_json::Value {
        let fallback = Listing::default();
        serde_json::json!({
            "page": crate::routes::SHIPS,
            "partial": crate::routes::SHIP_ROWS,
            "target": "ship-index",
            "order": ["q", "state", "per", "page"],
            "defaults": {
                "q": "",
                "state": fallback.state.slug(),
                "per": fallback.per.to_string(),
                "page": fallback.page.to_string(),
            },
        })
    }
}

/// The query the shard is asked for one account's ships, newest first.
pub fn of_account(account: uuid::Uuid) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("account", &account.to_string())
        .append_pair("limit", "100")
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(query: &str) -> Listing {
        Listing::from_params(serde_urlencoded::from_str(query).expect("a query string"))
    }

    #[test]
    fn defaults_are_left_out_of_the_url() {
        assert_eq!(parsed("state=any&page=1&per=25").query_string(), "");
        assert_eq!(parsed("state=ended&page=3&q=Tachi").query_string(), "q=Tachi&state=ended&page=3");
    }

    #[test]
    fn the_shard_is_asked_in_rows() {
        let listing = parsed("state=flying&page=3&per=50");
        assert_eq!(listing.shard_query(), "q=&state=flying&offset=100&limit=50");
        assert_eq!(parsed("state=nonsense").state, Flying::Any);
    }
}
