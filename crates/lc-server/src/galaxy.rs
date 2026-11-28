//! The authored galaxy as the administration console reads it: the manifest this shard loaded,
//! its chunks, and how much any craft knows about each. See
//! `lightcone/docs/34-galaxy-authoring.md`, "In the administration console".
//!
//! Discovery is read from the knowledge store and the checkpoint. *Visited* is where a craft is
//! now, inside a star's shell: the checkpoint keeps no history of where craft have been.

use std::collections::{HashMap, HashSet};

use lc_world::knowledge::Subject;
use lc_world::sky::CatalogStar;
use lc_world::sky::galaxy::{GalaxyChunk, Manifest, Tier};
use serde::{Deserialize, Serialize};

/// What `--sky` loaded, when it named a galaxy.
#[derive(Debug)]
pub struct Published {
    /// The `--sky` it was read from.
    pub source: String,
    /// What clients are told to load, from `--sky-url`.
    pub url: Option<String>,
    pub manifest: Manifest,
    /// In manifest order.
    pub chunks: Vec<GalaxyChunk>,
}

/// How much any craft knows about a star, or the most about any star in a chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Unseen,
    /// Some craft holds a file on the star.
    Charted,
    /// Some craft holds a file on a body or a population of its system.
    Resolved,
    /// Some craft is inside its shell now.
    Visited,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkRow {
    pub chunk: (i32, i32, i32),
    /// `A` or `B`.
    pub tier: String,
    pub graph: String,
    pub hash: String,
    pub stars: u32,
    pub charted: u32,
    pub resolved: u32,
    pub visited: u32,
    /// Craft holding any file on a star here.
    pub craft: u32,
    pub level: Level,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Overview {
    pub source: String,
    pub url: Option<String>,
    pub name: String,
    pub parent: Option<String>,
    pub chunks: Vec<ChunkRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StarRow {
    pub id: u64,
    pub position_ly: (f64, f64, f64),
    /// Index into the chunk's populations.
    pub population: u8,
    pub initial_mass_solar: f64,
    pub age_gyr: f64,
    pub feh: f64,
    /// NaN for a star the sky leaves out.
    pub shell_ly: f64,
    /// From the loaded sky; `None` for a dark remnant, which the sky leaves out.
    pub mass_solar: Option<f64>,
    pub teff_k: Option<f64>,
    pub luminosity_solar: Option<f64>,
    pub level: Level,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkStars {
    pub chunk: (i32, i32, i32),
    pub tier: String,
    pub stars: Vec<StarRow>,
}

/// What craft know, by star: the files they hold and who is in which system now.
#[derive(Debug, Default)]
pub struct Known {
    charted: HashMap<u64, HashSet<i64>>,
    resolved: HashMap<u64, HashSet<i64>>,
    present: HashMap<u64, u64>,
}

impl Known {
    /// `files` are each file's ship and subject; `present` is [`crate::systems::tally`].
    pub fn new(files: impl IntoIterator<Item = (i64, Subject)>, present: HashMap<u64, u64>) -> Self {
        let mut k = Known { present, ..Known::default() };
        for (ship, subject) in files {
            match subject {
                Subject::Star(star) => k.charted.entry(star.get()).or_default().insert(ship),
                Subject::Body { star, .. } | Subject::Population { star, .. } => k.resolved.entry(star.get()).or_default().insert(ship),
                _ => continue,
            };
        }
        k
    }

    pub fn level(&self, star: u64) -> Level {
        if self.present.get(&star).is_some_and(|&n| n > 0) {
            Level::Visited
        } else if self.resolved.contains_key(&star) {
            Level::Resolved
        } else if self.charted.contains_key(&star) {
            Level::Charted
        } else {
            Level::Unseen
        }
    }
}

fn tier_label(tier: Tier) -> String {
    format!("{tier:?}")
}

pub fn overview(published: &Published, known: &Known) -> Overview {
    let chunks = published
        .manifest
        .chunks
        .iter()
        .zip(&published.chunks)
        .map(|(entry, chunk)| {
            let ids: Vec<u64> = chunk.records.iter().filter_map(|r| r.id()).map(|id| id.get()).collect();
            let at = |l: Level| ids.iter().filter(|&&id| known.level(id) == l).count() as u32;
            let craft: HashSet<i64> = ids
                .iter()
                .flat_map(|id| known.charted.get(id).into_iter().chain(known.resolved.get(id)).flatten())
                .copied()
                .collect();
            ChunkRow {
                chunk: entry.chunk,
                tier: tier_label(entry.tier),
                graph: entry.graph.clone(),
                hash: entry.hash.clone(),
                stars: entry.stars,
                charted: at(Level::Charted),
                resolved: at(Level::Resolved),
                visited: at(Level::Visited),
                craft: craft.len() as u32,
                level: ids.iter().map(|&id| known.level(id)).max().unwrap_or(Level::Unseen),
            }
        })
        .collect();
    Overview {
        source: published.source.clone(),
        url: published.url.clone(),
        name: published.manifest.name.clone(),
        parent: published.manifest.parent.clone(),
        chunks,
    }
}

/// One chunk's stars, brightest first as stored. `key` is `x,y,z,A` or `x,y,z,B`.
pub fn chunk_stars(published: &Published, key: &str, stars: &[CatalogStar], known: &Known) -> Option<ChunkStars> {
    let parts: Vec<&str> = key.split(',').collect();
    let [x, y, z, tier] = parts.as_slice() else { return None };
    let at = (x.parse().ok()?, y.parse().ok()?, z.parse().ok()?);
    let tier = match *tier {
        "A" => Tier::A,
        "B" => Tier::B,
        _ => return None,
    };
    let i = published.manifest.chunks.iter().position(|e| e.chunk == at && e.tier == tier)?;
    let by_id: HashMap<u64, &CatalogStar> = stars.iter().map(|s| (s.id.get(), s)).collect();
    let rows = published.chunks[i]
        .records
        .iter()
        .filter_map(|r| {
            let id = r.id()?.get();
            let p = r.position_ly();
            let lit = by_id.get(&id);
            Some(StarRow {
                id,
                position_ly: (p.x, p.y, p.z),
                population: r.population(),
                initial_mass_solar: r.initial_mass_solar(),
                age_gyr: r.age_gyr(),
                feh: r.feh(),
                shell_ly: r.shell_ly(),
                mass_solar: lit.map(|s| s.mass_solar),
                teff_k: lit.map(|s| s.star.teff_k),
                luminosity_solar: lit.map(|s| s.luminosity_solar),
                level: known.level(id),
            })
        })
        .collect();
    Some(ChunkStars { chunk: at, tier: tier_label(tier), stars: rows })
}

#[cfg(test)]
mod tests {
    use glam::{DMat3, DVec3};
    use lc_world::sky::StarId;
    use lc_world::sky::galaxy::{GalaxyRecord, Kinematics, MANIFEST_FORMAT, ManifestChunk, Rotation};

    use super::*;

    fn published() -> Published {
        let kinematics = Kinematics {
            origin: DVec3::ZERO,
            axes: DMat3::IDENTITY,
            rotation: Rotation::Flat { speed_km_s: 220.0, core_ly: 3000.0 },
            dispersion_km_s: 10.0,
            dispersion_exponent: 0.5,
        };
        let chunk = |c: [i32; 3], tier_a: bool, n: u16| GalaxyChunk {
            graph: 1,
            chunk: c.map(|v| v.div_euclid(32)),
            populations: vec![kinematics.clone()],
            profiles: vec![Default::default()],
            records: (0..n).map(|i| GalaxyRecord::quantize(c, i, DVec3::from_array(c.map(f64::from)) * 8.0 + 1.0, 1.0, 1.0, 0.0, 0, tier_a)).collect(),
        };
        let chunks = vec![chunk([0, 0, 0], false, 3), chunk([0, 0, 0], true, 1), chunk([40, 0, 0], false, 2)];
        let entry = |c: &GalaxyChunk, tier: Tier| ManifestChunk {
            chunk: (c.chunk[0], c.chunk[1], c.chunk[2]),
            tier,
            graph: "0000000000000001".into(),
            file: String::new(),
            hash: String::new(),
            stars: c.records.len() as u32,
        };
        let manifest = Manifest {
            format: MANIFEST_FORMAT,
            name: "test".into(),
            parent: None,
            chunks: vec![entry(&chunks[0], Tier::B), entry(&chunks[1], Tier::A), entry(&chunks[2], Tier::B)],
        };
        Published { source: "x.galaxy".into(), url: None, manifest, chunks }
    }

    fn id(p: &Published, chunk: usize, record: usize) -> StarId {
        p.chunks[chunk].records[record].id().unwrap()
    }

    /// A chunk takes the most any craft knows of any of its stars; one nobody knows is unseen.
    #[test]
    fn a_chunk_is_as_known_as_its_best_known_star() {
        let p = published();
        let files = vec![(7, Subject::Star(id(&p, 0, 0))), (8, Subject::Star(id(&p, 0, 1))), (8, Subject::Population { star: id(&p, 1, 0), index: 0 })];
        let present = HashMap::from([(id(&p, 0, 2).get(), 1)]);
        let o = overview(&p, &Known::new(files, present));
        let levels: Vec<_> = o.chunks.iter().map(|c| (c.tier.as_str(), c.level, c.charted, c.visited, c.craft)).collect();
        assert_eq!(levels, vec![("B", Level::Visited, 2, 1, 2), ("A", Level::Resolved, 0, 0, 1), ("B", Level::Unseen, 0, 0, 0)]);

        let stars = chunk_stars(&p, "0,0,0,B", &[], &Known::default()).unwrap();
        assert_eq!(stars.stars.len(), 3);
        assert!(stars.stars.iter().all(|s| s.level == Level::Unseen && s.teff_k.is_none()));
        assert!(chunk_stars(&p, "0,0,0,C", &[], &Known::default()).is_none());
        assert!(chunk_stars(&p, "9,9,9,B", &[], &Known::default()).is_none());
    }

}
