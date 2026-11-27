use std::collections::BTreeSet;

use glam::DVec3;
use lc_galaxy::compiled::OpSpec;
use lc_galaxy::kinds::{Contribution, FrameKind, Plummer, PopulationKind};
use lc_galaxy::stats;
use lc_galaxy::{Aabb, Compiled, FeatureKind, Num, Registry, Target, Timeline, bake, codec};

fn phase_two() -> (Timeline, Compiled) {
    let t = codec::read(include_str!("../galaxies/phase-2.ron"), &Registry::standard()).unwrap();
    let c = t.compile(t.len(), None);
    (t, c)
}

/// A cluster whose density falls to nothing at its edge, where exclusion radii vary most: the
/// rule's choice of pair radius shows here and not in a uniform box. Twelve seeds put the Poisson
/// scatter of the mean near 1.3%.
#[test]
fn the_count_matches_the_field_where_density_falls() {
    let (mut t, _) = phase_two();
    let (mut got, mut want) = (0.0, 0.0);
    for seed in 0..12 {
        t.seed = seed;
        let c = t.compile(t.len(), None);
        want += stats::expected(&c, c.bounds(), 48).iter().map(|(_, n)| n).sum::<f64>();
        got += bake(&c, t.seed, c.bounds(), 8).unwrap().stars.len() as f64;
    }
    assert!((got / want - 1.0).abs() < 0.04, "baked {got} against {want}");
}

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
fn no_two_stars_are_closer_than_their_pair_radius() {
    let (t, c) = phase_two();
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    // Thinning saw exact positions; a baked star shows its stored one, up to half a step's
    // diagonal away, so two stars may close by twice that.
    let slack = 2.0 * 3f64.sqrt() * lc_galaxy::sample::CELL_LY / 65536.0;
    for (i, a) in b.stars.iter().enumerate() {
        for s in &b.stars[i + 1..] {
            let d = a.position_ly.distance(s.position_ly);
            let r = 0.5 * (a.exclusion_ly + s.exclusion_ly);
            assert!(d >= r - slack, "{d} < {r}");
        }
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
    // Poisson σ of the mean is √(2000/5) = 20, so 5% is five σ.
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
    let chunks = lc_galaxy::publish::chunks(&c, &b, 7).unwrap();
    assert!(chunks.len() > 1, "the ball straddles the origin, so several chunks");
    let decoded: Vec<_> = chunks.iter().map(|(_, ch)| GalaxyChunk::decode(&ch.encode().unwrap()).unwrap()).collect();
    let provider = lc_world::sky::galaxy::GalaxyProvider::new("t".into(), &decoded);
    use lc_galaxy::attributes::State;
    // The loader leaves out neutron stars and black holes, which give the sky nothing.
    let lit: Vec<_> = b
        .stars
        .iter()
        .filter(|s| s.attributes.luminosity_solar > 0.0 && !matches!(s.attributes.state, State::NeutronStar | State::BlackHole))
        .collect();
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

#[test]
fn a_seventeenth_population_is_a_problem() {
    let mut t = Timeline::new(1);
    let frame = t.push(Box::new(FrameKind::new()), None);
    let pops: Vec<_> = (0..17).map(|_| t.push(Box::new(PopulationKind { frame, ..PopulationKind::new() }), None)).collect();
    let c = t.compile(t.len(), None);
    assert_eq!(c.populations.len(), lc_galaxy::kinds::MAX_POPULATIONS);
    assert!(c.problems.contains_key(pops.last().unwrap()));
}

#[test]
fn republishing_removes_chunks_the_manifest_no_longer_names() {
    let out = std::env::temp_dir().join(format!("lc-galaxy-publish-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let (mut t, _) = uniform_ball(50.0, 6.0, 1);
    lc_galaxy::publish::publish(&t, "a", &out, 4).unwrap();
    t.seed = 2;
    let m = lc_galaxy::publish::publish(&t, "a", &out, 4).unwrap();
    let mut on_disk: Vec<String> =
        std::fs::read_dir(out.join("chunks")).unwrap().map(|e| format!("chunks/{}", e.unwrap().file_name().to_string_lossy())).collect();
    let mut listed: Vec<String> = m.chunks.iter().map(|c| c.file.clone()).collect();
    on_disk.sort();
    listed.sort();
    assert_eq!(on_disk, listed);
    let manifests: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".galaxy"))
        .collect();
    assert_eq!(manifests.len(), 2, "{manifests:?}");
    assert!(manifests.contains(&format!("{}.galaxy", m.hash())));
    std::fs::remove_dir_all(&out).unwrap();
}

/// Phase I's stars are closer than their nominal shells reach, so the clamp runs. The shells the
/// publish stored load as computed, rounded down, and do not overlap. A ship beside any star is
/// in that star's system.
#[test]
fn in_phase_one_a_ship_beside_a_star_is_in_its_system() {
    use lc_world::sky::StarProvider;
    use lc_world::sky::galaxy::{GalaxyChunk, GalaxyProvider};
    let (t, c) = phase_one();
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let chunks: Vec<_> = lc_galaxy::publish::chunks(&c, &b, 0).unwrap().iter().map(|(_, ch)| GalaxyChunk::decode(&ch.encode().unwrap()).unwrap()).collect();
    let provider = GalaxyProvider::new("phase-1".into(), &chunks);
    let stars = provider.stars();
    use lc_world::sky::shells::nominal_ly;
    let pairs = || stars.iter().flat_map(|a| stars.iter().filter(move |b| a.id != b.id).map(move |b| (a, b, a.position_ly.distance(b.position_ly))));
    assert!(pairs().any(|(a, b, d)| d < nominal_ly(a.mass_solar) + nominal_ly(b.mass_solar)), "premise: some nominal shells overlap");
    for (a, b, d) in pairs() {
        assert!(a.shell_radius_ly() + b.shell_radius_ly() <= d * (1.0 + 1e-12), "clamped shells overlap");
    }
    let computed: std::collections::HashMap<_, _> = b.stars.iter().map(|s| s.id).zip(lc_galaxy::publish::shells(&b.stars)).collect();
    let step = lc_world::sky::shells::MAX_SHELL_LY / u16::MAX as f64;
    for s in stars {
        let want = computed[&s.id];
        assert!(s.shell_ly <= want && want - s.shell_ly < step, "{} stored against {want} computed", s.shell_ly);
    }
    for s in stars {
        let beside = s.position_ly + DVec3::new(1e-6, 0.0, 0.0);
        assert_eq!(lc_world::system::holding(beside, stars).map(|h| h.id), Some(s.id));
    }
}

#[test]
fn the_imf_and_age_fractions_sum_to_one_and_match_the_draws() {
    use lc_galaxy::attributes::{age_fraction, imf_fraction, mass};
    use lc_galaxy::compiled::{Age, Imf};
    for imf in [Imf::Kroupa, Imf::Salpeter] {
        assert!((imf_fraction(&imf, 0.08, 100.0) - 1.0).abs() < 1e-12);
        // The share below 1 M☉ by fraction agrees with the share the sampler draws there.
        let below = (0..20_000).filter(|i| mass(&imf, (*i as f64 + 0.5) / 20_000.0) < 1.0).count() as f64 / 20_000.0;
        assert!((below - imf_fraction(&imf, 0.08, 1.0)).abs() < 1e-3, "{imf:?}: {below}");
    }
    for age in [Age::Uniform { min_gyr: 1.0, max_gyr: 5.0 }, Age::Burst { gyr: 0.5, spread_gyr: 0.05 }] {
        assert!((age_fraction(&age, 0.0, 13.8) - 1.0).abs() < 1e-9, "{age:?}");
    }
    assert!((age_fraction(&Age::Uniform { min_gyr: 1.0, max_gyr: 5.0 }, 2.0, 3.0) - 0.25).abs() < 1e-12);
}

/// A shape that sets a system field gives the stars it covers that profile, through publishing
/// and loading; stars it does not reach keep the default and load with none.
#[test]
fn system_fields_reach_the_loaded_star() {
    use lc_galaxy::compiled::SystemField;
    use lc_world::sky::StarProvider;
    use lc_world::sky::galaxy::{GalaxyChunk, GalaxyProvider};
    let (mut t, _) = uniform_ball(300.0, 8.0, 3);
    let frame = lc_galaxy::FeatureId(0);
    t.push(
        Box::new(Plummer {
            frame,
            center_ly: (4.0, 0.0, 0.0),
            core_ly: Num::Lit(100.0),
            radius_ly: Num::Lit(4.0),
            edge_ly: Num::Lit(0.0),
            contributes: vec![
                Contribution(Target::System(SystemField::Ocean), OpSpec::Set(Num::Lit(1.0))),
                Contribution(Target::System(SystemField::Solids), OpSpec::Add(Num::Lit(0.3))),
            ],
            ..Plummer::new()
        }),
        None,
    );
    let c = t.compile(t.len(), None);
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let chunks: Vec<_> = lc_galaxy::publish::chunks(&c, &b, 0).unwrap().iter().map(|(_, ch)| GalaxyChunk::decode(&ch.encode().unwrap()).unwrap()).collect();
    let provider = GalaxyProvider::new("t".into(), &chunks);
    let (mut inside, mut outside) = (0, 0);
    for s in provider.stars() {
        let near = s.position_ly.distance(glam::DVec3::new(4.0, 0.0, 0.0)) < 3.9;
        let far = s.position_ly.distance(glam::DVec3::new(4.0, 0.0, 0.0)) > 4.1;
        if near {
            let p = s.system.expect("a star inside has a profile");
            assert_eq!((p.ocean(), p.solids_dex()), (1.0, 0.3));
            inside += 1;
        } else if far {
            assert_eq!(s.system, None);
            outside += 1;
        }
    }
    assert!(inside > 5 && outside > 5, "{inside} inside, {outside} outside");
}
