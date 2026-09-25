//! What is meant to be on the CDN: the site's releases and channels, and the shelf's catalog.
//!
//! **The release types are a copy**, like `crate::shard`'s: the originals are
//! `lc_web::releases`, in another workspace, and JSON crosses instead. The captured payload in
//! the tests is what notices a field renamed over there.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::storage::Unavailable;

/// Mirrors `lc_web::releases::Release`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Release {
    pub build_id: String,
    pub cdn_base: String,
    pub wasm_bytes: Option<i64>,
    pub notes: Option<String>,
    pub yanked: bool,
    pub published_at: DateTime<Utc>,
}

/// Mirrors `lc_web::releases::Channel`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Channel {
    pub name: String,
    pub build_id: String,
    pub updated_at: DateTime<Utc>,
}

/// What `GET /internal/releases` answers.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Releases {
    pub releases: Vec<Release>,
    pub channels: Vec<Channel>,
}

/// The header `lc_web::internal` reads its tokens from.
const TOKEN_HEADER: &str = "x-release-token";

/// With the site's **read** token, which opens this listing and nothing else.
pub async fn releases(
    http: &reqwest::Client,
    site_api: &str,
    token: &str,
) -> Result<Releases, Unavailable> {
    let response = http
        .get(format!("{site_api}/internal/releases"))
        .header(TOKEN_HEADER, token)
        .send()
        .await
        .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
    match response.status() {
        s if s.is_success() => {}
        reqwest::StatusCode::UNAUTHORIZED => return Err(Unavailable::Refused),
        other => return Err(Unavailable::Unreachable(format!("it answered {other}"))),
    }
    let body = response
        .text()
        .await
        .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
    serde_json::from_str(&body).map_err(|why| {
        tracing::error!(%why, %body, "the site's releases did not parse");
        Unavailable::Unreachable("it answered in a shape this build does not read".into())
    })
}

/// One book in the shelf's catalog. The catalog has more; this is what an inventory checks.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub file: String,
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct Shelf {
    #[serde(default)]
    book: Vec<Book>,
}

/// `crates/lc-client/assets/books/books.toml`, mounted into the container. Read per request:
/// it is small, and a catalog edited on disk shows without a restart.
pub fn books(path: &Path) -> Result<Vec<Book>, Unavailable> {
    let text = std::fs::read_to_string(path)
        .map_err(|why| Unavailable::Unreachable(format!("{}: {why}", path.display())))?;
    parse_books(&text).map_err(|why| Unavailable::Unreachable(why.to_string()))
}

fn parse_books(text: &str) -> Result<Vec<Book>, toml::de::Error> {
    Ok(toml::from_str::<Shelf>(text)?.book)
}

/// Every channel pointing at each build.
pub fn channels_by_build(releases: &Releases) -> HashMap<&str, Vec<&str>> {
    let mut by_build: HashMap<&str, Vec<&str>> = HashMap::new();
    for channel in &releases.channels {
        by_build
            .entry(channel.build_id.as_str())
            .or_default()
            .push(channel.name.as_str());
    }
    for names in by_build.values_mut() {
        names.sort_unstable();
    }
    by_build
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Captured from the site's own serializer** — `lc_web::internal::list`'s `json!` over
    /// its `Release` and `Channel` — not written from memory.
    const FROM_THE_SITE: &str = r#"{"channels":[{"build_id":"2026.09.1+a10eac4","name":"stable","updated_at":"2026-09-20T10:00:00Z"}],"releases":[{"build_id":"2026.09.1+a10eac4","cdn_base":"https://cdn.lc.zanderlowry.com","notes":null,"published_at":"2026-09-19T09:30:00Z","wasm_bytes":24118272,"yanked":false}]}"#;

    #[test]
    fn parses_what_the_site_sends() {
        let releases: Releases = serde_json::from_str(FROM_THE_SITE).expect("the site's shape");
        assert_eq!(releases.releases[0].build_id, "2026.09.1+a10eac4");
        assert_eq!(releases.releases[0].wasm_bytes, Some(24118272));
        assert!(!releases.releases[0].yanked);
        assert_eq!(releases.channels[0].name, "stable");
        let by_build = channels_by_build(&releases);
        assert_eq!(by_build["2026.09.1+a10eac4"], ["stable"]);
    }

    /// The catalog's real header and one real entry, fields the inventory ignores included.
    #[test]
    fn reads_the_shelf_catalog_and_ignores_what_it_does_not_check() {
        let text = r#"
# The shelf.
[[book]]
id      = "a-princess-of-mars"
title   = "A Princess of Mars"
authors = [{ name = "Edgar Rice Burroughs", sort = "Burroughs, Edgar Rice" }]
year    = 1912
subjects = ["Science fiction"]
file    = "A Princess of Mars.epub"
sha256  = "6aacebcc8969ce4d79e59f05b326e10217c5211cda853a022f2c63978376075f"
"#;
        let books = parse_books(text).expect("the catalog's shape");
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].id, "a-princess-of-mars");
        assert_eq!(books[0].file, "A Princess of Mars.epub");
        assert!(books[0].sha256.is_some());
    }
}
