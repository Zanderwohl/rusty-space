//! What the CDN holds, asked of the storage behind it rather than of its public face.
//!
//! **S3's semantics, whatever answers.** [`Storage::list`] is `ListObjectsV2` with
//! `Delimiter=/`: the immediate children of a prefix, split into sub-prefixes and objects, and
//! an empty answer for a prefix that does not exist. Today the only backend is the development
//! CDN's internal listing port (`tools/dev-cdn/Caddyfile`); the bucket's is another arm here,
//! added with the bucket, and nothing above this module changes when it is.
//!
//! Keys are relative to the root with no leading slash, and a prefix ends in `/`.

use chrono::{DateTime, Utc};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;

pub enum Storage {
    /// The development CDN's `browse` port. One directory per request, behind basic auth.
    Caddy {
        base: String,
        user: String,
        password: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub prefixes: Vec<String>,
    pub objects: Vec<Object>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub key: String,
    pub bytes: u64,
    /// When the backend last saw the object change. For Caddy, the file's own modified time,
    /// which a copy may have carried over from the build; for S3, when it was uploaded.
    pub modified: DateTime<Utc>,
    /// S3's. Caddy has none.
    pub etag: Option<String>,
}

/// Why the storage could not be asked. Distinct, for the reason `shard::Missing` is.
#[derive(Clone, Debug, PartialEq)]
pub enum Unavailable {
    NotConfigured,
    Refused,
    Unreachable(String),
}

impl Unavailable {
    /// `what` is lowercase, as it reads mid-sentence.
    pub fn said(&self, what: &str) -> String {
        let mut chars = what.chars();
        let opening: String = chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default();
        match self {
            Unavailable::NotConfigured => format!("This console is not pointed at {what}."),
            Unavailable::Refused => format!("{opening} refused this console's credentials."),
            Unavailable::Unreachable(why) => format!("{opening} did not answer: {why}"),
        }
    }
}

/// Everything but the characters a key's path segments are made of, and `/` between them. `+`
/// stays: build ids carry one and a path reads it literally.
const PATH: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// Caddy's `browse` entry, as it answers `Accept: application/json`.
#[derive(Deserialize)]
struct Entry {
    name: String,
    size: u64,
    mod_time: DateTime<Utc>,
    is_dir: bool,
}

impl Storage {
    pub async fn list(&self, http: &reqwest::Client, prefix: &str) -> Result<Listing, Unavailable> {
        match self {
            Storage::Caddy {
                base,
                user,
                password,
            } => {
                let url = format!("{base}/{}", utf8_percent_encode(prefix, PATH));
                let response = http
                    .get(url)
                    .header(reqwest::header::ACCEPT, "application/json")
                    .basic_auth(user, Some(password))
                    .send()
                    .await
                    .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
                match response.status() {
                    s if s.is_success() => {}
                    // S3 answers a prefix with nothing under it with an empty list, not an error.
                    reqwest::StatusCode::NOT_FOUND => return Ok(Listing::default()),
                    reqwest::StatusCode::UNAUTHORIZED => return Err(Unavailable::Refused),
                    other => {
                        return Err(Unavailable::Unreachable(format!("it answered {other}")));
                    }
                }
                let body = response
                    .text()
                    .await
                    .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
                caddy_listing(prefix, &body).map_err(|why| {
                    tracing::error!(%why, %body, "the CDN's listing did not parse");
                    Unavailable::Unreachable(
                        "it answered in a shape this build does not read".into(),
                    )
                })
            }
        }
    }

    /// Every object under `prefix`, however deep. A request per level.
    pub async fn walk(
        &self,
        http: &reqwest::Client,
        prefix: &str,
    ) -> Result<Vec<Object>, Unavailable> {
        let mut pending = vec![prefix.to_owned()];
        let mut objects = Vec::new();
        while let Some(prefix) = pending.pop() {
            let listing = self.list(http, &prefix).await?;
            objects.extend(listing.objects);
            pending.extend(listing.prefixes);
        }
        objects.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(objects)
    }

    /// One object's bytes, or `None` when there is no such object.
    pub async fn get(
        &self,
        http: &reqwest::Client,
        key: &str,
    ) -> Result<Option<Vec<u8>>, Unavailable> {
        match self {
            Storage::Caddy {
                base,
                user,
                password,
            } => {
                let url = format!("{base}/{}", utf8_percent_encode(key, PATH));
                let response = http
                    .get(url)
                    .basic_auth(user, Some(password))
                    .send()
                    .await
                    .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
                match response.status() {
                    s if s.is_success() => {}
                    reqwest::StatusCode::NOT_FOUND => return Ok(None),
                    reqwest::StatusCode::UNAUTHORIZED => return Err(Unavailable::Refused),
                    other => {
                        return Err(Unavailable::Unreachable(format!("it answered {other}")));
                    }
                }
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|why| Unavailable::Unreachable(why.to_string()))?;
                Ok(Some(bytes.to_vec()))
            }
        }
    }
}

/// A directory's entries, as keys under `prefix`. Caddy marks a directory with `is_dir` and a
/// trailing `/` on its name.
fn caddy_listing(prefix: &str, body: &str) -> Result<Listing, serde_json::Error> {
    let entries: Vec<Entry> = serde_json::from_str(body)?;
    let mut listing = Listing::default();
    for entry in entries {
        let name = entry.name.trim_end_matches('/');
        if entry.is_dir {
            listing.prefixes.push(format!("{prefix}{name}/"));
        } else {
            listing.objects.push(Object {
                key: format!("{prefix}{name}"),
                bytes: entry.size,
                modified: entry.mod_time,
                etag: None,
            });
        }
    }
    Ok(listing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Captured from Caddy 2.10's `browse`** against a build directory shaped like a real
    /// one, asked with `Accept: application/json` — not written from its documentation.
    const FROM_CADDY: &str = r#"[{"name":"assets/","size":4096,"url":"./assets/","mod_time":"2026-09-25T08:50:18.364432399Z","mode":2147484141,"is_dir":true,"is_symlink":false},{"name":"lightcone_web_bg.wasm","size":5000,"url":"./lightcone_web_bg.wasm","mod_time":"2026-09-25T08:50:18.372367472Z","mode":420,"is_dir":false,"is_symlink":false},{"name":"lightcone_web_bg.wasm.br","size":1500,"url":"./lightcone_web_bg.wasm.br","mod_time":"2026-09-25T08:50:18.374207779Z","mode":420,"is_dir":false,"is_symlink":false},{"name":"manifest.json","size":32,"url":"./manifest.json","mod_time":"2026-09-25T08:50:18.370335647Z","mode":420,"is_dir":false,"is_symlink":false}]"#;

    #[test]
    fn a_caddy_listing_reads_as_s3_would_answer_it() {
        let prefix = "game/2026.09.1+a10eac4/";
        let listing = caddy_listing(prefix, FROM_CADDY).expect("Caddy's shape");
        assert_eq!(listing.prefixes, ["game/2026.09.1+a10eac4/assets/"]);
        let keys: Vec<&str> = listing.objects.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "game/2026.09.1+a10eac4/lightcone_web_bg.wasm",
                "game/2026.09.1+a10eac4/lightcone_web_bg.wasm.br",
                "game/2026.09.1+a10eac4/manifest.json",
            ]
        );
        assert_eq!(listing.objects[0].bytes, 5000);
        assert!(listing.objects.iter().all(|o| o.etag.is_none()));
    }

    #[test]
    fn a_key_is_encoded_as_a_path_and_keeps_its_plus() {
        let encoded = utf8_percent_encode("library/A Princess of Mars.epub", PATH).to_string();
        assert_eq!(encoded, "library/A%20Princess%20of%20Mars.epub");
        let encoded = utf8_percent_encode("game/2026.09.1+a10eac4/", PATH).to_string();
        assert_eq!(encoded, "game/2026.09.1+a10eac4/");
    }

    #[test]
    fn every_absence_says_what_was_asked() {
        let said: Vec<String> = [
            Unavailable::NotConfigured,
            Unavailable::Refused,
            Unavailable::Unreachable("connection refused".into()),
        ]
        .iter()
        .map(|u| u.said("the CDN"))
        .collect();
        let mut unique = said.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), said.len());
        assert!(said.iter().all(|s| s.contains("CDN")));
        assert!(said[1].starts_with("The CDN"), "{}", said[1]);
    }
}
