use glam::{DMat3, DVec3};

use super::*;
use crate::sky::generate::SystemProfile;
use crate::sky::StarProvider;

fn kinematics() -> Kinematics {
    Kinematics {
        origin: DVec3::ZERO,
        axes: DMat3::IDENTITY,
        rotation: Rotation::Flat { speed_km_s: 220.0, core_ly: 3000.0 },
        dispersion_km_s: 10.0,
        dispersion_exponent: 0.5,
    }
}

fn record(cell: [i32; 3], index: u16, at: DVec3, mass: f64) -> GalaxyRecord {
    GalaxyRecord::quantize(cell, index, at, mass, 1.0, -0.3, 0, false)
}

fn chunk() -> GalaxyChunk {
    GalaxyChunk {
        graph: 0xfeed,
        chunk: [-1, -1, -1],
        populations: vec![kinematics()],
        profiles: vec![SystemProfile::default(), SystemProfile::quantize(1.0, 0.75, 0.3, 0.0, -0.5)],
        records: vec![
            record([-1, -1, -1], 3, DVec3::new(-1.0, -2.0, -3.0), 1.0),
            record([-2, -1, -1], 0, DVec3::new(-9.5, -2.0, -3.0), 0.3),
            // A 30 M☉ star at 1 Gyr is a black hole and emits nothing.
            record([-1, -2, -1], 7, DVec3::new(-1.0, -9.0, -3.0), 30.0),
        ],
    }
}

#[test]
fn a_chunk_round_trips() {
    let c = chunk();
    assert_eq!(GalaxyChunk::decode(&c.encode().unwrap()).unwrap(), c);
}

#[test]
fn quantization_keeps_a_star_within_its_steps() {
    let at = DVec3::new(-1.234_567, -2.5, -7.999);
    let r = GalaxyRecord::quantize([-1, -1, -1], 1, at, 1.37, 4.2, -0.33, 2, true);
    assert!(r.position_ly().distance(at) < CELL_LY / 65536.0);
    assert!((r.initial_mass_solar() / 1.37 - 1.0).abs() < 1e-3);
    assert!((r.age_gyr() - 4.2).abs() < 1e-3);
    assert!((r.feh() + 0.34).abs() < 1e-9);
    assert_eq!((r.population(), r.tier_a()), (2, true));
}

#[test]
fn ids_are_built_and_distinct() {
    let a = star_id([0, 0, 0], 1, false).unwrap();
    assert_ne!(a, star_id([0, 0, 0], 2, false).unwrap());
    assert_ne!(a, star_id([0, 0, 0], 1, true).unwrap());
    assert_ne!(a, star_id([0, 0, 1], 1, false).unwrap());
    assert!(star_id([CELL_HALF_RANGE, 0, 0], 0, false).is_none());
}

#[test]
fn a_damaged_chunk_is_an_error_not_a_panic() {
    let bytes = chunk().encode().unwrap();
    for n in 0..bytes.len() {
        assert!(GalaxyChunk::decode(&bytes[..n]).is_err(), "{n} bytes decoded");
    }
    assert_eq!(GalaxyChunk::decode(b"LCSKY\x00\x01\x00"), Err(GalaxyError::NotAChunk));
}

#[test]
fn a_cell_outside_the_chunk_is_refused() {
    let mut c = chunk();
    c.records.push(record([0, 0, 0], 0, DVec3::splat(1.0), 1.0));
    assert!(matches!(c.encode(), Err(GalaxyError::WrongChunk { .. })));
}

#[test]
fn the_manifest_checks_each_chunk_and_skips_dark_remnants() {
    let bytes = chunk().encode().unwrap();
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        name: "test".into(),
        graph: "000000000000feed".into(),
        chunks: vec![ManifestChunk { chunk: (-1, -1, -1), file: "a.lcg".into(), hash: content_hash(&bytes), stars: 3 }],
    };
    let again = Manifest::parse(&manifest.write()).unwrap();
    assert_eq!(again, manifest);
    let provider = again.load(std::slice::from_ref(&bytes)).unwrap();
    assert_eq!((provider.len(), provider.skipped), (2, 1));
    assert!(provider.stars().iter().all(|s| s.provenance.source == SOURCE && s.provenance.name.is_none()));

    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(matches!(again.load(&[damaged]), Err(GalaxyError::HashMismatch { .. })));
}

#[test]
fn a_region_query_matches_a_scan() {
    let provider = GalaxyProvider::new("test".into(), &[chunk()]);
    let center = DVec3::new(-2.0, -2.0, -3.0);
    for radius in [0.5, 2.0, 8.0, 100.0] {
        let mut indexed: Vec<_> = provider.within(center, radius).iter().map(|s| s.id).collect();
        let mut scanned: Vec<_> =
            provider.stars().iter().filter(|s| s.position_ly.distance(center) <= radius).map(|s| s.id).collect();
        indexed.sort();
        scanned.sort();
        assert_eq!(indexed, scanned, "radius {radius}");
    }
}

#[test]
fn a_record_whose_cell_leaves_its_chunk_is_refused() {
    let c = chunk();
    let mut bytes = c.encode().unwrap();
    // The first record's cell-within-chunk field sits right after the record count.
    let header = bytes.len() - c.records.len() * 19;
    bytes[header..header + 2].copy_from_slice(&0x8000u16.to_le_bytes());
    assert_eq!(GalaxyChunk::decode(&bytes), Err(GalaxyError::BadRecord(0)));
}

#[test]
fn a_manifest_names_only_files_beside_it() {
    let entry = |file: &str| ManifestChunk { chunk: (0, 0, 0), file: file.into(), hash: String::new(), stars: 0 };
    assert!(entry("chunks/a.lcg").checked_file().is_ok());
    for bad in ["../a.lcg", "chunks/../../a.lcg", "/etc/hosts", "./a.lcg", ""] {
        assert!(entry(bad).checked_file().is_err(), "{bad}");
    }
}

/// A record's profile index reaches the loaded star's system profile; the default stays `None`.
#[test]
fn a_record_carries_its_system_profile() {
    let mut c = chunk();
    c.records[0].profile = 1;
    let decoded = GalaxyChunk::decode(&c.encode().unwrap()).unwrap();
    let (stars, _) = decoded.stars();
    let id0 = c.records[0].id().unwrap();
    for s in &stars {
        let want = if s.id == id0 { Some(c.profiles[1]) } else { None };
        assert_eq!(s.system, want);
    }
    c.records[0].profile = 2;
    assert!(GalaxyChunk::decode(&c.encode().unwrap()).is_err(), "an index past the table is refused");
}
