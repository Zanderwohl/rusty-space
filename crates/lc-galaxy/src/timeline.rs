//! The ordered feature list, the way a parametric CAD model's history is.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use crate::compiled::Compiled;
use crate::feature::{Feature, FeatureId, Params, Problem};
use crate::kinds::Variable;

/// A line this build cannot use. Kept verbatim, written back unchanged, never evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum Unread {
    /// The kind is not registered, or its version is newer than this build's.
    Unknown { id: FeatureId, kind: String, version: u32, line: String },
    /// The kind is known, but the line carries fields this build does not have.
    Unsupported { id: FeatureId, kind: String, name: Option<String>, fields: Vec<String>, line: String },
    /// Not readable RON, not the kind's shape, or a repeated id.
    Damaged { id: Option<FeatureId>, error: String, line: String },
}

impl Unread {
    pub fn line(&self) -> &str {
        match self {
            Unread::Unknown { line, .. } | Unread::Unsupported { line, .. } | Unread::Damaged { line, .. } => line,
        }
    }

    pub fn id(&self) -> Option<FeatureId> {
        match self {
            Unread::Unknown { id, .. } | Unread::Unsupported { id, .. } => Some(*id),
            Unread::Damaged { id, .. } => *id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Feature(Feature),
    Unread(Unread),
}

impl Row {
    pub fn id(&self) -> Option<FeatureId> {
        match self {
            Row::Feature(f) => Some(f.id),
            Row::Unread(u) => u.id(),
        }
    }

    pub fn feature(&self) -> Option<&Feature> {
        match self {
            Row::Feature(f) => Some(f),
            Row::Unread(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    /// Seeds every draw of the bake. Fixed across edits, so changing a parameter moves the
    /// stars it affects rather than reshuffling all of them.
    pub seed: u64,
    rows: Vec<Row>,
    next_id: u32,
}

impl Timeline {
    pub fn new(seed: u64) -> Self {
        Self { seed, rows: Vec::new(), next_id: 0 }
    }

    pub(crate) fn from_parts(seed: u64, rows: Vec<Row>, next_id: u32) -> Self {
        Self { seed, rows, next_id }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    pub fn index_of(&self, id: FeatureId) -> Option<usize> {
        self.rows.iter().position(|r| r.id() == Some(id))
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.rows.iter().filter_map(Row::feature).find(|f| f.id == id)
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> Option<&mut Feature> {
        self.rows.iter_mut().find_map(|r| match r {
            Row::Feature(f) if f.id == id => Some(f),
            _ => None,
        })
    }

    pub fn features(&self) -> impl Iterator<Item = &Feature> {
        self.rows.iter().filter_map(Row::feature)
    }

    /// Inserts a new feature at `at` and returns its id.
    pub fn insert(&mut self, at: usize, params: Box<dyn Params>, name: Option<String>) -> FeatureId {
        let id = FeatureId(self.next_id);
        self.next_id += 1;
        let at = at.min(self.rows.len());
        self.rows.insert(at, Row::Feature(Feature { id, enabled: true, name, group: None, params }));
        id
    }

    pub fn push(&mut self, params: Box<dyn Params>, name: Option<String>) -> FeatureId {
        self.insert(self.rows.len(), params, name)
    }

    pub fn remove(&mut self, index: usize) -> Row {
        self.rows.remove(index)
    }

    /// Replaces a row, as when a designer fixes an unread line. The id must stay the row's own
    /// or be new; a row that would repeat another's id is refused.
    pub fn replace(&mut self, index: usize, row: Row) -> Result<(), String> {
        if let Some(id) = row.id()
            && self.rows.iter().enumerate().any(|(i, r)| i != index && r.id() == Some(id))
        {
            return Err(format!("id {} is already taken", id.0));
        }
        if let Some(id) = row.id() {
            self.next_id = self.next_id.max(id.0 + 1);
        }
        self.rows[index] = row;
        Ok(())
    }

    /// What a row reads: its declared references, and the variable features its names
    /// resolve to.
    pub fn reads(&self, index: usize) -> Vec<FeatureId> {
        let Some(f) = self.rows[index].feature() else { return Vec::new() };
        let mut out = f.params.depends_on();
        for name in f.params.variables() {
            // As `compile` resolves it: the first enabled definition wins and later ones are
            // problems. A suppressed one counts only when none is enabled, so reading it is
            // still reported as reading something suppressed.
            let defines = |g: &&Feature| g.get::<Variable>().is_some_and(|v| v.name == name);
            let earlier = || self.rows[..index].iter().filter_map(Row::feature);
            if let Some(v) = earlier().filter(defines).find(|g| g.enabled).or_else(|| earlier().find(defines)) {
                out.push(v.id);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn dependants(&self, id: FeatureId) -> Vec<FeatureId> {
        (0..self.rows.len()).filter(|&i| self.reads(i).contains(&id)).filter_map(|i| self.rows[i].id()).collect()
    }

    /// Where the row at `index` may move to: after everything it reads, and before everything
    /// that reads it.
    pub fn legal_range(&self, index: usize) -> RangeInclusive<usize> {
        let id = self.rows[index].id();
        let without = |i: usize| if i < index { i } else { i - 1 };
        let low = self.reads(index).iter().filter_map(|d| self.index_of(*d)).map(|i| without(i) + 1).max().unwrap_or(0);
        let high = id
            .map(|id| self.dependants(id))
            .unwrap_or_default()
            .iter()
            .filter_map(|d| self.index_of(*d))
            .map(without)
            .fold(self.rows.len() - 1, usize::min);
        low..=high.max(low)
    }

    pub fn move_row(&mut self, from: usize, to: usize) -> Result<(), String> {
        if !self.legal_range(from).contains(&to) {
            return Err("that would put a feature before something it reads".into());
        }
        let row = self.rows.remove(from);
        self.rows.insert(to, row);
        Ok(())
    }

    /// Evaluates the first `cut` rows. A feature that fails becomes a [`Problem`] and the rest
    /// still evaluate. With `held`, a failed feature keeps the outputs it had there, so
    /// breaking one feature does not blank the view.
    pub fn compile(&self, cut: usize, held: Option<&Compiled>) -> Compiled {
        let mut out = Compiled::default();
        let mut suppressed = BTreeSet::new();
        for (index, row) in self.rows.iter().enumerate().take(cut) {
            let Row::Feature(f) = row else { continue };
            if !f.enabled {
                suppressed.insert(f.id);
                continue;
            }
            if let Some(off) = self.reads(index).into_iter().find(|d| suppressed.contains(d)) {
                out.problems.insert(f.id, Problem::Suppressed(off));
                continue;
            }
            if let Err(problem) = f.params.evaluate(f.id, &mut out) {
                out.problems.insert(f.id, problem);
                if let Some(held) = held
                    && hold(held, f.id, &mut out)
                {
                    out.held.insert(f.id);
                }
            }
        }
        out
    }
}

/// Copies what `id` produced in `from` into `out`. Whether anything was there to copy.
fn hold(from: &Compiled, id: FeatureId, out: &mut Compiled) -> bool {
    let mut any = false;
    if let Some(f) = from.frames.get(&id) {
        out.frames.insert(id, f.clone());
        any = true;
    }
    if let Some(p) = from.populations.get(&id) {
        out.populations.insert(id, p.clone());
        any = true;
    }
    for (k, s) in from.shapes.iter().filter(|(k, _)| k.feature == id) {
        out.shapes.insert(*k, s.clone());
        any = true;
    }
    for t in from.terms.iter().filter(|t| t.source == id) {
        out.terms.push(t.clone());
        any = true;
    }
    any
}
