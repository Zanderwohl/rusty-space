//! The open document: the timeline, its history, and what it compiled and baked to.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use lc_galaxy::sample::default_threads;
use lc_galaxy::{Bake, BakedStar, Compiled, FeatureId, Registry, Row, StarId, Timeline, codec, stats};

/// Above this many expected stars the editor stops baking on every edit. Phase III's spiral
/// is about 10 000.
pub const LIVE_BAKE_BUDGET: f64 = 100_000.0;

/// Above this many grid cells the editor stops baking on every edit too: a large, sparse field
/// can be under the star budget and still ask for billions of cells.
pub const LIVE_CELL_BUDGET: f64 = 200_000.0;

/// Points per axis for the field's expected count.
const EXPECTED_RESOLUTION: usize = 32;

/// While a drag is changing the timeline, rebuild at most this often.
const DRAG_REBUILD_INTERVAL: Duration = Duration::from_millis(150);

pub struct Result {
    pub compiled: Compiled,
    pub bake: std::result::Result<Bake, String>,
    pub expected: Vec<(FeatureId, f64)>,
    pub nearest: Vec<f64>,
    pub took: Duration,
}

#[derive(Resource)]
pub struct Doc {
    pub registry: Registry,
    pub timeline: Timeline,
    pub path: Option<PathBuf>,
    saved: Timeline,
    undo: Vec<Timeline>,
    redo: Vec<Timeline>,
    /// The last state an undo returns to; edits since are one step, however many frames a
    /// drag lasted.
    committed: Timeline,
    pub selected: Option<usize>,
    /// Rows evaluated; `None` is all of them. View state: neither saved nor undone.
    pub cut: Option<usize>,
    pub picked: Option<StarId>,
    pub notice: Option<String>,
    pub result: Option<Result>,
    built_from: Option<(Timeline, usize)>,
    built_at: Option<Instant>,
}

impl Doc {
    pub fn new(timeline: Timeline, path: Option<PathBuf>) -> Self {
        Self {
            registry: Registry::standard(),
            saved: timeline.clone(),
            committed: timeline.clone(),
            timeline,
            path,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            cut: None,
            picked: None,
            notice: None,
            result: None,
            built_from: None,
            built_at: None,
        }
    }

    pub fn open(path: PathBuf) -> std::result::Result<Self, String> {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let timeline = codec::read(&text, &Registry::standard()).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut doc = Self::new(timeline, Some(path));
        let unread = doc.timeline.rows().iter().filter(|r| matches!(r, Row::Unread(_))).count();
        if unread > 0 {
            doc.notice = Some(format!("{unread} line(s) could not be read and are kept as written"));
        }
        Ok(doc)
    }

    pub fn save(&mut self) -> std::result::Result<(), String> {
        let Some(path) = &self.path else { return Err("no path to save to".into()) };
        std::fs::write(path, codec::write(&self.timeline)).map_err(|e| format!("{}: {e}", path.display()))?;
        self.saved = self.timeline.clone();
        self.notice = Some(format!("saved {}", path.display()));
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.timeline != self.saved
    }

    pub fn cut(&self) -> usize {
        self.cut.unwrap_or(self.timeline.len()).min(self.timeline.len())
    }

    /// Called once a frame when no widget is being dragged or typed into.
    pub fn commit(&mut self) {
        if self.timeline != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, self.timeline.clone()));
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.timeline != self.committed
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.commit();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.timeline, prev));
            self.committed = self.timeline.clone();
            self.clamp_selection();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.timeline, next));
            self.committed = self.timeline.clone();
            self.clamp_selection();
        }
    }

    fn clamp_selection(&mut self) {
        if self.selected.is_some_and(|i| i >= self.timeline.len()) {
            self.selected = None;
        }
    }

    /// Recompiles and rebakes if the timeline or the cut changed since the last time. While
    /// `dragging`, at most every [`DRAG_REBUILD_INTERVAL`], showing the last result between.
    pub fn rebuild(&mut self, dragging: bool) {
        let key = (self.timeline.clone(), self.cut());
        if self.built_from.as_ref() == Some(&key) {
            return;
        }
        if dragging && self.built_at.is_some_and(|t| t.elapsed() < DRAG_REBUILD_INTERVAL) {
            return;
        }
        let start = Instant::now();
        let held = self.result.as_ref().map(|r| &r.compiled);
        let compiled = self.timeline.compile(key.1, held);
        let bounds = compiled.bounds();
        let expected = stats::expected(&compiled, bounds, EXPECTED_RESOLUTION);
        let total: f64 = expected.iter().map(|(_, n)| n).sum();
        let cells = if bounds.is_empty() {
            0.0
        } else {
            let span = (bounds.size() / lc_galaxy::sample::CELL_LY).ceil() + glam::DVec3::splat(3.0);
            span.x * span.y * span.z
        };
        let bake = if total > LIVE_BAKE_BUDGET {
            Err(format!("{total:.0} stars expected, over the live budget of {LIVE_BAKE_BUDGET:.0}"))
        } else if cells > LIVE_CELL_BUDGET {
            Err(format!("{cells:.0} cells to bake, over the live budget of {LIVE_CELL_BUDGET:.0}"))
        } else {
            lc_galaxy::bake(&compiled, self.timeline.seed, bounds, default_threads()).map_err(|e| e.to_string())
        };
        let nearest = bake.as_ref().map(|b| stats::nearest_neighbors(&b.stars)).unwrap_or_default();
        self.result = Some(Result { compiled, bake, expected, nearest, took: start.elapsed() });
        self.built_from = Some(key);
        self.built_at = Some(Instant::now());
    }

    pub fn stars(&self) -> &[BakedStar] {
        match &self.result {
            Some(Result { bake: Ok(b), .. }) => &b.stars,
            _ => &[],
        }
    }

    /// The picked star and its index, if the current bake still has it.
    pub fn picked_star(&self) -> Option<(usize, &BakedStar)> {
        let id = self.picked?;
        self.stars().iter().enumerate().find(|(_, s)| s.id == id)
    }

    pub fn feature_name(&self, id: FeatureId) -> String {
        match self.timeline.feature(id) {
            Some(f) => f.label(),
            None => format!("{id}"),
        }
    }
}
