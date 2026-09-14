//! Every page is a document. The shell wraps a body in the chrome and nothing else does.

pub mod about;
pub mod home;

use maud::{DOCTYPE, Markup, html};

use crate::assets;

/// The design documents are the only thing to read until there is a devlog.
pub const REPO: &str = "https://github.com/Zanderwohl/rusty-space/tree/master/lightcone";

/// `title` is the page's own; the site name is appended here so no caller repeats it.
pub fn shell(title: &str, description: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " — Lightcone" }
                meta name="description" content=(description);
                meta property="og:title" content=(title);
                meta property="og:description" content=(description);
                meta property="og:type" content="website";
                link rel="stylesheet" href=(assets::url(assets::STYLESHEET));
            }
            body {
                header {
                    a class="wordmark" href="/" { "Lightcone" }
                    nav {
                        a href="/about" { "About" }
                        a href=(REPO) { "Source" }
                    }
                }
                main class="page" { (body) }
                footer {
                    p {
                        "A relativistic sandbox in a volume of real stars. "
                        "Nothing here is playable yet."
                    }
                    p class="fine-print" { "Build " code { (assets::BUILD) } }
                }
            }
        }
    }
}
