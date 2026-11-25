//! egui text whose superscript characters are drawn as small raised digits.
//!
//! egui's default face has `¹²³` and none of `⁰⁴⁵⁶⁷⁸⁹⁻`, and the client's own faces have fewer, so
//! [`crate::number::scientific`] strings would show boxes. Laying the exponent out as raised small
//! text works in every face.

use egui::text::LayoutJob;
use egui::{Align, Color32, FontSelection, RichText, Style};

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
