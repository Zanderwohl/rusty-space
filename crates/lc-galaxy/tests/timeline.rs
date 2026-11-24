use lc_galaxy::kinds::{Plummer, Variable};
use lc_galaxy::{FeatureId, Problem, Registry, codec};

fn phase_one() -> lc_galaxy::Timeline {
    codec::read(include_str!("../galaxies/phase-1.ron"), &Registry::standard()).unwrap()
}

#[test]
fn phase_one_compiles_clean() {
    let t = phase_one();
    let c = t.compile(t.len(), None);
    assert!(c.problems.is_empty(), "{:?}", c.problems);
    assert_eq!(c.frames.len(), 1);
    assert_eq!(c.populations.len(), 1);
    assert_eq!(c.terms.len(), 1);
}

#[test]
fn a_feature_moves_only_between_what_it_reads_and_what_reads_it() {
    let t = phase_one();
    // The variable (2) is read by the cluster (3) and reads nothing.
    assert_eq!(t.legal_range(2), 0..=2);
    // The cluster reads the frame, the population and the variable.
    assert_eq!(t.legal_range(3), 3..=3);
    let mut moved = t.clone();
    assert!(moved.move_row(2, 0).is_ok());
    assert!(moved.move_row(0, 3).is_err());
}

#[test]
fn reading_a_suppressed_feature_is_a_problem_that_holds_nothing() {
    let t = phase_one();
    let held = t.compile(t.len(), None);
    let mut off = t.clone();
    off.feature_mut(FeatureId(1)).unwrap().enabled = false;
    let c = off.compile(off.len(), Some(&held));
    assert_eq!(c.problems.get(&FeatureId(3)), Some(&Problem::Suppressed(FeatureId(1))));
    assert!(c.held.is_empty());
    assert!(c.terms.is_empty());
}

#[test]
fn a_broken_feature_keeps_its_last_good_output() {
    let t = phase_one();
    let held = t.compile(t.len(), None);
    let mut broken = t.clone();
    broken.feature_mut(FeatureId(3)).unwrap().get_mut::<Plummer>().unwrap().radius_ly = (-1.0).into();
    let c = broken.compile(broken.len(), Some(&held));
    assert!(matches!(c.problems.get(&FeatureId(3)), Some(Problem::Invalid(_))));
    assert!(c.held.contains(&FeatureId(3)));
    assert_eq!(c.terms.len(), 1);
    // Without anything to hold, it contributes nothing.
    assert!(broken.compile(broken.len(), None).terms.is_empty());
}

#[test]
fn rollback_evaluates_only_what_comes_before_the_cut() {
    let t = phase_one();
    let c = t.compile(2, None);
    assert!(c.terms.is_empty() && c.problems.is_empty());
    assert_eq!(c.populations.len(), 1);
}

#[test]
fn a_renamed_variable_leaves_its_reader_with_a_problem() {
    let mut t = phase_one();
    t.feature_mut(FeatureId(2)).unwrap().get_mut::<Variable>().unwrap().name = "count".into();
    let c = t.compile(t.len(), None);
    assert_eq!(c.problems.get(&FeatureId(3)), Some(&Problem::UnknownVariable("stars".into())));
}

#[test]
fn a_reader_depends_on_the_variable_compile_uses() {
    let mut t = phase_one();
    // A second `stars` after the first: compile keeps the first and marks the second Invalid.
    t.insert(3, Box::new(Variable { name: "stars".into(), value: 99.0 }), None);
    let reader = t.len() - 1;
    assert!(t.reads(reader).contains(&FeatureId(2)));
    assert!(!t.reads(reader).contains(&FeatureId(4)));
}
