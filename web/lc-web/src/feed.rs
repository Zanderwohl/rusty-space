//! Syndication, and the two files crawlers look for.
//!
//! The feed carries **full content**, not summaries. A reader that has to click through to
//! read anything is a table of contents, not a feed.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use time::{OffsetDateTime, Time, UtcOffset};

use crate::AppState;
use crate::content::Post;

const TITLE: &str = "Lightcone Frontier devlog";
const DESCRIPTION: &str = "Notes on building a relativistic sandbox MMO.";

/// RSS 2.0. Built with the `rss` crate rather than by hand: XML escaping is a bug farm and
/// this is the one place where getting it wrong is silent.
pub async fn rss(State(state): State<AppState>) -> Response {
    let base = &state.base_url;
    let content = state.content();
    let items: Vec<rss::Item> = content
        .posts()
        .iter()
        .filter(|p| !p.draft)
        .map(|post| {
            let link = format!("{base}/blog/{}", post.slug);
            rss::ItemBuilder::default()
                .title(post.title.clone())
                .link(link.clone())
                // The guid is the permalink and never changes, so a reader that has seen a
                // post does not show it again when the feed is rebuilt.
                .guid(rss::GuidBuilder::default().value(link).permalink(true).build())
                .pub_date(rfc2822(post))
                .description(post.summary.clone())
                .content(post.html.clone())
                .categories(
                    post.tags
                        .iter()
                        .map(|t| rss::CategoryBuilder::default().name(t.clone()).build())
                        .collect::<Vec<_>>(),
                )
                .build()
        })
        .collect();

    let channel = rss::ChannelBuilder::default()
        .title(TITLE)
        .link(format!("{base}/blog"))
        .description(DESCRIPTION)
        .language("en".to_string())
        .items(items)
        .build();

    ([(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")], channel.to_string())
        .into_response()
}

/// JSON Feed 1.1. Hand-built, which is safe here in a way hand-built XML is not: serde_json
/// escapes every string it writes.
pub async fn json(State(state): State<AppState>) -> Response {
    let base = &state.base_url;
    let content = state.content();
    let items: Vec<_> = content
        .posts()
        .iter()
        .filter(|p| !p.draft)
        .map(|post| {
            serde_json::json!({
                "id": format!("{base}/blog/{}", post.slug),
                "url": format!("{base}/blog/{}", post.slug),
                "title": post.title,
                "summary": post.summary,
                "content_html": post.html,
                "date_published": rfc3339(post),
                "tags": post.tags,
            })
        })
        .collect();

    let feed = serde_json::json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": TITLE,
        "description": DESCRIPTION,
        "home_page_url": format!("{base}/blog"),
        "feed_url": format!("{base}/feed.json"),
        "language": "en",
        "items": items,
    });

    ([(header::CONTENT_TYPE, "application/feed+json; charset=utf-8")], feed.to_string())
        .into_response()
}

pub async fn sitemap(State(state): State<AppState>) -> Response {
    let base = &state.base_url;
    let content = state.content();
    let mut urls = vec![format!("{base}/"), format!("{base}/about"), format!("{base}/blog")];
    urls.extend(
        content.posts().iter().filter(|p| !p.draft).map(|p| format!("{base}/blog/{}", p.slug)),
    );
    urls.extend(content.tags().map(|(t, _)| format!("{base}/blog/tag/{t}")));

    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{}</urlset>\n",
        urls.iter().map(|u| format!("  <url><loc>{}</loc></url>\n", escape(u))).collect::<String>()
    );
    ([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], body).into_response()
}

pub async fn robots(State(state): State<AppState>) -> Response {
    let body = format!("User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n", state.base_url);
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// Midnight UTC on the post's date. The date is a day, not an instant; picking the start of
/// it is a convention, and a consistent one matters more than which end it picks.
fn midnight(post: &Post) -> OffsetDateTime {
    post.date.with_time(Time::MIDNIGHT).assume_offset(UtcOffset::UTC)
}

fn rfc2822(post: &Post) -> String {
    midnight(post)
        .format(&time::format_description::well_known::Rfc2822)
        .expect("a valid date formats")
}

fn rfc3339(post: &Post) -> String {
    midnight(post)
        .format(&time::format_description::well_known::Rfc3339)
        .expect("a valid date formats")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
