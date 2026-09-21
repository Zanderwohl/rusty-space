//! A packed sky, for clients that cannot read a filesystem.
//!
//! The browser has no `std::fs` and no reason to download 32 MB of CSV. This is the same
//! catalogue as a block of bytes: fetched over HTTP, decoded from a slice, and handed to the
//! same [`StarRecord::assemble`] the CSV importer uses.
//!
//! **What is stored is what cannot be recomputed.** Radius, temperature, mu, mass and
//! metallicity are all derived from color index, luminosity and velocity, so they are absent
//! here and computed on load. Storing them would be four times the size and a way for a chunk
//! to disagree with the code that made it.
//!
//! Positions and velocities are `f32`. That is not a compromise against the source: HYG gives
//! six significant digits, and `f32` carries seven. It would be a compromise against a
//! catalogue that knew better, and the format version is how that gets noticed.

use glam::DVec3;

use super::record::{StarRecord, assemble_all};
use super::{CatalogueStar, StarProvider};

const MAGIC: &[u8; 6] = b"LCSKY\x00";
const VERSION: u16 = 1;

/// Bytes per packed record: key, position, velocity, color index, luminosity, component.
const RECORD: usize = 4 + 12 + 12 + 2 + 4 + 1 + 4;

/// Color index is stored as a whole number of thousandths.
///
/// Not as `f32`, and the reason is a boundary rather than a size. `BV_VALID` starts at exactly
/// -0.4, `f32` cannot represent -0.4, and the nearest `f32` is *below* it — so eleven stars
/// recorded at exactly -0.4 assembled from the CSV and then failed to assemble from their own
/// chunk. Thousandths represent every value HYG actually publishes exactly, and they are two
/// bytes rather than four.
const BV_SCALE: f64 = 1000.0;

/// High bit of the component byte: set when the record belongs to a multiple.
///
/// A presence bit rather than a reserved group value. Zero was the obvious sentinel and was
/// wrong: HYG gives the Sun `comp_primary = 0`, so "no group" and "grouped under key 0" are
/// both real and must not collide. Component indices are single digits; the bit is free.
const HAS_GROUP: u8 = 0x80;

/// Names are rare — a few hundred stars in a hundred thousand — so they are a sparse table
/// rather than an empty string on every record. Each entry is an index, a length, the bytes.
#[cfg(test)]
const NAME_ENTRY_HEADER: usize = 4 + 2;

#[derive(Debug)]
pub enum ChunkError {
    NotAChunk,
    UnsupportedVersion(u16),
    Truncated { wanted: usize, had: usize },
    BadUtf8,
    /// A key that does not fit the packed width. Encoding only.
    KeyTooLarge(u64),
}

impl std::fmt::Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAChunk => write!(f, "not a sky chunk"),
            Self::UnsupportedVersion(v) => {
                write!(f, "sky chunk version {v}, this build reads {VERSION}")
            }
            Self::Truncated { wanted, had } => {
                write!(f, "sky chunk is truncated: wanted {wanted} bytes, had {had}")
            }
            Self::BadUtf8 => write!(f, "a star name is not valid UTF-8"),
            Self::KeyTooLarge(k) => write!(f, "catalogue key {k} does not fit in 32 bits"),
        }
    }
}
impl std::error::Error for ChunkError {}

/// Filters records to those that describe a star, then packs them.
///
/// **Use this rather than [`encode`] to build a shipped chunk.** The stored values are `f32`,
/// and a row sitting exactly on a validity boundary — a `B-V` a hair outside `BV_VALID`, say —
/// can round *into* range on the way in. Deciding with the source's own `f64` values and
/// packing only the survivors means a chunk can never contain a star the catalogue rejected.
///
/// Returns the bytes and the number of records dropped.
pub fn pack(source: &str, records: &[StarRecord]) -> Result<(Vec<u8>, usize), ChunkError> {
    let keep: Vec<StarRecord> =
        records.iter().filter(|r| r.assemble(source).is_some()).cloned().collect();
    let dropped = records.len() - keep.len();
    Ok((encode(source, &keep)?, dropped))
}

/// Packs records into the chunk format. Little-endian throughout.
///
/// Packs whatever it is given. [`pack`] is what a build should call.
pub fn encode(source: &str, records: &[StarRecord]) -> Result<Vec<u8>, ChunkError> {
    let named: Vec<(u32, &str)> = records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.name.as_deref().map(|n| (i as u32, n)))
        .collect();

    let mut out = Vec::with_capacity(
        MAGIC.len() + 2 + 1 + source.len() + 4 + records.len() * RECORD + 4 + named.len() * 16,
    );
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.push(u8::try_from(source.len()).unwrap_or(u8::MAX));
    out.extend_from_slice(&source.as_bytes()[..source.len().min(u8::MAX as usize)]);
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());

    for r in records {
        let key = u32::try_from(r.key).map_err(|_| ChunkError::KeyTooLarge(r.key))?;
        out.extend_from_slice(&key.to_le_bytes());
        for v in [r.position_ly.x, r.position_ly.y, r.position_ly.z] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        for v in [r.velocity.x, r.velocity.y, r.velocity.z] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        let bv = (r.color_index * BV_SCALE).round().clamp(i16::MIN as f64, i16::MAX as f64);
        out.extend_from_slice(&(bv as i16).to_le_bytes());
        out.extend_from_slice(&(r.luminosity_solar as f32).to_le_bytes());
        let index = r.component_index & !HAS_GROUP;
        out.push(if r.group.is_some() { index | HAS_GROUP } else { index });
        let group = match r.group {
            Some(g) => u32::try_from(g).map_err(|_| ChunkError::KeyTooLarge(g))?,
            None => 0,
        };
        out.extend_from_slice(&group.to_le_bytes());
    }

    out.extend_from_slice(&(named.len() as u32).to_le_bytes());
    for (index, name) in named {
        let bytes = name.as_bytes();
        let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&index.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&bytes[..len as usize]);
    }
    Ok(out)
}

/// Reads a chunk back into records and the source name they were packed under.
///
/// Every length is checked against what is left, so a truncated or hostile chunk produces an
/// error rather than a panic. This decodes bytes fetched over a network.
pub fn decode(bytes: &[u8]) -> Result<(String, Vec<StarRecord>), ChunkError> {
    let mut r = Reader { bytes, at: 0 };

    if r.take(MAGIC.len())? != MAGIC {
        return Err(ChunkError::NotAChunk);
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(ChunkError::UnsupportedVersion(version));
    }
    let source_len = r.u8()? as usize;
    let source = core::str::from_utf8(r.take(source_len)?).map_err(|_| ChunkError::BadUtf8)?;
    let source = source.to_owned();

    let count = r.u32()? as usize;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let key = r.u32()? as u64;
        let position_ly = DVec3::new(r.f32()? as f64, r.f32()? as f64, r.f32()? as f64);
        let velocity = DVec3::new(r.f32()? as f64, r.f32()? as f64, r.f32()? as f64);
        let color_index = r.i16()? as f64 / BV_SCALE;
        let luminosity_solar = r.f32()? as f64;
        let packed = r.u8()?;
        let component_index = packed & !HAS_GROUP;
        // Named for what it is: `key` is already the star's own, and shadowing it here puts
        // the group's key into the record's identity.
        let group_key = r.u32()? as u64;
        let group = (packed & HAS_GROUP != 0).then_some(group_key);
        records.push(StarRecord {
            key,
            name: None,
            position_ly,
            velocity,
            color_index,
            luminosity_solar,
            component_index,
            group,
        });
    }

    let named = r.u32()? as usize;
    for _ in 0..named {
        let index = r.u32()? as usize;
        let len = r.u16()? as usize;
        let name = core::str::from_utf8(r.take(len)?).map_err(|_| ChunkError::BadUtf8)?;
        // An index past the end is a corrupt chunk, not a reason to stop: the stars are all
        // here and only a label is lost.
        if let Some(record) = records.get_mut(index) {
            record.name = Some(name.to_owned());
        }
    }
    Ok((source, records))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChunkError> {
        let end = self.at.checked_add(n).ok_or(ChunkError::Truncated {
            wanted: usize::MAX,
            had: self.bytes.len(),
        })?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(ChunkError::Truncated { wanted: end, had: self.bytes.len() })?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ChunkError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ChunkError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes")))
    }

    fn u32(&mut self) -> Result<u32, ChunkError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn i16(&mut self) -> Result<i16, ChunkError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().expect("2 bytes")))
    }

    fn f32(&mut self) -> Result<f32, ChunkError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }
}

/// A [`StarProvider`] over a decoded chunk.
pub struct ChunkProvider {
    source: String,
    stars: Vec<CatalogueStar>,
    /// Records the chunk held that did not assemble into a star.
    pub skipped: usize,
}

impl ChunkProvider {
    pub fn decode(bytes: &[u8]) -> Result<Self, ChunkError> {
        let (source, records) = decode(bytes)?;
        let (stars, skipped) = assemble_all(&source, &records);
        Ok(ChunkProvider { source, stars, skipped })
    }
}

impl StarProvider for ChunkProvider {
    fn name(&self) -> &str {
        &self.source
    }
    fn stars(&self) -> &[CatalogueStar] {
        &self.stars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Vec<StarRecord> {
        vec![
            StarRecord {
                key: 0,
                name: Some("Sol".into()),
                position_ly: DVec3::ZERO,
                velocity: DVec3::ZERO,
                color_index: 0.656,
                luminosity_solar: 1.0,
                component_index: 1,
                group: None,
            },
            StarRecord {
                key: 32_263,
                name: Some("Sirius".into()),
                position_ly: DVec3::new(-1.6, 6.5, -5.2),
                velocity: DVec3::new(-3.1e3, 1.2e4, 2.0e3),
                color_index: 0.009,
                luminosity_solar: 25.4,
                component_index: 1,
                group: Some(32_263),
            },
            StarRecord {
                key: 32_349,
                name: None,
                position_ly: DVec3::new(-1.6, 6.5, -5.2),
                velocity: DVec3::new(-3.1e3, 1.2e4, 2.0e3),
                color_index: 1.34,
                luminosity_solar: 0.0024,
                component_index: 2,
                group: Some(32_263),
            },
        ]
    }

    #[test]
    fn a_chunk_round_trips() {
        let bytes = encode("hyg-v42", &records()).expect("encodes");
        let (source, back) = decode(&bytes).expect("decodes");
        assert_eq!(source, "hyg-v42");
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].name.as_deref(), Some("Sol"));
        assert_eq!(back[1].name.as_deref(), Some("Sirius"));
        assert_eq!(back[2].name, None, "an unnamed record stays unnamed");
        assert_eq!(back[2].group, Some(32_263));
        assert_eq!(back[2].component_index, 2);
        // f32 storage, so compare at the precision the format claims rather than exactly.
        assert!((back[1].position_ly - records()[1].position_ly).length() < 1e-4);
        assert!((back[1].luminosity_solar - 25.4).abs() < 1e-3);
        assert!((back[1].color_index - 0.009).abs() < 1e-9, "thousandths are exact");
    }

    #[test]
    fn the_provider_assembles_the_same_stars_the_records_do() {
        let bytes = encode("hyg-v42", &records()).unwrap();
        let provider = ChunkProvider::decode(&bytes).unwrap();
        assert_eq!(provider.len(), 3);
        assert_eq!(provider.skipped, 0);
        let sol = provider
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .unwrap();
        let direct = records()[0].assemble("hyg-v42").unwrap();
        assert_eq!(sol.id, direct.id, "the chunk must not change a star's identity");
        assert!((sol.star.teff_k - direct.star.teff_k).abs() < 1.0);
    }

    #[test]
    fn a_truncated_chunk_is_an_error_and_not_a_panic() {
        let bytes = encode("hyg-v42", &records()).unwrap();
        for cut in [0, 4, 8, 20, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode(&bytes[..cut]).is_err(), "{cut} bytes decoded");
        }
    }

    #[test]
    fn a_group_key_of_zero_is_not_the_same_as_no_group() {
        let mut r = records();
        r[0].group = Some(0);
        r[1].group = None;
        let bytes = encode("test", &r).unwrap();
        let (_, back) = decode(&bytes).unwrap();
        assert_eq!(back[0].group, Some(0), "HYG gives the Sun comp_primary = 0");
        assert_eq!(back[1].group, None);
    }

    #[test]
    fn a_color_index_on_the_validity_boundary_survives_the_round_trip() {
        // -0.4 is the low end of BV_VALID and is not representable in f32; the nearest f32
        // falls outside the range. Eleven real HYG stars sit exactly here.
        let mut r = records();
        r[0].color_index = em_spectra::color_index::BV_VALID.0;
        let bytes = encode("test", &r).unwrap();
        let (_, back) = decode(&bytes).unwrap();
        assert!(
            em_spectra::color_index::bv_is_valid(back[0].color_index),
            "{} fell out of range",
            back[0].color_index
        );
        assert!(ChunkProvider::decode(&bytes).unwrap().skipped == 0);
    }

    #[test]
    fn a_foreign_or_future_chunk_is_refused() {
        assert!(matches!(decode(b"not a chunk at all"), Err(ChunkError::NotAChunk)));
        let mut bytes = encode("hyg-v42", &records()).unwrap();
        bytes[6] = 99;
        assert!(matches!(decode(&bytes), Err(ChunkError::UnsupportedVersion(99))));
    }

    #[test]
    fn a_name_index_past_the_end_loses_the_label_and_not_the_stars() {
        let mut bytes = encode("hyg-v42", &records()).unwrap();
        // The name table follows the records; rewrite the first entry's index.
        let table = bytes.len() - (NAME_ENTRY_HEADER * 2 + 3 + 6);
        bytes[table..table + 4].copy_from_slice(&9999u32.to_le_bytes());
        let (_, back) = decode(&bytes).expect("still decodes");
        assert_eq!(back.len(), 3);
    }
}

/// The CSV and the chunk are two routes to one catalogue, and this is what keeps them honest.
///
/// Needs the bundled HYG data, so it is skipped where that is absent rather than failing.
#[cfg(all(test, feature = "hyg"))]
mod equivalence {
    use super::*;
    use crate::sky::hyg::{self, HygProvider};

    const CSV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/catalogs/hygdata_v42.csv");

    #[test]
    fn a_packed_catalogue_is_the_catalogue() {
        let Ok(records) = HygProvider::read_records(CSV) else {
            eprintln!("HYG not present; skipping");
            return;
        };
        let (direct, _) = crate::sky::record::assemble_all(hyg::SOURCE, &records);

        let (bytes, dropped) = pack(hyg::SOURCE, &records).expect("packs");
        let packed = ChunkProvider::decode(&bytes).expect("decodes");

        assert_eq!(packed.name(), hyg::SOURCE);
        assert_eq!(dropped, records.len() - direct.len(), "pack and assemble disagreed");
        // A chunk may never gain a star: `pack` decides with the source's f64 values.
        assert!(packed.len() <= direct.len(), "the chunk gained stars the catalogue rejected");
        // Losing one is possible in principle — a record that assembles in f64 and not after
        // rounding — and this asserts it does not happen with the bundled catalogue.
        assert_eq!(packed.skipped, 0, "{} packed records failed to assemble", packed.skipped);
        assert_eq!(packed.len(), direct.len());

        let mut worst_position = 0.0f64;
        let mut worst_teff = 0.0f64;
        for (a, b) in direct.iter().zip(packed.stars()) {
            // Identity, naming and grouping must survive exactly. These are not measurements.
            assert_eq!(a.id, b.id);
            assert_eq!(a.provenance.name, b.provenance.name);
            assert_eq!(a.component, b.component);
            assert_eq!(a.provenance, b.provenance);

            // The numbers go through f32, so they come back close rather than equal. The
            // tolerance is f32's own: about seven significant digits.
            worst_position =
                worst_position.max((a.position_ly - b.position_ly).length() / a.position_ly.length().max(1.0));
            worst_teff = worst_teff.max((a.star.teff_k - b.star.teff_k).abs() / a.star.teff_k);
        }
        assert!(worst_position < 1e-6, "position drifted by {worst_position:e} relative");
        // Color index is quantised to a thousandth, and temperature follows from it. Near
        // the Sun that is about two kelvin, so the bound is the quantum and not a guess.
        assert!(worst_teff < 2e-3, "temperature drifted by {worst_teff:e} relative");
    }

    #[test]
    fn the_chunk_is_much_smaller_than_the_csv() {
        let Ok(records) = HygProvider::read_records(CSV) else { return };
        let (bytes, _) = pack(hyg::SOURCE, &records).expect("packs");
        let csv = std::fs::metadata(CSV).expect("CSV present").len() as usize;
        // The point of the format. If this ever fails, the format grew a field it should not
        // have, and the browser build pays for it on every first visit.
        assert!(bytes.len() * 5 < csv, "chunk is {} bytes against {csv} of CSV", bytes.len());
    }
}
