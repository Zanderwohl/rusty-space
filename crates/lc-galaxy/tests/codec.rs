use lc_galaxy::codec::{self, ReadError};
use lc_galaxy::kinds::{FrameKind, Plummer, PopulationKind, Variable};
use lc_galaxy::{FeatureId, FeatureKind, Num, Registry, Row, Unread};

const PHASE_1: &str = include_str!("../galaxies/phase-1.ron");

fn read(text: &str) -> lc_galaxy::Timeline {
    codec::read(text, &Registry::standard()).expect("reads")
}

#[test]
fn phase_one_reads_every_feature_and_writes_back_byte_for_byte() {
    let t = read(PHASE_1);
    assert_eq!(t.len(), 4);
    assert!(t.rows().iter().all(|r| matches!(r, Row::Feature(_))), "{:#?}", t.rows());
    assert_eq!(codec::write(&t), PHASE_1);
}

#[test]
fn a_number_is_a_literal_or_a_variable() {
    let p = read(PHASE_1);
    let cluster = p.feature(FeatureId(3)).unwrap().get::<Plummer>().unwrap();
    assert_eq!(cluster.core_ly, Num::Lit(3.0));
    let lc_galaxy::kinds::Contribution(_, op) = &cluster.contributes[0];
    assert_eq!(*op, lc_galaxy::compiled::OpSpec::Add(Num::Var("stars".into())));
}

#[test]
fn fields_name_exactly_what_each_kind_serializes() {
    fn check<T: FeatureKind>() {
        let value: ron::Value = ron::from_str(&codec::to_ron(&T::new())).unwrap();
        let ron::Value::Map(m) = value else { panic!("{} params are not a struct", T::KIND) };
        let mut keys: Vec<String> = m.iter().map(|(k, _)| k.clone().into_rust::<String>().unwrap()).collect();
        let mut fields: Vec<String> = T::FIELDS.iter().map(|s| s.to_string()).collect();
        keys.sort();
        fields.sort();
        assert_eq!(keys, fields, "{}", T::KIND);
    }
    check::<Variable>();
    check::<FrameKind>();
    check::<PopulationKind>();
    check::<Plummer>();
}

fn with_line(line: &str) -> lc_galaxy::Timeline {
    read(&format!("{}{line}\n", PHASE_1))
}

#[test]
fn an_unknown_kind_or_a_newer_version_is_kept_verbatim() {
    for line in [
        r#"Feature(id: 4, kind: "log_spiral", version: 1, enabled: true, params: (arms: 2))"#,
        r#"Feature(id: 4, kind: "plummer", version: 9, enabled: true, params: (whatever: 1))"#,
    ] {
        let t = with_line(line);
        assert!(matches!(&t.rows()[4], Row::Unread(Unread::Unknown { id: FeatureId(4), .. })), "{:?}", t.rows()[4]);
        assert!(codec::write(&t).ends_with(&format!("{line}\n")));
    }
}

#[test]
fn every_unsupported_field_is_named() {
    let line = r#"Feature(id: 4, kind: "variable", version: 1, enabled: true, locked: true, params: (name: "y", value: 2.0, unit: "ly", note: "x"))"#;
    let t = with_line(line);
    let Row::Unread(Unread::Unsupported { fields, kind, .. }) = &t.rows()[4] else { panic!("{:?}", t.rows()[4]) };
    assert_eq!(kind, "variable");
    let mut fields = fields.clone();
    fields.sort();
    assert_eq!(fields, ["locked", "note", "unit"]);
    assert!(codec::write(&t).ends_with(&format!("{line}\n")));
}

#[test]
fn a_bad_line_damages_one_feature() {
    for line in [
        r#"Feature(id: 4, kind: "variable", version: 1, params: (name: "y", value: ))"#,
        r#"Feature(id: 4, kind: "variable", version: 1, params: (name: "y"))"#,
        r#"Feature(id: 2, kind: "variable", version: 1, params: (name: "y", value: 1.0))"#,
    ] {
        let t = with_line(line);
        assert_eq!(t.len(), 5);
        assert!(matches!(&t.rows()[4], Row::Unread(Unread::Damaged { .. })), "{:?}", t.rows()[4]);
        assert!(t.rows()[..4].iter().all(|r| matches!(r, Row::Feature(_))));
    }
}

#[test]
fn ids_are_never_reused_even_past_a_stale_next_id() {
    let t = with_line(r#"Feature(id: 9, kind: "variable", version: 1, params: (name: "y", value: 1.0))"#);
    assert_eq!(t.next_id(), 10);
}

#[test]
fn only_the_header_can_fail_the_file() {
    let r = Registry::standard();
    assert_eq!(codec::read("", &r).unwrap_err(), ReadError::NoHeader);
    assert_eq!(codec::read("Timeline(format: 2, next_id: 0)", &r).unwrap_err(), ReadError::NewerFormat(2));
}

/// RON drops the name of `Var("x")` before a deserializer sees it, so any name around one string
/// reads as a variable. Pinned so a change in that is noticed.
#[test]
fn any_name_around_one_string_reads_as_a_variable() {
    for text in [r#"Var("a")"#, r#"Vr("a")"#] {
        let n: Num = codec::options().from_str(text).unwrap();
        assert_eq!(n, Num::Var("a".into()), "{text}");
    }
    assert!(codec::options().from_str::<Num>(r#""a""#).is_err());
}
