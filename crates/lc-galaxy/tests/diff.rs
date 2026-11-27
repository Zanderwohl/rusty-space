//! `diff` against a manifest: what an edit changed of a published galaxy.

use lc_galaxy::diff::{Status, Tolerance, diff};
use lc_galaxy::{Registry, Timeline, bake, codec, publish};
use lc_world::sky::galaxy::{GalaxyChunk, Manifest, ManifestChunk};

fn phase_two() -> Timeline {
    codec::read(include_str!("../galaxies/phase-2.ron"), &Registry::standard()).unwrap()
}

/// What a publish of `t` would list, without writing it.
fn published(t: &Timeline) -> Vec<(ManifestChunk, GalaxyChunk)> {
    let c = t.compile(t.len(), None);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let graph = publish::graph_version(t);
    publish::chunks(&c, &b, graph)
        .unwrap()
        .into_iter()
        .map(|(tier, ch)| {
            let [x, y, z] = ch.chunk;
            let entry = ManifestChunk { chunk: (x, y, z), tier, graph: format!("{graph:016x}"), file: String::new(), hash: String::new(), stars: ch.records.len() as u32 };
            (entry, ch)
        })
        .collect()
}

fn against(t: &Timeline, was: &[(ManifestChunk, GalaxyChunk)]) -> Vec<lc_galaxy::diff::ChunkDiff> {
    let c = t.compile(t.len(), None);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    diff(&c, &b, publish::graph_version(t), was, Tolerance::default()).unwrap()
}

/// The graph that was published is the same in every chunk, and nothing is left unpublished.
#[test]
fn an_unchanged_graph_matches_its_publish() {
    let t = phase_two();
    let was = published(&t);
    let d = against(&t, &was);
    assert_eq!(d.len(), was.len());
    assert!(d.iter().all(|c| c.status == Status::Same && c.kept == c.published), "{d:?}");
}

/// One star missing from a published chunk is close, not the same; a new seed differs.
#[test]
fn a_lost_star_is_close_and_a_new_seed_differs() {
    let mut t = phase_two();
    let mut was = published(&t);
    let i = was.iter().position(|(_, c)| c.records.len() > 10).expect("a chunk with stars");
    was[i].1.records.pop();
    let d = against(&t, &was);
    assert_eq!(d[i].status, Status::Close, "{:?}", d[i]);
    assert_eq!((d[i].published + 1, d[i].kept), (d[i].now, d[i].published));
    assert!(d.iter().enumerate().all(|(j, c)| j == i || c.status == Status::Same));

    t.seed += 1;
    assert!(against(&t, &published(&phase_two())).iter().any(|c| c.status == Status::Differs));
}

/// Chunks the manifest does not list come back unpublished.
#[test]
fn chunks_missing_from_the_manifest_are_unpublished() {
    let t = phase_two();
    let mut was = published(&t);
    let (entry, _) = was.remove(0);
    let d = against(&t, &was);
    let lost = d.iter().find(|c| (c.chunk[0], c.chunk[1], c.chunk[2]) == entry.chunk && c.tier == entry.tier).unwrap();
    assert_eq!(lost.status, Status::Unpublished);
}

/// A publish on disk reads back into the same comparison.
#[test]
fn a_publish_on_disk_reads_back_as_the_same() {
    let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diff-read-back");
    let _ = std::fs::remove_dir_all(&out);
    let t = phase_two();
    publish::publish(&t, "p2", &out, 4).unwrap();
    let (manifest, chunks) = Manifest::read_from_disk(&out.join("manifest.galaxy")).unwrap();
    let was: Vec<_> = manifest.chunks.into_iter().zip(chunks).collect();
    assert!(against(&t, &was).iter().all(|c| c.status == Status::Same));
}
