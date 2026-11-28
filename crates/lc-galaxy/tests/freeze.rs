//! Publishing onto a manifest: published chunks stay as they are, and new stars keep clear of
//! them. Doc 34, "Publishing and freezing".

use std::path::{Path, PathBuf};

use lc_galaxy::publish::{Options, publish, publish_with};
use lc_galaxy::{Aabb, Registry, Timeline, codec};
use lc_world::sky::StarProvider;
use lc_world::sky::galaxy::{Manifest, Tier};
use lc_world::sky::shells::MAX_SHELL_LY;

fn phase_three() -> Timeline {
    codec::read(include_str!("../galaxies/phase-3.ron"), &Registry::standard()).unwrap()
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("freeze").join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn read(out: &Path) -> Manifest {
    Manifest::read_from_disk(&out.join("manifest.galaxy")).unwrap().0
}

/// No two shells in the galaxy at `out` overlap, and no star sits inside another's shell.
fn shells_apart(out: &Path) {
    let sky = Manifest::load_from_disk(&out.join("manifest.galaxy")).unwrap();
    for s in sky.stars() {
        for t in sky.within(s.position_ly, 2.0 * MAX_SHELL_LY) {
            if t.id == s.id {
                continue;
            }
            let d = s.position_ly.distance(t.position_ly);
            assert!(d >= s.shell_ly + t.shell_ly - 1e-6, "{:?} and {:?}: {d} ly apart, shells {} and {}", s.id, t.id, s.shell_ly, t.shell_ly);
        }
    }
}

/// Half the galaxy published, the graph changed, the rest published onto it: the first half's
/// files are kept byte for byte, the child names its parent, and the shells still do not overlap.
#[test]
fn a_publish_onto_keeps_what_was_published() {
    let mut t = phase_three();
    let first = dir("half");
    let west = Aabb { min: glam::DVec3::splat(-300.0), max: glam::DVec3::new(-1.0, 300.0, 300.0) };
    publish_with(&t, "p3", &first, 4, &Options { region: Some(west), ..Options::default() }).unwrap();
    let parent = read(&first);
    assert!(!parent.chunks.is_empty() && parent.chunks.iter().all(|c| c.chunk.0 < 0), "{:?}", parent.chunks);

    t.seed += 1;
    let second = dir("rest");
    let onto = first.join("manifest.galaxy");
    let child = publish_with(&t, "p3", &second, 4, &Options { onto: Some(onto.clone()), ..Options::default() }).unwrap();
    assert_eq!(child.parent.as_deref(), Some(lc_world::sky::galaxy::content_hash(&std::fs::read(&onto).unwrap()).as_str()));
    for c in &parent.chunks {
        assert!(child.chunks.contains(c), "{c:?} was not kept");
        assert_eq!(std::fs::read(first.join(&c.file)).unwrap(), std::fs::read(second.join(&c.file)).unwrap());
    }
    assert!(child.chunks.iter().any(|c| c.chunk.0 >= 0), "nothing new was published");
    shells_apart(&second);
}

/// Onto its own publish with the same graph, nothing changes.
#[test]
fn a_publish_onto_itself_adds_nothing() {
    let t = phase_three();
    let first = dir("whole");
    let parent = publish(&t, "p3", &first, 4).unwrap();
    let child = publish_with(&t, "p3", &dir("again"), 4, &Options { onto: Some(first.join("manifest.galaxy")), ..Options::default() }).unwrap();
    assert_eq!(child.chunks, parent.chunks);
}

/// A rebaked tier-B chunk is replaced and its tier A is not; the new tier-B stars keep clear of
/// the stored ones. A chunk the parent never published cannot be rebaked.
#[test]
fn a_rebake_replaces_tier_b_only() {
    let mut t = phase_three();
    let first = dir("before-rebake");
    let parent = publish(&t, "p3", &first, 4).unwrap();
    t.seed += 1;
    let onto = Some(first.join("manifest.galaxy"));
    let second = dir("rebaked");
    let child = publish_with(&t, "p3", &second, 4, &Options { onto: onto.clone(), rebake: vec![[0, 0, 0]], ..Options::default() }).unwrap();
    for c in &parent.chunks {
        let replaced = c.chunk == (0, 0, 0) && c.tier == Tier::B;
        assert_eq!(child.chunks.contains(c), !replaced, "{c:?}");
    }
    let new_b = child.chunks.iter().find(|c| c.chunk == (0, 0, 0) && c.tier == Tier::B).expect("tier B of the chunk baked again");
    assert_ne!(new_b.graph, parent.chunks[0].graph);
    shells_apart(&second);

    let err = publish_with(&t, "p3", &dir("nowhere"), 4, &Options { onto, rebake: vec![[9, 9, 9]], ..Options::default() }).unwrap_err();
    assert!(err.contains("not in"), "{err}");
}
