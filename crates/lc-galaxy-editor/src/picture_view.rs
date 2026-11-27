//! The Picture view: the galaxy's light marched through a coarse volume, from any direction.
//!
//! The volume rebuilds on a worker after each rebuild of the document, and the picture re-marches
//! on a worker whenever the view or the volume changes. One of each runs at a time, and the last
//! finished picture stays on screen until the next one lands.

use std::sync::{Arc, OnceLock};

use bevy_egui::egui::{self, ColorImage, Rect, TextureHandle, TextureOptions, pos2};
use glam::DVec3;
use lc_galaxy::picture::{Side, View, Volume, march};
use lc_galaxy::sample::default_threads;

use crate::doc::Doc;
use crate::palette;

/// Voxels along the volume's longest side. Coarse, because it rebuilds on every edit.
const VOLUME_RESOLUTION: usize = 96;
/// Screen pixels per marched pixel along each axis.
const DOWNSAMPLE: f32 = 2.0;

/// The newest result of a keyed computation, and the one running if any.
struct Job<K, T> {
    running: Option<(K, Arc<OnceLock<T>>)>,
    done: Option<(K, T)>,
}

impl<K, T> Default for Job<K, T> {
    fn default() -> Self {
        Self { running: None, done: None }
    }
}

impl<K: PartialEq + Clone, T: Send + Sync + 'static> Job<K, T> {
    /// Collects a finished run, then starts `work` for `key` unless that is done or running, or
    /// another run is still going.
    fn want(&mut self, key: &K, work: impl FnOnce() -> T + Send + 'static) {
        if let Some((k, slot)) = &self.running
            && slot.get().is_some()
        {
            let k = k.clone();
            let slot = self.running.take().unwrap().1;
            self.done = Some((k, Arc::try_unwrap(slot).ok().and_then(OnceLock::into_inner).unwrap()));
        }
        let fresh = self.done.as_ref().is_some_and(|(k, _)| k == key);
        if fresh || self.running.is_some() {
            return;
        }
        let slot = Arc::new(OnceLock::new());
        let out = slot.clone();
        std::thread::spawn(move || {
            let _ = out.set(work());
        });
        self.running = Some((key.clone(), slot));
    }

    fn busy(&self) -> bool {
        self.running.is_some()
    }
}

/// Where the picture looks from. The scale is per screen pixel so resizing the view keeps the
/// field.
#[derive(Clone, Copy, PartialEq)]
struct Camera {
    target: DVec3,
    yaw: f64,
    pitch: f64,
    ly_per_screen_px: f64,
}

#[derive(Clone, PartialEq)]
struct ImageKey {
    volume: u64,
    view: View,
    size: [u32; 2],
    decades: f64,
}

pub struct PictureView {
    camera: Option<Camera>,
    /// The side to look from at the next fit.
    side: Side,
    refit: bool,
    decades: f64,
    volume: Job<u64, Arc<Volume>>,
    image: Job<ImageKey, ColorImage>,
    texture: Option<TextureHandle>,
    shown: Option<ImageKey>,
}

impl Default for PictureView {
    fn default() -> Self {
        Self {
            camera: None,
            side: Side::named("+z").unwrap(),
            refit: true,
            decades: 3.0,
            volume: Job::default(),
            image: Job::default(),
            texture: None,
            shown: None,
        }
    }
}

impl PictureView {
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        if ui.button("Fit").clicked() {
            self.refit = true;
        }
        ui.separator();
        for side in Side::ALL {
            if ui.button(side.label()).clicked() {
                self.side = side;
                if let Some(c) = &mut self.camera {
                    (c.yaw, c.pitch) = side.angles();
                }
            }
        }
        ui.separator();
        ui.label("range");
        ui.add(egui::DragValue::new(&mut self.decades).range(0.5..=8.0).speed(0.05).suffix(" dex"));
    }

    pub fn show(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, response: &egui::Response, doc: &Doc) {
        let Some(result) = &doc.result else { return };
        let generation = doc.generation;
        let compiled = result.compiled.clone();
        self.volume.want(&generation, move || Arc::new(Volume::of(&compiled, VOLUME_RESOLUTION, default_threads())));
        let Some((volume_generation, volume)) = self.volume.done.as_ref().map(|(g, v)| (*g, v.clone())) else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "sampling the volume", egui::FontId::proportional(14.0), palette::UNREAD);
            ui.ctx().request_repaint();
            return;
        };

        let size = [((rect.width() / DOWNSAMPLE) as u32).max(1), ((rect.height() / DOWNSAMPLE) as u32).max(1)];
        if self.refit || self.camera.is_none() {
            let mut v = View::fitting(self.side, volume.bounds(), size[0], size[1]);
            if let Some(c) = self.camera {
                (v.yaw, v.pitch) = (c.yaw, c.pitch);
                v.fit(volume.bounds(), size[0], size[1]);
            }
            self.camera = Some(Camera { target: v.target, yaw: v.yaw, pitch: v.pitch, ly_per_screen_px: v.ly_per_px / f64::from(DOWNSAMPLE) });
            self.refit = false;
        }
        let camera = self.camera.as_mut().unwrap();
        steer(ui, response, camera);
        let view = View { target: camera.target, yaw: camera.yaw, pitch: camera.pitch, ly_per_px: camera.ly_per_screen_px * f64::from(DOWNSAMPLE) };

        let key = ImageKey { volume: volume_generation, view, size, decades: self.decades };
        let decades = self.decades;
        self.image.want(&key, move || {
            let rgba = march(&volume, &view, size[0], size[1], default_threads()).to_rgba8(decades);
            ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], &rgba)
        });
        if let Some((k, image)) = &self.image.done
            && self.shown.as_ref() != Some(k)
        {
            match &mut self.texture {
                Some(t) => t.set(image.clone(), TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("picture", image.clone(), TextureOptions::LINEAR)),
            }
            self.shown = Some(k.clone());
        }
        if let Some(t) = &self.texture {
            painter.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        if self.image.busy() || self.volume.busy() {
            ui.ctx().request_repaint();
        }
    }
}

/// Orbit on the primary button, pan on the others, zoom on the wheel.
fn steer(ui: &egui::Ui, response: &egui::Response, c: &mut Camera) {
    use std::f64::consts::FRAC_PI_2;
    let d = response.drag_delta();
    if response.dragged_by(egui::PointerButton::Primary) {
        c.yaw -= f64::from(d.x) * 0.008;
        c.pitch = (c.pitch + f64::from(d.y) * 0.008).clamp(-FRAC_PI_2, FRAC_PI_2);
    }
    if response.dragged_by(egui::PointerButton::Secondary) || response.dragged_by(egui::PointerButton::Middle) {
        let (_, right, up) = View { target: c.target, yaw: c.yaw, pitch: c.pitch, ly_per_px: 1.0 }.basis();
        c.target += (-right * f64::from(d.x) + up * f64::from(d.y)) * c.ly_per_screen_px;
    }
    if response.hovered() {
        let scroll = f64::from(ui.input(|i| i.smooth_scroll_delta.y));
        c.ly_per_screen_px *= (-scroll * 0.002).exp();
    }
}
