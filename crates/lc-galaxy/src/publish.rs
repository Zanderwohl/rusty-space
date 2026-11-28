//! A local publish: bake a whole timeline, cut it into chunks of one tier each, and write them
//! with a manifest.
//!
//! Refuses a timeline with any unread line or any problem: a bake that skipped a feature would
//! publish a different galaxy than the file describes.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use lc_world::sky::galaxy::{GalaxyChunk, GalaxyRecord, MANIFEST_FORMAT, Manifest, ManifestChunk, Tier, chunk_of, content_hash};
use lc_world::sky::generate::SystemProfile;

use crate::attributes::{State, kinematics};
use crate::sample::{Frozen, Tier as BakedTier};
use crate::{Bake, BakedStar, Compiled, Row, StarId, Timeline, codec};

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
    shells_around(stars, &Frozen::default())
}

/// [`shells`], with each new shell also kept clear of every stored one: a stored shell does not
/// change, so a new star beside it takes what room is left.
pub fn shells_around(stars: &[BakedStar], frozen: &Frozen) -> Vec<f64> {
    let in_sky = |s: &BakedStar| {
        s.attributes.luminosity_solar > 0.0 && !matches!(s.attributes.state, State::NeutronStar | State::BlackHole)
    };
    let kept: Vec<usize> = (0..stars.len()).filter(|&i| in_sky(&stars[i])).collect();
    let at: Vec<_> = kept.iter().map(|&i| (stars[i].attributes.record.position_ly(), stars[i].attributes.mass_solar)).collect();
    let mut out = vec![f64::NAN; stars.len()];
    for ((&i, r), (p, _)) in kept.iter().zip(lc_world::sky::shells::radii(&at)).zip(&at) {
        out[i] = frozen.near(*p, r + lc_world::sky::shells::MAX_SHELL_LY).filter(|(_, shell)| shell.is_finite()).fold(r, |r, (q, shell)| r.min(q.distance(*p) - shell));
    }
    out
}

/// The bake cut into chunks, one file per chunk and tier, each brightest first, each with its
/// table of system profiles and each star's shell.
pub fn chunks(compiled: &Compiled, bake: &Bake, graph: u64) -> Result<Vec<(Tier, GalaxyChunk)>, String> {
    chunks_around(compiled, bake, graph, &Frozen::default())
}

/// [`chunks`], with shells kept clear of `frozen`'s.
pub fn chunks_around(compiled: &Compiled, bake: &Bake, graph: u64, frozen: &Frozen) -> Result<Vec<(Tier, GalaxyChunk)>, String> {
    let shell: HashMap<StarId, f64> = bake.stars.iter().map(|s| s.id).zip(shells_around(&bake.stars, frozen)).collect();
    let populations: Vec<_> = compiled.populations.keys().map(|id| kinematics(compiled, *id).expect("a compiled population has its frame")).collect();
    let mut by_chunk: BTreeMap<([i32; 3], Tier), Vec<_>> = BTreeMap::new();
    for s in &bake.stars {
        let tier = if s.tier == BakedTier::A { Tier::A } else { Tier::B };
        by_chunk.entry((chunk_of(s.key.cell.0.to_array()), tier)).or_default().push(s);
    }
    by_chunk
        .into_iter()
        .map(|((chunk, tier), mut stars)| {
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
            Ok((tier, GalaxyChunk { graph, chunk, populations: populations.clone(), profiles, records }))
        })
        .collect()
}

/// What [`publish_with`] adds to, and where.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// A manifest to publish onto: its chunks are kept, and its stars are left as they are.
    pub onto: Option<std::path::PathBuf>,
    /// Bake only here, widened to whole chunks so a chunk is never published half baked.
    pub region: Option<crate::Aabb>,
    /// Tier-B chunks of `onto` to bake again. Doc 34 allows this only for a chunk no craft has
    /// seen; the caller is trusted to have checked.
    pub rebake: Vec<[i32; 3]>,
}

/// Bakes `timeline` and writes `out/chunks/<hash>.lcg`, the manifest as `out/<hash>.galaxy`, and
/// the same manifest as `out/manifest.galaxy` for a local shard or client to name. Files a
/// previous publish left in `out` are removed.
pub fn publish(timeline: &Timeline, name: &str, out: &Path, threads: usize) -> Result<Manifest, String> {
    publish_with(timeline, name, out, threads, &Options::default())
}

/// [`publish`], onto an earlier manifest or over a region.
pub fn publish_with(timeline: &Timeline, name: &str, out: &Path, threads: usize, options: &Options) -> Result<Manifest, String> {
    let compiled = timeline.compile(timeline.len(), None);
    let refused = refusals(timeline, &compiled);
    if !refused.is_empty() {
        return Err(format!("not publishable:\n  {}", refused.join("\n  ")));
    }
    let graph = graph_version(timeline);

    let mut kept: Vec<(ManifestChunk, Vec<u8>)> = Vec::new();
    let mut parent = None;
    let mut frozen = Frozen::default();
    if let Some(onto) = &options.onto {
        let text = std::fs::read(onto).map_err(|e| format!("{}: {e}", onto.display()))?;
        let (manifest, decoded) = Manifest::read_from_disk(onto)?;
        let published: std::collections::BTreeSet<[i32; 3]> = manifest.chunks.iter().map(|c| [c.chunk.0, c.chunk.1, c.chunk.2]).collect();
        if let Some(c) = options.rebake.iter().find(|c| !published.contains(*c)) {
            return Err(format!("chunk {c:?} is not in {}, so there is nothing to bake again", onto.display()));
        }
        let dir = onto.parent().unwrap_or(Path::new("."));
        let mut stored = Vec::new();
        for (entry, chunk) in manifest.chunks.iter().zip(&decoded) {
            if entry.tier == Tier::B && options.rebake.contains(&chunk.chunk) {
                continue;
            }
            stored.extend(chunk.records.iter().map(|r| (r.position_ly(), r.shell_ly())));
            let bytes = std::fs::read(dir.join(entry.checked_file().map_err(|e| e.to_string())?)).map_err(|e| format!("{}: {e}", entry.file))?;
            kept.push((entry.clone(), bytes));
        }
        // Tier A is fixed wherever anything was published, a chunk with no tier-A stars included.
        let tiers = published.iter().flat_map(|&c| {
            let b = (!options.rebake.contains(&c)).then_some((c, BakedTier::B));
            std::iter::once((c, BakedTier::A)).chain(b)
        });
        frozen = Frozen::new(tiers, stored);
        parent = Some(content_hash(&text));
    }

    let region = match options.region {
        Some(r) => {
            let side = f64::from(lc_world::sky::galaxy::CHUNK_CELLS) * crate::sample::CELL_LY;
            let snap = |v: glam::DVec3, f: fn(f64) -> f64| (v / side).map(f) * side;
            crate::Aabb { min: snap(r.min, f64::floor), max: snap(r.max, f64::ceil) - glam::DVec3::splat(1e-9) }.intersection(compiled.bounds())
        }
        None => compiled.bounds(),
    };
    let baked = crate::sample::bake_around(&compiled, timeline.seed, region, &frozen, threads).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out.join("chunks")).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut entries = Vec::new();
    for (entry, bytes) in &kept {
        std::fs::write(out.join(&entry.file), bytes).map_err(|e| format!("{}: {e}", entry.file))?;
        entries.push(entry.clone());
    }
    for (tier, chunk) in chunks_around(&compiled, &baked, graph, &frozen)? {
        let [x, y, z] = chunk.chunk;
        if entries.iter().any(|e| e.chunk == (x, y, z) && e.tier == tier) {
            return Err(format!("chunk {:?} tier {tier:?} is published already", chunk.chunk));
        }
        let bytes = chunk.encode().map_err(|e| e.to_string())?;
        let hash = content_hash(&bytes);
        let file = format!("chunks/{hash}.lcg");
        std::fs::write(out.join(&file), &bytes).map_err(|e| format!("{file}: {e}"))?;
        entries.push(ManifestChunk { chunk: (x, y, z), tier, graph: format!("{graph:016x}"), file, hash, stars: chunk.records.len() as u32 });
    }
    entries.sort_by_key(|e| (e.chunk, e.tier));
    let current: std::collections::BTreeSet<String> = entries.iter().map(|e| format!("{}.lcg", e.hash)).collect();
    for entry in std::fs::read_dir(out.join("chunks")).map_err(|e| format!("chunks: {e}"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lcg") && !current.contains(&name) {
            std::fs::remove_file(entry.path()).map_err(|e| format!("chunks/{name}: {e}"))?;
        }
    }
    let manifest = Manifest { format: MANIFEST_FORMAT, name: name.into(), parent, chunks: entries };
    let own = format!("{}.galaxy", manifest.hash());
    for entry in std::fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".galaxy") && name != own && name != "manifest.galaxy" {
            std::fs::remove_file(entry.path()).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    for file in [own.as_str(), "manifest.galaxy"] {
        std::fs::write(out.join(file), manifest.write()).map_err(|e| format!("{file}: {e}"))?;
    }
    Ok(manifest)
}
