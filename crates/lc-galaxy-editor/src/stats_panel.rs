//! What the field promises against what the bake gave, and the picked star.

use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, pos2, vec2};
use lc_galaxy::Target;

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
}

/// Bars over `[0, max]`, with the hard-core floor marked.
fn histogram(ui: &mut egui::Ui, values: &[f64], floor: Option<f64>) {
    const BINS: usize = 24;
    let max = values.iter().copied().fold(0.0, f64::max).max(1e-6) * 1.05;
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
        let mut row = |k: &str, v: String| {
            ui.label(k);
            ui.label(v);
            ui.end_row();
        };
        row("id", format!("{:016x}", s.id.get()));
        row("cell", format!("{} #{}", s.key.cell.0, s.key.index));
        let p = s.position_ly;
        row("position", format!("({:.3}, {:.3}, {:.3}) ly", p.x, p.y, p.z));
        row("population", a.population.map_or("none".into(), |p| doc.feature_name(p)));
        row("state", format!("{:?}", a.state));
        row("initial mass", format!("{:.3} M☉", a.initial_mass_solar));
        row("mass", format!("{:.3} M☉", a.mass_solar));
        row("age", format!("{:.3} Gyr", a.age_gyr));
        row("[Fe/H]", format!("{:+.2}", a.feh));
        row("luminosity", format!("{:.4} L☉", a.luminosity_solar));
        row("T_eff", format!("{:.0} K", a.teff_k));
        row("radius", format!("{:.4} R☉", a.radius_solar));
        let v = a.velocity_km_s;
        row("velocity", format!("({:.1}, {:.1}, {:.1}) km/s", v.x, v.y, v.z));
        row("exclusion", format!("{:.3} ly", s.exclusion_ly));
        if let Some(d) = r.nearest.get(index).filter(|d| d.is_finite()) {
            row("nearest", format!("{d:.3} ly"));
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
