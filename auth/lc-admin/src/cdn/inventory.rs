//! What is on the CDN against what is meant to be, as plain data in and plain data out.
//!
//! Nothing here asks anything: [`super::storage`] and [`super::records`] do, and the tests
//! hand these functions listings directly.

use std::cmp::Reverse;

use chrono::{DateTime, Utc};

use super::records::{Book, Release, Releases, channels_by_build};
use super::storage::Object;

/// Encodings the development CDN keeps beside a file rather than inside it.
const ENCODINGS: [&str; 2] = ["br", "gz"];

/// One file, with any pre-compressed copies of it folded in.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    /// Relative to the area or build it was listed under.
    pub name: String,
    pub bytes: u64,
    pub modified: DateTime<Utc>,
    /// `br`, `gz`: copies beside it. On a bucket the encoding is on the object and this is empty.
    pub encodings: Vec<&'static str>,
}

/// Objects under `prefix`, each `x.br` and `x.gz` folded into `x`. A compressed copy whose
/// original is absent stays a file of its own, since that is what is actually there.
pub fn fold(prefix: &str, objects: &[Object]) -> Vec<File> {
    let name = |o: &Object| o.key.strip_prefix(prefix).unwrap_or(&o.key).to_owned();
    let mut files: Vec<File> = Vec::new();
    let mut copies: Vec<(String, &'static str)> = Vec::new();
    for object in objects {
        let n = name(object);
        let copy = ENCODINGS.iter().find_map(|e| {
            let base = n.strip_suffix(&format!(".{e}"))?;
            objects
                .iter()
                .any(|o| name(o) == base)
                .then(|| (base.to_owned(), *e))
        });
        match copy {
            Some(copy) => copies.push(copy),
            None => files.push(File {
                name: n,
                bytes: object.bytes,
                modified: object.modified,
                encodings: Vec::new(),
            }),
        }
    }
    for (base, encoding) in copies {
        if let Some(file) = files.iter_mut().find(|f| f.name == base) {
            file.encodings.push(encoding);
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStatus {
    /// Registered, stored and not pulled. Whether players get it is its channels.
    Released,
    Yanked,
    /// Stored, and nothing on the site knows it: a leftover or a mistake.
    Unreleased,
    /// **Registered and not stored.** Promoting it breaks `/play`.
    Missing,
    /// Stored, and the site could not be asked about it.
    Unchecked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub id: String,
    pub status: BuildStatus,
    pub channels: Vec<String>,
    pub release: Option<Release>,
}

/// `stored` is the build directories under `game/`, as prefixes; `releases` is `None` when the
/// site could not be asked. Missing builds first, then newest registered, then the rest by id.
pub fn builds(stored: &[String], releases: Option<&Releases>) -> Vec<Build> {
    let on_cdn: Vec<&str> = stored
        .iter()
        .filter_map(|p| p.strip_prefix("game/")?.strip_suffix('/'))
        .collect();
    let Some(releases) = releases else {
        let mut builds: Vec<Build> = on_cdn
            .iter()
            .map(|id| Build {
                id: id.to_string(),
                status: BuildStatus::Unchecked,
                channels: Vec::new(),
                release: None,
            })
            .collect();
        builds.sort_by(|a, b| a.id.cmp(&b.id));
        return builds;
    };
    let channels = channels_by_build(releases);
    let channels_of = |id: &str| -> Vec<String> {
        channels
            .get(id)
            .map(|names| names.iter().map(|n| n.to_string()).collect())
            .unwrap_or_default()
    };

    let mut builds: Vec<Build> = releases
        .releases
        .iter()
        .map(|release| Build {
            id: release.build_id.clone(),
            status: match (on_cdn.contains(&release.build_id.as_str()), release.yanked) {
                (false, _) => BuildStatus::Missing,
                (true, true) => BuildStatus::Yanked,
                (true, false) => BuildStatus::Released,
            },
            channels: channels_of(&release.build_id),
            release: Some(release.clone()),
        })
        .collect();
    for id in on_cdn {
        if !releases.releases.iter().any(|r| r.build_id == id) {
            builds.push(Build {
                id: id.to_owned(),
                status: BuildStatus::Unreleased,
                channels: channels_of(id),
                release: None,
            });
        }
    }
    builds.sort_by_key(|b| {
        (
            b.status != BuildStatus::Missing,
            b.release.is_none(),
            Reverse(b.release.as_ref().map(|r| r.published_at)),
            b.id.clone(),
        )
    });
    builds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookStatus {
    /// In the catalog and stored.
    Shelved,
    /// Stored and in no catalog entry.
    Uncatalogued,
    /// In the catalog and not stored: a reader opening it gets nothing.
    Missing,
    /// No catalog to check against.
    Unchecked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shelved {
    pub book: Option<Book>,
    pub stored: Option<File>,
    pub status: BookStatus,
}

/// `stored` is `library/` folded. Missing first, then by title or file name.
pub fn books(stored: &[File], catalog: Option<&[Book]>) -> Vec<Shelved> {
    let Some(catalog) = catalog else {
        return stored
            .iter()
            .map(|file| Shelved {
                book: None,
                stored: Some(file.clone()),
                status: BookStatus::Unchecked,
            })
            .collect();
    };
    let mut shelf: Vec<Shelved> = catalog
        .iter()
        .map(|book| {
            let file = stored.iter().find(|f| f.name == book.file).cloned();
            Shelved {
                status: match file {
                    Some(_) => BookStatus::Shelved,
                    None => BookStatus::Missing,
                },
                book: Some(book.clone()),
                stored: file,
            }
        })
        .collect();
    for file in stored {
        if !catalog.iter().any(|b| b.file == file.name) {
            shelf.push(Shelved {
                book: None,
                stored: Some(file.clone()),
                status: BookStatus::Uncatalogued,
            });
        }
    }
    let title = |s: &Shelved| match (&s.book, &s.stored) {
        (Some(book), _) => book.title.to_lowercase(),
        (None, Some(file)) => file.name.to_lowercase(),
        (None, None) => String::new(),
    };
    shelf.sort_by_key(|s| (s.status != BookStatus::Missing, title(s)));
    shelf
}

/// When the CDN could not be listed, nothing registered can be called missing: it was not
/// looked for.
pub fn unlisted(builds: &mut [Build], books: &mut [Shelved]) {
    for build in builds
        .iter_mut()
        .filter(|b| b.status == BuildStatus::Missing)
    {
        build.status = BuildStatus::Unchecked;
    }
    for book in books.iter_mut().filter(|b| b.status == BookStatus::Missing) {
        book.status = BookStatus::Unchecked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdn::records::Channel;

    fn at(day: u32) -> DateTime<Utc> {
        format!("2026-09-{day:02}T00:00:00Z").parse().unwrap()
    }

    fn object(key: &str, bytes: u64) -> Object {
        Object {
            key: key.into(),
            bytes,
            modified: at(1),
            etag: None,
        }
    }

    fn release(id: &str, day: u32, yanked: bool) -> Release {
        Release {
            build_id: id.into(),
            cdn_base: "https://cdn.example".into(),
            wasm_bytes: None,
            notes: None,
            yanked,
            published_at: at(day),
        }
    }

    fn book(id: &str, file: &str) -> Book {
        Book {
            id: id.into(),
            title: id.into(),
            file: file.into(),
            sha256: None,
        }
    }

    #[test]
    fn compressed_copies_fold_into_their_original() {
        let files = fold(
            "game/b/",
            &[
                object("game/b/x.wasm", 5000),
                object("game/b/x.wasm.br", 1500),
                object("game/b/x.wasm.gz", 2000),
                object("game/b/orphan.js.br", 10),
            ],
        );
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            ["orphan.js.br", "x.wasm"],
            "an orphaned copy is still a file"
        );
        assert_eq!(files[1].bytes, 5000, "the original's size, not the sum");
        assert_eq!(files[1].encodings, ["br", "gz"]);
    }

    /// Every combination of stored and registered has its own status, and the dangerous one
    /// sorts first.
    #[test]
    fn each_build_is_placed_by_what_is_stored_and_what_is_registered() {
        let releases = Releases {
            releases: vec![
                release("old", 1, false),
                release("pulled", 2, true),
                release("gone", 3, false),
                release("new", 4, false),
            ],
            channels: vec![Channel {
                name: "stable".into(),
                build_id: "new".into(),
                updated_at: at(5),
            }],
        };
        let stored: Vec<String> = ["old", "pulled", "new", "stray"]
            .iter()
            .map(|id| format!("game/{id}/"))
            .collect();
        let builds = builds(&stored, Some(&releases));
        let seen: Vec<(&str, BuildStatus)> =
            builds.iter().map(|b| (b.id.as_str(), b.status)).collect();
        assert_eq!(
            seen,
            [
                ("gone", BuildStatus::Missing),
                ("new", BuildStatus::Released),
                ("pulled", BuildStatus::Yanked),
                ("old", BuildStatus::Released),
                ("stray", BuildStatus::Unreleased),
            ]
        );
        assert_eq!(builds[1].channels, ["stable"]);
        assert!(builds[4].release.is_none());

        // With the site down, what is stored is still listed, and nothing is claimed about it.
        let unchecked = super::builds(&stored, None);
        assert_eq!(unchecked.len(), 4);
        assert!(unchecked.iter().all(|b| b.status == BuildStatus::Unchecked));
    }

    #[test]
    fn each_book_is_placed_by_the_catalog_and_the_shelf() {
        let stored = fold(
            "library/",
            &[object("library/a.epub", 1), object("library/stray.epub", 2)],
        );
        let catalog = [book("alpha", "a.epub"), book("beta", "b.epub")];
        let shelf = books(&stored, Some(&catalog));
        let seen: Vec<BookStatus> = shelf.iter().map(|s| s.status).collect();
        assert_eq!(
            seen,
            [
                BookStatus::Missing,
                BookStatus::Shelved,
                BookStatus::Uncatalogued
            ]
        );
        assert_eq!(shelf[0].book.as_ref().unwrap().id, "beta");

        let mut unlisted_shelf = books(&[], Some(&catalog));
        unlisted(&mut [], &mut unlisted_shelf);
        assert!(
            unlisted_shelf
                .iter()
                .all(|s| s.status == BookStatus::Unchecked),
            "a CDN that did not answer made the catalog read as missing"
        );

        let unchecked = books(&stored, None);
        assert!(unchecked.iter().all(|s| s.status == BookStatus::Unchecked));
        assert_eq!(unchecked.len(), 2);
    }
}
