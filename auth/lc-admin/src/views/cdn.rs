//! What the CDN holds: every build, book and icon, and whether each is what the records say.
//!
//! Read-only. Promoting and yanking stay with `tools/release.sh` and the site's `/internal`
//! routes; this page is where to look before doing either.

use maud::{Markup, html};

use crate::cdn::inventory::{BookStatus, Build, BuildStatus, File, Shelved};
use crate::cdn::storage::Unavailable;
use crate::cdn::{BuildPage, Index};

pub fn page(index: &Index) -> Markup {
    html! {
        section class="stack" {
            h1 { "CDN" }
            (unavailable(&index.storage, "the CDN's listing"))
            (unavailable(&index.releases, "the site's releases"))
            (builds(&index.builds, index.storage.is_ok()))
            (unavailable(&index.catalog, "the shelf's catalog"))
            (books(&index.books, index.storage.is_ok()))
            (icons(&index.icons, index.storage.is_ok()))
        }
    }
}

fn unavailable(source: &Result<(), Unavailable>, what: &str) -> Markup {
    html! {
        @if let Err(why) = source {
            p class="nothing" { (why.said(what)) " Rows it would decide are unchecked." }
        }
    }
}

/// `listed` is whether the CDN answered: an empty section beside one that did not is not a
/// claim that nothing is stored.
fn empty(listed: bool, said: &str) -> Markup {
    html! {
        p class="nothing" {
            @if listed { (said) } @else { "Nothing to show without the CDN's listing." }
        }
    }
}

fn builds(builds: &[Build], listed: bool) -> Markup {
    html! {
        section class="panel" {
            h2 { "Builds" }
            @if builds.is_empty() {
                (empty(listed, "No build is stored or registered."))
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Build" }
                            th scope="col" { "Status" }
                            th scope="col" { "Registered" }
                            th scope="col" { "Wasm" }
                        }
                    }
                    tbody {
                        @for build in builds {
                            tr {
                                th scope="row" {
                                    a href=(crate::routes::cdn_build_url(&build.id)) {
                                        code { (build.id) }
                                    }
                                }
                                td { (build_badges(build)) }
                                td {
                                    @match &build.release {
                                        Some(release) => (super::when(release.published_at)),
                                        None => span class="nothing" { "—" },
                                    }
                                }
                                td {
                                    @match build.release.as_ref().and_then(|r| r.wasm_bytes) {
                                        Some(n) => (bytes(n.max(0) as u64)),
                                        None => span class="nothing" { "—" },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn build_badges(build: &Build) -> Markup {
    let (slug, label) = match build.status {
        BuildStatus::Released => ("released", "Released"),
        BuildStatus::Yanked => ("yanked", "Yanked"),
        BuildStatus::Unreleased => ("unreleased", "Never released"),
        BuildStatus::Missing => ("missing", "Missing from the CDN"),
        BuildStatus::Unchecked => ("unchecked", "Unchecked"),
    };
    html! {
        span class={ "badge badge-" (slug) } { (label) }
        @for channel in &build.channels {
            span class="badge badge-channel" { (channel) }
        }
    }
}

fn books(shelf: &[Shelved], listed: bool) -> Markup {
    html! {
        section class="panel" {
            h2 { "Books" }
            @if shelf.is_empty() {
                (empty(listed, "No book is stored or catalogued."))
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Book" }
                            th scope="col" { "Status" }
                            th scope="col" { "File" }
                            th scope="col" { "Size" }
                        }
                    }
                    tbody {
                        @for shelved in shelf {
                            tr {
                                th scope="row" {
                                    @match &shelved.book {
                                        Some(book) => {
                                            (book.title)
                                            span class="id" { code { (book.id) } }
                                        },
                                        None => span class="nothing" { "Not in the catalog" },
                                    }
                                }
                                td { (book_badge(shelved.status)) }
                                td {
                                    @match (&shelved.stored, &shelved.book) {
                                        (Some(file), _) => (file.name),
                                        (None, Some(book)) => (book.file),
                                        (None, None) => "",
                                    }
                                }
                                td {
                                    @match &shelved.stored {
                                        Some(file) => (bytes(file.bytes)),
                                        None => span class="nothing" { "—" },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn book_badge(status: BookStatus) -> Markup {
    let (slug, label) = match status {
        BookStatus::Shelved => ("released", "Shelved"),
        BookStatus::Uncatalogued => ("unreleased", "Uncatalogued"),
        BookStatus::Missing => ("missing", "Missing from the CDN"),
        BookStatus::Unchecked => ("unchecked", "Unchecked"),
    };
    html! { span class={ "badge badge-" (slug) } { (label) } }
}

fn icons(icons: &[File], listed: bool) -> Markup {
    html! {
        section class="panel" {
            h2 { "Icons" }
            @if icons.is_empty() {
                (empty(listed, "No icon is stored."))
            } @else {
                (files(icons))
            }
        }
    }
}

/// One build: its record, its manifest and every file it is made of.
pub fn build_page(page: &BuildPage) -> Markup {
    let build = &page.build;
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::routes::CDN) { "← CDN" } }
            header class="page-head" {
                h1 { (build.id) }
                p { (build_badges(build)) }
                (unavailable(&page.storage, "the CDN's listing"))
                (unavailable(&page.releases, "the site's releases"))
                @if let Some(release) = &build.release {
                    dl class="facts" {
                        dt { "Registered" }
                        dd { (super::when(release.published_at)) }
                        dt { "CDN base" }
                        dd { code { (release.cdn_base) } }
                        @if let Some(wasm) = release.wasm_bytes {
                            dt { "Wasm" }
                            dd { (bytes(wasm.max(0) as u64)) }
                        }
                        @if let Some(notes) = &release.notes {
                            dt { "Notes" }
                            dd { (notes) }
                        }
                    }
                }
            }
            section class="panel" {
                h2 { "Manifest" }
                @match &page.manifest {
                    None => p class="nothing" { "No manifest.json that parses." },
                    Some(manifest) => dl class="facts" {
                        @for (key, value) in manifest {
                            dt { (key) }
                            dd {
                                @match value {
                                    serde_json::Value::String(s) => (s),
                                    other => code { (other.to_string()) },
                                }
                            }
                        }
                    },
                }
            }
            section class="panel" {
                h2 { "Files" }
                @if page.files.is_empty() {
                    p class="nothing" { "Nothing is stored under this build." }
                } @else {
                    (files(&page.files))
                }
            }
        }
    }
}

fn files(files: &[File]) -> Markup {
    html! {
        table class="records" {
            thead {
                tr {
                    th scope="col" { "File" }
                    th scope="col" { "Size" }
                    th scope="col" { "Also as" }
                    th scope="col" { "Modified" }
                }
            }
            tbody {
                @for file in files {
                    tr {
                        th scope="row" { code { (file.name) } }
                        td { (bytes(file.bytes)) }
                        td {
                            @if file.encodings.is_empty() {
                                span class="nothing" { "—" }
                            } @else {
                                (file.encodings.join(", "))
                            }
                        }
                        // Not "published": see `crate::cdn::storage::Object::modified`.
                        td { (super::when(file.modified)) }
                    }
                }
            }
        }
    }
}

/// Decimal, as the sizes a browser's network panel and a bucket's console show.
fn bytes(n: u64) -> String {
    const STEPS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "kB")];
    for (size, unit) in STEPS {
        if n >= size {
            return format!("{:.1} {unit}", n as f64 / size as f64);
        }
    }
    format!("{n} B")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdn::records::Release;
    use chrono::{DateTime, Utc};

    fn at() -> DateTime<Utc> {
        "2026-09-19T09:30:00Z".parse().unwrap()
    }

    #[test]
    fn sizes_read_in_the_unit_that_fits() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_500), "1.5 kB");
        assert_eq!(bytes(24_118_272), "24.1 MB");
        assert_eq!(bytes(3_000_000_000), "3.0 GB");
    }

    #[test]
    fn a_missing_build_says_so_and_links_to_its_page() {
        let index = Index {
            storage: Ok(()),
            releases: Ok(()),
            catalog: Err(Unavailable::NotConfigured),
            builds: vec![Build {
                id: "2026.09.1+a10eac4".into(),
                status: BuildStatus::Missing,
                channels: vec!["stable".into()],
                release: Some(Release {
                    build_id: "2026.09.1+a10eac4".into(),
                    cdn_base: "https://cdn.example".into(),
                    wasm_bytes: Some(24_118_272),
                    notes: None,
                    yanked: false,
                    published_at: at(),
                }),
            }],
            books: Vec::new(),
            icons: Vec::new(),
        };
        let markup = page(&index).into_string();
        assert!(markup.contains("Missing from the CDN"), "{markup}");
        assert!(
            markup.contains(r#"class="badge badge-channel""#),
            "{markup}"
        );
        assert!(
            markup.contains(r#"href="/cdn/game/2026.09.1+a10eac4""#),
            "{markup}"
        );
        assert!(markup.contains("24.1 MB"), "{markup}");
        assert!(
            markup.contains("not pointed at the shelf's catalog"),
            "an unconfigured catalog is silent: {markup}"
        );
        assert!(markup.contains("No icon is stored."), "{markup}");
    }

    #[test]
    fn an_unlisted_cdn_claims_nothing_is_stored() {
        let index = Index {
            storage: Err(Unavailable::Refused),
            releases: Err(Unavailable::NotConfigured),
            catalog: Err(Unavailable::NotConfigured),
            builds: Vec::new(),
            books: Vec::new(),
            icons: Vec::new(),
        };
        let markup = page(&index).into_string();
        assert!(markup.contains("refused this console"), "{markup}");
        assert!(!markup.contains("No icon is stored."), "{markup}");
        assert!(!markup.contains("No build is stored"), "{markup}");
    }
}
