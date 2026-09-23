use maud::{Markup, html};

use super::{Head, shell};

const DESCRIPTION: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do \
                           eiusmod tempor incididunt ut labore et dolore magna aliqua.";

pub async fn page() -> Markup {
    shell(
        Head::new("Lorem ipsum dolor", DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Everything you know is out of date." }
                p class="lede" {
                    "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor. "
                    "Ut enim ad minim veniam. Quis nostrud exercitation ullamco laboris nisi ut aliquip, "
                    "ex ea commodo consequat — duis aute irure dolor in reprehenderit in voluptate "
                    "velit esse cillum dolore eu fugiat nulla pariatur."
                }
                p {
                    "Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia "
                    "deserunt mollit anim id est laborum. Sed ut perspiciatis unde omnis iste "
                    "natus error sit voluptatem accusantium doloremque laudantium. Totam rem "
                    "aperiam, eaque ipsa quae ab illo."
                }
            }

            section class="stack" {
                h2 { "One in-game year per real hour" }
                p {
                    "Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit, "
                    "sed quia consequuntur magni dolores eos qui ratione voluptatem."
                }
                table {
                    caption { "Travel and signal times at the default rate" }
                    thead {
                        tr {
                            td {}
                            th scope="col" { "Game Time" }
                            th scope="col" { "Real-Life Time" }
                        }
                    }
                    tbody {
                        tr { th scope="row" { "Moon to Earth transmission" } td { "1.3 s" } td { "0.15 ms" } }
                        tr { th scope="row" { "Sun to Earth light" } td { "8 min 19 s" } td { "57 ms" } }
                        tr { th scope="row" { "Earth to Voyager 1 transmission" } td { "23 h 51 min" } td { "9.8 s" } }
                        tr { th scope="row" { "Sun to Proxima Centauri light" } td { "4.25 years" } td { "4.25 h" } }
                        tr { th scope="row" { "Across the game galaxy at C" } td { "36,000 years" } td { "4.1 years" } }
                    }
                }
                p {
                    "Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, "
                    "consectetur, adipisci velit. Sed quia non numquam eius modi tempora "
                    "incidunt. Ut labore et dolore magnam aliquam quaerat voluptatem."
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
                p {
                    "Itaque earum "
                    a href=(super::REPO) { "rerum hic tenetur" }
                    " a sapiente delectus, ut aut reiciendis."
                }
                p {
                    a class="cta" href="/play" { "Play in your browser" }
                    " "
                    a class="cta" href="/blog" { "Read the devlog" }
                }
            }
        },
    )
}
