//! The region view, baked stars at their true positions from an orbit camera, and the sky
//! view, the same stars as seen from one point.
//!
//! Both draw through egui's painter, which is cheap up to the live-bake budget. The client's
//! starfield material and a GPU ray marcher replace it once views draw fields rather than every
//! star; see `34-galaxy-authoring.md`, "Live bake and scale".

use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use glam::DVec3;
use lc_galaxy::attributes::State;
use lc_galaxy::compiled::{Frame, SystemField};
use lc_galaxy::kinds::{PathKind, PathPoint};

use crate::compare::{self, Marks, Reference, Solo};
use lc_galaxy::sample::cells_in;
use lc_galaxy::{Aabb, BakedStar, FeatureId};

use crate::doc::Doc;
use crate::palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Region,
    Sky,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorBy {
    Camera,
    Temperature,
    Population,
    Metallicity,
    Age,
    Mass,
    System(SystemField),
}

impl ColorBy {
    pub const ALL: [ColorBy; 11] = [
        ColorBy::Camera,
        ColorBy::Temperature,
        ColorBy::Population,
        ColorBy::Metallicity,
        ColorBy::Age,
        ColorBy::Mass,
        ColorBy::System(SystemField::Ocean),
        ColorBy::System(SystemField::Solids),
        ColorBy::System(SystemField::Belts),
        ColorBy::System(SystemField::Cloud),
        ColorBy::System(SystemField::Life),
    ];

    pub fn label(self) -> String {
        match self {
            ColorBy::Camera => "camera".into(),
            ColorBy::Temperature => "temperature".into(),
            ColorBy::Population => "population".into(),
            ColorBy::Metallicity => "[Fe/H]".into(),
            ColorBy::Age => "age".into(),
            ColorBy::Mass => "mass".into(),
            ColorBy::System(f) => format!("system: {}", f.label()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub target: DVec3,
    /// Radians, about +Z from +X.
    pub yaw: f64,
    /// Radians above the XY plane.
    pub pitch: f64,
    pub distance: f64,
    pub fov: f64,
    /// Orthographic, at the scale perspective has at the target.
    pub ortho: bool,
}

/// The side of the galaxy's axes the camera looks from: `+Z` is face-on from galactic north,
/// `-Y` edge-on with +X to the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Side {
    /// 0, 1, 2 for X, Y, Z.
    pub axis: usize,
    pub positive: bool,
}

impl Side {
    pub const ALL: [Side; 6] = [
        Side { axis: 0, positive: true },
        Side { axis: 1, positive: true },
        Side { axis: 2, positive: true },
        Side { axis: 0, positive: false },
        Side { axis: 1, positive: false },
        Side { axis: 2, positive: false },
    ];

    pub fn label(self) -> String {
        format!("{}{}", if self.positive { '+' } else { '-' }, ["X", "Y", "Z"][self.axis])
    }

    pub fn named(name: &str) -> Option<Self> {
        Side::ALL.into_iter().find(|s| s.label().eq_ignore_ascii_case(name))
    }
}

impl Camera {
    /// Looks at the target from `side`, orthographic. The side views keep +Z up; the views from
    /// ±Z keep +X to the right.
    pub fn look_from(&mut self, side: Side) {
        use std::f64::consts::{FRAC_PI_2, PI};
        (self.yaw, self.pitch) = match (side.axis, side.positive) {
            (0, true) => (0.0, 0.0),
            (0, false) => (PI, 0.0),
            (1, true) => (FRAC_PI_2, 0.0),
            (1, false) => (-FRAC_PI_2, 0.0),
            (_, positive) => (-FRAC_PI_2, if positive { FRAC_PI_2 } else { -FRAC_PI_2 }),
        };
        self.ortho = true;
    }

    /// The screen's right comes from the yaw alone, so looking straight down or up still turns
    /// with the yaw, and the pitch may reach ±π/2.
    fn projector(&self, rect: Rect) -> Projector {
        let eye = self.eye();
        let (sy, cy) = self.yaw.sin_cos();
        let mut proj = Projector::with_right(eye, self.target - eye, DVec3::new(-sy, cy, 0.0), self.fov, rect);
        proj.ortho = self.ortho.then_some(self.distance);
        proj
    }

    fn eye(&self) -> DVec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + DVec3::new(cp * cy, cp * sy, sp) * self.distance
    }

    fn fit(&mut self, bounds: Aabb) {
        if bounds.is_empty() {
            return;
        }
        self.target = bounds.center();
        self.distance = (bounds.size().length() * 1.4).max(4.0);
    }
}

/// A pinhole looking along `forward`, Z-up.
struct Projector {
    eye: DVec3,
    forward: DVec3,
    right: DVec3,
    up: DVec3,
    focal: f64,
    center: Pos2,
    /// The depth whose scale an orthographic projection uses everywhere; `None` is perspective.
    ortho: Option<f64>,
}

impl Projector {
    fn new(eye: DVec3, forward: DVec3, fov: f64, rect: Rect) -> Self {
        let forward = forward.normalize();
        // Looking along ±Z, `cos(π/2)` leaves a remainder near 1e-17 that would pick the screen's
        // right at random; there it is +X.
        let across = forward.cross(DVec3::Z);
        let right = if across.length() < 1e-9 { DVec3::X } else { across.normalize() };
        Self::with_right(eye, forward, right, fov, rect)
    }

    /// `right` must be a unit vector perpendicular to `forward`.
    fn with_right(eye: DVec3, forward: DVec3, right: DVec3, fov: f64, rect: Rect) -> Self {
        let forward = forward.normalize();
        let up = right.cross(forward);
        let focal = rect.height() as f64 * 0.5 / (fov * 0.5).tan();
        Self { eye, forward, right, up, focal, center: rect.center(), ortho: None }
    }

    /// Pixels per ly at depth `z`.
    fn px_per_ly(&self, z: f64) -> f64 {
        self.focal / self.ortho.unwrap_or(z)
    }

    /// Screen position and depth along the view axis, ly.
    fn project(&self, p: DVec3) -> Option<(Pos2, f64)> {
        let rel = p - self.eye;
        let z = rel.dot(self.forward);
        if z <= 1e-3 && self.ortho.is_none() {
            return None;
        }
        let s = self.px_per_ly(z);
        Some((pos2(self.center.x + (rel.dot(self.right) * s) as f32, self.center.y - (rel.dot(self.up) * s) as f32), z))
    }

    fn line(&self, painter: &egui::Painter, a: DVec3, b: DVec3, stroke: Stroke) {
        if let (Some((pa, _)), Some((pb, _))) = (self.project(a), self.project(b)) {
            painter.line_segment([pa, pb], stroke);
        }
    }
}

pub struct ViewState {
    pub view: View,
    pub color_by: ColorBy,
    pub show_cells: bool,
    pub show_exclusion: bool,
    pub show_supports: bool,
    pub camera: Camera,
    /// Sky view direction, radians.
    pub look_yaw: f64,
    pub look_pitch: f64,
    pub sky_fov: f64,
    fitted: bool,
    pub solo: Option<Solo>,
    pub reference: Option<Reference>,
    /// Colors stars against `reference`; off draws them by `color_by`.
    pub difference: bool,
    pins: u64,
    marks: Option<Marks>,
    /// The control point of the selected path being dragged in the region view.
    handle: Option<usize>,
}

impl ViewState {
    /// Drops the pin, which holds the previous document's bake.
    pub fn forget_document(&mut self) {
        self.reference = None;
        self.difference = false;
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            view: View::Region,
            color_by: ColorBy::Camera,
            show_cells: true,
            show_exclusion: false,
            show_supports: true,
            camera: Camera { target: DVec3::ZERO, yaw: 0.6, pitch: 0.45, distance: 20.0, fov: 50f64.to_radians(), ortho: false },
            look_yaw: 0.0,
            look_pitch: 0.0,
            sky_fov: 90f64.to_radians(),
            fitted: false,
            solo: None,
            reference: None,
            difference: false,
            pins: 0,
            marks: None,
            handle: None,
        }
    }
}

pub fn toolbar(ui: &mut egui::Ui, state: &mut ViewState, doc: &Doc) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.view, View::Region, "Region");
        ui.selectable_value(&mut state.view, View::Sky, "Sky");
        ui.separator();
        egui::ComboBox::from_id_salt("color by").selected_text(state.color_by.label()).show_ui(ui, |ui| {
            for c in ColorBy::ALL {
                ui.selectable_value(&mut state.color_by, c, c.label());
            }
        });
        let options = doc.result.as_ref().map(|r| Solo::options(&r.compiled)).unwrap_or_default();
        if state.solo.is_some_and(|s| !options.contains(&s)) {
            state.solo = None;
        }
        let shown = state.solo.map_or("everything".to_string(), |s| format!("solo {}", s.label(doc)));
        egui::ComboBox::from_id_salt("solo").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut state.solo, None, "everything");
            for s in options {
                ui.selectable_value(&mut state.solo, Some(s), s.label(doc));
            }
        });
        ui.separator();
        let baked = doc.result.as_ref().is_some_and(|r| r.bake.is_ok());
        if ui.add_enabled(baked, egui::Button::new("Pin")).clicked() {
            state.pins += 1;
            state.reference = Some(Reference::of(doc.stars(), state.pins));
        }
        if state.reference.is_some() {
            ui.toggle_value(&mut state.difference, "difference");
            if ui.button("Unpin").clicked() {
                state.reference = None;
                state.difference = false;
            }
            if state.difference {
                // Worked out here, after this frame's toggles; the view below reuses them.
                let reference = state.reference.as_ref();
                let (new, changed, gone) = compare::marks(&mut state.marks, doc, state.solo, reference).counts();
                ui.label(format!("{new} new, {changed} changed, {gone} gone"));
            }
        }
        ui.separator();
        if state.view == View::Region {
            ui.toggle_value(&mut state.show_cells, "cells");
            ui.toggle_value(&mut state.show_supports, "supports");
            ui.toggle_value(&mut state.show_exclusion, "exclusion");
            if ui.button("Fit").clicked() {
                state.fitted = false;
            }
            ui.separator();
            for side in Side::ALL {
                if ui.button(side.label()).clicked() {
                    state.camera.look_from(side);
                }
            }
            ui.toggle_value(&mut state.camera.ortho, "ortho");
        } else {
            let from = doc.picked_star().map_or("from the center".to_string(), |(i, _)| format!("from star {i}"));
            ui.label(from);
        }
    });
}

pub fn show(ui: &mut egui::Ui, state: &mut ViewState, doc: &mut Doc) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette::VIEW_BACKGROUND);
    let Some(result) = &doc.result else { return };
    let bounds = result.compiled.bounds();
    if !state.fitted && !bounds.is_empty() {
        state.camera.fit(bounds);
        state.fitted = true;
    }
    if let Err(e) = &result.bake {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(14.0), palette::PROBLEM);
    }
    match state.view {
        View::Region => region(ui, &painter, rect, &response, state, doc, bounds),
        View::Sky => sky(ui, &painter, rect, &response, state, doc, bounds),
    }
}

fn star_color(s: &BakedStar, by: ColorBy, doc: &Doc) -> Color32 {
    let a = &s.attributes;
    match by {
        ColorBy::Camera if a.teff_k > 0.0 => palette::camera(a.teff_k),
        ColorBy::Temperature if a.teff_k > 0.0 => palette::blackbody(a.teff_k),
        ColorBy::Camera | ColorBy::Temperature => Color32::from_gray(90),
        ColorBy::Population => {
            let i = a
                .population
                .and_then(|p| doc.result.as_ref()?.compiled.population_index(p))
                .unwrap_or(0);
            palette::POPULATIONS[i % palette::POPULATIONS.len()]
        }
        ColorBy::Metallicity => palette::diverging((a.feh + 1.0) / 1.5),
        ColorBy::Age => palette::sequential(a.age_gyr / 13.8),
        ColorBy::Mass => palette::sequential((a.initial_mass_solar.log10() + 1.1) / 3.1),
        ColorBy::System(f) => {
            let (lo, hi) = f.range();
            let t = (f.read(&a.profile) - lo) / (hi - lo);
            // Dex fields are signed about zero, so they diverge from the middle.
            if f.in_dex() { palette::diverging(t) } else { palette::sequential(t) }
        }
    }
}

/// Dot radius in px for a star of luminosity `l` L☉ in the region view.
fn region_radius(l: f64) -> f32 {
    (5.0 + 1.0 * l.max(1e-4).log10()).clamp(2.5, 10.0) as f32
}

fn region(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    response: &egui::Response,
    state: &mut ViewState,
    doc: &mut Doc,
    bounds: Aabb,
) {
    let path = selected_path(doc);
    if response.drag_started_by(egui::PointerButton::Primary) {
        let before = state.camera.projector(rect);
        let at = ui.input(|i| i.pointer.press_origin());
        state.handle = path.as_ref().zip(at).and_then(|((_, frame, points), at)| nearest_handle(&before, frame, points, at));
    }
    let cam = &mut state.camera;
    if response.dragged_by(egui::PointerButton::Primary) {
        let d = response.drag_delta();
        if let (Some(i), Some((id, frame, points))) = (state.handle, &path) {
            let before = cam.projector(rect);
            drag_point(doc, &before, *id, frame, points, i, d);
        } else {
            cam.yaw -= d.x as f64 * 0.008;
            cam.pitch = (cam.pitch + d.y as f64 * 0.008).clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        }
    }
    if response.drag_stopped() {
        state.handle = None;
    }
    let proj = cam.projector(rect);
    if response.dragged_by(egui::PointerButton::Secondary) || response.dragged_by(egui::PointerButton::Middle) {
        let d = response.drag_delta();
        let per_px = cam.distance / proj.focal;
        cam.target += (-proj.right * d.x as f64 + proj.up * d.y as f64) * per_px;
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y) as f64;
        cam.distance = (cam.distance * (-scroll * 0.002).exp()).clamp(0.5, 1e6);
    }

    // The frame plane's grid, on cell lines.
    let extent = if bounds.is_empty() { 16.0 } else { (bounds.size().max_element() * 0.75).max(8.0) };
    let step = lc_galaxy::sample::CELL_LY;
    let n = (extent / step).ceil() as i32;
    let reach = n as f64 * step;
    for i in -n..=n {
        let v = i as f64 * step;
        proj.line(painter, DVec3::new(v, -reach, 0.0), DVec3::new(v, reach, 0.0), Stroke::new(1.0, palette::GRID));
        proj.line(painter, DVec3::new(-reach, v, 0.0), DVec3::new(reach, v, 0.0), Stroke::new(1.0, palette::GRID));
    }
    for (axis, color) in [(DVec3::X, palette::AXIS_X), (DVec3::Y, palette::AXIS_Y), (DVec3::Z, palette::AXIS_Z)] {
        proj.line(painter, DVec3::ZERO, axis * step * 0.5, Stroke::new(1.5, color));
    }

    if state.show_cells {
        for cell in cells_in(bounds) {
            boxed(painter, &proj, cell.bounds(), Stroke::new(1.0, palette::CELL));
        }
    }
    if state.show_supports
        && let Some(r) = &doc.result
    {
        for shape in r.compiled.shapes.values() {
            boxed(painter, &proj, shape.bounds(), Stroke::new(1.0, palette::SUPPORT));
        }
    }
    selected_gizmo(painter, &proj, doc, cam.distance);

    let stars = doc.stars();
    let reference = state.reference.as_ref().filter(|_| state.difference);
    let marks = compare::marks(&mut state.marks, doc, state.solo, reference);
    let mut drawn: Vec<(usize, Pos2, f64)> = stars
        .iter()
        .enumerate()
        .filter(|(i, _)| marks.shown[*i])
        .filter_map(|(i, s)| proj.project(s.position_ly).map(|(p, z)| (i, p, z)))
        .collect();
    drawn.sort_by(|a, b| b.2.total_cmp(&a.2));
    let picked = doc.picked_star().map(|(i, _)| i);
    for &(i, p, z) in &drawn {
        let s = &stars[i];
        let color = if reference.is_some() { marks.change[i].color() } else { star_color(s, state.color_by, doc) };
        let r = region_radius(s.attributes.luminosity_solar);
        if matches!(s.attributes.state, State::WhiteDwarf | State::NeutronStar | State::BlackHole) {
            painter.circle_stroke(p, r.max(2.5), Stroke::new(1.0, color));
        } else {
            painter.circle_filled(p, r, color);
        }
        if state.show_exclusion {
            let px = (s.exclusion_ly * proj.px_per_ly(z)) as f32;
            painter.circle_stroke(p, px, Stroke::new(1.0, color.gamma_multiply(0.35)));
        }
        if picked == Some(i) {
            painter.circle_stroke(p, r + 4.0, Stroke::new(1.5, palette::PICK));
        }
    }
    if let Some(r) = reference {
        for &i in &marks.gone {
            let s = &r.stars[i];
            if let Some((p, _)) = proj.project(s.position_ly) {
                painter.circle_stroke(p, region_radius(s.attributes.luminosity_solar).max(2.5), Stroke::new(1.5, palette::DIFF_GONE));
            }
        }
    }
    // Over the stars, which a stream's own stars would otherwise hide.
    if let Some((_, frame, points)) = &path {
        path_gizmo(painter, &proj, frame, points, state.handle);
    }
    scale_bar(painter, rect, proj.focal / state.camera.distance);

    if response.clicked()
        && let Some(at) = response.interact_pointer_pos()
    {
        let nearest = drawn
            .iter()
            .map(|&(i, p, _)| (i, p.distance(at)))
            .filter(|&(_, d)| d < 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        doc.picked = nearest.map(|(i, _)| stars[i].id);
    }
}

/// Screen radius of a path's control point, px; a press within twice this drags it.
const HANDLE_PX: f32 = 6.0;

/// The selected feature when it is a path, with its frame and points.
fn selected_path(doc: &Doc) -> Option<(FeatureId, Frame, Vec<PathPoint>)> {
    let id = doc.selected.and_then(|i| doc.timeline.rows().get(i)?.id())?;
    let path = doc.timeline.feature(id)?.get::<PathKind>()?;
    let frame = doc.result.as_ref()?.compiled.frames.get(&path.frame)?.clone();
    Some((id, frame, path.points.clone()))
}

fn nearest_handle(proj: &Projector, frame: &Frame, points: &[PathPoint], at: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, proj.project(frame.to_galaxy(DVec3::from(p.at_ly)))?.0.distance(at))))
        .filter(|&(_, d)| d <= HANDLE_PX * 2.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Moves point `i` by `d` px in the screen plane at its own depth.
fn drag_point(doc: &mut Doc, proj: &Projector, id: FeatureId, frame: &Frame, points: &[PathPoint], i: usize, d: egui::Vec2) {
    let Some(p) = points.get(i) else { return };
    let Some((_, depth)) = proj.project(frame.to_galaxy(DVec3::from(p.at_ly))) else { return };
    let world = (proj.right * d.x as f64 - proj.up * d.y as f64) / proj.px_per_ly(depth);
    let local = frame.axes.transpose() * world;
    if let Some(path) = doc.timeline.feature_mut(id).and_then(|f| f.get_mut::<PathKind>())
        && let Some(point) = path.points.get_mut(i)
    {
        point.at_ly = (DVec3::from(point.at_ly) + local).into();
    }
}

fn path_gizmo(painter: &egui::Painter, proj: &Projector, frame: &Frame, points: &[PathPoint], held: Option<usize>) {
    let placed: Vec<(DVec3, f64)> = points.iter().map(|p| (frame.to_galaxy(DVec3::from(p.at_ly)), p.radius_ly)).collect();
    for w in lc_galaxy::kinds::curve(&placed).windows(2) {
        proj.line(painter, w[0].0, w[1].0, Stroke::new(2.0, palette::PICK));
    }
    for (i, (p, _)) in placed.iter().enumerate() {
        if let Some((at, _)) = proj.project(*p) {
            let fill = if held == Some(i) { palette::PICK } else { palette::VIEW_BACKGROUND };
            painter.circle(at, HANDLE_PX, fill, Stroke::new(1.5, palette::PICK));
        }
    }
}

/// The selected feature in the viewport: a frame's axes and its plane, or a shape's support.
fn selected_gizmo(painter: &egui::Painter, proj: &Projector, doc: &Doc, camera_distance: f64) {
    let Some(r) = &doc.result else { return };
    let Some(id) = doc.selected.and_then(|i| doc.timeline.rows().get(i)?.id()) else { return };
    if let Some(frame) = r.compiled.frames.get(&id) {
        let len = camera_distance * 0.2;
        let o = frame.origin;
        for (i, color) in [palette::AXIS_X, palette::AXIS_Y, palette::AXIS_Z].into_iter().enumerate() {
            let tip = o + frame.axes.col(i) * len;
            proj.line(painter, o, tip, Stroke::new(2.5, color));
            if let Some((p, _)) = proj.project(tip) {
                painter.text(p, egui::Align2::LEFT_BOTTOM, ["x", "y", "z"][i], egui::FontId::proportional(13.0), color);
            }
        }
        let ring: Vec<DVec3> = (0..=72)
            .map(|k| {
                let a = k as f64 / 72.0 * std::f64::consts::TAU;
                frame.to_galaxy(DVec3::new(a.cos(), a.sin(), 0.0) * len)
            })
            .collect();
        for w in ring.windows(2) {
            proj.line(painter, w[0], w[1], Stroke::new(1.0, palette::AXIS_Z.gamma_multiply(0.6)));
        }
    }
    for (_, shape) in r.compiled.shapes.iter().filter(|(k, _)| k.feature == id) {
        boxed(painter, proj, shape.bounds(), Stroke::new(2.0, palette::PICK.gamma_multiply(0.8)));
    }
}

fn boxed(painter: &egui::Painter, proj: &Projector, b: Aabb, stroke: Stroke) {
    let c = |x: usize, y: usize, z: usize| {
        DVec3::new([b.min.x, b.max.x][x], [b.min.y, b.max.y][y], [b.min.z, b.max.z][z])
    };
    for (a, d) in [
        ((0, 0, 0), (1, 0, 0)), ((0, 1, 0), (1, 1, 0)), ((0, 0, 1), (1, 0, 1)), ((0, 1, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 1, 0)), ((1, 0, 0), (1, 1, 0)), ((0, 0, 1), (0, 1, 1)), ((1, 0, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 0, 1)), ((1, 0, 0), (1, 0, 1)), ((0, 1, 0), (0, 1, 1)), ((1, 1, 0), (1, 1, 1)),
    ] {
        proj.line(painter, c(a.0, a.1, a.2), c(d.0, d.1, d.2), stroke);
    }
}

/// A bar of a round length at the camera target's depth.
fn scale_bar(painter: &egui::Painter, rect: Rect, px_per_ly: f64) {
    let want = 120.0 / px_per_ly;
    let magnitude = 10f64.powf(want.log10().floor());
    let ly = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * magnitude).rfind(|l| *l <= want).unwrap_or(magnitude);
    let len = (ly * px_per_ly) as f32;
    let start = pos2(rect.left() + 16.0, rect.bottom() - 18.0);
    let color = Color32::from_gray(170);
    painter.line_segment([start, start + vec2(len, 0.0)], Stroke::new(1.5, color));
    for x in [0.0, len] {
        painter.line_segment([start + vec2(x, -4.0), start + vec2(x, 4.0)], Stroke::new(1.5, color));
    }
    painter.text(start + vec2(len + 6.0, 0.0), egui::Align2::LEFT_CENTER, format!("{ly} ly"), egui::FontId::proportional(12.0), color);
}

fn sky(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    response: &egui::Response,
    state: &mut ViewState,
    doc: &Doc,
    bounds: Aabb,
) {
    if response.dragged() {
        let d = response.drag_delta();
        let per_px = state.sky_fov / rect.height() as f64;
        state.look_yaw += d.x as f64 * per_px;
        state.look_pitch = (state.look_pitch + d.y as f64 * per_px).clamp(-1.55, 1.55);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y) as f64;
        state.sky_fov = (state.sky_fov * (-scroll * 0.002).exp()).clamp(5f64.to_radians(), 150f64.to_radians());
    }
    let (eye, own) = match doc.picked_star() {
        Some((i, s)) => (s.position_ly, Some(i)),
        None => (if bounds.is_empty() { DVec3::ZERO } else { bounds.center() }, None),
    };
    let (sy, cy) = state.look_yaw.sin_cos();
    let (sp, cp) = state.look_pitch.sin_cos();
    let proj = Projector::new(eye, DVec3::new(cp * cy, cp * sy, sp), state.sky_fov, rect);

    // The frame plane as a horizon.
    let horizon: Vec<Pos2> = (0..=360)
        .filter_map(|deg| {
            let a = (deg as f64).to_radians();
            proj.project(eye + DVec3::new(a.cos(), a.sin(), 0.0)).map(|(p, _)| p)
        })
        .collect();
    for w in horizon.windows(2) {
        if w[0].distance(w[1]) < rect.width() * 0.5 {
            painter.line_segment([w[0], w[1]], Stroke::new(1.0, palette::GRID));
        }
    }

    let reference = state.reference.as_ref().filter(|_| state.difference);
    let marks = compare::marks(&mut state.marks, doc, state.solo, reference);
    for (i, s) in doc.stars().iter().enumerate() {
        if Some(i) == own || s.attributes.luminosity_solar <= 0.0 || !marks.shown[i] {
            continue;
        }
        let d2 = s.position_ly.distance_squared(eye).max(1e-6);
        let Some((p, _)) = proj.project(s.position_ly) else { continue };
        if !rect.contains(p) {
            continue;
        }
        // Flux in L☉/ly²; a Sun at 1 ly is 1. Six decades of flux span the dot sizes.
        let b = (s.attributes.luminosity_solar / d2).log10();
        let r = (3.0 + 0.7 * b).clamp(1.5, 8.0) as f32;
        let alpha = ((b + 5.0) / 4.0).clamp(0.5, 1.0) as f32;
        let base = if reference.is_some() { marks.change[i].color() } else { star_color(s, state.color_by, doc) };
        let color = base.gamma_multiply(alpha);
        painter.add(Shape::circle_filled(p, r, color));
        if r > 3.5 {
            painter.circle_filled(p, r * 2.2, color.gamma_multiply(0.12));
        }
    }
    if let Some(r) = reference {
        for &i in &marks.gone {
            if let Some((p, _)) = proj.project(r.stars[i].position_ly).filter(|(p, _)| rect.contains(*p)) {
                painter.circle_stroke(p, 3.0, Stroke::new(1.5, palette::DIFF_GONE));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From each side, the axes land where `look_from` says: the side views with +Z up, the
    /// views along Z with +X to the right.
    #[test]
    fn each_side_puts_the_axes_where_it_says() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        // Screen offset of a unit step along `axis` from the target: x right, y up.
        let seen = |side: Side, axis: DVec3| {
            let mut cam = Camera { target: DVec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 50.0, fov: 0.8, ortho: false };
            cam.look_from(side);
            let proj = cam.projector(rect);
            let (p, _) = proj.project(axis).unwrap();
            let c = rect.center();
            (((p.x - c.x).signum()) as i32 * i32::from((p.x - c.x).abs() > 1.0), ((c.y - p.y).signum()) as i32 * i32::from((c.y - p.y).abs() > 1.0))
        };
        let (x, y, z) = (DVec3::X, DVec3::Y, DVec3::Z);
        let side = |s: &str| Side::named(s).unwrap();
        assert_eq!((seen(side("+z"), x), seen(side("+z"), y)), ((1, 0), (0, 1)));
        assert_eq!((seen(side("-z"), x), seen(side("-z"), y)), ((1, 0), (0, -1)));
        assert_eq!((seen(side("-y"), x), seen(side("-y"), z)), ((1, 0), (0, 1)));
        assert_eq!((seen(side("+y"), x), seen(side("+y"), z)), ((-1, 0), (0, 1)));
        assert_eq!((seen(side("+x"), y), seen(side("+x"), z)), ((1, 0), (0, 1)));
        assert_eq!((seen(side("-x"), y), seen(side("-x"), z)), ((-1, 0), (0, 1)));
    }
}
