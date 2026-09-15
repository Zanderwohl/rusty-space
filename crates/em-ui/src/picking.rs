//! Deciding what the cursor is on.
//!
//! Screen space only. A caller reduces everything it drew to a [`Candidate`] — where it put it,
//! how big it drew it, and what sort of thing it is — and this says which one was meant. That
//! reduction is the point: picking then agrees with the screen *by construction*, whatever the
//! renderer did to get there. Lightcone draws stars at their aberrated direction and bodies at
//! their true one, and neither needs a special case here, because each candidate carries the
//! position its own pass used.
//!
//! No Bevy types in the signatures and no camera, so the rule is testable without a window.

use bevy::math::Vec2;

/// How far off a thing the cursor may be and still be on it, pixels.
///
/// Small bodies are the whole reason for it. A moon four pixels across cannot be hit exactly,
/// and the alternative — preferring the more massive of two hits — makes a moon in front of its
/// planet unselectable at any distance, which is worse.
pub const SLACK_PX: f32 = 12.0;

/// One thing the cursor could be on, reduced to where it was drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// Whatever the caller needs to identify it again.
    pub id: u64,
    /// Where it is drawn, viewport pixels.
    ///
    /// For anything that is not a disc — a belt, a ring, a trajectory — the nearest point of it
    /// to the cursor, with a [`radius_px`](Self::radius_px) of zero. A band is then clickable
    /// along its whole length without being a disc, and without this module knowing what a band
    /// is.
    pub at: Vec2,
    /// How big it is drawn. Zero for a point.
    pub radius_px: f32,
    /// What sort of thing it is. **Lower wins outright**, whatever the distances are.
    ///
    /// The ordering is a statement about intent rather than about geometry: a swarm is drawn
    /// across half the sky and would otherwise swallow every planet inside it. See
    /// [`rank`](self) for the scale both products use.
    pub rank: u8,
}

/// The ranks, lowest first. A caller may use its own, but these are what the two products mean.
pub mod rank {
    /// A craft. Never more than a few pixels, and unreachable if anything outranks it.
    pub const CRAFT: u8 = 0;
    /// A planet, moon or asteroid.
    pub const BODY: u8 = 1;
    /// A star, including the local one.
    ///
    /// Below bodies deliberately. From an inner orbit the primary's disc can be most of the
    /// screen, and a click next to Mercury means Mercury.
    pub const STAR: u8 = 2;
    /// A belt, a ring system, a cloud: something with extent and no surface.
    pub const SWARM: u8 = 3;
}

/// Which candidate the cursor is on, if any.
///
/// In reach when the cursor is within `slack_px` of the thing as drawn. Among those, the lowest
/// [`rank`](Candidate::rank) wins outright; within a rank, the one whose *centre* is nearest.
///
/// Nearest centre rather than nearest edge, which is what makes a small body in front of a
/// large one selectable: a big disc's centre is far from wherever you clicked on it, so a moon
/// three pixels from the cursor beats the planet behind it without needing a rule of its own.
pub fn pick(candidates: &[Candidate], cursor: Vec2, slack_px: f32) -> Option<u64> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let distance = cursor.distance(candidate.at);
            let reach = candidate.radius_px.max(0.0) + slack_px.max(0.0);
            (distance <= reach).then_some((candidate.rank, distance, candidate.id))
        })
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .map(|(_, _, id)| id)
}

/// The nearest point to `cursor` on any of `runs`, and how far off it is.
///
/// For a candidate with extent and no surface — a belt, a ring, a trajectory. The caller passes
/// the nearest point as the candidate's [`at`](Candidate::at) with a zero radius, which makes
/// the whole length of it clickable through the ordinary slack without this module needing to
/// know what a belt is.
///
/// Segments, not vertices. A ring sampled every few degrees has vertices far apart where it
/// passes close by, and snapping to the nearest of those puts the answer — and the mark drawn
/// on it — visibly off the line.
pub fn nearest_on_path(runs: &[Vec<Vec2>], cursor: Vec2) -> Option<(Vec2, f32)> {
    runs.iter()
        .flat_map(|run| run.windows(2))
        .map(|pair| {
            let at = nearest_on_segment(pair[0], pair[1], cursor);
            (at, cursor.distance(at))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The point of the segment `a`-`b` nearest `to`.
fn nearest_on_segment(a: Vec2, b: Vec2, to: Vec2) -> Vec2 {
    let span = b - a;
    let length_squared = span.length_squared();
    if length_squared <= f32::MIN_POSITIVE {
        return a;
    }
    a + span * ((to - a).dot(span) / length_squared).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: u64, x: f32, y: f32, radius_px: f32, rank: u8) -> Candidate {
        Candidate { id, at: Vec2::new(x, y), radius_px, rank }
    }

    #[test]
    fn nothing_under_the_cursor_is_nothing() {
        let far = [at(1, 500.0, 500.0, 4.0, rank::BODY)];
        assert_eq!(pick(&far, Vec2::ZERO, SLACK_PX), None);
        assert_eq!(pick(&[], Vec2::ZERO, SLACK_PX), None);
    }

    /// The reason the slack exists: a moon four pixels across is not hittable exactly.
    #[test]
    fn a_point_is_reachable_without_being_hit_exactly() {
        let moon = [at(1, 100.0, 100.0, 2.0, rank::BODY)];
        assert_eq!(pick(&moon, Vec2::new(108.0, 100.0), SLACK_PX), Some(1));
        assert_eq!(pick(&moon, Vec2::new(130.0, 100.0), SLACK_PX), None);
    }

    /// The case the old mass rule got wrong. Close to a planet *and* its moon, aiming at the
    /// moon selects the moon — even though the planet is a thousand times the size on screen.
    #[test]
    fn a_moon_in_front_of_its_planet_is_selectable() {
        let scene = [
            at(1, 400.0, 400.0, 380.0, rank::BODY), // the planet, filling the view
            at(2, 420.0, 390.0, 3.0, rank::BODY),   // the moon, near its centre
        ];
        assert_eq!(pick(&scene, Vec2::new(421.0, 391.0), SLACK_PX), Some(2));
        // And the planet is still what you get everywhere else on it.
        assert_eq!(pick(&scene, Vec2::new(600.0, 500.0), SLACK_PX), Some(1));
        assert_eq!(pick(&scene, Vec2::new(400.0, 400.0), SLACK_PX), Some(1));
    }

    /// Rank beats distance outright, which is what keeps a swarm from swallowing a system.
    #[test]
    fn a_planet_beats_a_swarm_the_cursor_is_dead_centre_on() {
        let scene = [
            at(1, 300.0, 300.0, 0.0, rank::SWARM), // the band, right under the cursor
            at(2, 308.0, 300.0, 2.0, rank::BODY),  // a planet, eight pixels off
        ];
        assert_eq!(pick(&scene, Vec2::new(300.0, 300.0), SLACK_PX), Some(2));
        // With no planet in reach the swarm is selectable, which is the other half of the rule.
        assert_eq!(pick(&scene[..1], Vec2::new(300.0, 300.0), SLACK_PX), Some(1));
    }

    /// And the primary's disc does not swallow the inner planets in front of it.
    #[test]
    fn a_planet_beats_the_star_it_is_drawn_against() {
        let scene = [
            at(1, 640.0, 360.0, 400.0, rank::STAR),
            at(2, 700.0, 300.0, 1.0, rank::BODY),
        ];
        assert_eq!(pick(&scene, Vec2::new(701.0, 301.0), SLACK_PX), Some(2));
        assert_eq!(pick(&scene, Vec2::new(500.0, 360.0), SLACK_PX), Some(1));
    }

    /// A craft outranks everything, because at a few pixels it is unreachable otherwise.
    #[test]
    fn a_craft_outranks_the_world_it_is_in_front_of() {
        let scene = [
            at(1, 200.0, 200.0, 150.0, rank::BODY),
            at(2, 205.0, 200.0, 0.0, rank::CRAFT),
        ];
        assert_eq!(pick(&scene, Vec2::new(203.0, 200.0), SLACK_PX), Some(2));
    }

    /// Ties inside a rank go to the nearer, and the answer does not depend on the order the
    /// candidates came in.
    #[test]
    fn the_order_of_the_candidates_does_not_decide() {
        let mut scene = vec![
            at(1, 100.0, 100.0, 1.0, rank::BODY),
            at(2, 104.0, 100.0, 1.0, rank::BODY),
        ];
        let cursor = Vec2::new(103.0, 100.0);
        assert_eq!(pick(&scene, cursor, SLACK_PX), Some(2));
        scene.reverse();
        assert_eq!(pick(&scene, cursor, SLACK_PX), Some(2));
    }

    /// No slack means the cursor has to be on the drawn thing.
    #[test]
    fn zero_slack_is_exactly_the_drawn_shape() {
        let disc = [at(1, 0.0, 0.0, 10.0, rank::BODY)];
        assert_eq!(pick(&disc, Vec2::new(9.9, 0.0), 0.0), Some(1));
        assert_eq!(pick(&disc, Vec2::new(10.1, 0.0), 0.0), None);
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// Along the segment, not snapped to an end of it. A ring sampled every few degrees has
    /// vertices a long way apart where it passes close by.
    #[test]
    fn the_nearest_point_is_on_the_line_and_not_at_a_vertex() {
        let run = vec![vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)]];
        let (at, distance) = nearest_on_path(&run, Vec2::new(40.0, 7.0)).expect("a segment");
        assert_eq!(at, Vec2::new(40.0, 0.0));
        assert!((distance - 7.0).abs() < 1.0e-4);
    }

    /// Past an end it stops at the end, rather than running off along the line.
    #[test]
    fn a_cursor_past_the_end_gets_the_end() {
        let run = vec![vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)]];
        let (at, _) = nearest_on_path(&run, Vec2::new(140.0, 0.0)).unwrap();
        assert_eq!(at, Vec2::new(100.0, 0.0));
    }

    /// Several runs — which is what a ring cut by the camera plane comes back as — and the
    /// nearest of all of them wins.
    #[test]
    fn the_nearest_of_several_runs_wins() {
        let runs = vec![
            vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)],
            vec![Vec2::new(0.0, 50.0), Vec2::new(100.0, 50.0)],
        ];
        let (at, _) = nearest_on_path(&runs, Vec2::new(50.0, 40.0)).unwrap();
        assert_eq!(at.y, 50.0);
        assert!(nearest_on_path(&[], Vec2::ZERO).is_none());
        assert!(nearest_on_path(&[vec![Vec2::ZERO]], Vec2::ZERO).is_none(), "a point is not a path");
    }

    /// A degenerate segment is a point, not a division by zero.
    #[test]
    fn a_zero_length_segment_is_its_own_nearest_point() {
        let run = vec![vec![Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0)]];
        let (at, distance) = nearest_on_path(&run, Vec2::new(5.0, 9.0)).unwrap();
        assert_eq!(at, Vec2::new(5.0, 5.0));
        assert!((distance - 4.0).abs() < 1.0e-4);
    }
}
