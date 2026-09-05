//! Shared formatting and widgets for the Viewer and the Editor.
//!
//! Labels stay short: whatever a reader needs beyond the name goes in the tooltip, so
//! the two-column grids line up no matter how technical the quantity is.

use bevy::math::DVec3;
use bevy_egui::egui::{self, Ui};
use em_foundations::time::{Instant, TimeDelta};

use crate::util::format;

pub const AU_M: f64 = 1.495_978_707e11;
pub const EARTH_MASS_KG: f64 = 5.972_168e24;
pub const SUN_MASS_KG: f64 = 1.988_47e30;
pub const STANDARD_GRAVITY: f64 = 9.806_65;

/// Placeholder for a quantity that does not exist for this body.
pub const NONE: &str = "—";

// ------------------------------------------------------------------ layout

/// A titled block. Collapsed state is remembered by egui, per window and per title.
pub fn section<R>(ui: &mut Ui, title: &str, open: bool, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    egui::CollapsingHeader::new(title)
        .default_open(open)
        .show(ui, contents)
        .body_returned
}

/// The two-column `label | value` grid every section is built from.
pub fn grid<R>(ui: &mut Ui, id: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, contents)
        .inner
}

/// One grid row. `hint` is the label's tooltip and may be empty.
pub fn row<R>(ui: &mut Ui, label: &str, hint: &str, value: impl FnOnce(&mut Ui) -> R) -> R {
    let response = ui.label(label);
    if !hint.is_empty() {
        response.on_hover_text(hint);
    }
    let returned = value(ui);
    ui.end_row();
    returned
}

/// A read-only row whose value carries its own tooltip — the exact figure behind a
/// rounded one, usually.
pub fn text_row(ui: &mut Ui, label: &str, hint: &str, value: &str, value_hint: &str) {
    row(ui, label, hint, |ui| {
        let response = ui.label(value);
        if !value_hint.is_empty() {
            response.on_hover_text(value_hint);
        }
    });
}

// ------------------------------------------------------------------ formatting

/// Metres in whichever unit keeps the number readable; the tooltip has the metres.
pub fn distance(metres: f64) -> String {
    if !metres.is_finite() {
        return NONE.to_string();
    }
    let magnitude = metres.abs();
    if magnitude < 1.0e3 {
        format!("{metres:.1} m")
    } else if magnitude < 0.01 * AU_M {
        format!("{:.3} km", metres / 1.0e3)
    } else {
        format!("{:.6} AU", metres / AU_M)
    }
}

pub fn distance_hint(metres: f64) -> String {
    format!("{} m", format::sci_not(metres))
}

pub fn speed(metres_per_second: f64) -> String {
    if !metres_per_second.is_finite() {
        return NONE.to_string();
    }
    if metres_per_second.abs() < 1.0e3 {
        format!("{metres_per_second:.2} m/s")
    } else {
        format!("{:.3} km/s", metres_per_second / 1.0e3)
    }
}

pub fn mass(kilograms: f64) -> String {
    format!("{} kg", format::sci_not(kilograms))
}

/// The same mass against the two yardsticks people actually carry around.
pub fn mass_hint(kilograms: f64) -> String {
    format!(
        "{:.4} Earth masses\n{:.6} Solar masses",
        kilograms / EARTH_MASS_KG,
        kilograms / SUN_MASS_KG,
    )
}

pub fn angle(degrees: f64) -> String {
    if !degrees.is_finite() {
        return NONE.to_string();
    }
    format!("{degrees:.4}°")
}

pub fn duration(delta: TimeDelta) -> String {
    let seconds = delta.to_seconds();
    if !seconds.is_finite() {
        return NONE.to_string();
    }
    if seconds.abs() < 1.0 {
        return format!("{seconds:.3} s");
    }
    format::format_duration(seconds.round() as i64)
}

pub fn duration_hint(delta: TimeDelta) -> String {
    format!(
        "{} s\n{:.6} days\n{:.6} years",
        format::sci_not(delta.to_seconds()),
        delta.to_days(),
        delta.to_julian_years(),
    )
}

pub fn instant(time: Instant) -> String {
    format!("JD {:.5}", time.to_julian_day())
}

pub fn instant_hint(time: Instant) -> String {
    format!("{} s since J2000", format::sci_not(time.to_j2000_seconds()))
}

pub fn vector(v: DVec3) -> String {
    format!(
        "{}, {}, {}",
        format::sci_not(v.x),
        format::sci_not(v.y),
        format::sci_not(v.z),
    )
}

/// Three rows of a vector, each in readable units, with the raw components on hover.
pub fn vector_rows(ui: &mut Ui, label: &str, hint: &str, v: DVec3, as_speed: bool) {
    let component = |c: f64| if as_speed { speed(c) } else { distance(c) };
    let magnitude = if as_speed { speed(v.length()) } else { distance(v.length()) };
    let raw = vector(v);
    text_row(ui, label, hint, &magnitude, &raw);
    for (axis, value) in [("X", v.x), ("Y", v.y), ("Z", v.z)] {
        text_row(ui, axis, "", &component(value), &format::sci_not(value));
    }
}

// ------------------------------------------------------------------ editing

/// Drag over many orders of magnitude: the shoulders step the leading digit and the
/// outer pair move the exponent. Reports whether the value moved.
pub fn sci_drag(ui: &mut Ui, value: &mut f64, unit: &str) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        if ui.small_button("÷10").on_hover_text("Down one order of magnitude").clicked() {
            *value /= 10.0;
        }
        if ui.small_button("−").on_hover_text("Down one leading digit").clicked() {
            *value = bump_leading_digit(*value, -1.0);
        }
        ui.add(
            egui::DragValue::new(value)
                .speed(0.01)
                .range(f64::MIN..=f64::MAX)
                .custom_formatter(|n, _| format::sci_not(n))
                .custom_parser(format::sci_not_parser),
        );
        if ui.small_button("+").on_hover_text("Up one leading digit").clicked() {
            *value = bump_leading_digit(*value, 1.0);
        }
        if ui.small_button("×10").on_hover_text("Up one order of magnitude").clicked() {
            *value *= 10.0;
        }
        if !unit.is_empty() {
            ui.label(unit);
        }
    });
    *value != before
}

/// Degrees on a circle. Values are kept in `[0, 360)` so the readout matches the elements.
pub fn angle_drag(ui: &mut Ui, degrees: &mut f64) -> bool {
    let before = *degrees;
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(degrees)
                .speed(0.1)
                .range(f64::MIN..=f64::MAX)
                .fixed_decimals(4)
                .suffix("°"),
        );
    });
    if *degrees != before {
        *degrees = degrees.rem_euclid(360.0);
        true
    } else {
        false
    }
}

/// A plain number with a unit after it, for quantities that stay near unity.
pub fn unit_drag(ui: &mut Ui, value: &mut f64, speed: f64, range: std::ops::RangeInclusive<f64>, unit: &str) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(value)
                .speed(speed)
                .range(range)
                .clamp_existing_to_range(false)
                .max_decimals(6),
        );
        if !unit.is_empty() {
            ui.label(unit);
        }
    });
    *value != before
}

/// One `sci_drag` per component, as three grid rows.
pub fn vector_edit(ui: &mut Ui, label: &str, hint: &str, v: &mut DVec3, unit: &str) -> bool {
    let mut changed = false;
    row(ui, label, hint, |ui| {
        ui.label(format!("{} {unit}", format::sci_not(v.length())))
            .on_hover_text(vector(*v));
    });
    changed |= row(ui, "X", "", |ui| sci_drag(ui, &mut v.x, unit));
    changed |= row(ui, "Y", "", |ui| sci_drag(ui, &mut v.y, unit));
    changed |= row(ui, "Z", "", |ui| sci_drag(ui, &mut v.z, unit));
    changed
}

/// Move the leading digit by one step, keeping the order of magnitude.
fn bump_leading_digit(x: f64, direction: f64) -> f64 {
    if x == 0.0 {
        return direction;
    }
    let exponent = x.abs().log10().floor();
    let scale = 10f64.powf(exponent);
    let normalized = x / scale;
    let bumped = (normalized * 10.0 + direction).round() / 10.0;
    bumped * scale.copysign(x)
}
