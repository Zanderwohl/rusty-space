//! Every shape kind, through a compiled timeline: its distance against its declared bound, its
//! support against its box, and `Add` against the stars it asked for.

use glam::DVec3;
use lc_galaxy::compiled::OpSpec;
use lc_galaxy::kinds::{Bar, Contribution, Ellipsoid, ExponentialDisc, FrameKind, King, LogSpiral, Plummer, PopulationKind, PowerHalo, Ring, Sersic};
use lc_galaxy::{Compiled, FeatureId, FeatureKind, Num, Params, Registry, Target, Timeline, codec, stats};

const STARS: f64 = 1000.0;

/// A tilted frame, a population, and `T::new()` adding `STARS` to it.
fn one<T: FeatureKind>(set: impl FnOnce(&mut T, FeatureId, FeatureId)) -> (Timeline, Compiled) {
    let mut t = Timeline::new(3);
    let frame = t.push(Box::new(FrameKind { origin_ly: (5.0, -3.0, 2.0), tilt_deg: 30.0, azimuth_deg: 40.0, ..FrameKind::new() }), None);
    let pop = t.push(Box::new(PopulationKind { frame, ..PopulationKind::new() }), None);
    let mut kind = T::new();
    set(&mut kind, frame, pop);
    t.push(Box::new(kind), None);
    let c = t.compile(t.len(), None);
    assert!(c.problems.is_empty(), "{}: {:?}", T::KIND, c.problems);
    (t, c)
}

fn adding<T: FeatureKind + lc_galaxy::kinds::ShapeKind>() -> (Timeline, Compiled) {
    one::<T>(|k, frame, pop| {
        *k.frame_mut() = frame;
        *k.contributes_mut() = vec![Contribution(Target::Density(pop), OpSpec::Add(Num::Lit(STARS)))];
    })
}

fn every_kind() -> Vec<(&'static str, Compiled)> {
    vec![
        ("plummer", adding::<Plummer>().1),
        ("king", adding::<King>().1),
        ("exponential_disc", adding::<ExponentialDisc>().1),
        ("sersic", adding::<Sersic>().1),
        ("power_halo", adding::<PowerHalo>().1),
        ("ellipsoid", adding::<Ellipsoid>().1),
        ("ring", adding::<Ring>().1),
        ("bar", adding::<Bar>().1),
        ("log_spiral", adding::<LogSpiral>().1),
    ]
}

/// Deterministic points in `[-1, 1]`, so a failure reproduces.
fn unit(i: u64) -> f64 {
    // SplitMix64's finalizer.
    let mut h = i.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// Doc 34's test: the distance changes no faster than the declared bound allows, it is never
/// positive where the weight is, and the support stays inside the shape's box.
#[test]
fn distances_respect_their_bounds_and_supports_their_boxes() {
    for (kind, c) in every_kind() {
        for shape in c.shapes.values() {
            let b = shape.bounds();
            let (center, half) = (b.center(), b.size() * 0.55);
            let at = |i: u64| center + DVec3::new(unit(3 * i), unit(3 * i + 1), unit(3 * i + 2)) * half;
            let mut inside = 0;
            for i in 0..50_000u64 {
                let (p, q) = (at(i), at(i + 1_000_003));
                // Pairs close together as well as far apart: a bound broken locally shows only there.
                let q = if i % 2 == 0 { p + (q - p) * 0.01 } else { q };
                let (dp, dq) = (shape.distance(p), shape.distance(q));
                assert!((dp - dq).abs() <= shape.lipschitz() * p.distance(q) * (1.0 + 1e-9) + 1e-9, "{kind}: {dp} at {p}, {dq} at {q}");
                if shape.weight(p) > 0.0 {
                    inside += 1;
                    assert!(dp <= 1e-9, "{kind}: weight {} where the distance is {dp}", shape.weight(p));
                    assert!(b.contains(p), "{kind}: weight outside the box at {p}");
                }
            }
            assert!(inside > 50, "{kind}: only {inside} samples inside");
        }
    }
}

/// `Add` puts the stars asked for in the field. The midpoint sum is the bake's own expectation,
/// so this checks each kind's volume.
#[test]
fn an_add_puts_the_stars_it_asks_for_into_the_field() {
    for (kind, c) in every_kind() {
        let got: f64 = stats::expected(&c, c.bounds(), 64).iter().map(|(_, n)| n).sum();
        assert!((got / STARS - 1.0).abs() < 0.01, "{kind}: {got} stars in the field for {STARS} asked");
    }
}

#[test]
fn a_spiral_makes_one_shape_per_arm_and_survives_a_round_trip() {
    let (t, c) = one::<LogSpiral>(|k, frame, _| {
        k.frame = frame;
        k.arms = 3;
    });
    assert_eq!(c.shapes.len(), 3);
    assert!(c.shapes.keys().all(|o| o.of == 3));
    let back = codec::read(&codec::write(&t), &Registry::standard()).unwrap();
    assert_eq!(codec::write(&back), codec::write(&t));
}

#[test]
fn shapes_that_would_not_be_finite_are_problems() {
    let mut t = Timeline::new(1);
    let frame = t.push(Box::new(FrameKind::new()), None);
    let bad: Vec<Box<dyn Params>> = vec![
        Box::new(Ring { frame, half_width_ly: Num::Lit(30.0), ..Ring::new() }),
        Box::new(LogSpiral { frame, arms: 0, ..LogSpiral::new() }),
        Box::new(LogSpiral { frame, pitch_deg: Num::Lit(5.0), half_width_ly: Num::Lit(4.0), ..LogSpiral::new() }),
        Box::new(Sersic { frame, index: Num::Lit(0.1), ..Sersic::new() }),
        Box::new(King { frame, tidal_ly: Num::Lit(0.5), ..King::new() }),
        Box::new(ExponentialDisc { frame, radius_ly: Num::Lit(40_000.0), ..ExponentialDisc::new() }),
    ];
    let ids: Vec<FeatureId> = bad.into_iter().map(|p| t.push(p, None)).collect();
    let c = t.compile(t.len(), None);
    for id in ids {
        assert!(c.problems.contains_key(&id), "{id} compiled");
    }
}
