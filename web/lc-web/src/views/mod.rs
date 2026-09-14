//! Every page is a document. The shell wraps a body in the chrome and nothing else does.

pub mod blog;
pub mod home;
pub mod page;
pub mod play;

use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::assets;

/// The design documents are the only thing to read until the devlog has more in it.
pub const REPO: &str = "https://github.com/Zanderwohl/rusty-space/tree/master/lightcone";

/// What goes in `<head>`, gathered in one place so no page half-fills it.
pub struct Head<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// Set for posts. Turns the card into an article and carries the date.
    pub published: Option<String>,
}

impl<'a> Head<'a> {
    pub fn new(title: &'a str, description: &'a str) -> Self {
        Head { title, description, published: None }
    }

    pub fn article(mut self, published: String) -> Self {
        self.published = Some(published);
        self
    }
}

/// The prose chrome: masthead, a reading column, footer. Every page but `/play`.
pub fn shell(head: Head<'_>, body: Markup) -> Markup {
    document(
        head,
        html! {
            header { (masthead()) }
            main class="page" { (body) }
            footer { (colophon()) }
        },
    )
}

/// The document itself. `/play` uses this directly, because a full-bleed canvas is not a
/// reading column and pretending otherwise would mean fighting the stylesheet.
pub fn document(head: Head<'_>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (head.title) " — Lightcone" }
                meta name="description" content=(head.description);
                meta property="og:site_name" content="Lightcone";
                meta property="og:title" content=(head.title);
                meta property="og:description" content=(head.description);
                meta property="og:type" content=(if head.published.is_some() { "article" } else { "website" });
                @if let Some(published) = &head.published {
                    meta property="article:published_time" content=(published);
                }
                meta name="twitter:card" content="summary";
                link rel="stylesheet" href=(assets::url(assets::STYLESHEET));
                link rel="alternate" type="application/rss+xml" title="Lightcone devlog" href="/feed.xml";
                link rel="alternate" type="application/feed+json" title="Lightcone devlog" href="/feed.json";
            }
            body { (body) }
        }
    }
}

fn masthead() -> Markup {
    html! {
        a class="wordmark" href="/" { "Lightcone" }
        nav {
            a href="/play" { "Play" }
            a href="/blog" { "Devlog" }
            a href="/about" { "About" }
            a href=(REPO) { "Source" }
        }
    }
}

fn colophon() -> Markup {
    html! {
        p { "A relativistic sandbox in a volume of real stars." }
        p class="fine-print" {
            "Build " code { (assets::BUILD) } " · "
            a href="/feed.xml" { "RSS" } " · "
            a href="/feed.json" { "JSON feed" }
        }
    }
}

/// Rendered markdown. The only place raw HTML enters a page, and it comes from files in the
/// image rather than from anything a request carries.
pub fn prose(html: &str) -> Markup {
    PreEscaped(html.to_owned())
}
