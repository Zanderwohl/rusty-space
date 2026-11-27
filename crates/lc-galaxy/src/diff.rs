//! The current graph's bake against a published manifest, per chunk and tier: whether an edit
//! still describes what players were given. See `lightcone/docs/34-galaxy-authoring.md`,
//! "Publishing and freezing".
//!
//! Stars are matched by id, which a candidate keeps through any graph change that still accepts
//! it, and compared by their published records, shell included.

use std::collections::{BTreeMap, HashMap};

use lc_world::sky::galaxy::{GalaxyChunk, GalaxyRecord, ManifestChunk, Tier};

use crate::{Bake, Compiled, StarId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Every star as published.
    Same,
    /// Within [`Tolerance`] in count, metallicity and population mix.
    Close,
    Differs,
    /// The current bake has stars here and the manifest has none.
    Unpublished,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Same => "same",
            Status::Close => "close",
            Status::Differs => "differs",
            Status::Unpublished => "unpublished",
        }
    }
}

/// How far a published chunk may drift and still count as [`Status::Close`].
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    /// Of the published count, or `min_count`, whichever is larger.
    pub count_fraction: f64,
    pub min_count: usize,
    /// Mean `[Fe/H]`, dex.
    pub feh_dex: f64,
    /// Largest change in any population's share of the chunk.
    pub share: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self { count_fraction: 0.05, min_count: 2, feh_dex: 0.05, share: 0.05 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChunkDiff {
    pub chunk: [i32; 3],
    pub tier: Tier,
    pub status: Status,
    /// The graph version the published chunk was baked under.
    pub graph: Option<String>,
    pub published: usize,
    pub now: usize,
    /// Ids in both, with identical records.
    pub kept: usize,
    /// Ids in both, with a record that changed.
    pub changed: usize,
    /// Mean `[Fe/H]` now less as published, dex; 0 where either side is empty.
    pub feh_shift: f64,
    /// Largest change in a population's share.
    pub share_shift: f64,
}

/// Each published chunk against the same chunk and tier of `bake`, then the chunks only the bake
/// has, in chunk order. `graph` is the current graph version, as [`crate::publish::graph_version`].
pub fn diff(
    compiled: &Compiled,
    bake: &Bake,
    graph: u64,
    published: &[(ManifestChunk, GalaxyChunk)],
    tolerance: Tolerance,
) -> Result<Vec<ChunkDiff>, String> {
    let current: BTreeMap<([i32; 3], Tier), GalaxyChunk> =
        crate::publish::chunks(compiled, bake, graph)?.into_iter().map(|(tier, c)| ((c.chunk, tier), c)).collect();
    let mut out = Vec::new();
    for (entry, was) in published {
        let now = current.get(&(was.chunk, entry.tier));
        out.push(compare(was.chunk, entry.tier, Some(entry.graph.clone()), Some(was), now, tolerance));
    }
    for ((chunk, tier), now) in &current {
        if !published.iter().any(|(e, c)| c.chunk == *chunk && e.tier == *tier) {
            out.push(compare(*chunk, *tier, None, None, Some(now), tolerance));
        }
    }
    Ok(out)
}

/// A record with its profile index replaced by the profile, so two chunks whose tables list
/// profiles in different orders still compare equal.
fn resolved(c: &GalaxyChunk) -> HashMap<StarId, (GalaxyRecord, lc_world::sky::generate::SystemProfile)> {
    c.records
        .iter()
        .filter_map(|r| Some((r.id()?, (GalaxyRecord { profile: 0, ..*r }, c.profiles.get(r.profile as usize).copied()?))))
        .collect()
}

fn compare(chunk: [i32; 3], tier: Tier, graph: Option<String>, was: Option<&GalaxyChunk>, now: Option<&GalaxyChunk>, tolerance: Tolerance) -> ChunkDiff {
    let before = was.map(resolved).unwrap_or_default();
    let after = now.map(resolved).unwrap_or_default();
    let kept = before.iter().filter(|(id, r)| after.get(id) == Some(r)).count();
    let changed = before.keys().filter(|id| after.contains_key(id)).count() - kept;
    let records = |c: Option<&GalaxyChunk>| c.map_or(Vec::new(), |c| c.records.clone());
    let (rb, ra) = (records(was), records(now));
    let mean_feh = |rs: &[GalaxyRecord]| rs.iter().map(GalaxyRecord::feh).sum::<f64>() / rs.len() as f64;
    let feh_shift = if rb.is_empty() || ra.is_empty() { 0.0 } else { mean_feh(&ra) - mean_feh(&rb) };
    let shares = |rs: &[GalaxyRecord]| {
        let mut n = [0.0f64; 16];
        for r in rs {
            n[r.population() as usize & 15] += 1.0;
        }
        n.map(|k| if rs.is_empty() { 0.0 } else { k / rs.len() as f64 })
    };
    let (sb, sa) = (shares(&rb), shares(&ra));
    let share_shift = sb.iter().zip(sa).map(|(b, a)| (a - b).abs()).fold(0.0, f64::max);
    let (published, count) = (rb.len(), ra.len());
    let status = if was.is_none() {
        Status::Unpublished
    } else if kept == published && kept == count {
        Status::Same
    } else {
        let allowed = (tolerance.count_fraction * published as f64).max(tolerance.min_count as f64);
        let close = (count as f64 - published as f64).abs() <= allowed && feh_shift.abs() <= tolerance.feh_dex && share_shift <= tolerance.share;
        if close { Status::Close } else { Status::Differs }
    };
    ChunkDiff { chunk, tier, status, graph, published, now: count, kept, changed, feh_shift, share_shift }
}
