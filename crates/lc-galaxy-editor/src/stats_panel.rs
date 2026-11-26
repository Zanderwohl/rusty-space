//! What the field promises against what the bake gave, and the picked star.

use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, pos2, vec2};
use lc_galaxy::{FeatureId, Target};
use lc_world::sky::generate::census::Census;

use em_ui::egui_text::{Unit, quantity};

use lc_world::sky::shells;

use crate::census::CensusJob;
use crate::doc::Doc;
use crate::palette;

pub fn statistics(ui: &mut egui::Ui, doc: &Doc) {
    let Some(r) = &doc.result else { return };
    let bake = r.bake.as_ref().ok();
    egui::Grid::new("counts").num_columns(3).striped(true).show(ui, |ui| {
        ui.label(RichText::new("population").strong());
        ui.label(RichText::new("expected").strong());
        ui.label(RichText::new("baked").strong());
        ui.end_row();
        let mut totals = (0.0, 0usize);
        for (id, expected) in &r.expected {
            let baked = bake.map_or(0, |b| b.stars.iter().filter(|s| s.attributes.population == Some(*id)).count());
            ui.label(doc.feature_name(*id));
            ui.label(format!("{expected:.1}"));
            ui.label(baked.to_string());
            ui.end_row();
            totals = (totals.0 + expected, totals.1 + baked);
        }
        ui.label("total");
        ui.label(format!("{:.1}", totals.0));
        ui.label(totals.1.to_string());
        ui.end_row();
    });
    if let Some(b) = bake {
        ui.label(format!("{} cells, {} candidates, {:.1} ms", b.cells, b.candidates, r.took.as_secs_f64() * 1e3));
    }
    let finite: Vec<f64> = r.nearest.iter().copied().filter(|d| d.is_finite()).collect();
    if finite.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("nearest neighbor").strong());
    let floor = bake.map(|b| b.stars.iter().map(|s| s.exclusion_ly).fold(f64::INFINITY, f64::min));
    histogram(ui, &finite, floor);
    let mean = finite.iter().sum::<f64>() / finite.len() as f64;
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    ui.label(format!("min {min:.2} ly, mean {mean:.2} ly, hard-core floor {:.2} ly", floor.unwrap_or(0.0)));

    let Some(b) = bake else { return };
    let shells: Vec<(f64, f64)> = r
        .shells
        .iter()
        .zip(&b.stars)
        .filter(|(r, _)| r.is_finite())
        .map(|(r, s)| (*r, shells::nominal_ly(s.attributes.mass_solar)))
        .collect();
    if shells.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("shell radius").strong());
    let radii: Vec<f64> = shells.iter().map(|(r, _)| *r).collect();
    histogram(ui, &radii, Some(shells::SOLAR_SHELL_LY));
    // A clamped shell is the proportional share of a gap, never more than its nominal radius.
    let clamped = shells.iter().filter(|(r, nominal)| *r < nominal * (1.0 - 1e-9)).count();
    let mean = radii.iter().sum::<f64>() / radii.len() as f64;
    ui.label(format!(
        "clamped {clamped} of {} ({:.0}%), mean {mean:.2} ly, solar nominal {:.2} ly",
        shells.len(),
        100.0 * clamped as f64 / shells.len() as f64,
        shells::SOLAR_SHELL_LY
    ));
}

/// What the generator made of the baked stars' systems, per population and in total.
pub fn systems(ui: &mut egui::Ui, census: &CensusJob, doc: &Doc) {
    let (Some(r), Some((_, outcome))) = (&doc.result, census.latest()) else {
        if census.running() {
            ui.spinner();
            ui.ctx().request_repaint();
        }
        return;
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("systems").strong());
        if census.running() {
            ui.spinner();
            ui.ctx().request_repaint();
        }
    });
    let counted = match outcome {
        Ok(c) => c,
        Err(why) => {
            ui.label(format!("no census: {why}"));
            return;
        }
    };
    let of = |id: Option<FeatureId>| -> Vec<&Census> {
        counted.iter().filter(|c| id.is_none() || c.population == id).filter_map(|c| c.census.as_ref()).collect()
    };
    let mut columns: Vec<(String, Vec<&Census>)> = r.compiled.populations.keys().map(|id| (doc.feature_name(*id), of(Some(*id)))).collect();
    columns.push(("total".into(), of(None)));

    type Measure = fn(&[&Census]) -> String;
    fn per_star(cs: &[&Census], f: fn(&Census) -> u32) -> String {
        if cs.is_empty() { "–".into() } else { format!("{:.2}", cs.iter().map(|c| f(c) as f64).sum::<f64>() / cs.len() as f64) }
    }
    fn share(cs: &[&Census], f: fn(&Census) -> bool) -> String {
        if cs.is_empty() { "–".into() } else { format!("{:.0}%", 100.0 * cs.iter().filter(|c| f(c)).count() as f64 / cs.len() as f64) }
    }
    let rows: [(&str, Measure); 9] = [
        ("systems", |cs| cs.len().to_string()),
        ("planets / star", |cs| per_star(cs, |c| c.planets)),
        ("giants / star", |cs| per_star(cs, |c| c.giants)),
        ("with a belt", |cs| share(cs, |c| c.belt)),
        ("with an Oort cloud", |cs| share(cs, |c| c.oort_cloud)),
        ("habitable / star", |cs| per_star(cs, |c| c.habitable)),
        ("ocean worlds", |cs| cs.iter().map(|c| c.ocean_worlds).sum::<u32>().to_string()),
        ("eligible", |cs| cs.iter().filter(|c| c.eligible).count().to_string()),
        ("ocean-first", |cs| cs.iter().filter(|c| c.ocean_first).count().to_string()),
    ];
    egui::Grid::new("systems").num_columns(columns.len() + 1).striped(true).show(ui, |ui| {
        ui.label("");
        for (name, _) in &columns {
            ui.label(RichText::new(name).strong());
        }
        ui.end_row();
        for (label, measure) in rows {
            ui.label(label);
            for (_, cs) in &columns {
                ui.label(measure(cs));
            }
            ui.end_row();
        }
    });
}

/// Bars over `[0, max]`, with one value marked: the hard-core floor, or the solar shell.
fn histogram(ui: &mut egui::Ui, values: &[f64], floor: Option<f64>) {
    const BINS: usize = 24;
    let marked = floor.filter(|f| f.is_finite()).unwrap_or(0.0);
    let max = values.iter().copied().fold(marked, f64::max).max(1e-6) * 1.05;
    let mut counts = [0usize; BINS];
    for v in values {
        counts[((v / max * BINS as f64) as usize).min(BINS - 1)] += 1;
    }
    let peak = *counts.iter().max().unwrap_or(&1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 90.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, palette::VIEW_BACKGROUND);
    let plot = rect.shrink2(vec2(4.0, 14.0));
    let w = plot.width() / BINS as f32;
    for (i, c) in counts.iter().enumerate() {
        let h = plot.height() * *c as f32 / peak;
        let x = plot.left() + i as f32 * w;
        painter.rect_filled(egui::Rect::from_min_max(pos2(x + 1.0, plot.bottom() - h), pos2(x + w - 1.0, plot.bottom())), 0.0, palette::HISTOGRAM);
    }
    if let Some(f) = floor.filter(|f| f.is_finite()) {
        let x = plot.left() + (f / max) as f32 * plot.width();
        painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.5, palette::PICK));
    }
    let label = Color32::from_gray(150);
    painter.text(pos2(plot.left(), rect.bottom() - 2.0), egui::Align2::LEFT_BOTTOM, "0", egui::FontId::proportional(10.0), label);
    painter.text(pos2(plot.right(), rect.bottom() - 2.0), egui::Align2::RIGHT_BOTTOM, format!("{max:.1} ly"), egui::FontId::proportional(10.0), label);
}

pub fn inspector(ui: &mut egui::Ui, doc: &Doc) {
    let Some((index, s)) = doc.picked_star() else { return };
    let Some(r) = &doc.result else { return };
    let a = &s.attributes;
    egui::Grid::new("inspector").num_columns(2).striped(true).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.label(k);
            ui.label(v);
            ui.end_row();
        };
        let solar = |ui: &mut egui::Ui, k: &str, v: String, unit: Unit| {
            ui.label(k);
            quantity(ui, &v, unit);
            ui.end_row();
        };
        row(ui, "id", format!("{:016x}", s.id.get()));
        row(ui, "cell", format!("{} #{}", s.key.cell.0, s.key.index));
        let p = s.position_ly;
        row(ui, "position", format!("({:.3}, {:.3}, {:.3}) ly", p.x, p.y, p.z));
        row(ui, "population", a.population.map_or("none".into(), |p| doc.feature_name(p)));
        row(ui, "state", format!("{:?}", a.state));
        solar(ui, "initial mass", format!("{:.3}", a.initial_mass_solar), Unit::SolarMass);
        solar(ui, "mass", format!("{:.3}", a.mass_solar), Unit::SolarMass);
        row(ui, "age", format!("{:.3} Gyr", a.age_gyr));
        row(ui, "[Fe/H]", format!("{:+.2}", a.feh));
        solar(ui, "luminosity", format!("{:.4}", a.luminosity_solar), Unit::SolarLuminosity);
        row(ui, "T_eff", format!("{:.0} K", a.teff_k));
        solar(ui, "radius", format!("{:.4}", a.radius_solar), Unit::SolarRadius);
        let v = a.velocity_km_s;
        row(ui, "velocity", format!("({:.1}, {:.1}, {:.1}) km/s", v.x, v.y, v.z));
        row(ui, "exclusion", format!("{:.3} ly", s.exclusion_ly));
        if let Some(d) = r.nearest.get(index).filter(|d| d.is_finite()) {
            row(ui, "nearest", format!("{d:.3} ly"));
        }
        if let Some(shell) = r.shells.get(index).filter(|d| d.is_finite()) {
            row(ui, "shell", format!("{shell:.3} ly of {:.3} nominal", shells::nominal_ly(a.mass_solar)));
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("fields here").strong());
    egui::Grid::new("contributions").num_columns(3).striped(true).show(ui, |ui| {
        for c in r.compiled.contributions(s.position_ly) {
            ui.label(doc.feature_name(c.source));
            ui.label(match c.target {
                Target::Density(p) => format!("density of {}", doc.feature_name(p)),
                Target::FeH(p) => format!("[Fe/H] of {}", doc.feature_name(p)),
                Target::Exclusion => "packing".into(),
                Target::System(f) => format!("system: {}", f.label()),
            });
            ui.label(format!("{:.4} to {:.4}", c.before, c.after));
            ui.end_row();
        }
    });
}
