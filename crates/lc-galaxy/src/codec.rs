//! The timeline file: a header line, then one feature per line of RON.
//!
//! A line this build cannot use is kept verbatim as an [`Unread`] row and written back
//! unchanged, so one bad line costs one feature and a newer build's file survives an older
//! build's save. Only a missing header or a newer file format fails the whole file.

use std::collections::BTreeSet;
use std::fmt;

use ron::extensions::Extensions;
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

use crate::feature::{Feature, FeatureId, Registry};
use crate::timeline::{Row, Timeline, Unread};

pub const FORMAT: u32 = 1;

/// The fields of a feature line itself, as against its params.
const LINE_FIELDS: &[&str] = &["id", "kind", "version", "enabled", "name", "group", "params"];

pub fn options() -> ron::Options {
    ron::Options::default().with_default_extension(Extensions::IMPLICIT_SOME)
}

/// Compact RON with a space after each `,` and `:`, so a line reads as it would be typed.
pub fn to_ron<T: Serialize + ?Sized>(value: &T) -> String {
    let compact = options().to_string(value).expect("params serialize to RON");
    let mut out = String::with_capacity(compact.len() + compact.len() / 4);
    let (mut in_string, mut escaped) = (false, false);
    for c in compact.chars() {
        out.push(c);
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' || c == ':' {
            out.push(' ');
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadError {
    NoHeader,
    Header(String),
    NewerFormat(u32),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::NoHeader => f.write_str("the file has no Timeline(...) header"),
            ReadError::Header(e) => write!(f, "the header does not read: {e}"),
            ReadError::NewerFormat(v) => write!(f, "format {v} is newer than this build's {FORMAT}"),
        }
    }
}

impl std::error::Error for ReadError {}

#[derive(Deserialize)]
#[serde(rename = "Timeline")]
struct Header {
    format: u32,
    next_id: u32,
    #[serde(default)]
    seed: u64,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename = "Feature")]
struct Head {
    id: u32,
    kind: String,
    version: u32,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    group: Option<u32>,
    params: Box<RawValue>,
}

pub fn read(text: &str, registry: &Registry) -> Result<Timeline, ReadError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or(ReadError::NoHeader)?;
    let header: Header = options().from_str(header).map_err(|e| ReadError::Header(e.to_string()))?;
    if header.format > FORMAT {
        return Err(ReadError::NewerFormat(header.format));
    }
    let mut rows = Vec::new();
    let mut taken = BTreeSet::new();
    for line in lines {
        let row = read_row(line, registry);
        let row = match row.id() {
            Some(id) if !taken.insert(id) => {
                Row::Unread(Unread::Damaged { id: None, error: format!("repeats id {}", id.0), line: line.to_string() })
            }
            _ => row,
        };
        rows.push(row);
    }
    // A hand-added line may carry an id at or past `next_id`; ids are never reused.
    let next_id = taken.iter().map(|id| id.0 + 1).max().unwrap_or(0).max(header.next_id);
    Ok(Timeline::from_parts(header.seed, rows, next_id))
}

/// One feature line, read on its own: also how the editor re-reads a line a designer fixed.
pub fn read_row(line: &str, registry: &Registry) -> Row {
    let damaged = |id: Option<u32>, error: String| {
        Row::Unread(Unread::Damaged { id: id.map(FeatureId), error, line: line.to_string() })
    };
    let value: ron::Value = match options().from_str(line) {
        Ok(v) => v,
        Err(e) => return damaged(None, e.to_string()),
    };
    let head: Head = match options().from_str(line) {
        Ok(h) => h,
        Err(e) => return damaged(id_in(&value), e.to_string()),
    };
    let id = FeatureId(head.id);
    let Some(kind) = registry.get(&head.kind).filter(|k| head.version <= k.version) else {
        return Row::Unread(Unread::Unknown { id, kind: head.kind, version: head.version, line: line.to_string() });
    };
    let mut unsupported: Vec<String> = keys(&value).into_iter().filter(|k| !LINE_FIELDS.contains(&k.as_str())).collect();
    // An older version's fields are its migration's business.
    if head.version == kind.version {
        let params: ron::Value = match options().from_str(head.params.get_ron()) {
            Ok(v) => v,
            Err(e) => return damaged(Some(head.id), e.to_string()),
        };
        unsupported.extend(keys(&params).into_iter().filter(|k| !kind.fields.contains(&k.as_str())));
    }
    if !unsupported.is_empty() {
        return Row::Unread(Unread::Unsupported {
            id,
            kind: head.kind,
            name: head.name,
            fields: unsupported,
            line: line.to_string(),
        });
    }
    match (kind.read)(head.version, head.params.get_ron()) {
        Ok(params) => Row::Feature(Feature {
            id,
            enabled: head.enabled,
            name: head.name,
            group: head.group.map(FeatureId),
            params,
        }),
        Err(e) => damaged(Some(head.id), e),
    }
}

fn keys(value: &ron::Value) -> Vec<String> {
    match value {
        ron::Value::Map(m) => m
            .iter()
            .filter_map(|(k, _)| match k {
                ron::Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn id_in(value: &ron::Value) -> Option<u32> {
    let ron::Value::Map(m) = value else { return None };
    m.iter().find_map(|(k, v)| match (k, v) {
        (ron::Value::String(k), ron::Value::Number(n)) if k == "id" => n.into_f64().to_string().parse().ok(),
        _ => None,
    })
}

pub fn write(timeline: &Timeline) -> String {
    let mut out = format!("Timeline(format: {FORMAT}, next_id: {}, seed: {})\n", timeline.next_id(), timeline.seed);
    for row in timeline.rows() {
        out.push_str(&write_row(row));
        out.push('\n');
    }
    out
}

pub fn write_row(row: &Row) -> String {
    match row {
        Row::Unread(u) => u.line().to_string(),
        Row::Feature(f) => {
            let mut line = format!(
                "Feature(id: {}, kind: \"{}\", version: {}, enabled: {}",
                f.id.0,
                f.params.kind(),
                f.params.version(),
                f.enabled
            );
            if let Some(name) = &f.name {
                line.push_str(&format!(", name: {}", to_ron(name)));
            }
            if let Some(group) = f.group {
                line.push_str(&format!(", group: {}", group.0));
            }
            line.push_str(&format!(", params: {})", f.params.to_ron()));
            line
        }
    }
}
