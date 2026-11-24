//! A local publish: bake a whole timeline, cut it into chunks, and write them with a manifest.
//!
//! Refuses a timeline with any unread line or any problem: a bake that skipped a feature would
//! publish a different galaxy than the file describes. The CDN upload is Phase III.

use std::collections::BTreeMap;
use std::path::Path;

use lc_world::sky::galaxy::{GalaxyChunk, MANIFEST_FORMAT, Manifest, ManifestChunk, chunk_of, content_hash};

use crate::attributes::kinematics;
use crate::sample::default_threads;
use crate::{Bake, Compiled, Row, Timeline, bake, codec};

/// Bumped whenever the generator would bake a different galaxy from the same timeline.
pub const GENERATOR_VERSION: u64 = 2;

/// The content hash of the timeline as written, together with the generator's version.
pub fn graph_version(timeline: &Timeline) -> u64 {
    let text = codec::write(timeline);
    let h = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    lc_world::rng::hash(&[h, GENERATOR_VERSION])
}

/// Why a timeline cannot be published.
pub fn refusals(timeline: &Timeline, compiled: &Compiled) -> Vec<String> {
    let mut out: Vec<String> = timeline
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r, Row::Unread(_)))
        .map(|(i, _)| format!("row {i} is not readable by this build"))
        .collect();
    out.extend(compiled.problems.iter().map(|(id, p)| format!("{id}: {p}")));
    out
}

/// The bake cut into chunks, each brightest first.
pub fn chunks(compiled: &Compiled, bake: &Bake, graph: u64) -> Vec<GalaxyChunk> {
    let populations: Vec<_> = compiled.populations.keys().map(|id| kinematics(compiled, *id).expect("a compiled population has its frame")).collect();
    let mut by_chunk: BTreeMap<[i32; 3], Vec<_>> = BTreeMap::new();
    for s in &bake.stars {
        by_chunk.entry(chunk_of(s.key.cell.0.to_array())).or_default().push(s);
    }
    by_chunk
        .into_iter()
        .map(|(chunk, mut stars)| {
            stars.sort_by(|a, b| b.attributes.luminosity_solar.total_cmp(&a.attributes.luminosity_solar).then(a.id.cmp(&b.id)));
            GalaxyChunk { graph, chunk, populations: populations.clone(), records: stars.iter().map(|s| s.attributes.record).collect() }
        })
        .collect()
}

/// Bakes `timeline` and writes `out/manifest.galaxy` and `out/chunks/<hash>.lcg`.
pub fn publish(timeline: &Timeline, name: &str, out: &Path) -> Result<Manifest, String> {
    let compiled = timeline.compile(timeline.len(), None);
    let refused = refusals(timeline, &compiled);
    if !refused.is_empty() {
        return Err(format!("not publishable:\n  {}", refused.join("\n  ")));
    }
    let graph = graph_version(timeline);
    let baked = bake(&compiled, timeline.seed, compiled.bounds(), default_threads()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out.join("chunks")).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut entries = Vec::new();
    for chunk in chunks(&compiled, &baked, graph) {
        let bytes = chunk.encode().map_err(|e| e.to_string())?;
        let hash = content_hash(&bytes);
        let file = format!("chunks/{hash}.lcg");
        std::fs::write(out.join(&file), &bytes).map_err(|e| format!("{file}: {e}"))?;
        let [x, y, z] = chunk.chunk;
        entries.push(ManifestChunk { chunk: (x, y, z), file, hash, stars: chunk.records.len() as u32 });
    }
    let current: std::collections::BTreeSet<String> = entries.iter().map(|e| format!("{}.lcg", e.hash)).collect();
    for entry in std::fs::read_dir(out.join("chunks")).map_err(|e| format!("chunks: {e}"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lcg") && !current.contains(&name) {
            std::fs::remove_file(entry.path()).map_err(|e| format!("chunks/{name}: {e}"))?;
        }
    }
    let manifest = Manifest { format: MANIFEST_FORMAT, name: name.into(), graph: format!("{graph:016x}"), chunks: entries };
    std::fs::write(out.join("manifest.galaxy"), manifest.write()).map_err(|e| format!("manifest: {e}"))?;
    Ok(manifest)
}
