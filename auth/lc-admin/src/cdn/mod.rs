//! What the CDN holds, checked against what the site and the shelf say it should.
//!
//! Three sources, each optional, each failing on its own: the storage behind the CDN
//! ([`storage`]), the site's releases and the shelf's catalog ([`records`]). A source that does
//! not answer leaves its rows unchecked and says so; it never makes the others fail. The
//! comparison itself is [`inventory`], which asks nothing. See `lightcone/docs/14-hosting.md`.

pub mod inventory;
pub mod records;
pub mod storage;

use std::path::PathBuf;

use inventory::{Build, File, Shelved};
use records::Releases;
use storage::{Storage, Unavailable};

pub const GAME: &str = "game/";
pub const LIBRARY: &str = "library/";
pub const ICONS: &str = "icons/";

/// Where each source is. `None` is a deployment that has not been given it.
pub struct Sources {
    pub storage: Option<Storage>,
    /// The site's internal address, and its read token.
    pub site: Option<(String, String)>,
    pub catalog: Option<PathBuf>,
}

pub struct Index {
    pub storage: Result<(), Unavailable>,
    pub releases: Result<(), Unavailable>,
    pub catalog: Result<(), Unavailable>,
    pub builds: Vec<Build>,
    pub books: Vec<Shelved>,
    pub icons: Vec<File>,
}

pub struct BuildPage {
    pub storage: Result<(), Unavailable>,
    pub releases: Result<(), Unavailable>,
    pub build: Build,
    pub files: Vec<File>,
    /// Absent when the build has none, or none that parses.
    pub manifest: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Sources {
    async fn releases(&self, http: &reqwest::Client) -> Result<Releases, Unavailable> {
        match &self.site {
            None => Err(Unavailable::NotConfigured),
            Some((api, token)) => records::releases(http, api, token).await,
        }
    }

    pub async fn index(&self, http: &reqwest::Client) -> Index {
        let releases = self.releases(http).await;
        let catalog = match &self.catalog {
            None => Err(Unavailable::NotConfigured),
            Some(path) => records::books(path),
        };
        let stored = match &self.storage {
            None => Err(Unavailable::NotConfigured),
            Some(storage) => {
                // Four small requests, one after another: an index is not a hot path, and a
                // CDN that fails part-way is reported once rather than three times.
                async {
                    let game = storage.list(http, GAME).await?;
                    let library = storage.list(http, LIBRARY).await?;
                    let icons = storage.list(http, ICONS).await?;
                    Ok((game, library, icons))
                }
                .await
            }
        };
        let (game, library, icons) = stored.clone().unwrap_or_default();
        let mut builds = inventory::builds(&game.prefixes, releases.as_ref().ok());
        let mut books = inventory::books(
            &inventory::fold(LIBRARY, &library.objects),
            catalog.as_deref().ok(),
        );
        if stored.is_err() {
            inventory::unlisted(&mut builds, &mut books);
        }

        Index {
            builds,
            books,
            icons: inventory::fold(ICONS, &icons.objects),
            storage: stored.map(|_| ()),
            releases: releases.map(|_| ()),
            catalog: catalog.map(|_| ()),
        }
    }

    /// `None` when the build is neither stored nor registered: there is nothing to show.
    pub async fn build(&self, http: &reqwest::Client, id: &str) -> Option<BuildPage> {
        let releases = self.releases(http).await;
        let prefix = format!("{GAME}{id}/");
        let stored = match &self.storage {
            None => Err(Unavailable::NotConfigured),
            Some(storage) => storage.walk(http, &prefix).await,
        };
        let objects = stored.clone().unwrap_or_default();
        let listed = if objects.is_empty() {
            Vec::new()
        } else {
            vec![prefix.clone()]
        };
        let mut builds = inventory::builds(&listed, releases.as_ref().ok());
        if stored.is_err() {
            inventory::unlisted(&mut builds, &mut []);
        }
        let build = builds.into_iter().find(|b| b.id == id)?;

        let manifest = match &self.storage {
            Some(storage) if !objects.is_empty() => storage
                .get(http, &format!("{prefix}manifest.json"))
                .await
                .ok()
                .flatten()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
            _ => None,
        };
        Some(BuildPage {
            storage: stored.map(|_| ()),
            releases: releases.map(|_| ()),
            build,
            files: inventory::fold(&prefix, &objects),
            manifest,
        })
    }
}
