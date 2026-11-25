//! egui text whose superscripts are raised, and astronomical unit symbols drawn as shapes.
//!
//! egui's default face has `¹²³` and none of `⁰⁴⁵⁶⁷⁸⁹⁻` or `⊕`, and the client's own faces have
//! fewer, so [`crate::number::scientific`] strings and `M⊕` would show boxes. Raising the exponent
//! at layout time, and painting `☉` and `⊕`, work in every face.

use egui::text::LayoutJob;
use egui::{Align, Color32, FontSelection, Response, RichText, Sense, Stroke, Style, TextStyle, Ui, pos2, vec2};

/// The ASCII a superscript character stands for.
fn lowered(c: char) -> Option<char> {
    Some(match c {
        '⁰' => '0',
        '¹' => '1',
        '²' => '2',
        '³' => '3',
        '⁴' => '4',
        '⁵' => '5',
        '⁶' => '6',
        '⁷' => '7',
        '⁸' => '8',
        '⁹' => '9',
        '⁻' => '-',
        _ => return None,
    })
}

/// `text` laid out in `style`, superscripts raised. `color` of `None` takes the widget's.
pub fn raised(style: &Style, text: &str, color: Option<Color32>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let flush = |job: &mut LayoutJob, run: &mut String, up: bool| {
        if run.is_empty() {
            return;
        }
        let mut piece = RichText::new(std::mem::take(run));
        if up {
            piece = piece.small_raised();
        }
        if let Some(c) = color {
            piece = piece.color(c);
        }
        piece.append_to(&mut *job, style, FontSelection::Default, Align::Center);
    };
    let (mut run, mut up) = (String::new(), false);
    for c in text.chars() {
        let (shown, is_up) = lowered(c).map_or((c, false), |d| (d, true));
        if is_up != up {
            flush(&mut job, &mut run, up);
            up = is_up;
        }
        run.push(shown);
    }
    flush(&mut job, &mut run, up);
    job
}

/// A unit written as a letter with the Sun's or the Earth's symbol as its subscript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    SolarMass,
    SolarRadius,
    SolarLuminosity,
    EarthMass,
    EarthRadius,
}

impl Unit {
    fn letter(self) -> &'static str {
        match self {
            Unit::SolarMass | Unit::EarthMass => "M",
            Unit::SolarRadius | Unit::EarthRadius => "R",
            Unit::SolarLuminosity => "L",
        }
    }

    fn solar(self) -> bool {
        matches!(self, Unit::SolarMass | Unit::SolarRadius | Unit::SolarLuminosity)
    }

    pub fn name(self) -> &'static str {
        match self {
            Unit::SolarMass => "Solar Mass",
            Unit::SolarRadius => "Solar Radius",
            Unit::SolarLuminosity => "Solar Luminosity",
            Unit::EarthMass => "Earth Mass",
            Unit::EarthRadius => "Earth Radius",
        }
    }
}

/// `value`, superscripts raised, then `unit`'s symbol.
pub fn quantity(ui: &mut Ui, value: &str, unit: Unit) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(raised(ui.style(), &format!("{value} "), None));
        symbol(ui, unit)
    })
    .inner
}

/// `unit` as `M☉`, `R⊕` and so on, in the body font and text color; hovering names it.
pub fn symbol(ui: &mut Ui, unit: Unit) -> Response {
    let font = TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let letter = ui.painter().layout_no_wrap(unit.letter().into(), font.clone(), color);
    let diameter = font.size * 0.6;
    let gap = font.size * 0.05;
    let size = vec2(letter.size().x + gap + diameter, letter.size().y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let width = (font.size * 0.08).max(1.0);
        // A subscript: the symbol's bottom sits a little under the letter's baseline.
        let baseline = rect.top() + letter.rows.first().map_or(size.y * 0.8, |r| r.max_y());
        let center = pos2(rect.left() + letter.size().x + gap + diameter / 2.0, baseline - diameter * 0.35);
        let radius = (diameter - width) / 2.0;
        painter.galley(rect.min, letter, color);
        painter.circle_stroke(center, radius, Stroke::new(width, color));
        if unit.solar() {
            painter.circle_filled(center, width * 1.1, color);
        } else {
            let stroke = Stroke::new(width, color);
            painter.line_segment([center - vec2(radius, 0.0), center + vec2(radius, 0.0)], stroke);
            painter.line_segment([center - vec2(0.0, radius), center + vec2(0.0, radius)], stroke);
        }
    }
    response.on_hover_text(unit.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exponent_is_its_own_raised_section() {
        let job = raised(&Style::default(), "1.2 × 10⁻⁵ kg", None);
        assert_eq!(job.text, "1.2 × 10-5 kg");
        let pieces: Vec<(&str, Align)> =
            job.sections.iter().map(|s| (&job.text[s.byte_range.start.0..s.byte_range.end.0], s.format.valign)).collect();
        assert_eq!(pieces, [("1.2 × 10", Align::Center), ("-5", Align::TOP), (" kg", Align::Center)]);
    }
}
