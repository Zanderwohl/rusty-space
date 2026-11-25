//! The picked star's system, generated on a worker thread only while this section is open.

use std::sync::{Arc, OnceLock};

use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, pos2, vec2};
use lc_galaxy::StarId;
use lc_world::sky::galaxy::{GalaxyRecord, catalog_star};
use lc_world::sky::generate::listing::{Entry, Kind, Listing, listing};
use lc_world::sky::generate::{Class, SystemProfile, system_with};

use crate::doc::Doc;
use crate::palette;

/// What a system is generated from. Kinematics only move the star, so they are left out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    id: StarId,
    record: GalaxyRecord,
    profile: SystemProfile,
}

/// `None`: the star is a remnant the sky leaves out, so it has no system.
type Outcome = Option<Listing>;

#[derive(Default)]
pub enum Job {
    #[default]
    NotDispatched,
    /// The worker fills the slot; a `Resource` must be `Sync`, which a channel receiver is not.
    Waiting { key: Key, results: Arc<OnceLock<Outcome>> },
    Ready { key: Key, results: Arc<Outcome> },
}

impl Job {
    fn key(&self) -> Option<Key> {
        match self {
            Job::NotDispatched => None,
            Job::Waiting { key, .. } | Job::Ready { key, .. } => Some(*key),
        }
    }

    /// Moves a finished job to `Ready`. A worker that died is retried on the next frame.
    fn poll(&mut self) {
        let Job::Waiting { key, results } = self else { return };
        if let Some(outcome) = results.get() {
            *self = Job::Ready { key: *key, results: Arc::new(outcome.clone()) };
        } else if Arc::strong_count(results) == 1 {
            *self = Job::NotDispatched;
        }
    }
}

fn dispatch(doc: &Doc, key: Key) -> Option<Job> {
    let (_, s) = doc.picked_star()?;
    let compiled = &doc.result.as_ref()?.compiled;
    let kinematics = lc_galaxy::attributes::kinematics(compiled, s.attributes.population?)?;
    let slot = Arc::new(OnceLock::new());
    let filled = slot.clone();
    let spawned = std::thread::Builder::new().name("system".into()).spawn(move || {
        let outcome = catalog_star(&key.record, &kinematics, key.profile).map(|star| {
            let tuning = key.profile.tuning();
            listing(&system_with(&star, &tuning), &tuning)
        });
        let _ = filled.set(outcome);
    });
    spawned.ok().map(|_| Job::Waiting { key, results: slot })
}

pub fn show(ui: &mut egui::Ui, job: &mut Job, doc: &Doc) {
    let Some((_, s)) = doc.picked_star() else {
        *job = Job::NotDispatched;
        return;
    };
    let key = Key { id: s.id, record: s.attributes.record, profile: s.attributes.profile };
    if job.key() != Some(key) {
        *job = dispatch(doc, key).unwrap_or_default();
    }
    job.poll();
    match job {
        Job::NotDispatched => {
            ui.label("no system: the star has no population");
        }
        Job::Waiting { .. } => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("generating");
            });
            ui.ctx().request_repaint();
        }
        Job::Ready { results, .. } => match results.as_ref() {
            None => {
                ui.label("no system: a neutron star or black hole");
            }
            Some(l) => {
                strip(ui, l);
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("system list").max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                    if l.entries.is_empty() {
                        ui.label("nothing orbits it");
                    }
                    for e in &l.entries {
                        entry(ui, e);
                    }
                });
            }
        },
    }
}

fn color(kind: Kind) -> Color32 {
    match kind {
        Kind::Planet(Class::Rocky) => Color32::from_rgb(200, 150, 110),
        Kind::Planet(Class::Icy) => Color32::from_rgb(190, 220, 240),
        Kind::Planet(Class::IceGiant) => Color32::from_rgb(100, 160, 230),
        Kind::Planet(Class::GasGiant) => Color32::from_rgb(230, 190, 120),
        Kind::Belt | Kind::OuterDisc => Color32::from_gray(150),
        Kind::OortCloud => Color32::from_gray(110),
        Kind::Swarm => palette::READ_BY,
    }
}

/// One body or population.
fn entry(ui: &mut egui::Ui, e: &Entry) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            // The star's name is the section's; a planet is its letter.
            let short = e.name.rsplit(' ').next().unwrap_or(&e.name);
            ui.label(RichText::new(if e.world.is_some() { short } else { &e.name }).strong());
            ui.label(RichText::new(e.kind.label()).color(color(e.kind)));
            if let Some(w) = &e.world {
                if w.ocean_first {
                    ui.label(RichText::new("ocean-first").color(palette::PICK));
                }
                if w.migrated {
                    ui.label(RichText::new("migrated").color(palette::KIND));
                }
            }
        });
        egui::Grid::new(("entry", &e.name)).num_columns(4).spacing(vec2(12.0, 2.0)).show(ui, |ui| {
            let orbit = match e.kind {
                Kind::Planet(_) => format!("{:.3} AU", e.inner_au),
                _ => format!("{:.3} to {:.3} AU", e.inner_au, e.outer_au),
            };
            ui.label("orbit");
            ui.label(orbit);
            ui.label("mass");
            ui.label(e.mass_earths.map_or("unknown".into(), |m| format!("{m:.3e} Earths")));
            ui.end_row();
            if let Some(w) = &e.world {
                ui.label("water");
                ui.label(if w.water_fraction > 0.0 { format!("{:.2e}", w.water_fraction) } else { "none".into() });
                ui.label("habitable");
                ui.label(if w.habitable { "yes" } else { "no" });
                ui.end_row();
                ui.label("life");
                ui.label(format!("{:.2}", w.life));
                ui.label("moons");
                ui.label(w.moons.to_string());
                ui.end_row();
            }
        });
    });
}

/// The system on a log-AU line: habitable zone, snow line, belts as bands and planets as dots
/// sized by mass. The Oort cloud is left off; it would push everything else into a corner.
fn strip(ui: &mut egui::Ui, l: &Listing) {
    let z = &l.zones;
    let shown = || l.entries.iter().filter(|e| e.kind != Kind::OortCloud);
    let lo = shown().map(|e| e.inner_au).fold(z.inner_au, f64::min).max(1e-4) / 1.3;
    let hi = shown().map(|e| e.outer_au).fold(z.outer_au, f64::max) * 1.3;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 80.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, palette::VIEW_BACKGROUND);
    let plot = rect.shrink2(vec2(10.0, 14.0));
    let x = |au: f64| plot.left() + ((au.ln() - lo.ln()) / (hi.ln() - lo.ln())) as f32 * plot.width();
    let mid = plot.center().y;

    let hz = egui::Rect::from_x_y_ranges(x(z.habitable_au.0)..=x(z.habitable_au.1), plot.y_range());
    painter.rect_filled(hz, 0.0, Color32::from_rgba_unmultiplied(60, 160, 80, 50));
    painter.line_segment([pos2(x(z.snow_au), plot.top()), pos2(x(z.snow_au), plot.bottom())], Stroke::new(1.0, palette::READS));
    painter.line_segment([pos2(x(z.inner_au), mid), pos2(x(z.outer_au), mid)], Stroke::new(1.0, palette::GRID));
    for e in shown().filter(|e| e.world.is_none()) {
        let band = egui::Rect::from_x_y_ranges(x(e.inner_au)..=x(e.outer_au), (mid - 6.0)..=(mid + 6.0));
        painter.rect_filled(band, 1.0, color(e.kind).gamma_multiply(0.35));
    }
    let mut hovered: Option<(f32, &Entry)> = None;
    for e in shown().filter(|e| e.world.is_some()) {
        let mass = e.mass_earths.unwrap_or(0.0);
        let radius = (2.5 + 2.2 * (1.0 + mass).log10()).clamp(2.5, 10.0) as f32;
        let at = pos2(x(e.inner_au), mid);
        painter.circle_filled(at, radius, color(e.kind));
        if e.world.as_ref().is_some_and(|w| w.ocean_first) {
            painter.circle_stroke(at, radius + 2.5, Stroke::new(1.5, palette::PICK));
        }
        if let Some(p) = response.hover_pos() {
            let d = (p.x - at.x).abs();
            if d < radius + 4.0 && hovered.is_none_or(|(best, _)| d < best) {
                hovered = Some((d, e));
            }
        }
    }

    let label = Color32::from_gray(150);
    let font = egui::FontId::proportional(10.0);
    let mut decade = 10f64.powf(lo.log10().ceil());
    while decade <= hi {
        let text = if decade >= 1.0 { format!("{decade:.0}") } else { format!("{decade}") };
        painter.text(pos2(x(decade), rect.bottom() - 2.0), egui::Align2::CENTER_BOTTOM, text, font.clone(), label);
        decade *= 10.0;
    }
    painter.text(pos2(rect.right() - 4.0, rect.top() + 2.0), egui::Align2::RIGHT_TOP, "AU", font, label);
    if let Some((_, e)) = hovered {
        response.on_hover_text(format!("{} ({}), {:.3} AU", e.name, e.kind.label(), e.inner_au));
    }
}
