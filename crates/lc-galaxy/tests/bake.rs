use std::collections::BTreeSet;

use glam::DVec3;
use lc_galaxy::compiled::OpSpec;
use lc_galaxy::kinds::{Contribution, FrameKind, Plummer, PopulationKind};
use lc_galaxy::stats;
use lc_galaxy::{Aabb, Compiled, FeatureKind, Num, Registry, Target, Timeline, bake, codec};

fn phase_one() -> (Timeline, Compiled) {
    let t = codec::read(include_str!("../galaxies/phase-1.ron"), &Registry::standard()).unwrap();
    let c = t.compile(t.len(), None);
    (t, c)
}

/// One population in a Plummer sphere whose core is far wider than its radius, so its density
/// is uniform to within a few percent.
fn uniform_ball(stars: f64, radius_ly: f64, seed: u64) -> (Timeline, Compiled) {
    let mut t = Timeline::new(seed);
    let frame = t.push(Box::new(FrameKind::new()), None);
    let pop = t.push(Box::new(PopulationKind { frame, ..PopulationKind::new() }), None);
    t.push(
        Box::new(Plummer {
            frame,
            core_ly: Num::Lit(radius_ly * 50.0),
            radius_ly: Num::Lit(radius_ly),
            edge_ly: Num::Lit(0.0),
            contributes: vec![Contribution(Target::Density(pop), OpSpec::Add(Num::Lit(stars)))],
            ..Plummer::new()
        }),
        None,
    );
    let c = t.compile(t.len(), None);
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    (t, c)
}

#[test]
fn phase_one_is_about_ten_stars_at_its_seed() {
    let (t, c) = phase_one();
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let expected: f64 = stats::expected(&c, c.bounds(), 48).iter().map(|(_, n)| n).sum();
    assert!((expected - 10.0).abs() < 0.2, "the field promises {expected}");
    assert!((5..=16).contains(&b.stars.len()), "{} stars", b.stars.len());
}

#[test]
fn a_bake_does_not_depend_on_thread_count() {
    let (t, c) = uniform_ball(500.0, 8.6, 3);
    let one = bake(&c, t.seed, c.bounds(), 1).unwrap();
    let many = bake(&c, t.seed, c.bounds(), 7).unwrap();
    let ids = |b: &lc_galaxy::Bake| b.stars.iter().map(|s| (s.id, s.position_ly.to_array().map(f64::to_bits))).collect::<Vec<_>>();
    assert_eq!(ids(&one), ids(&many));
}

#[test]
fn a_region_bakes_the_stars_a_whole_bake_puts_there() {
    let (t, c) = uniform_ball(500.0, 8.6, 5);
    let whole = bake(&c, t.seed, c.bounds(), 4).unwrap();
    // One octant, cut along cell faces so the region is a whole number of cells.
    let region = Aabb { min: DVec3::ZERO, max: DVec3::splat(15.9) };
    let part = bake(&c, t.seed, region, 4).unwrap();
    let in_part: BTreeSet<_> = part.stars.iter().map(|s| s.id).collect();
    let expected: BTreeSet<_> = whole.stars.iter().filter(|s| s.position_ly.cmpge(DVec3::ZERO).all()).map(|s| s.id).collect();
    assert!(!expected.is_empty());
    assert_eq!(in_part, expected);
}

#[test]
fn no_two_stars_are_closer_than_their_exclusion() {
    let (t, c) = uniform_ball(500.0, 8.6, 7);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let nn = stats::nearest_neighbors(&b.stars);
    for (s, d) in b.stars.iter().zip(&nn) {
        assert!(*d >= s.exclusion_ly * (1.0 - 1e-9), "{d} < {}", s.exclusion_ly);
    }
    let ids: BTreeSet<_> = b.stars.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), b.stars.len());
}

/// The surviving count tracks the field's promise: the thinning formula is exact for uniform
/// density, so this holds to Poisson scatter over several seeds.
#[test]
fn the_count_matches_the_field() {
    let target = 2000.0;
    let total: usize = (0..5)
        .map(|seed| {
            let (t, c) = uniform_ball(target, 14.0, seed);
            bake(&c, t.seed, c.bounds(), 8).unwrap().stars.len()
        })
        .sum();
    let mean = total as f64 / 5.0;
    // Poisson σ of the mean is √(2000/5) = 20; allow the few percent `r(x)` grading costs.
    assert!((mean - target).abs() < 0.05 * target, "mean {mean} against {target}");
}

#[test]
fn an_edit_moves_only_the_stars_it_touches() {
    let (t, c) = uniform_ball(500.0, 8.6, 11);
    let before: BTreeSet<_> = bake(&c, t.seed, c.bounds(), 4).unwrap().stars.iter().map(|s| s.id).collect();
    let (t2, c2) = uniform_ball(520.0, 8.6, 11);
    let after: BTreeSet<_> = bake(&c2, t2.seed, c2.bounds(), 4).unwrap().stars.iter().map(|s| s.id).collect();
    let kept = before.intersection(&after).count();
    assert!(kept as f64 > 0.7 * before.len() as f64, "kept {kept} of {}", before.len());
}

#[test]
fn a_density_set_without_any_add_still_bakes() {
    let mut t = Timeline::new(1);
    let frame = t.push(Box::new(FrameKind::new()), None);
    let pop = t.push(Box::new(PopulationKind { frame, ..PopulationKind::new() }), None);
    t.push(
        Box::new(Plummer {
            frame,
            core_ly: Num::Lit(100.0),
            radius_ly: Num::Lit(6.0),
            edge_ly: Num::Lit(0.0),
            contributes: vec![Contribution(Target::Density(pop), OpSpec::Set(Num::Lit(0.1)))],
            ..Plummer::new()
        }),
        None,
    );
    let c = t.compile(t.len(), None);
    assert!(!c.bounds().is_empty());
    assert!(!bake(&c, t.seed, c.bounds(), 2).unwrap().stars.is_empty());
}

#[test]
fn features_outside_the_box_are_problems() {
    let mut t = Timeline::new(1);
    let frame = t.push(Box::new(FrameKind { origin_ly: (70_000.0, 0.0, 0.0), ..FrameKind::new() }), None);
    let c = t.compile(t.len(), None);
    assert!(c.problems.contains_key(&frame));
}

/// A published chunk loads back into the stars the bake made: same ids, positions within a
/// quantization step, and the same luminosity, since both ends derive it from the same record.
#[test]
fn a_published_galaxy_loads_as_baked() {
    use lc_world::sky::StarProvider;
    use lc_world::sky::galaxy::GalaxyChunk;
    let (t, c) = uniform_ball(300.0, 8.0, 13);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let chunks = lc_galaxy::publish::chunks(&c, &b, 7);
    assert!(chunks.len() > 1, "the ball straddles the origin, so several chunks");
    let decoded: Vec<_> = chunks.iter().map(|ch| GalaxyChunk::decode(&ch.encode().unwrap()).unwrap()).collect();
    let provider = lc_world::sky::galaxy::GalaxyProvider::new("t".into(), &decoded);
    let lit: Vec<_> = b.stars.iter().filter(|s| s.attributes.luminosity_solar > 0.0).collect();
    assert_eq!(provider.len(), lit.len());
    for s in lit {
        let loaded = provider.get(s.id).expect("every lit star loads");
        assert!(loaded.position_ly.distance(s.position_ly) < 1e-3);
        assert_eq!(loaded.luminosity_solar, s.attributes.luminosity_solar);
        assert!((loaded.velocity / 1000.0 - s.attributes.velocity_km_s).length() < 1e-6);
    }
    for ch in &decoded {
        let (stars, _) = ch.stars();
        assert!(stars.windows(2).all(|w| w[0].luminosity_solar >= w[1].luminosity_solar));
    }
}
