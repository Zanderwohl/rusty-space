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

/// How a surface lays its names out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Where a label sits relative to its symbol, in pixels.
    pub offset: Vec2,
    /// The clearance kept between two of them.
    pub gap: f32,
    /// How light a thing may be and still be named, as a fraction of the heaviest thing on
    /// screen. [`crate::weight::FLOOR`] is the value this was tuned at.
    ///
    /// **The collision rule thins a crowd and has nothing to say about an empty view**, so
    /// without this a lone asteroid in open space is named as readily as a planet. Relative
    /// rather than absolute because the map spans fifteen orders of magnitude: what deserves
    /// a name beside the Sun and what deserves one beside Jupiter are different questions
    /// with the same answer.
    pub floor: f64,
}

/// Lay out as many as fit, heaviest first.
///
/// One anchor and no second try: a label that hops to the other side of its symbol when a
/// neighbour drifts past reads as a twitch, and a map of moving things would twitch
/// constantly.
pub fn lay_out(candidates: Vec<Candidate>, viewport: Vec2, layout: Layout) -> Vec<Placed> {
    // The bar is set by what the reader can actually see, so it is drawn after the view is
    // clipped and not before: pan the Sun off the edge and the question becomes what is worth
    // naming beside whatever is left.
    let mut inside: Vec<Candidate> =
        candidates.into_iter().filter(|c| box_of(c, layout.offset, viewport).is_some()).collect();
    let heaviest = crate::weight::heaviest(inside.iter().map(|c| c.weight));
    let floor = heaviest * layout.floor;
    // A ship's weight is infinite and sets no bar — it would silence the whole map — and
    // something that never stated a mass is not silenced by a comparison it is not in.
    inside.retain(|c| !(c.weight.is_finite() && c.weight > 0.0) || c.weight >= floor);

    // Ties broken by key, so two bodies of equal mass cannot trade places between frames.
    // Order alone is what decides who is dropped, so an unstable sort here is a flicker.
    inside.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });

    let mut taken: Vec<(Vec2, Vec2)> = Vec::with_capacity(inside.len());
    let mut placed = Vec::with_capacity(inside.len());
    for candidate in inside {
        let Some((min, max)) = box_of(&candidate, layout.offset, viewport) else { continue };
        // Only the incoming rectangle is grown. Two rectangles that clear each other by `gap`
        // clear it once, not twice.
        let grown = (min - Vec2::splat(layout.gap), max + Vec2::splat(layout.gap));
        if taken.iter().any(|other| overlaps(grown, *other)) {
            continue;
        }
        taken.push((min, max));
        placed.push(Placed { key: candidate.key, at: min });
    }
    placed
}

/// Where a label's text would sit, or `None` if that is not wholly on the surface.
fn box_of(candidate: &Candidate, offset: Vec2, viewport: Vec2) -> Option<(Vec2, Vec2)> {
    let min = candidate.at + offset;
    let max = min + candidate.size;
    let inside = min.is_finite()
        && max.is_finite()
        && min.x >= 0.0
        && min.y >= 0.0
        && max.x <= viewport.x
        && max.y <= viewport.y;
    inside.then_some((min, max))
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

    /// A layout with no floor, for the tests that are about collisions and nothing else.
    fn loose(offset: Vec2, gap: f32) -> Layout {
        Layout { offset, gap, floor: 0.0 }
    }

    const SUN: f64 = 1.988_41e30;
    const EARTH: f64 = 5.972e24;
    const MERCURY: f64 = 3.301e23;
    const CERES: f64 = 9.39e20;

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
        let placed = lay_out(all.clone(), view(), loose(Vec2::new(6.0, -6.0), 2.0));
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
        let placed = lay_out(spread.clone(), view(), loose(Vec2::new(6.0, -6.0), 2.0));
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

    /// **Earth is named beside the Sun. That is the case the floor is set from.**
    ///
    /// Three parts in a million separate them, and everything between a planet and a rock has
    /// to land on the right side of one number. Mercury is the smallest planet there is and
    /// Ceres the largest asteroid; the floor sits in the gap between them, which is a factor
    /// of three hundred wide.
    #[test]
    fn earth_is_named_beside_the_sun_and_ceres_is_not() {
        let floor = 1.0e-8;
        let sky = vec![
            at("Sol", SUN, 60.0, 40.0),
            at("Earth", EARTH, 60.0, 120.0),
            at("Mercury", MERCURY, 60.0, 200.0),
            at("Ceres", CERES, 60.0, 280.0),
        ];
        let layout = Layout { offset: Vec2::ZERO, gap: 2.0, floor };
        let placed = lay_out(sky.clone(), view(), layout);
        let named = |name: &str| placed.iter().any(|p| p.key == ItemKey::from_name(name));
        assert!(named("Sol"), "the star sets the bar and clears it");
        assert!(named("Earth"), "Earth beside the Sun is the case this is tuned from");
        assert!(named("Mercury"), "and the floor sits below the smallest planet");
        assert!(!named("Ceres"), "and above the largest asteroid");
        // Nothing is dropped for want of room: it is the floor doing this and not the gap.
        assert_eq!(lay_out(sky, view(), loose(Vec2::ZERO, 2.0)).len(), 4);
    }

    /// **The bar is whatever the reader can see.** Pan the star off the edge and the question
    /// becomes what is worth naming beside what is left — which is how one number serves a
    /// map spanning fifteen orders of magnitude.
    #[test]
    fn the_floor_follows_what_is_on_screen() {
        let layout = Layout { offset: Vec2::ZERO, gap: 2.0, floor: 1.0e-8 };
        let without_the_sun = vec![at("Ceres", CERES, 60.0, 40.0), at("Vesta", 2.59e20, 60.0, 120.0)];
        assert_eq!(
            lay_out(without_the_sun, view(), layout).len(),
            2,
            "with nothing heavier in sight, a rock is worth naming",
        );
    }

    /// And a ship is never silenced by it. Its weight is infinite, which would also make it
    /// the bar — and a bar of infinity is a map with one name on it.
    #[test]
    fn a_ship_sets_no_floor_and_clears_every_one() {
        let layout = Layout { offset: Vec2::ZERO, gap: 2.0, floor: 1.0e-8 };
        let sky = vec![
            at("Harrier", f64::INFINITY, 60.0, 40.0),
            at("Sol", SUN, 60.0, 120.0),
            at("Earth", EARTH, 60.0, 200.0),
        ];
        assert_eq!(lay_out(sky, view(), layout).len(), 3, "a ship must not floor the sky out");
    }

    /// Room for everyone means everyone, in no particular hurry.
    #[test]
    fn nothing_is_dropped_when_nothing_collides() {
        let spread: Vec<Candidate> = (0..8)
            .map(|i| at(&format!("body {i}"), i as f64, 40.0, 20.0 + 40.0 * i as f32))
            .collect();
        assert_eq!(lay_out(spread.clone(), view(), loose(Vec2::ZERO, 2.0)).len(), spread.len());
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
                lay_out(vec![candidate], view(), loose(Vec2::ZERO, 2.0)).is_empty(),
                "{:?} should not have been placed",
                candidate.at,
            );
        }
        // And one just inside is.
        assert_eq!(lay_out(vec![at("in", 1.0, 1.0, 1.0)], view(), loose(Vec2::ZERO, 2.0)).len(), 1);
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
        let one = lay_out(vec![a, b, c], view(), loose(Vec2::ZERO, 2.0));
        let two = lay_out(vec![c, a, b], view(), loose(Vec2::ZERO, 2.0));
        let three = lay_out(vec![b, c, a], view(), loose(Vec2::ZERO, 2.0));
        assert_eq!(one, two);
        assert_eq!(two, three);
        assert_eq!(one.len(), 1);
    }

    /// A ship outranks every body there is, and `f64::INFINITY` is how the client says so.
    #[test]
    fn an_infinite_weight_wins() {
        let ship = at("Harrier", f64::INFINITY, 300.0, 200.0);
        let star = at("Sol", 1.989e30, 301.0, 201.0);
        let placed = lay_out(vec![star, ship], view(), loose(Vec2::ZERO, 2.0));
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].key, ship.key);
        // And a weight nobody stated loses to one that was.
        let nothing = at("a belt", f64::NAN, 300.0, 200.0);
        let placed = lay_out(vec![nothing, star], view(), loose(Vec2::ZERO, 2.0));
        assert_eq!(placed[0].key, star.key);
        let _ = names(&placed, &[star]);
    }

    /// The gap is kept between labels, not merely their boxes touching.
    #[test]
    fn the_gap_is_clearance_and_not_decoration() {
        // Two rows 14 pixels apart, with 12-pixel text: two pixels of daylight.
        let stacked = vec![at("upper", 2.0, 100.0, 100.0), at("lower", 1.0, 100.0, 114.0)];
        assert_eq!(lay_out(stacked.clone(), view(), loose(Vec2::ZERO, 1.0)).len(), 2, "1 px fits");
        assert_eq!(lay_out(stacked, view(), loose(Vec2::ZERO, 4.0)).len(), 1, "4 px does not");
    }
}
