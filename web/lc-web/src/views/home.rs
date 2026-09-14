use maud::{Markup, html};

use super::shell;

const DESCRIPTION: &str = "A real-time strategy sandbox across a volume of real stars, where \
                           information propagates at the speed of light and never faster.";

pub async fn page() -> Markup {
    shell("Information travels at c", DESCRIPTION, html! {
        section class="stack" {
            h1 { "Everything you know is out of date." }
            p class="lede" {
                "Lightcone is a real-time strategy sandbox set across a volume of real stars. "
                "You control one ship. It mines, refines, builds, and launches further ships — "
                "and every action it takes is an event with a place and a time, which nobody "
                "else learns about until its light reaches them."
            }
            p {
                "What you see of a distant star is what it emitted years ago. What a rival "
                "sees of your fleet is where it was, not where it is. There is no scanner "
                "that fixes this, because the limit is not a game mechanic. It is the speed "
                "of light, and it is the game."
            }
        }

        section class="stack" {
            h2 { "One in-game year per real hour" }
            p {
                "The server runs at 8766×. That single number decides the genre, because it "
                "sets what a light-year costs you in wall-clock time."
            }
            table {
                caption { "Travel and signal times at the default rate" }
                thead { tr { th scope="col" { "In-game" } th scope="col" { "Real time" } } }
                tbody {
                    tr { td { "Sun to Earth (499 s)" } td { "57 ms" } }
                    tr { td { "Sun to Neptune (4.2 h)" } td { "1.7 s" } }
                    tr { td { "Sun to 1000 AU" } td { "5.7 min" } }
                    tr { td { "Proxima Centauri, one way" } td { "4.25 h" } }
                    tr { td { "Proxima Centauri, round trip" } td { "8.5 h" } }
                }
            }
            p {
                "So in-system play is immediate and interstellar play is asynchronous, "
                "measured in hours. An order sent to a probe at Proxima is confirmed "
                "tomorrow. That is not a limitation being engineered around."
            }
        }

        section class="stack" {
            h2 { "What that costs, and what it buys" }
            div class="columns" {
                div class="stack" {
                    h3 { "In" }
                    ul {
                        li { "Special relativity: light delay, Doppler, aberration, proper time." }
                        li { "Real Keplerian orbits, on real catalogue stars." }
                        li { "Passive observation — photometry, transits, spectroscopy." }
                        li { "Radio and tight-beam, and the difference in who hears you." }
                        li { "Extraction, construction, self-replicating probes." }
                    }
                }
                div class="stack" {
                    h3 { "Out" }
                    ul {
                        li { "FTL of any kind, including sensors." }
                        li { "General relativity. No lensing, no curvature." }
                        li { "Avatar-scale play. The smallest unit is a ship." }
                        li { "Combat as the primary loop." }
                    }
                }
            }
        }

        section class="stack" {
            h2 { "Not yet playable" }
            p {
                "The physics, the photometry and the client exist and run. The server, the "
                "browser build and the game on top of them do not. This site will carry the "
                "devlog as that changes, and the browser build when there is one to run. "
                "Until then the design documents are public and are the whole of it."
            }
            p { a class="cta" href=(super::REPO) { "Read the design documents" } }
        }
    })
}
