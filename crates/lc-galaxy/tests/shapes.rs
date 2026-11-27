//! Every shape kind, through a compiled timeline: its distance against its declared bound, its
//! support against its box, and `Add` against the stars it asked for.

use glam::DVec3;
use lc_galaxy::compiled::OpSpec;
use lc_galaxy::kinds::{Bar, BoolOp, Boolean, Contribution, Ellipsoid, ExponentialDisc, FrameKind, King, LogSpiral, PathKind, PathPoint, Plummer, PopulationKind, PowerHalo, Ring, Sersic, ShapeRef};
use lc_galaxy::{Compiled, FeatureId, FeatureKind, Num, Output, Params, Registry, Target, Timeline, codec, stats};

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
        ("path", adding::<PathKind>().1),
        ("tapering path", one::<PathKind>(|k, frame, pop| {
            k.frame = frame;
            k.points = vec![PathPoint { at_ly: (0.0, 0.0, 0.0), radius_ly: 3.2 }, PathPoint { at_ly: (5.0, 3.0, 1.0), radius_ly: 1.5 }];
            k.contributes = vec![Contribution(Target::Density(pop), OpSpec::Add(Num::Lit(STARS)))];
        })
        .1),
        ("union", boolean(BoolOp::Union).1),
        ("intersection", boolean(BoolOp::Intersect).1),
        ("difference", boolean(BoolOp::Difference).1),
    ]
}

/// A disc, an arm of a spiral and an ellipsoid that crosses both, as construction geometry, and
/// a boolean of the three adding `STARS`.
fn boolean(op: BoolOp) -> (Timeline, Compiled) {
    let mut t = Timeline::new(3);
    let frame = t.push(Box::new(FrameKind { tilt_deg: 20.0, ..FrameKind::new() }), None);
    let pop = t.push(Box::new(PopulationKind { frame, ..PopulationKind::new() }), None);
    let disc = t.push(Box::new(ExponentialDisc { frame, ..ExponentialDisc::new() }), None);
    let arms = t.push(Box::new(LogSpiral { frame, ..LogSpiral::new() }), None);
    let blob = t.push(Box::new(Ellipsoid { frame, center_ly: (20.0, 0.0, 0.0), radii_ly: (15.0, 10.0, 6.0), ..Ellipsoid::new() }), None);
    let of = vec![ShapeRef(Output::only(disc)), ShapeRef(Output { feature: arms, index: 1, of: 2 }), ShapeRef(Output::only(blob))];
    let contributes = vec![Contribution(Target::Density(pop), OpSpec::Add(Num::Lit(STARS)))];
    t.push(Box::new(Boolean { op, blend_ly: Num::Lit(1.5), of, contributes }), None);
    let c = t.compile(t.len(), None);
    assert!(c.problems.is_empty(), "{op:?}: {:?}", c.problems);
    (t, c)
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
        // Not the 64 a boolean integrates its own volume on, so its grid is not checked against
        // itself; and even, so no point lands on a cusp at a center.
        let got: f64 = stats::expected(&c, c.bounds(), 100).iter().map(|(_, n)| n).sum();
        assert!((got / STARS - 1.0).abs() < 0.02, "{kind}: {got} stars in the field for {STARS} asked");
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

/// A file names a spiral's arm with `(id, index, of)` and a lone shape with its bare id, and both
/// write back as they were read.
#[test]
fn a_boolean_reads_and_writes_both_forms_of_reference() {
    let (t, _) = boolean(BoolOp::Intersect);
    let text = codec::write(&t);
    assert!(text.contains("of: [2, (3, 1, 2), 4]"), "{text}");
    assert_eq!(codec::write(&codec::read(&text, &Registry::standard()).unwrap()), text);
}

#[test]
fn a_boolean_of_shapes_that_are_not_there_is_a_problem() {
    let (mut t, _) = boolean(BoolOp::Intersect);
    let id = t.features().last().unwrap().id;
    let cases = [
        // Arm 2 of 3, when the spiral has two.
        vec![ShapeRef(Output::only(FeatureId(2))), ShapeRef(Output { feature: FeatureId(3), index: 1, of: 3 })],
        vec![ShapeRef(Output::only(FeatureId(2)))],
        vec![ShapeRef(Output::only(FeatureId(2))), ShapeRef(Output::only(FeatureId(40)))],
    ];
    for of in cases {
        t.feature_mut(id).unwrap().get_mut::<Boolean>().unwrap().of = of.clone();
        let c = t.compile(t.len(), None);
        assert!(c.problems.contains_key(&id), "{of:?} compiled");
    }
}

#[test]
fn an_intersection_of_shapes_apart_is_a_problem() {
    let mut t = Timeline::new(1);
    let frame = t.push(Box::new(FrameKind::new()), None);
    let a = t.push(Box::new(Ellipsoid { frame, center_ly: (-20.0, 0.0, 0.0), ..Ellipsoid::new() }), None);
    let b = t.push(Box::new(Ellipsoid { frame, center_ly: (20.0, 0.0, 0.0), ..Ellipsoid::new() }), None);
    let of = vec![ShapeRef(Output::only(a)), ShapeRef(Output::only(b))];
    let id = t.push(Box::new(Boolean { of, ..Boolean::new() }), None);
    assert!(t.compile(t.len(), None).problems.contains_key(&id));
}

/// The difference keeps nothing inside the shape it removes, and the intersection nothing
/// outside either shape.
#[test]
fn a_boolean_weighs_only_where_its_operation_keeps() {
    for op in [BoolOp::Intersect, BoolOp::Difference] {
        let (_, c) = boolean(op);
        let shapes: Vec<_> = c.shapes.iter().map(|(k, s)| (k.feature, s.clone())).collect();
        let get = |f: u32| shapes.iter().filter(|(k, _)| k.0 == f).map(|(_, s)| s.clone()).collect::<Vec<_>>();
        let (disc, blob, result) = (get(2).remove(0), get(4).remove(0), get(5).remove(0));
        let arm = get(3).remove(1);
        let b = disc.bounds();
        for i in 0..40_000u64 {
            let p = b.center() + DVec3::new(unit(3 * i), unit(3 * i + 1), unit(3 * i + 2)) * b.size() * 0.5;
            if result.weight(p) == 0.0 {
                continue;
            }
            match op {
                BoolOp::Intersect => assert!(disc.weight(p) > 0.0 && arm.weight(p) > 0.0 && blob.weight(p) > 0.0, "{p}"),
                _ => assert!(disc.weight(p) > 0.0 && arm.distance(p) > 0.0 && blob.distance(p) > 0.0, "{p}"),
            }
        }
    }
}

#[test]
fn a_path_passes_through_its_points_and_two_in_one_place_are_a_problem() {
    let points = [(DVec3::ZERO, 2.0), (DVec3::new(10.0, 4.0, 0.0), 3.0), (DVec3::new(15.0, -6.0, 2.0), 1.0)];
    let nodes = lc_galaxy::kinds::curve(&points);
    for (p, r) in points {
        assert!(nodes.iter().any(|&(q, s)| q.distance(p) < 1e-12 && (s - r).abs() < 1e-12), "{p} is not on the curve");
    }
    let (mut t, _) = adding::<PathKind>();
    let id = t.features().last().unwrap().id;
    let path = t.feature_mut(id).unwrap().get_mut::<PathKind>().unwrap();
    path.points[1].at_ly = path.points[0].at_ly;
    assert!(t.compile(t.len(), None).problems.contains_key(&id));
}
