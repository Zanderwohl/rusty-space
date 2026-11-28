//! The Galaxy section: the manifests on the CDN, the one the shard loaded, and how much craft
//! know about each of its chunks. See `lightcone/docs/34-galaxy-authoring.md`, "In the
//! administration console".
//!
//! Chunk files are binary and this service may not link the crate that reads them, so a chunk's
//! stars come from the shard, which has them decoded. A manifest is RON and is read here.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::AppState;
use crate::auth::Admin;
use crate::cdn::storage::Unavailable;
use crate::views::{self, Head};

pub const GALAXY: &str = "/galaxy";
const MANIFEST: &str = "/galaxy/manifests/{name}/{file}";
const CHUNK: &str = "/galaxy/chunks/{key}";

/// Under the CDN's root.
pub const PREFIX: &str = "galaxy/";

pub fn manifest_url(name: &str, file: &str) -> String {
    format!("/galaxy/manifests/{name}/{file}")
}

/// `tier` is `A` or `B`.
pub fn chunk_url(chunk: (i32, i32, i32), tier: &str) -> String {
    format!("/galaxy/chunks/{},{},{},{tier}", chunk.0, chunk.1, chunk.2)
}

pub fn routes() -> Router<AppState> {
    Router::new().route(GALAXY, get(index)).route(MANIFEST, get(manifest)).route(CHUNK, get(chunk))
}

/// Mirrors `lc_world::sky::galaxy::Tier`, which RON writes as a bare identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Tier {
    A,
    B,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::A => "A",
            Tier::B => "B",
        }
    }
}

/// Mirrors `lc_world::sky::galaxy::ManifestChunk`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Entry {
    pub chunk: (i32, i32, i32),
    pub tier: Tier,
    pub graph: String,
    pub file: String,
    pub hash: String,
    pub stars: u32,
}

/// Mirrors `lc_world::sky::galaxy::Manifest`, format 2. A test reads a manifest that
/// `lc-galaxy publish` wrote, since there is no compiler between the two.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub chunks: Vec<Entry>,
}

pub const MANIFEST_FORMAT: u32 = 2;

/// A manifest file on the CDN.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub name: String,
    pub file: String,
    pub bytes: u64,
    pub modified: DateTime<Utc>,
    /// Whether the shard loaded this one.
    pub loaded: bool,
}

/// Every `galaxy/<name>/<hash>.galaxy`, newest first; `loaded` against what the shard read.
pub fn stored(objects: &[crate::cdn::storage::Object], loaded: Option<&str>) -> Vec<Stored> {
    let mut out: Vec<Stored> = objects
        .iter()
        .filter_map(|o| {
            let rest = o.key.strip_prefix(PREFIX)?;
            let (name, file) = rest.split_once('/')?;
            if file.contains('/') || !file.ends_with(".galaxy") {
                return None;
            }
            Some(Stored {
                name: name.to_owned(),
                file: file.to_owned(),
                bytes: o.bytes,
                modified: o.modified,
                loaded: loaded.is_some_and(|l| l.ends_with(&o.key)),
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
    out
}

pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let m: Manifest = ron::from_str(text).map_err(|e| e.to_string())?;
    if m.format != MANIFEST_FORMAT {
        return Err(format!("format {}, but this console reads {MANIFEST_FORMAT}", m.format));
    }
    Ok(m)
}

/// A name or file from a URL, refused unless it is one path segment.
fn segment(s: &str) -> Option<&str> {
    (!s.is_empty() && !s.contains('/') && !s.starts_with('.')).then_some(s)
}

async fn index(State(state): State<AppState>, admin: Admin) -> Response {
    let galaxy = match state.shard() {
        None => Err(crate::shard::Missing::NotConfigured),
        Some(shard) => shard.galaxy(&admin.id.to_string()).await,
    };
    let loaded = galaxy.as_ref().ok().and_then(|g| g.as_ref()).map(|g| g.source.clone());
    let listed = match &state.cdn.storage {
        None => Err(Unavailable::NotConfigured),
        Some(storage) => storage.walk(&state.http, PREFIX).await,
    };
    let manifests = listed.as_ref().map(|objects| stored(objects, loaded.as_deref())).map_err(Clone::clone);
    views::shell(&state.assets, Head { title: "Galaxy" }, Some(&admin), views::galaxy::page(&galaxy, &manifests)).into_response()
}

async fn manifest(State(state): State<AppState>, admin: Admin, Path((name, file)): Path<(String, String)>) -> Response {
    let not_found = || {
        views::refusal(&state.assets, StatusCode::NOT_FOUND, "No such manifest", "The CDN holds no manifest by that name.", (GALAXY, "← Galaxy"))
    };
    let (Some(name), Some(file)) = (segment(&name), segment(&file)) else { return not_found() };
    let read = match &state.cdn.storage {
        None => Err(Unavailable::NotConfigured.said("the CDN's listing")),
        Some(storage) => match storage.get(&state.http, &format!("{PREFIX}{name}/{file}")).await {
            Ok(None) => return not_found(),
            Ok(Some(bytes)) => parse(&bytes),
            Err(why) => Err(why.said("the CDN")),
        },
    };
    let loaded = match state.shard() {
        Some(shard) => shard.galaxy(&admin.id.to_string()).await.ok().flatten(),
        None => None,
    };
    let is_loaded = loaded.as_ref().is_some_and(|g| g.source.ends_with(&format!("{PREFIX}{name}/{file}")));
    views::shell(
        &state.assets,
        Head { title: file },
        Some(&admin),
        views::galaxy::manifest_page(name, file, &read, loaded.as_ref().filter(|_| is_loaded)),
    )
    .into_response()
}

async fn chunk(State(state): State<AppState>, admin: Admin, Path(key): Path<String>) -> Response {
    let found = match state.shard() {
        None => Err(crate::shard::Missing::NotConfigured),
        Some(shard) => shard.galaxy_chunk(&admin.id.to_string(), &key).await,
    };
    let status = match &found {
        Err(crate::shard::Missing::NoChunk) => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    };
    let page = views::shell(&state.assets, Head { title: &format!("Chunk {key}") }, Some(&admin), views::galaxy::chunk_page(&key, &found));
    (status, page).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by `lc-galaxy publish`, trimmed to two chunks: the contract with that crate.
    const PUBLISHED: &str = r#"(
    format: 2,
    name: "phase-3",
    parent: Some("a78385468c481c9e"),
    chunks: [
        (
            chunk: (-1, -1, -1),
            tier: A,
            graph: "40bd10adcc87950b",
            file: "chunks/1e55cbd6aa873a55.lcg",
            hash: "1e55cbd6aa873a55",
            stars: 5,
        ),
        (
            chunk: (-1, -1, -1),
            tier: B,
            graph: "40bd10adcc87950b",
            file: "chunks/20b22961338aa81f.lcg",
            hash: "20b22961338aa81f",
            stars: 1210,
        ),
    ],
)"#;

    #[test]
    fn reads_a_published_manifest() {
        let m = parse(PUBLISHED.as_bytes()).unwrap();
        assert_eq!(m.parent.as_deref(), Some("a78385468c481c9e"));
        assert_eq!((m.chunks[0].tier, m.chunks[1].stars), (Tier::A, 1210));
        assert!(parse(PUBLISHED.replace("format: 2", "format: 1").as_bytes()).unwrap_err().contains("format 1"));
    }

    #[test]
    fn lists_only_manifests_and_marks_the_loaded_one() {
        let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let o = |key: &str| crate::cdn::storage::Object { key: key.into(), bytes: 10, modified: at, etag: None };
        let objects = [o("galaxy/p3/aaaa.galaxy"), o("galaxy/p3/chunks/bbbb.lcg"), o("galaxy/p3/cccc.galaxy"), o("galaxy/loose.galaxy")];
        let listed = stored(&objects, Some("http://lightcone-cdn:3101/galaxy/p3/cccc.galaxy"));
        let files: Vec<(&str, bool)> = listed.iter().map(|s| (s.file.as_str(), s.loaded)).collect();
        assert_eq!(files, vec![("aaaa.galaxy", false), ("cccc.galaxy", true)]);
        assert_eq!((segment("p3"), segment("../x"), segment("a/b")), (Some("p3"), None, None));
    }
}
