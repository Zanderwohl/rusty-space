use maud::{Markup, html};

use super::{REPO, shell};

const DESCRIPTION: &str = "What Lightcone is, who is building it, and what state it is in.";

pub async fn page() -> Markup {
    shell(
        "About",
        DESCRIPTION,
        html! {
            section class="stack" {
                h1 { "About" }
                p class="lede" {
                    "Lightcone is a one-person project, built in the open, on top of an orbital "
                    "mechanics library written for something else entirely."
                }
                p {
                    "That something else is "
                    em { "Exotic Matters" }
                    ", a trajectory tool for a tabletop game. Its Keplerian propagation, its "
                    "reference frames and its epoch handling are checked against JPL Horizons, "
                    "and they turned out to be most of what a space game needs. The shared "
                    "libraries are engine-free and belong to neither product; the game-specific "
                    "code sits above them."
                }
            }

            section class="stack" {
                h2 { "State of play" }
                p {
                    "The spacetime layer, the photometry, the sky and a single-process client "
                    "all exist and run. The premise has been tested in the smallest program that "
                    "could disprove it: an observer thirty light-years out sees a swarm change "
                    "thirty years late, and not before."
                }
                p {
                    "What does not exist yet: the server, the browser build, and the game on top "
                    "of them. This site is the first of those three."
                }
            }

            section class="stack" {
                h2 { "Reading" }
                p {
                    "Every design decision is written down before it is built, including the ones "
                    "that were wrong and had to be revised. "
                    a href=(REPO) { "The documents are here" }
                    ", and the buildout plan is the order of work."
                }
            }
        },
    )
}
