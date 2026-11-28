//! The Galaxy section's pages: the loaded galaxy's chunks and a map of what craft know, the
//! manifests on the CDN, one manifest's entries, and one chunk's stars.
//!
//! A game master reads the levels before changing a region: only an unseen tier-B chunk may be
//! baked again, and contents may be added only where no craft has resolved or visited a system.

use std::collections::BTreeMap;

use maud::{Markup, html};

use crate::cdn::storage::Unavailable;
use crate::galaxy::{Manifest, Stored, chunk_url, manifest_url};
use crate::shard::{ChunkRow, ChunkStars, Galaxy, Level, Missing};

const LEVELS: [Level; 4] = [Level::Unseen, Level::Charted, Level::Resolved, Level::Visited];

fn level_badge(level: Level) -> Markup {
    html! { span class={ "badge badge-" (level.slug()) } { (level.slug()) } }
}

fn chunk_label(c: (i32, i32, i32)) -> String {
    format!("{}, {}, {}", c.0, c.1, c.2)
}

pub fn page(galaxy: &Result<Option<Galaxy>, Missing>, manifests: &Result<Vec<Stored>, Unavailable>) -> Markup {
    html! {
        section class="stack" {
            h1 { "Galaxy" }
            @match galaxy {
                Err(missing) => p class="nothing" { (missing.said()) },
                Ok(None) => p class="nothing" { "This shard's sky is a catalog, not an authored galaxy." },
                Ok(Some(g)) => (loaded(g)),
            }
            section class="panel" {
                h2 { "Manifests on the CDN" }
                @match manifests {
                    Err(why) => p class="nothing" { (why.said("the CDN's listing")) },
                    Ok(list) if list.is_empty() => p class="nothing" { "The CDN holds no galaxy." },
                    Ok(list) => (stored(list)),
                }
            }
        }
    }
}

fn loaded(g: &Galaxy) -> Markup {
    let count = |l: Level| g.chunks.iter().filter(|c| c.level == l).count();
    html! {
        section class="panel" {
            h2 { "Loaded: " (g.name) }
            dl class="facts" {
                dt { "Read from" }
                dd { code { (g.source) } }
                dt { "Sent to clients" }
                dd {
                    @match &g.url {
                        Some(url) => code { (url) },
                        None => span class="nothing" { "nothing: clients load the sky in their build" },
                    }
                }
                dt { "Built on" }
                dd {
                    @match &g.parent {
                        Some(p) => code { (p) },
                        None => span class="nothing" { "—" },
                    }
                }
                dt { "Chunk files" }
                dd {
                    @for l in LEVELS {
                        (level_badge(l)) " " (count(l)) " "
                    }
                }
            }
            (map(&g.chunks))
            (chunks(&g.chunks))
        }
    }
}

/// Face-on, +y up: one square per chunk column, in the level of the most known chunk in it.
fn map(rows: &[ChunkRow]) -> Markup {
    const CELL: i32 = 36;
    let mut columns: BTreeMap<(i32, i32), (Level, u32)> = BTreeMap::new();
    for r in rows {
        let c = columns.entry((r.chunk.0, r.chunk.1)).or_insert((Level::Unseen, 0));
        c.0 = c.0.max(r.level);
        c.1 += r.stars;
    }
    let Some(((x0, _), _)) = columns.first_key_value() else { return html! {} };
    let x1 = columns.keys().map(|k| k.0).max().unwrap_or(*x0);
    let y0 = columns.keys().map(|k| k.1).min().unwrap_or(0);
    let y1 = columns.keys().map(|k| k.1).max().unwrap_or(0);
    let (w, h) = ((x1 - x0 + 1) * CELL, (y1 - y0 + 1) * CELL);
    html! {
        figure class="discovery-map" {
            svg viewBox={ "0 0 " (w) " " (h) } width=(w) height=(h) role="img" aria-label="Chunks seen from +z, colored by what craft know" {
                @for ((x, y), (level, stars)) in &columns {
                    rect class=(level.slug()) x=((x - x0) * CELL + 1) y=((y1 - y) * CELL + 1) width=(CELL - 2) height=(CELL - 2) {
                        title { (format!("chunk {x}, {y}: {stars} stars, {}", level.slug())) }
                    }
                }
            }
            figcaption class="fine-print" { "Seen from +z, one square per 256 ly column of chunks." }
        }
    }
}

fn chunks(rows: &[ChunkRow]) -> Markup {
    html! {
        table class="records" {
            thead {
                tr {
                    th scope="col" { "Chunk" }
                    th scope="col" { "Tier" }
                    th scope="col" { "Level" }
                    th scope="col" { "Stars" }
                    th scope="col" { "Charted" }
                    th scope="col" { "Resolved" }
                    th scope="col" { "Visited" }
                    th scope="col" { "Craft" }
                    th scope="col" { "Graph" }
                }
            }
            tbody {
                @for r in rows {
                    tr {
                        th scope="row" { a href=(chunk_url(r.chunk, &r.tier)) { (chunk_label(r.chunk)) } }
                        td { (r.tier) }
                        td { (level_badge(r.level)) }
                        td { (r.stars) }
                        td { (r.charted) }
                        td { (r.resolved) }
                        td { (r.visited) }
                        td { (r.craft) }
                        td { code { (r.graph) } }
                    }
                }
            }
        }
    }
}

fn stored(list: &[Stored]) -> Markup {
    html! {
        table class="records" {
            thead {
                tr {
                    th scope="col" { "Galaxy" }
                    th scope="col" { "Manifest" }
                    th scope="col" { "Copied" }
                    th scope="col" { "Size" }
                }
            }
            tbody {
                @for s in list {
                    tr {
                        th scope="row" { (s.name) }
                        td {
                            a href=(manifest_url(&s.name, &s.file)) { code { (s.file) } }
                            @if s.loaded { " " span class="badge badge-released" { "Loaded" } }
                        }
                        td { (super::when(s.modified)) }
                        td { (s.bytes) " B" }
                    }
                }
            }
        }
    }
}

/// `loaded` is the shard's galaxy when this is the manifest it read, so the rows can say what
/// craft know.
pub fn manifest_page(name: &str, file: &str, read: &Result<Manifest, String>, loaded: Option<&Galaxy>) -> Markup {
    let level = |e: &crate::galaxy::Entry| {
        loaded.and_then(|g| g.chunks.iter().find(|c| c.chunk == e.chunk && c.tier == e.tier.label())).map(|c| c.level)
    };
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::galaxy::GALAXY) { "← Galaxy" } }
            header class="page-head" {
                h1 { (name) " " code { (file) } }
            }
            @match read {
                Err(why) => p class="nothing" { (why) },
                Ok(m) => {
                    dl class="facts" {
                        dt { "Format" }
                        dd { (m.format) }
                        dt { "Built on" }
                        dd {
                            @match &m.parent {
                                Some(p) => a href=(manifest_url(name, &format!("{p}.galaxy"))) { code { (p) } },
                                None => span class="nothing" { "—" },
                            }
                        }
                        dt { "Chunk files" }
                        dd { (m.chunks.len()) ", " (m.chunks.iter().map(|c| u64::from(c.stars)).sum::<u64>()) " stars" }
                    }
                    table class="records" {
                        thead {
                            tr {
                                th scope="col" { "Chunk" }
                                th scope="col" { "Tier" }
                                th scope="col" { "Stars" }
                                th scope="col" { "Graph" }
                                th scope="col" { "Hash" }
                                @if loaded.is_some() { th scope="col" { "Level" } }
                            }
                        }
                        tbody {
                            @for e in &m.chunks {
                                tr {
                                    th scope="row" {
                                        @if loaded.is_some() {
                                            a href=(chunk_url(e.chunk, e.tier.label())) { (chunk_label(e.chunk)) }
                                        } @else {
                                            (chunk_label(e.chunk))
                                        }
                                    }
                                    td { (e.tier.label()) }
                                    td { (e.stars) }
                                    td { code { (e.graph) } }
                                    td { code { (e.hash) } }
                                    @if loaded.is_some() {
                                        td { @if let Some(l) = level(e) { (level_badge(l)) } }
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

fn number(v: Option<f64>, digits: usize) -> Markup {
    html! {
        @match v {
            Some(v) if v.is_finite() => (format!("{v:.digits$}")),
            _ => span class="nothing" { "—" },
        }
    }
}

pub fn chunk_page(key: &str, found: &Result<ChunkStars, Missing>) -> Markup {
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::galaxy::GALAXY) { "← Galaxy" } }
            @match found {
                Err(missing) => p class="nothing" { (missing.said()) },
                Ok(c) => {
                    header class="page-head" {
                        h1 { "Chunk " (chunk_label(c.chunk)) ", tier " (c.tier) }
                        p class="fine-print" { (c.stars.len()) " stars, brightest first. " code { (key) } }
                    }
                    table class="records" {
                        thead {
                            tr {
                                th scope="col" { "System" }
                                th scope="col" { "Level" }
                                th scope="col" { "Position, ly" }
                                th scope="col" { "Population" }
                                th scope="col" { "Mass, M☉" }
                                th scope="col" { "Initial" }
                                th scope="col" { "Age, Gyr" }
                                th scope="col" { "[Fe/H]" }
                                th scope="col" { "T, K" }
                                th scope="col" { "L, L☉" }
                                th scope="col" { "Shell, ly" }
                            }
                        }
                        tbody {
                            @for s in &c.stars {
                                tr {
                                    th scope="row" {
                                        @if s.luminosity_solar.is_some() {
                                            a href=(crate::routes::system_url(s.id)) { code { (s.id) } }
                                        } @else {
                                            code { (s.id) }
                                        }
                                    }
                                    td { (level_badge(s.level)) }
                                    td { (format!("{:.2}, {:.2}, {:.2}", s.position_ly.0, s.position_ly.1, s.position_ly.2)) }
                                    td { (s.population) }
                                    td { (number(s.mass_solar, 3)) }
                                    td { (format!("{:.3}", s.initial_mass_solar)) }
                                    td { (format!("{:.2}", s.age_gyr)) }
                                    td { (format!("{:+.2}", s.feh)) }
                                    td { (number(s.teff_k, 0)) }
                                    td { (number(s.luminosity_solar, 4)) }
                                    td { (number(Some(s.shell_ly), 2)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(chunk: (i32, i32, i32), tier: &str, level: Level) -> ChunkRow {
        ChunkRow { chunk, tier: tier.into(), graph: "0".into(), hash: "h".into(), stars: 10, charted: 0, resolved: 0, visited: 0, craft: 0, level }
    }

    /// The map draws one square per column of chunks, in the level of the most known chunk in
    /// it, and the table links each chunk to its stars.
    #[test]
    fn the_map_takes_each_column_s_most_known_chunk() {
        let g = Galaxy {
            source: "http://lightcone-cdn:3101/galaxy/p3/a.galaxy".into(),
            url: None,
            name: "p3".into(),
            parent: None,
            chunks: vec![row((0, 0, 0), "B", Level::Unseen), row((0, 0, -1), "B", Level::Visited), row((1, 0, 0), "A", Level::Charted)],
        };
        let html = page(&Ok(Some(g)), &Ok(Vec::new())).into_string();
        assert_eq!(html.matches("<rect").count(), 2, "{html}");
        assert!(html.contains(r#"<rect class="visited""#) && html.contains(r#"<rect class="charted""#), "{html}");
        assert!(html.contains(r#"href="/galaxy/chunks/0,0,-1,B""#), "{html}");
    }
}
