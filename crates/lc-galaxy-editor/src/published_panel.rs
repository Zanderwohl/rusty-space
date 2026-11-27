//! The Published panel: a manifest on disk, its chunks, and how the current bake differs from
//! each. See `lightcone/docs/34-galaxy-authoring.md`, "Publishing and freezing".

use std::path::PathBuf;
use std::pin::Pin;

use bevy::tasks::{IoTaskPool, Task, futures::check_ready};
use bevy::window::RawHandleWrapper;
use bevy_egui::egui::{self, Color32, RichText};
use lc_galaxy::Aabb;
use lc_galaxy::diff::{ChunkDiff, Status, Tolerance};
use lc_world::sky::galaxy::{CELL_LY, CHUNK_CELLS, GalaxyChunk, Manifest, ManifestChunk};

use crate::doc::Doc;
use crate::palette;

struct Opened {
    path: PathBuf,
    manifest: Manifest,
    chunks: Vec<(ManifestChunk, GalaxyChunk)>,
}

#[derive(Default)]
pub struct Published {
    opened: Option<Opened>,
    failed: Option<String>,
    pick: Option<Task<Option<PathBuf>>>,
    /// Against the bake of this doc generation.
    diffs: Option<(u64, Result<Vec<ChunkDiff>, String>)>,
    /// Draw each chunk's box in the region view, colored by status.
    pub in_views: bool,
}

pub fn color(status: Status) -> Color32 {
    match status {
        Status::Same => palette::PUBLISHED_SAME,
        Status::Close => palette::PUBLISHED_CLOSE,
        Status::Differs => palette::PUBLISHED_DIFFERS,
        Status::Unpublished => palette::PUBLISHED_NONE,
    }
}

fn chunk_bounds(chunk: [i32; 3]) -> Aabb {
    let side = f64::from(CHUNK_CELLS) * CELL_LY;
    let min = glam::DVec3::from_array(chunk.map(f64::from)) * side;
    Aabb { min, max: min + glam::DVec3::splat(side) }
}

impl Published {
    /// Opened on `path`, with its chunks drawn in the views.
    pub fn opening(path: PathBuf) -> Self {
        let mut p = Self { in_views: true, ..Self::default() };
        p.open(path);
        p
    }

    pub fn is_open(&self) -> bool {
        self.opened.is_some()
    }

    /// One box per chunk, in the color of its worse tier.
    pub fn boxes(&self) -> Vec<(Aabb, Color32)> {
        let Some((_, Ok(diffs))) = &self.diffs else { return Vec::new() };
        let mut worst: std::collections::BTreeMap<[i32; 3], Status> = Default::default();
        for d in diffs {
            let w = worst.entry(d.chunk).or_insert(d.status);
            *w = (*w).max(d.status);
        }
        worst.into_iter().map(|(c, s)| (chunk_bounds(c), color(s))).collect()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, doc: &Doc, parent: Option<&RawHandleWrapper>) {
        if let Some(mut task) = self.pick.take() {
            match check_ready(&mut task) {
                None => self.pick = Some(task),
                Some(Some(path)) => self.open(path),
                Some(None) => {}
            }
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.pick.is_none(), egui::Button::new("Open manifest…")).clicked() {
                self.pick = Some(pick(doc, parent));
            }
            if let Some(path) = self.opened.as_ref().map(|o| o.path.clone()) {
                if ui.button("Reload").clicked() {
                    self.open(path);
                }
                if ui.button("Close").clicked() {
                    self.opened = None;
                    self.diffs = None;
                }
            }
            ui.checkbox(&mut self.in_views, "chunks in view");
        });
        if let Some(e) = &self.failed {
            ui.label(RichText::new(e).color(palette::PROBLEM));
        }
        let Some(opened) = &self.opened else { return };
        ui.label(format!(
            "{} — {} chunk files, {} stars{}",
            opened.manifest.name,
            opened.manifest.chunks.len(),
            opened.manifest.chunks.iter().map(|c| u64::from(c.stars)).sum::<u64>(),
            opened.manifest.parent.as_ref().map_or(String::new(), |p| format!(", built on {p}")),
        ));
        ui.label(RichText::new(opened.path.display().to_string()).weak());

        if self.diffs.as_ref().map(|(g, _)| *g) != Some(doc.generation) {
            self.diffs = Some((doc.generation, work_out(doc, &opened.chunks)));
        }
        match &self.diffs {
            Some((_, Ok(diffs))) => table(ui, diffs),
            Some((_, Err(e))) => {
                ui.label(RichText::new(e).color(palette::PROBLEM));
            }
            None => {}
        }
    }

    fn open(&mut self, path: PathBuf) {
        self.diffs = None;
        match Manifest::read_from_disk(&path) {
            Ok((manifest, chunks)) => {
                let chunks = manifest.chunks.iter().cloned().zip(chunks).collect();
                self.opened = Some(Opened { path, manifest, chunks });
                self.failed = None;
            }
            Err(e) => self.failed = Some(e),
        }
    }
}

fn work_out(doc: &Doc, published: &[(ManifestChunk, GalaxyChunk)]) -> Result<Vec<ChunkDiff>, String> {
    let Some(result) = &doc.result else { return Err("nothing baked yet".into()) };
    let bake = result.bake.as_ref().map_err(|e| format!("no bake to compare: {e}"))?;
    if doc.cut() < doc.timeline.len() {
        return Err("the timeline is cut; a publish bakes every row".into());
    }
    lc_galaxy::diff::diff(&result.compiled, bake, lc_galaxy::publish::graph_version(&doc.timeline), published, Tolerance::default())
}

fn table(ui: &mut egui::Ui, diffs: &[ChunkDiff]) {
    let count = |s: Status| diffs.iter().filter(|d| d.status == s).count();
    ui.horizontal(|ui| {
        for s in [Status::Same, Status::Close, Status::Differs, Status::Unpublished] {
            ui.label(RichText::new(format!("{} {}", count(s), s.label())).color(color(s)));
        }
    });
    egui::Grid::new("published").striped(true).num_columns(7).show(ui, |ui| {
        for h in ["chunk", "tier", "status", "published", "now", "changed", "Δ[Fe/H]"] {
            ui.label(RichText::new(h).strong());
        }
        ui.end_row();
        for d in diffs {
            let [x, y, z] = d.chunk;
            ui.label(format!("{x},{y},{z}"));
            ui.label(format!("{:?}", d.tier));
            ui.label(RichText::new(d.status.label()).color(color(d.status)));
            ui.label(d.published.to_string());
            ui.label(d.now.to_string());
            ui.label(d.changed.to_string());
            ui.label(format!("{:+.3}", d.feh_shift));
            ui.end_row();
        }
    });
}

fn pick(doc: &Doc, parent: Option<&RawHandleWrapper>) -> Task<Option<PathBuf>> {
    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Galaxy manifest", &["galaxy"]).set_title("Open manifest");
    if let Some(parent) = parent {
        // SAFETY: called from the `ui` system, which runs on the main thread.
        dialog = dialog.set_parent(&unsafe { parent.get_handle() });
    }
    if let Some(dir) = doc.path.as_ref().and_then(|p| p.parent()) {
        dialog = dialog.set_directory(dir);
    }
    // Made here, on the main thread, as `files::pick` does.
    let chosen: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> = Box::pin(dialog.pick_file());
    IoTaskPool::get().spawn(async move { chosen.await.map(|h| h.path().to_path_buf()) })
}
