//! An authored galaxy as published: chunk files addressed by content hash, and a manifest
//! listing them. Read by the shard and the client; written by `lc-galaxy`, which neither links.
//!
//! Positions are in the galaxy frame of `lightcone/docs/34-galaxy-authoring.md`, which for a
//! galaxy shard is the frame the simulation uses. See that document, "What a published star
//! stores".

use std::collections::HashMap;

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use super::{CatalogStar, Component, Provenance, StarProvider};
use crate::sky::generate::SystemProfile;
use crate::star::Star;

mod kinematics;
mod record;
pub mod tracks;

pub use kinematics::{Kinematics, Rotation};
pub use record::{CELL_HALF_RANGE, CELL_LY, FEH_STEP, GalaxyRecord, MASS_RANGE, MAX_AGE_GYR, cell_in_grid, pack_cell, star_id};

const MAGIC: &[u8; 6] = b"LCGAL\x00";
const VERSION: u16 = 2;

/// Cells per chunk along each axis: a chunk is 256 ly on a side, and a record names its cell
/// within the chunk in 15 bits.
pub const CHUNK_CELLS: i32 = 32;

/// Bytes per record: cell in chunk, index, offset, mass, age, `[Fe/H]`, flags, profile.
const RECORD: usize = 2 + 2 + 6 + 2 + 2 + 1 + 1 + 1;

/// The `source` a galaxy star's provenance names.
pub const SOURCE: &str = "galaxy";

pub fn chunk_of(cell: [i32; 3]) -> [i32; 3] {
    cell.map(|v| v.div_euclid(CHUNK_CELLS))
}

#[derive(Debug, PartialEq)]
pub enum GalaxyError {
    NotAChunk,
    UnsupportedVersion(u16),
    Truncated,
    /// A record names a population the chunk does not carry, or a cell outside the grid.
    BadRecord(usize),
    /// A record's cell is not in the chunk being written.
    WrongChunk { chunk: [i32; 3], cell: [i32; 3] },
    TooManyPopulations(usize),
    /// A chunk needs from 1 to 256 system profiles.
    Profiles(usize),
    /// A chunk's bytes do not hash to what the manifest says.
    HashMismatch { file: String },
    Manifest(String),
}

impl std::fmt::Display for GalaxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAChunk => write!(f, "not a galaxy chunk"),
            Self::UnsupportedVersion(v) => write!(f, "galaxy chunk version {v}, this build reads {VERSION}"),
            Self::Truncated => write!(f, "galaxy chunk is truncated"),
            Self::BadRecord(i) => write!(f, "record {i} does not describe a star"),
            Self::WrongChunk { chunk, cell } => write!(f, "cell {cell:?} is not in chunk {chunk:?}"),
            Self::TooManyPopulations(n) => write!(f, "{n} populations; a chunk holds at most 16"),
            Self::Profiles(n) => write!(f, "{n} system profiles; a chunk holds 1 to 256"),
            Self::HashMismatch { file } => write!(f, "{file} does not hash to what the manifest says"),
            Self::Manifest(e) => write!(f, "the manifest does not read: {e}"),
        }
    }
}

impl std::error::Error for GalaxyError {}

/// The stars of up to 32³ cells, brightest first, so a prefix of the file holds the brightest.
#[derive(Clone, Debug, PartialEq)]
pub struct GalaxyChunk {
    /// The graph version the bake recorded.
    pub graph: u64,
    pub chunk: [i32; 3],
    /// Indexed by a record's population.
    pub populations: Vec<Kinematics>,
    /// Indexed by a record's profile; the first is the default profile.
    pub profiles: Vec<SystemProfile>,
    pub records: Vec<GalaxyRecord>,
}

impl GalaxyChunk {
    /// Little-endian throughout. Records keep the order given.
    pub fn encode(&self) -> Result<Vec<u8>, GalaxyError> {
        if self.populations.len() > 16 {
            return Err(GalaxyError::TooManyPopulations(self.populations.len()));
        }
        let mut out = Vec::with_capacity(64 + self.populations.len() * 128 + self.records.len() * RECORD);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.graph.to_le_bytes());
        for v in self.chunk {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(self.populations.len() as u8);
        let f = |out: &mut Vec<u8>, v: f64| out.extend_from_slice(&v.to_le_bytes());
        for k in &self.populations {
            for v in k.origin.to_array() {
                f(&mut out, v);
            }
            for v in k.axes.to_cols_array() {
                f(&mut out, v);
            }
            match k.rotation {
                Rotation::Static => {
                    out.push(0);
                    f(&mut out, 0.0);
                    f(&mut out, 0.0);
                }
                Rotation::Flat { speed_km_s, core_ly } => {
                    out.push(1);
                    f(&mut out, speed_km_s);
                    f(&mut out, core_ly);
                }
            }
            f(&mut out, k.dispersion_km_s);
            f(&mut out, k.dispersion_exponent);
        }
        if self.profiles.is_empty() || self.profiles.len() > 256 {
            return Err(GalaxyError::Profiles(self.profiles.len()));
        }
        out.push((self.profiles.len() - 1) as u8);
        for p in &self.profiles {
            out.extend_from_slice(&[p.ocean, p.life, p.solids as u8, p.belts as u8, p.cloud as u8]);
        }
        out.extend_from_slice(&(self.records.len() as u32).to_le_bytes());
        let base = self.chunk.map(|v| v * CHUNK_CELLS);
        for r in &self.records {
            if chunk_of(r.cell) != self.chunk {
                return Err(GalaxyError::WrongChunk { chunk: self.chunk, cell: r.cell });
            }
            let local = [r.cell[0] - base[0], r.cell[1] - base[1], r.cell[2] - base[2]];
            let at = (local[0] + CHUNK_CELLS * (local[1] + CHUNK_CELLS * local[2])) as u16;
            out.extend_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&r.index.to_le_bytes());
            for o in r.offset {
                out.extend_from_slice(&o.to_le_bytes());
            }
            out.extend_from_slice(&r.mass.to_le_bytes());
            out.extend_from_slice(&r.age.to_le_bytes());
            out.push(r.feh as u8);
            out.push(r.flags);
            out.push(r.profile);
        }
        Ok(out)
    }

    /// Every length is checked against what is left: these bytes come over a network.
    pub fn decode(bytes: &[u8]) -> Result<Self, GalaxyError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(MAGIC.len())? != MAGIC {
            return Err(GalaxyError::NotAChunk);
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(GalaxyError::UnsupportedVersion(version));
        }
        let graph = r.u64()?;
        let chunk = [r.i32()?, r.i32()?, r.i32()?];
        let count = r.u8()? as usize;
        let mut populations = Vec::with_capacity(count);
        for _ in 0..count {
            let origin = DVec3::new(r.f64()?, r.f64()?, r.f64()?);
            let mut axes = [0.0; 9];
            for a in &mut axes {
                *a = r.f64()?;
            }
            let kind = r.u8()?;
            let (speed_km_s, core_ly) = (r.f64()?, r.f64()?);
            let rotation = if kind == 1 { Rotation::Flat { speed_km_s, core_ly } } else { Rotation::Static };
            populations.push(Kinematics {
                origin,
                axes: DMat3::from_cols_array(&axes),
                rotation,
                dispersion_km_s: r.f64()?,
                dispersion_exponent: r.f64()?,
            });
        }
        let profile_count = r.u8()? as usize + 1;
        let mut profiles = Vec::with_capacity(profile_count);
        for _ in 0..profile_count {
            let b = r.take(5)?;
            profiles.push(SystemProfile { ocean: b[0], life: b[1], solids: b[2] as i8, belts: b[3] as i8, cloud: b[4] as i8 });
        }
        let n = r.u32()? as usize;
        if bytes.len() - r.at < n.saturating_mul(RECORD) {
            return Err(GalaxyError::Truncated);
        }
        let base = chunk.map(|v| v * CHUNK_CELLS);
        let mut records = Vec::with_capacity(n);
        for i in 0..n {
            let at = r.u16()? as i32;
            if at >= CHUNK_CELLS * CHUNK_CELLS * CHUNK_CELLS {
                return Err(GalaxyError::BadRecord(i));
            }
            let local = [at % CHUNK_CELLS, (at / CHUNK_CELLS) % CHUNK_CELLS, at / (CHUNK_CELLS * CHUNK_CELLS)];
            let cell = [base[0] + local[0], base[1] + local[1], base[2] + local[2]];
            let record = GalaxyRecord {
                cell,
                index: r.u16()?,
                offset: [r.u16()?, r.u16()?, r.u16()?],
                mass: r.u16()?,
                age: r.u16()?,
                feh: r.u8()? as i8,
                flags: r.u8()?,
                profile: r.u8()?,
            };
            if record.population() as usize >= populations.len()
                || record.profile as usize >= profiles.len()
                || !cell_in_grid(cell)
            {
                return Err(GalaxyError::BadRecord(i));
            }
            records.push(record);
        }
        Ok(Self { graph, chunk, populations, profiles, records })
    }

    /// The chunk's stars, and the number of remnants left out because they emit nothing.
    pub fn stars(&self) -> (Vec<CatalogStar>, usize) {
        let mut skipped = 0;
        let stars = self
            .records
            .iter()
            .filter_map(|r| {
                let profile = self.profiles[r.profile as usize];
                let star = catalog_star(r, &self.populations[r.population() as usize], profile);
                if star.is_none() {
                    skipped += 1;
                }
                star
            })
            .collect();
        (stars, skipped)
    }
}

/// The star a record loads as; `None` for one the sky leaves out: a neutron star, a black hole, or
/// a star with no light.
pub fn catalog_star(r: &GalaxyRecord, kinematics: &Kinematics, profile: SystemProfile) -> Option<CatalogStar> {
    let id = r.id()?;
    let e = tracks::evolve_step(r.initial_mass_solar(), r.age_gyr(), r.feh);
    // Neutron stars and black holes are left out: their light is X-rays or nothing, and the sky
    // has nothing to draw.
    if matches!(e.state, tracks::State::NeutronStar | tracks::State::BlackHole) || e.luminosity_solar <= 0.0 {
        return None;
    }
    let position_ly = r.position_ly();
    Some(CatalogStar {
        id,
        provenance: Provenance { source: SOURCE.into(), key: id.get(), name: None },
        position_ly,
        velocity: kinematics.velocity_km_s(position_ly, r.age_gyr(), id) * 1000.0,
        star: Star {
            radius_m: e.radius_solar * em_spectra::stellar::SOLAR_RADIUS,
            teff_k: e.teff_k,
            mu: em_spectra::stellar::mu_from_mass_solar(e.mass_solar),
            limb_darkening: (0.4, 0.26),
        },
        luminosity_solar: e.luminosity_solar,
        mass_solar: e.mass_solar,
        metallicity: r.feh(),
        component: Component { index: 1, group: None },
        system: (!profile.is_default()).then_some(profile),
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], GalaxyError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or(GalaxyError::Truncated)?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, GalaxyError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, GalaxyError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, GalaxyError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, GalaxyError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, GalaxyError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, GalaxyError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// FNV-1a over the bytes, as 16 hex digits: what names a chunk file. It detects a wrong or
/// damaged file, not a hostile one; the CDN and the manifest are trusted.
pub fn content_hash(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3));
    format!("{h:016x}")
}

pub const MANIFEST_FORMAT: u32 = 1;

/// What a publish wrote: the chunks of one galaxy, and the graph version they were baked from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    /// The graph version, as 16 hex digits.
    pub graph: String,
    pub chunks: Vec<ManifestChunk>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestChunk {
    pub chunk: (i32, i32, i32),
    /// Relative to the manifest.
    pub file: String,
    pub hash: String,
    pub stars: u32,
}

impl ManifestChunk {
    /// The file, refused if it is absolute or climbs out of the manifest's directory: a
    /// manifest names files beside it and nothing else.
    pub fn checked_file(&self) -> Result<&str, GalaxyError> {
        let path = std::path::Path::new(&self.file);
        let escapes = path.components().any(|c| !matches!(c, std::path::Component::Normal(_)));
        if escapes || self.file.is_empty() {
            return Err(GalaxyError::Manifest(format!("{:?} is not a file beside the manifest", self.file)));
        }
        Ok(&self.file)
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, GalaxyError> {
        let m: Self = ron::from_str(text).map_err(|e| GalaxyError::Manifest(e.to_string()))?;
        if m.format > MANIFEST_FORMAT {
            return Err(GalaxyError::Manifest(format!("format {} is newer than {MANIFEST_FORMAT}", m.format)));
        }
        Ok(m)
    }

    pub fn write(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).expect("a manifest serializes")
    }

    /// Reads a manifest and the chunks beside it from a filesystem.
    pub fn load_from_disk(path: &std::path::Path) -> Result<GalaxyProvider, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let manifest = Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let bytes = manifest
            .chunks
            .iter()
            .map(|c| {
                let file = c.checked_file().map_err(|e| e.to_string())?;
                std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        manifest.load(&bytes).map_err(|e| e.to_string())
    }

    /// Decodes each chunk's bytes, given in manifest order, after checking its hash.
    pub fn load(&self, bytes: &[Vec<u8>]) -> Result<GalaxyProvider, GalaxyError> {
        if bytes.len() != self.chunks.len() {
            return Err(GalaxyError::Manifest(format!("{} chunks listed, {} given", self.chunks.len(), bytes.len())));
        }
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for (entry, b) in self.chunks.iter().zip(bytes) {
            if content_hash(b) != entry.hash {
                return Err(GalaxyError::HashMismatch { file: entry.file.clone() });
            }
            chunks.push(GalaxyChunk::decode(b)?);
        }
        Ok(GalaxyProvider::new(self.name.clone(), &chunks))
    }
}

/// The stars of a set of chunks, indexed by cell for region queries.
pub struct GalaxyProvider {
    name: String,
    stars: Vec<CatalogStar>,
    cells: HashMap<[i32; 3], Vec<usize>>,
    /// Records that were remnants with nothing to show.
    pub skipped: usize,
}

impl GalaxyProvider {
    /// Nearest the origin first, ties by id, as the HYG catalog is: a shard starts a new craft at
    /// its first star, and a client's session sorts the same way, so a shard in the box and a
    /// standalone one agree on it whatever order the chunks loaded in.
    pub fn new(name: String, chunks: &[GalaxyChunk]) -> Self {
        let mut stars = Vec::new();
        let mut skipped = 0;
        for c in chunks {
            let (s, k) = c.stars();
            stars.extend(s);
            skipped += k;
        }
        stars.sort_by(|a: &CatalogStar, b: &CatalogStar| {
            a.position_ly.length().total_cmp(&b.position_ly.length()).then(a.id.cmp(&b.id))
        });
        let mut cells: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
        for (i, s) in stars.iter().enumerate() {
            cells.entry(cell_containing(s.position_ly)).or_default().push(i);
        }
        Self { name, stars, cells, skipped }
    }

    pub fn into_stars(self) -> Vec<CatalogStar> {
        self.stars
    }
}

fn cell_containing(p: DVec3) -> [i32; 3] {
    (p / CELL_LY).floor().as_ivec3().to_array()
}

impl StarProvider for GalaxyProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn stars(&self) -> &[CatalogStar] {
        &self.stars
    }

    fn within(&self, center: DVec3, radius_ly: f64) -> Vec<&CatalogStar> {
        let lo = cell_containing(center - DVec3::splat(radius_ly));
        let hi = cell_containing(center + DVec3::splat(radius_ly));
        let span = (0..3).map(|i| (hi[i] - lo[i] + 1) as f64).product::<f64>();
        if span > self.stars.len() as f64 {
            return self.stars.iter().filter(|s| s.position_ly.distance(center) <= radius_ly).collect();
        }
        let mut out = Vec::new();
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    for &i in self.cells.get(&[x, y, z]).into_iter().flatten() {
                        if self.stars[i].position_ly.distance(center) <= radius_ly {
                            out.push(&self.stars[i]);
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
