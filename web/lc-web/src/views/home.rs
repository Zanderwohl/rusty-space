use maud::{Markup, html};

use super::{Head, shell};
use crate::assets;

const DESCRIPTION: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do \
                           eiusmod tempor incididunt ut labore et dolore magna aliqua.";

pub async fn page() -> Markup {
    shell(
        Head::site(DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Everything you know is out of date." }
                // Empty, and so no height, until the script draws into it.
                div #micro-galaxy width="100%" {}
                script type="module" src=(assets::url("scripts/micro-galaxy.js")) {}
                p class="lede" {
                    "The galaxy is big, and light only travels so fast. Even with allies and probes "
                    "at the next star, it takes years for relevant data to arrive."
                }
                p {
                    em { "Lightcone Frontier" } " is a game about the limitations of the universe. "
                    "Light travels at C, and you travel slower. If you can't see beyond your "
                    "lightcone, how do you ensure your safety? How do you coordinate with your "
                    "allies? How do you build and manage infrastructure?"
                }
                div class="actions" {
                    a class="cta primary-button" href="/play" { "Play Now" }
                }
            }

            section class="stack" {
                h2 { "One in-game year per real hour" }
                p {
                    "Time ticks away, whether you're online or not. Transmit orders or requests "
                    "across the lightyears, and you may see what happened next time you log on, if "
                    "things went to plan. If they didn't, you might not know for some time."
                }
                table {
                    caption {
                        "Cause and effect in " cite { "Lightcone Frontier" } " travels at the "
                        "speed of light. Communication and travel take real-life hours, days, or "
                        "even longer."
                    }
                    thead {
                        tr {
                            td {}
                            th scope="col" { "Game Time" }
                            th scope="col" { "Real-Life Time" }
                        }
                    }
                    tbody {
                        tr { th scope="row" { "Moon to Earth transmission" } td { "1.3 s" } td { "0.15 ms" } }
                        tr { th scope="row" { "Sun to Earth at C" } td { "8 min 19 s" } td { "57 ms" } }
                        tr { th scope="row" { "Earth to Voyager 1 transmission" } td { "23 h 51 min" } td { "9.8 s" } }
                        tr { th scope="row" { "Sun to Proxima Centauri at C" } td { "4.25 years" } td { "4.25 h" } }
                        tr { th scope="row" { "Across the galaxy at C" } td { "36,000 years" } td { "4.1 years" } }
                    }
                }
                p {
                    "Travel at subluminal speed to the next star to set up shop. Communicate with "
                    "your allies in the stellar neighborhood to coordinate. Relay transmissions, "
                    "system reports, or news of a battle seen on the galactic horizon."
                }
            }

            section class="stack" {
                h2 { "Lorem ipsum dolor sit amet" }
                div class="columns" {
                    div class="stack" {
                        h3 { "Lorem" }
                        ul {
                            li { "Ut enim ad minima veniam, quis nostrum exercitationem ullam." }
                            li { "Corporis suscipit laboriosam, nisi ut aliquid." }
                            li { "Quis autem vel eum iure — reprehenderit, qui in ea." }
                            li { "Voluptate velit esse quam nihil molestiae consequatur." }
                            li { "Vel illum qui dolorem eum fugiat, quo voluptas nulla." }
                        }
                    }
                    div class="stack" {
                        h3 { "Ipsum" }
                        ul {
                            li { "At vero eos et accusamus et iusto odio." }
                            li { "Dignissimos ducimus qui blanditiis. Praesentium voluptatum." }
                            li { "Deleniti atque corrupti. Quos dolores et quas." }
                            li { "Molestias excepturi sint occaecati." }
                        }
                    }
                }
            }

            section class="stack" {
                h2 { "Lorem ipsum dolor" }
                p {
                    "Nam libero tempore, cum soluta nobis est eligendi optio cumque nihil impedit "
                    "quo minus id quod maxime placeat facere possimus, omnis voluptas assumenda "
                    "est. Omnis dolor repellendus — temporibus autem quibusdam et aut officiis "
                    "debitis aut rerum necessitatibus."
                }
                p { "Itaque earum rerum hic tenetur a sapiente delectus, ut aut reiciendis." }
                div class="actions closing-actions" {
                    a class="cta primary-button" href="/play" { "Play Now" }
                    a class="cta" href="/blog" { "Read the devlog" }
                }
            }
        },
    )
}
