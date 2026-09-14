//! Standing pages from `content/pages/`. Same pipeline as a post, without the date or tags.
//!
//! One route per page, added when the page is. A catch-all `/{slug}` would have to be the
//! router's last resort and would quietly shadow every 404.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use maud::html;

use super::{Head, prose, shell};
use crate::AppState;

pub async fn about(State(state): State<AppState>) -> Response {
    let content = state.content();
    let Some(page) = content.page("about") else {
        return crate::not_found().await.into_response();
    };
    shell(
        Head::new(&page.title, &page.summary),
        html! {
            article class="stack" {
                h1 { (page.title) }
                (prose(&page.html))
            }
        },
    )
    .into_response()
}
