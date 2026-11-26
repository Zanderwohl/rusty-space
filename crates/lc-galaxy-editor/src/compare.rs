//! What the views show of a bake: one population or shape alone, and how it differs from a pinned
//! earlier bake. See `lightcone/docs/34-galaxy-authoring.md`, "Views".
//!
//! Stars are matched across bakes by id, which is the candidate's cell and index: the same
//! candidate keeps its id through any graph change that still accepts it.

use std::collections::{HashMap, HashSet};

use bevy_egui::egui::Color32;
use glam::DVec3;
use lc_galaxy::{BakedStar, Compiled, FeatureId, StarId};

use crate::doc::Doc;
use crate::palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Solo {
    Population(FeatureId),
    /// Stars where this feature's shape contributes to the field.
    Shape(FeatureId),
}

impl Solo {
    /// Every population, then every feature with a shape that contributes.
    pub fn options(compiled: &Compiled) -> Vec<Solo> {
        let mut out: Vec<Solo> = compiled.populations.keys().map(|id| Solo::Population(*id)).collect();
        for term in &compiled.terms {
            let shape = Solo::Shape(term.source);
            if !out.contains(&shape) {
                out.push(shape);
            }
        }
        out
    }

    pub fn label(self, doc: &Doc) -> String {
        match self {
            Solo::Population(id) => format!("population {}", doc.feature_name(id)),
            Solo::Shape(id) => doc.feature_name(id),
        }
    }

    fn admits(self, population: Option<FeatureId>, position: DVec3, compiled: &Compiled) -> bool {
        match self {
            Solo::Population(id) => population == Some(id),
            Solo::Shape(id) => compiled.contributions(position).iter().any(|c| c.source == id),
        }
    }
}

pub struct Reference {
    pub stars: Vec<BakedStar>,
    by_id: HashMap<StarId, usize>,
    /// Which pin this is, so marks worked out against an earlier one are not reused.
    serial: u64,
}

impl Reference {
    pub fn of(stars: &[BakedStar], serial: u64) -> Self {
        let by_id = stars.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        Self { stars: stars.to_vec(), by_id, serial }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Same,
    /// Kept its id, but its draws, position or system profile moved.
    Changed,
    New,
}

impl Change {
    pub fn color(self) -> Color32 {
        match self {
            Change::Same => palette::DIFF_SAME,
            Change::Changed => palette::DIFF_CHANGED,
            Change::New => palette::DIFF_NEW,
        }
    }
}

/// Per star of the current bake, worked out once per bake, solo and reference.
pub struct Marks {
    key: (u64, Option<Solo>, Option<u64>),
    /// Whether the solo admits each star.
    pub shown: Vec<bool>,
    /// Against the reference; all `Same` without one.
    pub change: Vec<Change>,
    /// Indices into the reference's stars that the current bake lost and the solo admits.
    pub gone: Vec<usize>,
}

impl Marks {
    /// New, changed and gone stars among those shown.
    pub fn counts(&self) -> (usize, usize, usize) {
        let of = |c: Change| self.change.iter().zip(&self.shown).filter(|(k, s)| **s && **k == c).count();
        (of(Change::New), of(Change::Changed), self.gone.len())
    }
}

pub fn marks<'a>(cache: &'a mut Option<Marks>, doc: &Doc, solo: Option<Solo>, reference: Option<&Reference>) -> &'a Marks {
    let key = (doc.generation, solo, reference.map(|r| r.serial));
    if cache.as_ref().map(|m| m.key) != Some(key) {
        *cache = Some(work_out(key, doc, solo, reference));
    }
    cache.as_ref().expect("filled above")
}

fn work_out(key: (u64, Option<Solo>, Option<u64>), doc: &Doc, solo: Option<Solo>, reference: Option<&Reference>) -> Marks {
    let stars = doc.stars();
    let compiled = doc.result.as_ref().map(|r| &r.compiled);
    let admits = |population, position| match (solo, compiled) {
        (Some(solo), Some(c)) => solo.admits(population, position, c),
        _ => true,
    };
    let shown = stars.iter().map(|s| admits(s.attributes.population, s.position_ly)).collect();
    let change = stars
        .iter()
        .map(|s| match reference.and_then(|r| r.by_id.get(&s.id).map(|&i| &r.stars[i])) {
            None if reference.is_some() => Change::New,
            Some(was) if was.attributes.record != s.attributes.record || was.attributes.profile != s.attributes.profile => Change::Changed,
            _ => Change::Same,
        })
        .collect();
    let gone = match reference {
        Some(r) => {
            let now: HashSet<StarId> = stars.iter().map(|s| s.id).collect();
            (0..r.stars.len())
                .filter(|&i| !now.contains(&r.stars[i].id) && admits(r.stars[i].attributes.population, r.stars[i].position_ly))
                .collect()
        }
        None => Vec::new(),
    };
    Marks { key, shown, change, gone }
}
