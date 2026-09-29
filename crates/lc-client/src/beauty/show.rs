//! The square the photographs are shown in, and what is drawn over them.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use super::{Beauty, FADE_S};
use crate::app::Ui;

/// The square beside the map's, in the same row and the same size.
fn square(viewport: egui::Rect) -> egui::Rect {
    let corner = crate::map_panel::corner(viewport);
    let rect = corner.translate(egui::vec2(corner.width() + crate::map_panel::CORNER_INSET, 0.0));
    rect.intersect(viewport.shrink(crate::map_panel::CORNER_INSET))
}

/// Show the latest photograph, faded in over the one before it.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    time: Res<Time<Real>>,
    mut beauty: ResMut<Beauty>,
) {
    if !ui_state.beauty_shots {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let rect = square(ctx.viewport_rect());
    if !rect.is_positive() {
        return;
    }
    beauty.side_px = (rect.height() * ctx.pixels_per_point()).round().max(1.0) as u32;
    let now = time.elapsed_secs();
    let text = crate::map_panel::color_of(em_ui::vfd::TEXT);
    let dim = crate::map_panel::color_of(em_ui::vfd::TEXT_DIM);
    let caption = match (&beauty.front, beauty.idle) {
        (None, true) => "nothing to photograph".to_string(),
        (None, false) => "focusing".to_string(),
        (Some(_), _) => beauty.caption.clone(),
    };
    egui::Area::new(egui::Id::new("beauty shot"))
        // Middle, as the map's square is: an open window covers it.
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.allocate_rect(rect, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
            let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            if let Some((front, since)) = beauty.front {
                let t = ((now - since) / FADE_S).clamp(0.0, 1.0);
                if t < 1.0 && beauty.taken > 1 {
                    painter.image(beauty.textures[1 - front], rect, whole, egui::Color32::WHITE);
                }
                painter.image(beauty.textures[front], rect, whole,
                    egui::Color32::WHITE.gamma_multiply(t));
                if let Some(chroma) = beauty.spikes {
                    spikes(painter, rect, chroma, t);
                }
            }
            let font = egui::TextStyle::Small.resolve(ui.style());
            let galley = painter.layout_no_wrap(caption, font, text);
            let at = if beauty.caption_top && beauty.front.is_some() {
                rect.left_top() + egui::vec2(4.0, 4.0)
            } else {
                rect.left_bottom() + egui::vec2(4.0, -4.0 - galley.size().y)
            };
            painter.rect_filled(
                egui::Rect::from_min_size(at, galley.size()).expand(2.0),
                0.0,
                egui::Color32::from_black_alpha(160),
            );
            painter.galley(at, galley, text);
            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, dim), egui::StrokeKind::Inside);
        });
    // Repaint through the fade; nothing else asks for a frame while the ship is still.
    if beauty.front.is_some_and(|(_, since)| now - since < FADE_S) {
        ctx.request_repaint();
    }
}

/// How far a diffraction spike reaches, as a fraction of the square's side, and how wide it is
/// at the star, in points. The glow round the star is in points too.
const SPIKE_REACH: f32 = 0.45;
const SPIKE_WIDTH: f32 = 1.2;
const GLOW_RADIUS: f32 = 5.0;

/// Four spikes and a glow at the middle of the frame. Drawn over the photograph because the
/// starfield's glare is round by design; the spikes belong to this instrument.
fn spikes(painter: &egui::Painter, rect: egui::Rect, chroma: Vec3, alpha: f32) {
    let c = chroma.clamp(Vec3::ZERO, Vec3::ONE);
    let color = |a: f32| {
        egui::Color32::from_rgb((c.x * 255.0) as u8, (c.y * 255.0) as u8, (c.z * 255.0) as u8)
            .gamma_multiply(a * alpha)
    };
    let middle = rect.center();
    let reach = rect.width() * SPIKE_REACH;
    let mut mesh = egui::Mesh::default();
    for k in 0..4 {
        let angle = k as f32 * std::f32::consts::FRAC_PI_2;
        let along = egui::vec2(angle.cos(), angle.sin());
        let across = egui::vec2(-along.y, along.x) * SPIKE_WIDTH * 0.5;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(middle + across, color(0.7));
        mesh.colored_vertex(middle - across, color(0.7));
        mesh.colored_vertex(middle + along * reach, color(0.0));
        mesh.add_triangle(base, base + 1, base + 2);
    }
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(middle, color(0.9));
    const SEGMENTS: u32 = 16;
    for k in 0..SEGMENTS {
        let angle = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(middle + egui::vec2(angle.cos(), angle.sin()) * GLOW_RADIUS, color(0.0));
        mesh.add_triangle(center, center + 1 + k, center + 1 + (k + 1) % SEGMENTS);
    }
    painter.add(egui::Shape::mesh(mesh));
}
