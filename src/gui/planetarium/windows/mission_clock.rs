use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use chrono::{DateTime, Utc};

use crate::gui::style::vfd;
use crate::sim::SimTime;

const UNIX_EPOCH_AT_J2000_UTC: f64 = 946_728_000.0;
const SIDECAR_GAP: f32 = 8.0;
const SIDECAR_WIDTH: f32 = 150.0;
const SIDECAR_BUTTON_WIDTH: f32 = 26.0;
const THROTTLE_ARROW_COUNT: usize = 8;
const THROTTLE_ARROW_WIDTH_SCALE: f32 = 0.8;
const THROTTLE_ARROW_HEIGHT_SCALE: f32 = 0.25;
const THROTTLE_GROUP_PAD_X: f32 = 6.0;
const THROTTLE_MATCH_RATIO: f64 = 0.95;

const THROTTLE_SPEEDS: [f64; THROTTLE_ARROW_COUNT] = [
    1.0,      // 1 s/s
    10.0,     // 10 s/s
    60.0,     // 1 min/s
    3600.0,    // 1 hour/s
    86400.0,   // 1 day/s
    604800.0,  // 1 week/s
    2592000.0, // 1 month/s
    31536000.0, // 1 year/s
];

#[derive(Default, Clone, Copy)]
pub(crate) enum MissionClockMode {
    #[default]
    JulianDay,
    Utc,
}

impl MissionClockMode {
    fn toggle(&mut self) {
        *self = match self {
            Self::JulianDay => Self::Utc,
            Self::Utc => Self::JulianDay,
        };
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeThrottleLevel {
    Level1,
    Level2,
    Level3,
    Level4,
    Level5,
    Level6,
    Level7,
    Level8,
    #[default]
    Other,
}

impl TimeThrottleLevel {
    fn to_speed(self) -> Option<f64> {
        match self {
            Self::Level1 => Some(THROTTLE_SPEEDS[0]),
            Self::Level2 => Some(THROTTLE_SPEEDS[1]),
            Self::Level3 => Some(THROTTLE_SPEEDS[2]),
            Self::Level4 => Some(THROTTLE_SPEEDS[3]),
            Self::Level5 => Some(THROTTLE_SPEEDS[4]),
            Self::Level6 => Some(THROTTLE_SPEEDS[5]),
            Self::Level7 => Some(THROTTLE_SPEEDS[6]),
            Self::Level8 => Some(THROTTLE_SPEEDS[7]),
            Self::Other => None,
        }
    }

    fn active_count(self) -> usize {
        match self {
            Self::Level1 => 1,
            Self::Level2 => 2,
            Self::Level3 => 3,
            Self::Level4 => 4,
            Self::Level5 => 5,
            Self::Level6 => 6,
            Self::Level7 => 7,
            Self::Level8 => 8,
            Self::Other => 0,
        }
    }

    fn from_arrow_index(index: usize) -> Self {
        match index {
            0 => Self::Level1,
            1 => Self::Level2,
            2 => Self::Level3,
            3 => Self::Level4,
            4 => Self::Level5,
            5 => Self::Level6,
            6 => Self::Level7,
            7 => Self::Level8,
            _ => Self::Other,
        }
    }

    fn from_speed(speed: f64) -> Self {
        let mut best_idx = 0usize;
        let mut best_ratio = 0.0f64;

        for (idx, notch_speed) in THROTTLE_SPEEDS.iter().enumerate() {
            let ratio = speed_match_ratio(speed, *notch_speed);
            if ratio > best_ratio {
                best_ratio = ratio;
                best_idx = idx;
            }
        }

        if best_ratio >= THROTTLE_MATCH_RATIO {
            Self::from_arrow_index(best_idx)
        } else {
            Self::Other
        }
    }
}

pub fn mission_clock_widget(
    mut contexts: EguiContexts,
    mut sim_time: ResMut<SimTime>,
    mut mode: Local<MissionClockMode>,
    mut throttle_level: Local<TimeThrottleLevel>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() {
        return;
    }
    let ctx = ctx.unwrap();

    let jd = sim_time.time.to_julian_day();
    let unix_seconds = sim_time.time.to_j2000_seconds() + UNIX_EPOCH_AT_J2000_UTC;
    let utc_display = format_utc_string(unix_seconds);
    let mut clock_panel_rect: Option<egui::Rect> = None;

    // External speed changes update UI throttle state.
    *throttle_level = TimeThrottleLevel::from_speed(sim_time.gui_speed);

    egui::Area::new("mission_clock".into())
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
        .show(ctx, |ui| {
            let panel_bg = to_egui_color(vfd::PANEL_BG);
            let border = to_egui_color(vfd::BUTTON_BORDER);
            let text = to_egui_color(vfd::TEXT);
            let text_dim = to_egui_color(vfd::TEXT_DIM);

            egui::Frame::default()
                .fill(panel_bg)
                .stroke(egui::Stroke::new(1.0, border))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        if draw_mode_cycler(ui, *mode, text, text_dim).clicked() {
                            mode.toggle();
                        }

                        let display_text = match *mode {
                            MissionClockMode::JulianDay => format!("JD {:.2}", jd),
                            MissionClockMode::Utc => utc_display.clone(),
                        };

                        let min_display_width = minimum_utc_display_width(ui, text);
                        ui.add_sized(
                            [min_display_width, 0.0],
                            egui::Label::new(egui::RichText::new(display_text).color(text)),
                        );
                    });
                });
            clock_panel_rect = Some(ui.min_rect());
        });

    if let Some(rect) = clock_panel_rect {
        egui::Area::new("mission_clock_play_pause_sidecar".into())
            .fixed_pos(egui::pos2(rect.right() + SIDECAR_GAP, rect.top()))
            .show(ctx, |ui| {
                let panel_bg = to_egui_color(vfd::PANEL_BG);
                let border = to_egui_color(vfd::BUTTON_BORDER);
                let text = to_egui_color(vfd::TEXT);
                let button_bg = to_egui_color(vfd::BUTTON_BG);
                let button_hover = to_egui_color(vfd::BUTTON_HOVER);

                let (play_response, throttle_clicked_level) = draw_play_pause_sidecar(
                    ui,
                    rect.height(),
                    sim_time.playing,
                    *throttle_level,
                    panel_bg,
                    border,
                    text,
                    button_bg,
                    button_hover,
                );

                if play_response.clicked() {
                    sim_time.playing = !sim_time.playing;
                }
                if let Some(level) = throttle_clicked_level {
                    *throttle_level = level;
                    if let Some(speed) = throttle_level.to_speed() {
                        sim_time.gui_speed = speed;
                    }
                }
            });
    }
}

fn draw_mode_cycler(
    ui: &mut egui::Ui,
    mode: MissionClockMode,
    active: egui::Color32,
    inactive: egui::Color32,
) -> egui::Response {
    let desired_size = egui::vec2(40.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = 4.0;
        let top_center = egui::pos2(rect.left() + 7.0, rect.top() + 7.0);
        let bottom_center = egui::pos2(rect.left() + 7.0, rect.bottom() - 7.0);

        let top_color = match mode {
            MissionClockMode::JulianDay => active,
            MissionClockMode::Utc => inactive,
        };
        let bottom_color = match mode {
            MissionClockMode::JulianDay => inactive,
            MissionClockMode::Utc => active,
        };

        painter.circle_filled(top_center, radius, top_color);
        painter.circle_stroke(top_center, radius, egui::Stroke::new(1.0, active));
        painter.circle_filled(bottom_center, radius, bottom_color);
        painter.circle_stroke(bottom_center, radius, egui::Stroke::new(1.0, active));

        let label_font = egui::FontId::proportional(9.0);
        painter.text(
            egui::pos2(rect.left() + 15.0, top_center.y),
            egui::Align2::LEFT_CENTER,
            "JD",
            label_font.clone(),
            top_color,
        );
        painter.text(
            egui::pos2(rect.left() + 15.0, bottom_center.y),
            egui::Align2::LEFT_CENTER,
            "UTC",
            label_font,
            bottom_color,
        );
    }

    response
}

fn minimum_utc_display_width(ui: &egui::Ui, color: egui::Color32) -> f32 {
    let sample = "0000-00-00 00:00:00";
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(sample.to_string(), font_id, color);
    galley.size().x
}

fn draw_play_pause_sidecar(
    ui: &mut egui::Ui,
    height: f32,
    is_playing: bool,
    throttle_level: TimeThrottleLevel,
    panel_bg: egui::Color32,
    border_color: egui::Color32,
    icon_color: egui::Color32,
    button_bg: egui::Color32,
    button_hover: egui::Color32,
) -> (egui::Response, Option<TimeThrottleLevel>) {
    let (sidecar_rect, _) =
        ui.allocate_exact_size(egui::vec2(SIDECAR_WIDTH, height), egui::Sense::hover());
    let painter = ui.painter();

    painter.rect_filled(sidecar_rect, 0.0, panel_bg);
    painter.rect_stroke(
        sidecar_rect,
        0.0,
        egui::Stroke::new(1.0, border_color),
        egui::StrokeKind::Middle,
    );

    let button_rect = egui::Rect::from_min_max(
        sidecar_rect.left_top(),
        egui::pos2(sidecar_rect.left() + SIDECAR_BUTTON_WIDTH, sidecar_rect.bottom()),
    );
    let play_response = ui.interact(
        button_rect,
        ui.id().with("mission_clock_play_pause_button"),
        egui::Sense::click(),
    );

    let fill = if play_response.hovered() {
        button_hover
    } else {
        button_bg
    };
    painter.rect_filled(button_rect, 0.0, fill);
    painter.rect_stroke(
        button_rect,
        0.0,
        egui::Stroke::new(1.0, border_color),
        egui::StrokeKind::Middle,
    );

    if is_playing {
        draw_pause_icon(painter, button_rect, icon_color);
    } else {
        draw_play_icon(painter, button_rect, icon_color);
    }

    let throttle_rect = egui::Rect::from_min_max(
        egui::pos2(button_rect.right(), sidecar_rect.top()),
        sidecar_rect.right_bottom(),
    );
    let throttle_response = ui.interact(
        throttle_rect,
        ui.id().with("mission_clock_throttle"),
        egui::Sense::click(),
    );
    let throttle_draw_rect = throttle_rect.shrink2(egui::vec2(THROTTLE_GROUP_PAD_X, 0.0));
    if throttle_response.hovered() {
        painter.rect_filled(throttle_draw_rect, 0.0, button_hover.gamma_multiply(0.25));
    }

    draw_throttle_arrows(
        painter,
        throttle_draw_rect,
        throttle_level.active_count(),
        icon_color,
        border_color.gamma_multiply(0.8),
    );

    let mut clicked_level = None;
    if throttle_response.clicked() {
        if let Some(pos) = throttle_response.interact_pointer_pos() {
            let rel_x = (pos.x - throttle_draw_rect.left()).clamp(0.0, throttle_draw_rect.width());
            let idx = ((rel_x / throttle_draw_rect.width()) * THROTTLE_ARROW_COUNT as f32)
                .floor()
                .clamp(0.0, (THROTTLE_ARROW_COUNT - 1) as f32) as usize;
            clicked_level = Some(TimeThrottleLevel::from_arrow_index(idx));
        }
    }

    (play_response, clicked_level)
}

fn draw_play_icon(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let center = rect.center();
    let half_w = 4.5;
    let half_h = 5.5;
    let points = vec![
        egui::pos2(center.x - half_w, center.y - half_h),
        egui::pos2(center.x - half_w, center.y + half_h),
        egui::pos2(center.x + half_w, center.y),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
}

fn draw_pause_icon(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let center = rect.center();
    let bar_half_h = 5.5;
    let bar_half_w = 1.5;
    let gap = 3.0;

    let left = egui::Rect::from_center_size(
        egui::pos2(center.x - gap, center.y),
        egui::vec2(bar_half_w * 2.0, bar_half_h * 2.0),
    );
    let right = egui::Rect::from_center_size(
        egui::pos2(center.x + gap, center.y),
        egui::vec2(bar_half_w * 2.0, bar_half_h * 2.0),
    );

    painter.rect_filled(left, 0.0, color);
    painter.rect_filled(right, 0.0, color);
}

fn draw_throttle_arrows(
    painter: &egui::Painter,
    rect: egui::Rect,
    active_count: usize,
    active_color: egui::Color32,
    inactive_color: egui::Color32,
) {
    let step = rect.width() / THROTTLE_ARROW_COUNT as f32;
    let y_center = rect.center().y;
    let arrow_half_h = (rect.height() * THROTTLE_ARROW_HEIGHT_SCALE) * 0.5;
    let y_top = y_center - arrow_half_h;
    let y_bottom = y_center + arrow_half_h;
    let arrow_w = step * THROTTLE_ARROW_WIDTH_SCALE;

    for i in 0..THROTTLE_ARROW_COUNT {
        let slot_left = rect.left() + i as f32 * step;
        let slot_right = slot_left + step;
        let left = slot_left + (step - arrow_w) * 0.5;
        let right = slot_right - (step - arrow_w) * 0.5;
        let points = vec![
            egui::pos2(right, y_center),
            egui::pos2(left, y_top),
            egui::pos2(left, y_bottom),
        ];
        let color = if i < active_count {
            active_color
        } else {
            inactive_color
        };
        painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    }
}

fn speed_match_ratio(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    a.min(b) / a.max(b)
}

fn format_utc_string(unix_seconds: f64) -> String {
    let rounded = unix_seconds.round() as i64;
    let Some(datetime): Option<DateTime<Utc>> = DateTime::from_timestamp(rounded, 0) else {
        return "UTC out of range".to_string();
    };
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn to_egui_color(color: Color) -> egui::Color32 {
    let srgb = color.to_srgba();
    egui::Color32::from_rgba_premultiplied(
        (srgb.red * 255.0) as u8,
        (srgb.green * 255.0) as u8,
        (srgb.blue * 255.0) as u8,
        (srgb.alpha * 255.0) as u8,
    )
}
