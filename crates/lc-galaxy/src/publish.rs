//! A local publish: bake a whole timeline, cut it into chunks, and write them with a manifest.
//!
//! Refuses a timeline with any unread line or any problem: a bake that skipped a feature would
//! publish a different galaxy than the file describes. The CDN upload is Phase III.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use lc_world::sky::galaxy::{GalaxyChunk, GalaxyRecord, MANIFEST_FORMAT, Manifest, ManifestChunk, chunk_of, content_hash};
use lc_world::sky::generate::SystemProfile;

use crate::attributes::{State, kinematics};
use crate::{Bake, BakedStar, Compiled, Row, StarId, Timeline, bake, codec};

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

/// Each baked star's shell radius against every other star the bake holds, ly, as the game loads
/// them: stored positions, evolved masses, and NaN for a star the sky leaves out. Across chunk
/// faces too, which is why it is worked out here and stored rather than at load, where a loader
/// may hold only some chunks.
pub fn shells(stars: &[BakedStar]) -> Vec<f64> {
    let in_sky = |s: &BakedStar| {
        s.attributes.luminosity_solar > 0.0 && !matches!(s.attributes.state, State::NeutronStar | State::BlackHole)
    };
    let kept: Vec<usize> = (0..stars.len()).filter(|&i| in_sky(&stars[i])).collect();
    let at: Vec<_> = kept.iter().map(|&i| (stars[i].attributes.record.position_ly(), stars[i].attributes.mass_solar)).collect();
    let mut out = vec![f64::NAN; stars.len()];
    for (&i, r) in kept.iter().zip(lc_world::sky::shells::radii(&at)) {
        out[i] = r;
    }
    out
}

/// The bake cut into chunks, each brightest first, each with its table of system profiles and
/// each star's shell. A region published alone would clamp its edge stars against nothing
/// outside it, so publish whole graphs, or regions with a margin of twice `MAX_SHELL_LY`.
pub fn chunks(compiled: &Compiled, bake: &Bake, graph: u64) -> Result<Vec<GalaxyChunk>, String> {
    let shell: HashMap<StarId, f64> = bake.stars.iter().map(|s| s.id).zip(shells(&bake.stars)).collect();
    let populations: Vec<_> = compiled.populations.keys().map(|id| kinematics(compiled, *id).expect("a compiled population has its frame")).collect();
    let mut by_chunk: BTreeMap<[i32; 3], Vec<_>> = BTreeMap::new();
    for s in &bake.stars {
        by_chunk.entry(chunk_of(s.key.cell.0.to_array())).or_default().push(s);
    }
    by_chunk
        .into_iter()
        .map(|(chunk, mut stars)| {
            stars.sort_by(|a, b| b.attributes.luminosity_solar.total_cmp(&a.attributes.luminosity_solar).then(a.id.cmp(&b.id)));
            let mut profiles = vec![SystemProfile::default()];
            let mut records = Vec::with_capacity(stars.len());
            for s in &stars {
                let index = match profiles.iter().position(|p| *p == s.attributes.profile) {
                    Some(i) => i,
                    None => {
                        profiles.push(s.attributes.profile);
                        profiles.len() - 1
                    }
                };
                if index > u8::MAX as usize {
                    return Err(format!("chunk {chunk:?} needs more than 256 system profiles; widen the system fields' edges"));
                }
                records.push(GalaxyRecord { profile: index as u8, ..s.attributes.record }.with_shell(shell[&s.id]));
            }
            Ok(GalaxyChunk { graph, chunk, populations: populations.clone(), profiles, records })
        })
        .collect()
}

/// Bakes `timeline` and writes `out/manifest.galaxy` and `out/chunks/<hash>.lcg`.
pub fn publish(timeline: &Timeline, name: &str, out: &Path, threads: usize) -> Result<Manifest, String> {
    let compiled = timeline.compile(timeline.len(), None);
    let refused = refusals(timeline, &compiled);
    if !refused.is_empty() {
        return Err(format!("not publishable:\n  {}", refused.join("\n  ")));
    }
    let graph = graph_version(timeline);
    let baked = bake(&compiled, timeline.seed, compiled.bounds(), threads).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out.join("chunks")).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut entries = Vec::new();
    for chunk in chunks(&compiled, &baked, graph)? {
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
