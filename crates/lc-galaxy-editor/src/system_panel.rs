//! The picked star's system, generated on a worker thread only while this section is open.

use std::sync::{Arc, OnceLock};

use bevy_egui::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use lc_galaxy::StarId;
use lc_world::sky::galaxy::{GalaxyRecord, catalog_star};
use lc_world::population::Kind as PopulationKind;
use lc_world::sky::generate::listing::{Entry, Kind, Listing, listing};
use lc_world::sky::generate::{Class, SystemProfile, system_with};

use em_ui::egui_text::{Unit, quantity, raised};
use em_ui::number::scientific;

use crate::doc::Doc;
use crate::palette;

/// What a system is generated from. Kinematics only move the star, so they are left out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    id: StarId,
    record: GalaxyRecord,
    profile: SystemProfile,
}

/// `Err` says why the star has no system to show.
type Outcome = Result<Listing, &'static str>;

#[derive(Default)]
pub enum Job {
    #[default]
    NotDispatched,
    /// The worker fills the slot; a `Resource` must be `Sync`, which a channel receiver is not.
    Waiting { key: Key, results: Arc<OnceLock<Outcome>> },
    /// Kept until the pick or its record changes, so a refused or failed star is not retried.
    Ready { key: Key, results: Arc<Outcome> },
}

impl Job {
    fn key(&self) -> Option<Key> {
        match self {
            Job::NotDispatched => None,
            Job::Waiting { key, .. } | Job::Ready { key, .. } => Some(*key),
        }
    }

    fn poll(&mut self) {
        let Job::Waiting { key, results } = self else { return };
        if let Some(outcome) = results.get() {
            *self = Job::Ready { key: *key, results: Arc::new(outcome.clone()) };
        }
    }
}

fn dispatch(doc: &Doc, key: Key) -> Job {
    let refused = |why| Job::Ready { key, results: Arc::new(Err(why)) };
    let Some((_, s)) = doc.picked_star() else { return refused("no star picked") };
    let Some(result) = &doc.result else { return refused("nothing baked") };
    let Some(population) = s.attributes.population else { return refused("the star has no population") };
    let Some(kinematics) = lc_galaxy::attributes::kinematics(&result.compiled, population) else {
        return refused("its population has no frame");
    };
    let slot = Arc::new(OnceLock::new());
    let filled = slot.clone();
    let spawned = std::thread::Builder::new().name("system".into()).spawn(move || {
        // Caught so the slot is always filled: an empty slot would leave the section waiting.
        let outcome = std::panic::catch_unwind(|| {
            let star = catalog_star(&key.record, &kinematics, key.profile).ok_or("the sky leaves this star out")?;
            let tuning = key.profile.tuning();
            Ok(listing(&system_with(&star, &tuning), &tuning))
        })
        .unwrap_or(Err("generating it panicked"));
        let _ = filled.set(outcome);
    });
    match spawned {
        Ok(_) => Job::Waiting { key, results: slot },
        Err(_) => refused("no worker thread could be started"),
    }
}

/// The section's state: the generation job and which of its entries is selected.
#[derive(Default)]
pub struct SystemView {
    job: Job,
    /// Index into the listing's entries.
    selected: Option<usize>,
}

pub fn show(ui: &mut egui::Ui, view: &mut SystemView, doc: &Doc) {
    let Some((_, s)) = doc.picked_star() else {
        *view = SystemView::default();
        return;
    };
    let key = Key { id: s.id, record: s.attributes.record, profile: s.attributes.profile };
    if view.job.key() != Some(key) {
        view.job = dispatch(doc, key);
        view.selected = None;
    }
    view.job.poll();
    match &view.job {
        Job::NotDispatched => {}
        Job::Waiting { .. } => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("generating");
            });
            ui.ctx().request_repaint();
        }
        Job::Ready { results, .. } => match results.as_ref() {
            Err(why) => {
                ui.label(format!("no system: {why}"));
            }
            Ok(l) => {
                let picked_on_strip = strip(ui, l, &mut view.selected);
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("system list").min_scrolled_height(600.0).max_height(600.0).auto_shrink([false, true]).show(ui, |ui| {
                    if l.entries.is_empty() {
                        ui.label("nothing orbits it");
                    }
                    for (i, e) in l.entries.iter().enumerate() {
                        let selected = view.selected == Some(i);
                        let response = entry(ui, e, selected);
                        if response.clicked() {
                            view.selected = if selected { None } else { Some(i) };
                        }
                        if picked_on_strip && selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
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
        Kind::Population(PopulationKind::Belt | PopulationKind::OuterDisc | PopulationKind::Ring) => Color32::from_gray(150),
        Kind::Population(PopulationKind::OortCloud) => Color32::from_gray(110),
        Kind::Population(PopulationKind::Swarm) => palette::READ_BY,
    }
}

/// "Planet C" for a planet, whose full name repeats the star's; a population's own name otherwise.
fn display_name(e: &Entry) -> String {
    if e.world.is_some() {
        let letter = e.name.rsplit(' ').next().unwrap_or(&e.name);
        return format!("Planet {}", letter.to_uppercase());
    }
    let mut chars = e.name.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn entry(ui: &mut egui::Ui, e: &Entry, selected: bool) -> egui::Response {
    let mut frame = egui::Frame::group(ui.style());
    if selected {
        frame = frame.stroke(Stroke::new(1.5, palette::PICK));
    }
    // The scope senses the click before its children are added, so they stay on top for their
    // own hover text; labels stop being selectable so a click on text reaches the scope.
    ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
        ui.style_mut().interaction.selectable_labels = false;
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(display_name(e)).strong());
                ui.label(RichText::new(e.kind.label()).color(color(e.kind)));
                if let Some(w) = &e.world {
                    if w.ocean_first {
                        ui.label(RichText::new("ocean-first").color(palette::READS));
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
                match e.mass_earths {
                    Some(m) => quantity(ui, &scientific(m, 3), Unit::EarthMass),
                    None => ui.label("unknown"),
                };
                ui.end_row();
                if let Some(w) = &e.world {
                    ui.label("water");
                    ui.label(raised(ui.style(), &if w.water_fraction > 0.0 { scientific(w.water_fraction, 2) } else { "none".into() }, None));
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
    })
    .response
}

enum Mark {
    Dot { at: Pos2, radius: f32 },
    Band(Rect),
}

/// The system on a log-AU line: habitable zone, snow line, belts as bands and planets as dots
/// sized by mass. The Oort cloud is left off; it would push everything else into a corner.
/// Clicking picks the entry under the pointer, or clears the pick; returns whether it did.
fn strip(ui: &mut egui::Ui, l: &Listing, selected: &mut Option<usize>) -> bool {
    let z = &l.zones;
    let shown = || l.entries.iter().enumerate().filter(|(_, e)| e.kind != Kind::OORT_CLOUD);
    let lo = shown().map(|(_, e)| e.inner_au).fold(z.inner_au, f64::min).max(1e-4) / 1.3;
    let hi = shown().map(|(_, e)| e.outer_au).fold(z.outer_au, f64::max) * 1.3;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 80.0), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, palette::VIEW_BACKGROUND);
    let plot = rect.shrink2(vec2(10.0, 14.0));
    let x = |au: f64| plot.left() + ((au.ln() - lo.ln()) / (hi.ln() - lo.ln())) as f32 * plot.width();
    let mid = plot.center().y;

    let hz = Rect::from_x_y_ranges(x(z.habitable_au.0)..=x(z.habitable_au.1), plot.y_range());
    painter.rect_filled(hz, 0.0, Color32::from_rgba_unmultiplied(60, 160, 80, 50));
    painter.line_segment([pos2(x(z.snow_au), plot.top()), pos2(x(z.snow_au), plot.bottom())], Stroke::new(1.0, palette::READS));
    painter.line_segment([pos2(x(z.inner_au), mid), pos2(x(z.outer_au), mid)], Stroke::new(1.0, palette::GRID));

    // Bands first, so dots draw over them.
    let mut marks: Vec<(usize, Mark)> = shown()
        .filter(|(_, e)| e.world.is_none())
        .map(|(i, e)| (i, Mark::Band(Rect::from_x_y_ranges(x(e.inner_au)..=x(e.outer_au), (mid - 6.0)..=(mid + 6.0)))))
        .collect();
    marks.extend(shown().filter(|(_, e)| e.world.is_some()).map(|(i, e)| {
        let radius = (2.5 + 2.2 * (1.0 + e.mass_earths.unwrap_or(0.0)).log10()).clamp(2.5, 10.0) as f32;
        (i, Mark::Dot { at: pos2(x(e.inner_au), mid), radius })
    }));
    let pick = Stroke::new(1.5, palette::PICK);
    for (i, mark) in &marks {
        let e = &l.entries[*i];
        match *mark {
            Mark::Band(band) => {
                painter.rect_filled(band, 1.0, color(e.kind).gamma_multiply(0.35));
                if *selected == Some(*i) {
                    painter.rect_stroke(band.expand(2.0), 2.0, pick, egui::StrokeKind::Outside);
                }
            }
            Mark::Dot { at, radius } => {
                painter.circle_filled(at, radius, color(e.kind));
                if e.world.as_ref().is_some_and(|w| w.ocean_first) {
                    painter.circle_stroke(at, radius + 2.5, Stroke::new(1.5, palette::READS));
                }
                if *selected == Some(*i) {
                    painter.circle_stroke(at, radius + 5.0, pick);
                }
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

    let hovered = response.hover_pos().and_then(|p| under(&marks, p).map(|i| (p, i)));
    if let Some((p, i)) = hovered {
        pointer_label(ui.ctx(), p, &display_name(&l.entries[i]));
    }
    if response.clicked() {
        *selected = hovered.map(|(_, i)| i);
        return true;
    }
    false
}

/// The entry under `p`: the nearest dot within reach, else the narrowest band.
fn under(marks: &[(usize, Mark)], p: Pos2) -> Option<usize> {
    const REACH_PX: f32 = 4.0;
    let dot = marks
        .iter()
        .filter_map(|(i, m)| match m {
            Mark::Dot { at, radius } => Some((at.distance(p), *radius, *i)),
            Mark::Band(_) => None,
        })
        .filter(|(d, radius, _)| *d <= radius + REACH_PX)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, _, i)| i);
    dot.or_else(|| {
        marks
            .iter()
            .filter_map(|(i, m)| match m {
                Mark::Band(band) if band.expand(REACH_PX).contains(p) => Some((band.width(), *i)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    })
}

/// `text` in a box beside the pointer: above and to the left if it fits on screen, else the first
/// of the other three corners that does. The gap keeps the box out from under the pointer.
fn pointer_label(ctx: &egui::Context, pointer: Pos2, text: &str) {
    const GAP_PX: f32 = 12.0;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("system strip label")));
    let color = Color32::from_gray(230);
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(13.0), color);
    let pad = vec2(5.0, 3.0);
    let size = galley.size() + 2.0 * pad;
    let corners = [
        vec2(-GAP_PX - size.x, -GAP_PX - size.y),
        vec2(GAP_PX, -GAP_PX - size.y),
        vec2(-GAP_PX - size.x, GAP_PX),
        vec2(GAP_PX, GAP_PX),
    ];
    let bounds = ctx.viewport_rect();
    let at = |offset: Vec2| Rect::from_min_size(pointer + offset, size);
    let rect = corners.iter().map(|o| at(*o)).find(|r| bounds.contains_rect(*r)).unwrap_or_else(|| {
        // Nowhere fits whole, as in a window narrower than the label: keep the first on screen.
        let r = at(corners[0]);
        let shift = vec2(
            (bounds.left() - r.left()).max(0.0) + (bounds.right() - r.right()).min(0.0),
            (bounds.top() - r.top()).max(0.0) + (bounds.bottom() - r.bottom()).min(0.0),
        );
        r.translate(shift)
    });
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(210));
    painter.galley(rect.min + pad, galley, color);
}
