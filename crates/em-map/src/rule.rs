//! The scale rule: a bar of known length, and what to call it.
//!
//! A map that can be zoomed across fifteen orders of magnitude has no fixed scale, so the one
//! in force has to be drawn. What is decided here is the *length* — how long a bar may be, in
//! meters, such that it is a round number in a unit a reader holds. Where it goes on screen and
//! what it is drawn with belongs to the host.

use crate::snapshot::{M_PER_AU, M_PER_LY};

/// The units a length is offered in, longest first.
///
/// Astronomical units and light-years sit among the metric prefixes because this is a map of
/// space: between a gigametre and an astronomical unit there is nothing anyone measures in, and
/// "150 Gm" is a worse answer than "1 AU" to the same question.
const UNITS: [(&str, f64); 6] = [
    ("ly", M_PER_LY),
    ("AU", M_PER_AU),
    ("Gm", 1.0e9),
    ("Mm", 1.0e6),
    ("km", 1.0e3),
    ("m", 1.0),
];

/// A bar to draw, and what to call it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// How long the bar is, in meters.
    pub meters: f64,
    /// What it says: `5 Gm`, `2 AU`, `1 ly`.
    pub label: String,
    /// Equal divisions to tick it into. One is an undivided bar.
    ///
    /// Only where the bar is a small whole number of its own unit, so each division is one of
    /// them: five ticks on a `5 Gm` bar are five gigametres, which is a second scale for free.
    /// A `1 Gm` bar divides into nothing, because tenths are not what the label says.
    pub parts: u32,
}

/// The longest round length that fits between `min_meters` and `max_meters`.
///
/// `None` when nothing round fits: a window narrower than about two and a half to one can fall
/// between the rungs, and a bar of a number nobody recognises is worse than no bar. Also when
/// the whole window is below a metre, which the ladder does not reach — the camera's closest
/// stand-off is a kilometre, so a bar that short is not a view anyone can be looking at.
pub fn choose(max_meters: f64, min_meters: f64) -> Option<Rule> {
    if !max_meters.is_finite() || max_meters <= 0.0 || max_meters < min_meters {
        return None;
    }
    for (name, unit) in UNITS {
        let count = max_meters / unit;
        if count < 1.0 {
            continue;
        }
        let rounded = round_down(count);
        let meters = rounded * unit;
        if meters < min_meters {
            // A shorter unit divides more finely, so it may land inside the window where this
            // one overshot it. Keep looking rather than giving up on the first miss.
            continue;
        }
        return Some(Rule {
            meters,
            label: format!("{rounded:.0} {name}"),
            parts: match mantissa(rounded) {
                m @ (2 | 5) => m,
                _ => 1,
            },
        });
    }
    None
}

/// The largest of `1`, `2` or `5` times a power of ten that does not exceed `count`.
fn round_down(count: f64) -> f64 {
    let decade = 10f64.powf(count.log10().floor());
    let m = count / decade;
    decade
        * if m >= 5.0 {
            5.0
        } else if m >= 2.0 {
            2.0
        } else {
            1.0
        }
}

/// The `1`, `2` or `5` a rounded count was built from.
fn mantissa(rounded: f64) -> u32 {
    let decade = 10f64.powf(rounded.log10().floor());
    (rounded / decade).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window a host actually asks with: a bar between a sixth and the whole of the space
    /// it is given. Something round always fits in that, from a metre to well past the
    /// camera's furthest stand-off.
    #[test]
    fn a_sixfold_window_always_finds_a_length() {
        let mut meters = 1.0;
        while meters < 1.0e20 {
            let rule = choose(meters, meters / 6.0);
            assert!(rule.is_some(), "nothing fit between {:e} and {meters:e}", meters / 6.0);
            let rule = rule.unwrap();
            assert!(rule.meters <= meters && rule.meters >= meters / 6.0, "{rule:?}");
            meters *= 1.7;
        }
    }

    /// It reads in the unit a person would use, and says a round number of them.
    #[test]
    fn it_says_a_round_number_of_a_unit_someone_uses() {
        let cases = [
            (7.0e9, "5 Gm"),
            (3.0e11, "2 AU"),
            (2.4e16, "2 ly"),
            (4.0e3, "2 km"),
            (60.0, "50 m"),
            (9.0e6, "5 Mm"),
        ];
        for (max, want) in cases {
            let rule = choose(max, max / 6.0).expect("a length");
            assert_eq!(rule.label, want, "for a limit of {max:e}");
        }
    }

    /// The divisions are whole units of the label, or there are none. Five ticks on a `5 Gm`
    /// bar are a gigametre each; tenths of one are not what the label says.
    #[test]
    fn it_divides_into_its_own_units_or_not_at_all() {
        for (max, parts) in [(7.0e9, 5), (3.0e11, 2), (1.2e9, 1), (6.0e3, 5)] {
            let rule = choose(max, max / 6.0).expect("a length");
            assert_eq!(rule.parts, parts, "{}", rule.label);
        }
    }

    /// Every bar is exactly as long as it says it is, which is the one thing a scale rule is
    /// for. A label that rounded away from the bar would be a ruler with the wrong numbers.
    #[test]
    fn the_label_is_the_length() {
        let mut meters = 1.0;
        while meters < 1.0e19 {
            let rule = choose(meters, meters / 6.0).expect("a length");
            let (count, name) = rule.label.split_once(' ').expect("a number and a unit");
            let unit = UNITS.iter().find(|(n, _)| *n == name).expect("a known unit").1;
            let said = count.parse::<f64>().expect("a number") * unit;
            assert!(
                (said / rule.meters - 1.0).abs() < 1e-9,
                "{} says {said:e} and is {:e}",
                rule.label,
                rule.meters,
            );
            meters *= 2.3;
        }
    }

    #[test]
    fn nothing_below_a_metre_has_a_name_here() {
        assert!(choose(1.0e-2, 1.0e-3).is_none());
        assert!(choose(1.0, 0.16).is_some(), "and a metre itself does");
    }

    #[test]
    fn nonsense_gets_no_rule() {
        assert!(choose(0.0, 0.0).is_none());
        assert!(choose(-5.0, 1.0).is_none());
        assert!(choose(f64::NAN, 1.0).is_none());
        assert!(choose(f64::INFINITY, 1.0).is_none());
        assert!(choose(10.0, 100.0).is_none(), "a window the wrong way round");
    }
}
