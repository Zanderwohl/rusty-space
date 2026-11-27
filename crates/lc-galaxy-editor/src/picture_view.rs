//! The Picture view: the galaxy's light marched through a coarse volume, from any direction.
//!
//! Both halves refine. The volume rebuilds on a worker after each edit at [`VOLUME_LEVELS`]'s
//! first resolution, then at each finer one in turn. The picture is marched on a worker at a
//! resolution chosen from the last march's cost so a moving view updates within
//! [`MOVING_BUDGET_S`], and once the view rests it halves the downsampling each march until it
//! reaches the screen's. Last, with the finest volume in, it marches the graph itself, whose edges
//! no voxel blurs. Work that a newer edit or a move makes stale is stopped.

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bevy_egui::egui::{self, ColorImage, Rect, TextureHandle, TextureOptions, pos2};
use glam::DVec3;
use lc_galaxy::Compiled;
use lc_galaxy::picture::{Progress, Side, View, Volume, lights, march_exact, march_until};
use lc_galaxy::sample::default_threads;

use crate::doc::Doc;
use crate::palette;

/// Voxels along the volume's longest side, coarsest first. The last bounds memory: 256 is about
/// 155 MB for a galaxy as flat as Phase III's, with the level before it held until it lands.
const VOLUME_LEVELS: [usize; 4] = [64, 128, 192, 256];
/// How long a march may take while the view moves, s.
const MOVING_BUDGET_S: f64 = 1.0 / 30.0;
/// How long after the last move the view counts as moving, so a pause between drags does not
/// start a full-resolution march.
const SETTLE: Duration = Duration::from_millis(250);
/// The coarsest a moving picture gets: screen pixels per marched pixel along each axis.
const MAX_DOWNSAMPLE: f64 = 32.0;

struct Running<K, T> {
    key: K,
    slot: Arc<OnceLock<Option<T>>>,
    progress: Arc<Progress>,
}

/// A keyed computation on a worker: the newest finished result, and the run in progress.
struct Job<K, T> {
    running: Option<Running<K, T>>,
    done: Option<(K, T)>,
}

impl<K, T> Default for Job<K, T> {
    fn default() -> Self {
        Self { running: None, done: None }
    }
}

impl<K: Clone, T: Send + Sync + 'static> Job<K, T> {
    /// Moves a finished run to `done`; a stopped one leaves nothing.
    fn poll(&mut self) {
        if self.running.as_ref().is_some_and(|r| r.slot.get().is_some()) {
            let r = self.running.take().unwrap();
            if let Some(Some(t)) = Arc::try_unwrap(r.slot).ok().and_then(OnceLock::into_inner) {
                self.done = Some((r.key, t));
            }
        }
    }

    /// Stops any run in progress and starts `work`, which should count into its [`Progress`] and
    /// return `None` once the stop flag is set.
    fn start(&mut self, key: K, work: impl FnOnce(&Progress) -> Option<T> + Send + 'static) {
        if let Some(r) = self.running.take() {
            r.progress.stop.store(true, Ordering::Relaxed);
        }
        let (slot, progress) = (Arc::new(OnceLock::new()), Arc::new(Progress::default()));
        let (out, counted) = (slot.clone(), progress.clone());
        std::thread::spawn(move || {
            let _ = out.set(work(&counted));
        });
        self.running = Some(Running { key, slot, progress });
    }
}

/// Where the picture looks from. The scale is per screen pixel, so resizing or downsampling
/// keeps the field.
#[derive(Clone, Copy, PartialEq)]
struct Camera {
    target: DVec3,
    yaw: f64,
    pitch: f64,
    ly_per_screen_px: f64,
}

/// Which edit, and which of [`VOLUME_LEVELS`].
type VolumeKey = (u64, usize);

#[derive(Clone, PartialEq)]
struct ImageKey {
    volume: VolumeKey,
    camera: Camera,
    downsample: f64,
    screen: [u32; 2],
    decades: f64,
    saturation: f64,
    /// Through the graph rather than the volume.
    exact: bool,
}

struct Marched {
    image: ColorImage,
    took: Duration,
}

pub struct PictureView {
    camera: Option<Camera>,
    /// The side to look from at the next fit.
    side: Side,
    refit: bool,
    decades: f64,
    saturation: f64,
    volume: Job<VolumeKey, Arc<Volume>>,
    image: Job<ImageKey, Marched>,
    texture: Option<TextureHandle>,
    shown: Option<ImageKey>,
    last_camera: Option<Camera>,
    last_move: Option<Instant>,
    downsample: f64,
    /// Seconds per marched pixel, smoothed over recent marches.
    cost: Option<f64>,
}

impl Default for PictureView {
    fn default() -> Self {
        Self {
            camera: None,
            side: Side::named("+z").unwrap(),
            refit: true,
            decades: 3.0,
            saturation: 1.2,
            volume: Job::default(),
            image: Job::default(),
            texture: None,
            shown: None,
            last_camera: None,
            last_move: None,
            downsample: 8.0,
            cost: None,
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
        ui.label("saturation");
        ui.add(egui::DragValue::new(&mut self.saturation).range(0.0..=3.0).speed(0.01));
    }

    pub fn show(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: Rect, response: &egui::Response, doc: &Doc) {
        let Some(result) = &doc.result else { return };
        self.refine_volume(doc.generation, &result.compiled);
        let Some((volume_key, volume)) = self.volume.done.as_ref().filter(|(k, _)| k.0 == doc.generation).map(|(k, v)| (*k, v.clone())) else {
            self.draw(painter, rect);
            if self.texture.is_none() {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "sampling the volume", egui::FontId::proportional(14.0), palette::UNREAD);
            }
            ui.ctx().request_repaint();
            return;
        };

        let screen = [rect.width().max(1.0) as u32, rect.height().max(1.0) as u32];
        if self.refit || self.camera.is_none() {
            let mut v = View::fitting(self.side, volume.bounds(), screen[0], screen[1]);
            if let Some(c) = self.camera {
                (v.yaw, v.pitch) = (c.yaw, c.pitch);
                v.fit(volume.bounds(), screen[0], screen[1]);
            }
            self.camera = Some(Camera { target: v.target, yaw: v.yaw, pitch: v.pitch, ly_per_screen_px: v.ly_per_px });
            self.refit = false;
        }
        let camera = self.camera.as_mut().unwrap();
        steer(ui, response, camera);
        let camera = *camera;
        if self.last_camera != Some(camera) {
            self.last_move = Some(Instant::now());
            self.last_camera = Some(camera);
        }
        let moving = self.last_move.is_some_and(|t| t.elapsed() < SETTLE);

        self.image.poll();
        if let Some((k, m)) = &self.image.done
            && self.shown.as_ref() != Some(k)
        {
            let pixels = (m.image.size[0] * m.image.size[1]).max(1) as f64;
            let cost = m.took.as_secs_f64() / pixels;
            self.cost = Some(self.cost.map_or(cost, |c| 0.5 * c + 0.5 * cost));
        }
        let current = |k: &ImageKey| k.camera == camera && k.volume == volume_key && k.screen == screen && k.decades == self.decades && k.saturation == self.saturation;
        self.downsample = if moving {
            let pixels = f64::from(screen[0]) * f64::from(screen[1]);
            self.cost.map_or(8.0, |c| (pixels * c / MOVING_BUDGET_S).sqrt()).clamp(1.0, MAX_DOWNSAMPLE)
        } else {
            match &self.image.done {
                // Finer by half each march once the last one at this view has landed.
                Some((k, _)) if current(k) && k.downsample <= self.downsample => (self.downsample / 2.0).max(1.0),
                _ => self.downsample,
            }
        };
        let finest = volume_key.1 + 1 == VOLUME_LEVELS.len();
        let exact = !moving && finest && self.downsample <= 1.0 && self.image.done.as_ref().is_some_and(|(k, _)| current(k) && k.downsample <= 1.0);
        let key = ImageKey { volume: volume_key, camera, downsample: self.downsample, screen, decades: self.decades, saturation: self.saturation, exact };
        let stale = |r: &Running<ImageKey, Marched>| r.key.volume != key.volume || r.key.downsample < key.downsample;
        let done_here = self.image.done.as_ref().is_some_and(|(k, _)| *k == key);
        let running_here = self.image.running.as_ref().is_some_and(|r| r.key == key);
        if !done_here && !running_here && self.image.running.as_ref().is_none_or(stale) {
            // Only the exact march reads it, and cloning it is cheap beside any march.
            let compiled = result.compiled.clone();
            self.image.start(key.clone(), move |stop| march(&volume, &compiled, &key, stop));
        }

        if let Some((k, m)) = &self.image.done
            && self.shown.as_ref() != Some(k)
        {
            match &mut self.texture {
                Some(t) => t.set(m.image.clone(), TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("picture", m.image.clone(), TextureOptions::LINEAR)),
            }
            self.shown = Some(k.clone());
        }
        self.draw(painter, rect);
        if self.image.running.is_some() || self.volume.running.is_some() || moving || self.downsample > 1.0 {
            ui.ctx().request_repaint();
        }
    }

    /// Starts the coarsest volume after an edit, and each finer one once the last has landed.
    fn refine_volume(&mut self, generation: u64, compiled: &lc_galaxy::Compiled) {
        self.volume.poll();
        let want = match &self.volume.done {
            Some(((g, level), _)) if *g == generation => (level + 1 < VOLUME_LEVELS.len()).then_some((generation, level + 1)),
            _ => Some((generation, 0)),
        };
        let Some(want) = want else { return };
        if self.volume.running.as_ref().is_some_and(|r| r.key == want) {
            return;
        }
        let compiled = compiled.clone();
        self.volume.start(want, move |progress| Volume::of_until(&compiled, VOLUME_LEVELS[want.1], default_threads(), progress).map(Arc::new));
    }

    fn draw(&self, painter: &egui::Painter, rect: Rect) {
        if let Some(t) = &self.texture {
            painter.image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        // The volume on screen, and the finer one being sampled.
        let shown = self.shown.as_ref().map(|k| if k.exact { "exact".to_string() } else { format!("{} voxels", VOLUME_LEVELS[k.volume.1]) });
        let building = self.volume.running.as_ref().map(|r| {
            let (done, total) = r.progress.read();
            format!("{} voxels {done}/{total} in progress", VOLUME_LEVELS[r.key.1])
        });
        let exact = self.image.running.as_ref().filter(|r| r.key.exact).map(|r| {
            let (done, total) = r.progress.read();
            format!("exact {done}/{total} in progress")
        });
        let text = [shown, building, exact].into_iter().flatten().collect::<Vec<_>>().join("    ");
        if !text.is_empty() {
            let at = rect.right_bottom() - egui::vec2(10.0, 8.0);
            painter.text(at, egui::Align2::RIGHT_BOTTOM, text, egui::FontId::monospace(12.0), palette::UNREAD);
        }
    }
}

fn march(volume: &Volume, compiled: &Compiled, key: &ImageKey, progress: &Progress) -> Option<Marched> {
    let start = Instant::now();
    let size = key.screen.map(|n| ((f64::from(n) / key.downsample).round() as u32).max(1));
    let c = key.camera;
    let view = View { target: c.target, yaw: c.yaw, pitch: c.pitch, ly_per_px: c.ly_per_screen_px * f64::from(key.screen[0]) / f64::from(size[0]) };
    let picture = if key.exact {
        march_exact(compiled, &lights(compiled), &view, size[0], size[1], volume.voxel_ly, default_threads(), progress)?
    } else {
        march_until(volume, &view, size[0], size[1], default_threads(), progress)?
    };
    let image = ColorImage::from_rgba_unmultiplied([size[0] as usize, size[1] as usize], &picture.to_rgba8(key.decades, key.saturation));
    Some(Marched { image, took: start.elapsed() })
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
