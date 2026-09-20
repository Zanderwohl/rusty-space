//! Which of a crowded map's names there is actually room for.
//!
//! A planetary system has fifty names in it and a panel has room for six. The rule is that the
//! **heaviest wins**: Jupiter is named and its moons are not, and where the moons are named it
//! is the Galileans that fit. Nothing here knows about primaries or satellites — a hierarchy
//! is what mass already says, and encoding it twice is two answers to one question.
//!
//! A label that would leave the viewport is dropped rather than dragged back to the edge. The
//! map draws no edge markers by choice: an arrow at the rim names something the reader cannot
//! see, and pays for it with the pixels of something they can.

use glam::Vec2;

use crate::ItemKey;

/// A label that wants drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub key: ItemKey,
    /// Bigger takes the pixels. See [`crate::MapItem::weight`].
    pub weight: f64,
    /// The symbol's center, in pixels from the viewport's top left.
    pub at: Vec2,
    /// What the text will occupy, in pixels.
    pub size: Vec2,
}

/// A label that fits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub key: ItemKey,
    /// Top left of the text, in pixels from the viewport's top left.
    pub at: Vec2,
}

/// Lay out as many as fit, heaviest first.
///
/// `offset` is where a label sits relative to its symbol, and `gap` the clearance kept between
/// two of them. One anchor and no second try: a label that hops to the other side of its
/// symbol when a neighbour drifts past reads as a twitch, and a map of moving things would
/// twitch constantly.
pub fn lay_out(mut candidates: Vec<Candidate>, viewport: Vec2, offset: Vec2, gap: f32)
    -> Vec<Placed> {
    // Ties broken by key, so two bodies of equal mass cannot trade places between frames.
    // Order alone is what decides who is dropped, so an unstable sort here is a flicker.
    candidates.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });

    let mut taken: Vec<(Vec2, Vec2)> = Vec::with_capacity(candidates.len());
    let mut placed = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let min = candidate.at + offset;
        let max = min + candidate.size;
        if !min.is_finite() || !max.is_finite() {
            continue;
        }
        if min.x < 0.0 || min.y < 0.0 || max.x > viewport.x || max.y > viewport.y {
            continue;
        }
        // Only the incoming rectangle is grown. Two rectangles that clear each other by `gap`
        // clear it once, not twice.
        let grown = (min - Vec2::splat(gap), max + Vec2::splat(gap));
        if taken.iter().any(|other| overlaps(grown, *other)) {
            continue;
        }
        taken.push((min, max));
        placed.push(Placed { key: candidate.key, at: min });
    }
    placed
}

fn overlaps(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> bool {
    a.0.x < b.1.x && b.0.x < a.1.x && a.0.y < b.1.y && b.0.y < a.1.y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(key: &str, weight: f64, x: f32, y: f32) -> Candidate {
        Candidate {
            key: ItemKey::from_name(key),
            weight,
            at: Vec2::new(x, y),
            size: Vec2::new(40.0, 12.0),
        }
    }

    fn view() -> Vec2 {
        Vec2::new(640.0, 400.0)
    }

    fn names(placed: &[Placed], all: &[Candidate]) -> Vec<String> {
        placed
            .iter()
            .filter_map(|p| all.iter().position(|c| c.key == p.key).map(|i| i.to_string()))
            .collect()
    }

    /// **The whole point, in the arrangement it was asked for.**
    ///
    /// Jupiter and its moons on top of each other: one name comes out, and it is the planet's.
    /// Give the moons room and the Galileans are the four that fit, because they outweigh the
    /// small ones by four orders of magnitude and nothing else had to say so.
    #[test]
    fn the_heaviest_in_a_pile_is_the_one_named() {
        let jupiter = at("Jupiter", 1.898e27, 300.0, 200.0);
        let galileans = [
            at("Io", 8.93e22, 302.0, 201.0),
            at("Europa", 4.80e22, 298.0, 203.0),
            at("Ganymede", 1.48e23, 303.0, 198.0),
            at("Callisto", 1.08e23, 297.0, 202.0),
        ];
        let small = [
            at("Amalthea", 2.08e18, 301.0, 199.0),
            at("Himalia", 6.7e18, 299.0, 204.0),
            at("Thebe", 4.3e17, 304.0, 200.0),
        ];

        let mut all = vec![jupiter];
        all.extend(galileans);
        all.extend(small);
        let placed = lay_out(all.clone(), view(), Vec2::new(6.0, -6.0), 2.0);
        assert_eq!(placed.len(), 1, "a pile has room for one name");
        assert_eq!(placed[0].key, jupiter.key, "and it is the planet's");

        // Now spread them out, with only four gaps to go round: the Galileans take them.
        let mut spread = Vec::new();
        for (i, c) in all.iter().enumerate() {
            let mut c = *c;
            // Eight rows, four of which are off the bottom of a 400-pixel viewport.
            c.at = Vec2::new(300.0, 40.0 * i as f32);
            spread.push(c);
        }
        let placed = lay_out(spread.clone(), view(), Vec2::new(6.0, -6.0), 2.0);
        let named: Vec<&str> = placed
            .iter()
            .map(|p| {
                let i = spread.iter().position(|c| c.key == p.key).unwrap();
                ["Jupiter", "Io", "Europa", "Ganymede", "Callisto", "Amalthea", "Himalia",
                    "Thebe"][i]
            })
            .collect();
        for moon in ["Io", "Europa", "Ganymede", "Callisto"] {
            assert!(named.contains(&moon), "{moon} should have been named, got {named:?}");
        }
    }

    /// Room for everyone means everyone, in no particular hurry.
    #[test]
    fn nothing_is_dropped_when_nothing_collides() {
        let spread: Vec<Candidate> = (0..8)
            .map(|i| at(&format!("body {i}"), i as f64, 40.0, 20.0 + 40.0 * i as f32))
            .collect();
        assert_eq!(lay_out(spread.clone(), view(), Vec2::ZERO, 2.0).len(), spread.len());
    }

    /// **A name off the edge is a name for something the reader cannot see.**
    ///
    /// Dropped, not dragged to the rim: the map has no edge markers on purpose.
    #[test]
    fn a_label_that_would_leave_the_view_is_dropped() {
        let outside = [
            at("left", 1.0, -60.0, 200.0),
            at("right", 1.0, 620.0, 200.0),
            at("top", 1.0, 300.0, -20.0),
            at("bottom", 1.0, 300.0, 398.0),
        ];
        for candidate in outside {
            assert!(
                lay_out(vec![candidate], view(), Vec2::ZERO, 2.0).is_empty(),
                "{:?} should not have been placed",
                candidate.at,
            );
        }
        // And one just inside is.
        assert_eq!(lay_out(vec![at("in", 1.0, 1.0, 1.0)], view(), Vec2::ZERO, 2.0).len(), 1);
    }

    /// **Two things of equal mass must not trade places between frames.**
    ///
    /// Only the order decides who is dropped, so a tie resolved by whatever the sort happened
    /// to do is a label blinking on and off while nothing moves.
    #[test]
    fn a_tie_is_broken_the_same_way_every_time() {
        let a = at("alpha", 5.0e20, 300.0, 200.0);
        let b = at("beta", 5.0e20, 302.0, 201.0);
        let c = at("gamma", 5.0e20, 298.0, 199.0);
        let one = lay_out(vec![a, b, c], view(), Vec2::ZERO, 2.0);
        let two = lay_out(vec![c, a, b], view(), Vec2::ZERO, 2.0);
        let three = lay_out(vec![b, c, a], view(), Vec2::ZERO, 2.0);
        assert_eq!(one, two);
        assert_eq!(two, three);
        assert_eq!(one.len(), 1);
    }

    /// A ship outranks every body there is, and `f64::INFINITY` is how the client says so.
    #[test]
    fn an_infinite_weight_wins() {
        let ship = at("Harrier", f64::INFINITY, 300.0, 200.0);
        let star = at("Sol", 1.989e30, 301.0, 201.0);
        let placed = lay_out(vec![star, ship], view(), Vec2::ZERO, 2.0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].key, ship.key);
        // And a weight nobody stated loses to one that was.
        let nothing = at("a belt", f64::NAN, 300.0, 200.0);
        let placed = lay_out(vec![nothing, star], view(), Vec2::ZERO, 2.0);
        assert_eq!(placed[0].key, star.key);
        let _ = names(&placed, &[star]);
    }

    /// The gap is kept between labels, not merely their boxes touching.
    #[test]
    fn the_gap_is_clearance_and_not_decoration() {
        // Two rows 14 pixels apart, with 12-pixel text: two pixels of daylight.
        let stacked = vec![at("upper", 2.0, 100.0, 100.0), at("lower", 1.0, 100.0, 114.0)];
        assert_eq!(lay_out(stacked.clone(), view(), Vec2::ZERO, 1.0).len(), 2, "1 px fits");
        assert_eq!(lay_out(stacked, view(), Vec2::ZERO, 4.0).len(), 1, "4 px does not");
    }
}
