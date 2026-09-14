//! The devlog: an index, a post, and a tag.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use maud::{Markup, html};

use super::{Head, prose, shell};
use crate::AppState;
use crate::content::{Content, Post};

const DESCRIPTION: &str = "Notes on building a relativistic sandbox: what was decided, what \
                           was measured, and what turned out to be wrong.";

pub async fn index(State(state): State<AppState>) -> Markup {
    let content = state.content();
    let posts = content.posts();
    shell(
        Head::new("Devlog", DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Devlog" }
                p class="lede" {
                    "Design decisions get written down before they are built, including the ones "
                    "that were wrong and had to be revised."
                }
            }

            @if posts.is_empty() {
                section class="stack" {
                    p { "Nothing written yet." }
                }
            } @else {
                ol class="post-list" {
                    @for post in posts { li { (summary(post)) } }
                }
                (tag_index(&content))
            }
        },
    )
}

pub async fn post(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let content = state.content();
    let Some(post) = content.post(&slug) else {
        return crate::not_found().await.into_response();
    };
    shell(
        Head::new(&post.title, &post.summary).article(post.date_iso()),
        html! {
            article class="stack" {
                header class="stack" {
                    h1 { (post.title) }
                    (meta(post))
                }
                (prose(&post.html))
            }
            nav class="stack" {
                p { a class="cta" href="/blog" { "All posts" } }
            }
        },
    )
    .into_response()
}

pub async fn tag(State(state): State<AppState>, Path(tag): Path<String>) -> Response {
    let content = state.content();
    let Some(posts) = content.tagged(&tag) else {
        return crate::not_found().await.into_response();
    };
    let heading = format!("Tagged “{tag}”");
    let description = format!("Devlog posts tagged {tag}.");
    shell(
        Head::new(&heading, &description),
        html! {
            section class="stack" {
                h1 { (heading) }
                p class="lede" {
                    (posts.len())
                    @if posts.len() == 1 { " post." } @else { " posts." }
                }
            }
            ol class="post-list" {
                @for post in &posts { li { (summary(post)) } }
            }
            (tag_index(&content))
        },
    )
    .into_response()
}

/// One entry in a list of posts.
fn summary(post: &Post) -> Markup {
    html! {
        article class="stack" {
            h2 { a href={ "/blog/" (post.slug) } { (post.title) } }
            (meta(post))
            p { (post.summary) }
        }
    }
}

/// Date, reading time, tags, and the draft marker. Same line under a title and in a list, so
/// a post looks like itself in both places.
/// A `div`, not a `p`. The tag list is a `ul`, and a `ul` inside a `p` is invalid HTML: the
/// parser closes the paragraph before it, so the tags land on their own line no matter what
/// the stylesheet says.
fn meta(post: &Post) -> Markup {
    html! {
        div class="post-meta" {
            time datetime=(post.date_iso()) { (post.date_human()) }
            " · " (post.minutes) " min"
            @if post.draft { " · " span class="draft-marker" { "draft" } }
            @if !post.tags.is_empty() {
                " · "
                ul class="tag-list" {
                    @for tag in &post.tags {
                        li { a href={ "/blog/tag/" (tag) } { (tag) } }
                    }
                }
            }
        }
        @if let Some(updated) = post.updated {
            div class="post-meta" {
                "Updated "
                time datetime=(updated.to_string()) { (updated) }
            }
        }
    }
}

fn tag_index(content: &Content) -> Markup {
    html! {
        section class="stack" {
            h2 { "Tags" }
            ul class="tag-list" {
                @for (tag, count) in content.tags() {
                    li { a href={ "/blog/tag/" (tag) } { (tag) } " " span class="fine-print" { (count) } }
                }
            }
        }
    }
}
