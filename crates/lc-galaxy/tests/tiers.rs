//! The two tiers of doc 34's "Two tiers": tier A holds the luminous stars, drawn first on its own
//! grid; tier B the rest, kept clear of tier A.

use lc_galaxy::attributes::L_A;
use lc_galaxy::sample::Tier;
use lc_galaxy::{Registry, bake, codec, stats};

fn phase_three() -> lc_galaxy::Timeline {
    codec::read(include_str!("../galaxies/phase-3.ron"), &Registry::standard()).unwrap()
}

/// Every tier-A star is luminous and says so in its id and record; no tier-B star is; and every
/// id is distinct.
#[test]
fn the_tiers_split_at_the_luminosity_threshold() {
    let t = phase_three();
    let c = t.compile(t.len(), None);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let (a, rest): (Vec<_>, Vec<_>) = b.stars.iter().partition(|s| s.tier == Tier::A);
    assert!(!a.is_empty(), "no tier-A stars");
    for s in &a {
        assert!(s.attributes.luminosity_solar >= L_A && s.attributes.record.tier_a() && s.attributes.record.id() == Some(s.id), "{s:?}");
    }
    assert!(rest.iter().all(|s| s.attributes.luminosity_solar < L_A && !s.attributes.record.tier_a()));
    let mut ids: Vec<_> = b.stars.iter().map(|s| s.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), b.stars.len(), "two stars share an id");
}

/// No tier-B star sits within the pair radius of a tier-A star.
#[test]
fn tier_b_keeps_clear_of_tier_a() {
    let t = phase_three();
    let c = t.compile(t.len(), None);
    let b = bake(&c, t.seed, c.bounds(), 4).unwrap();
    let a: Vec<_> = b.stars.iter().filter(|s| s.tier == Tier::A).collect();
    for s in b.stars.iter().filter(|s| s.tier == Tier::B) {
        for l in &a {
            // Positions are the stored ones, quantized to 7.7 AU; the rule ran on the drawn ones.
            let r = 0.5 * (s.exclusion_ly + l.exclusion_ly);
            assert!(s.position_ly.distance(l.position_ly) >= r - 1e-3, "{:?} within {r} of tier A {:?}", s.id, l.id);
        }
    }
}

/// Over many seeds, tier A has the luminous share of the field's stars.
#[test]
fn tier_a_has_the_luminous_share_of_the_field() {
    let mut t = phase_three();
    let (mut got, mut want) = (0.0, 0.0);
    for seed in 0..16 {
        t.seed = seed;
        let c = t.compile(t.len(), None);
        want += stats::expected_luminous(&c, c.bounds(), 48);
        got += bake(&c, seed, c.bounds(), 4).unwrap().stars.iter().filter(|s| s.tier == Tier::A).count() as f64;
    }
    // About 30 a seed, so the Poisson scatter of the total is near 4%.
    assert!((got / want - 1.0).abs() < 0.15, "baked {got} tier-A stars against {want}");
}
