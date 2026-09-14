//! Saying what is picked: a mark on the thing, or an arrow at the edge when it is not on screen.
//!
//! Screen-space geometry and nothing else. Everything comes back as line segments in viewport
//! pixels, so the look is shared while the drawing stays the caller's — both products already
//! paint overlays through egui and neither needs this crate to learn how.

use bevy::math::{Rect, Vec2, Vec4};

/// How far inside the viewport edge an off-screen arrow sits, pixels.
pub const EDGE_INSET_PX: f32 = 10.0;

/// Gap between a thing and its label, pixels.
pub const LABEL_GAP_PX: f32 = 6.0;

/// Smallest ring drawn round a hovered thing, pixels. A point needs something to be circled.
pub const MIN_RING_PX: f32 = 9.0;

/// Where the indicator for a target goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Marker {
    /// On the thing itself.
    On { at: Vec2, radius_px: f32 },
    /// Off the edge: an arrow on the safe rect, pointing the way the thing lies.
    Off { at: Vec2, direction: Vec2 },
}

/// Place the indicator for something at `clip`, its position in **clip space** — the projection
/// applied, the perspective divide *not*.
///
/// Clip space rather than a viewport position, because a viewport position cannot describe the
/// half of the sky that matters here. Bevy's `world_to_viewport` returns an error for anything
/// behind the camera, so an arrow pointing at what is behind you cannot be built from it at all.
///
/// And the naive repair — divide anyway — is worse than nothing. With the camera down `-Z`,
/// `clip.w` is `-view.z`, so behind the camera it is *negative* while `clip.x` keeps the sign of
/// `view.x`. Something behind you and to the right divides to an `ndc.x` of the wrong sign and
/// the arrow goes to the left edge, pointing away from it. So `w <= 0` never divides: it takes
/// the direction straight from `clip.xy`, whose sign was right all along.
pub fn place(clip: Vec4, radius_px: f32, viewport: Vec2, inset_px: f32) -> Marker {
    let safe = safe_rect(viewport, inset_px);
    let centre = viewport * 0.5;

    if clip.w <= 0.0 {
        // Behind the camera, or exactly abeam. The projected point is meaningless; the way to
        // turn is not, and it is the sign `clip.xy` already carries.
        let direction = to_screen_direction(clip.x, clip.y);
        return Marker::Off { at: on_edge(direction, safe), direction };
    }

    let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
    let at = Vec2::new((ndc.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc.y) * 0.5 * viewport.y);
    if safe.contains(at) {
        return Marker::On { at, radius_px };
    }
    let direction = (at - centre).normalize_or_zero();
    let direction = if direction == Vec2::ZERO { Vec2::X } else { direction };
    Marker::Off { at: on_edge(direction, safe), direction }
}

/// Where a label of `size` goes for a thing at `anchor` of `radius_px`, in the safe rect.
///
/// Above the thing by preference — clear of it, and clear of whatever is drawn on it. When that
/// would not fit, below, then right, then left, and if none of them fit the first is clamped
/// in. `preferred` is tried before all of those, which is how an off-screen arrow gets its
/// label on the inward side rather than stacked above it.
///
/// Returns the label's **centre**.
pub fn place_label(
    anchor: Vec2,
    radius_px: f32,
    size: Vec2,
    safe: Rect,
    gap_px: f32,
    preferred: Option<Vec2>,
) -> Vec2 {
    const LADDER: [Vec2; 4] =
        [Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)];

    let offset = |direction: Vec2| {
        let d = direction.normalize_or_zero();
        // How far the label reaches along `d`, so a wide label clears more sideways than up.
        let reach = d.x.abs() * size.x * 0.5 + d.y.abs() * size.y * 0.5;
        anchor + d * (radius_px.max(0.0) + gap_px + reach)
    };
    let fits = |centre: Vec2| {
        let half = size * 0.5;
        (centre - half).cmpge(safe.min).all() && (centre + half).cmple(safe.max).all()
    };

    let first = preferred.map(offset).unwrap_or_else(|| offset(LADDER[0]));
    preferred
        .into_iter()
        .chain(LADDER)
        .map(offset)
        .find(|centre| fits(*centre))
        .unwrap_or_else(|| clamp_into(first, size, safe))
}

/// The viewport less its border. Where an arrow or a label is allowed to be.
pub fn safe_rect(viewport: Vec2, inset_px: f32) -> Rect {
    let inset = Vec2::splat(inset_px.max(0.0)).min(viewport * 0.5);
    Rect::from_corners(inset, (viewport - inset).max(inset))
}

/// A ring, for whatever the cursor is over. `segments` sets how round it looks.
pub fn ring(at: Vec2, radius_px: f32, segments: usize) -> Vec<[Vec2; 2]> {
    let radius = radius_px.max(MIN_RING_PX);
    let count = segments.max(3);
    (0..count)
        .map(|i| {
            let step = |k: usize| {
                let theta = std::f32::consts::TAU * k as f32 / count as f32;
                at + Vec2::new(theta.cos(), theta.sin()) * radius
            };
            [step(i), step(i + 1)]
        })
        .collect()
}

/// Four corner brackets, for what is selected. Open on the sides, so the thing stays visible.
pub fn brackets(at: Vec2, radius_px: f32, arm_px: f32) -> Vec<[Vec2; 2]> {
    let r = radius_px.max(MIN_RING_PX);
    let arm = arm_px.max(1.0).min(r);
    let mut out = Vec::with_capacity(8);
    for sx in [-1.0f32, 1.0] {
        for sy in [-1.0f32, 1.0] {
            let corner = at + Vec2::new(sx * r, sy * r);
            out.push([corner, corner - Vec2::new(sx * arm, 0.0)]);
            out.push([corner, corner - Vec2::new(0.0, sy * arm)]);
        }
    }
    out
}

/// A solid-looking arrowhead at the edge, pointing along `direction`.
pub fn arrow(at: Vec2, direction: Vec2, size_px: f32) -> Vec<[Vec2; 2]> {
    let d = direction.normalize_or_zero();
    if d == Vec2::ZERO {
        return Vec::new();
    }
    let side = Vec2::new(-d.y, d.x);
    let tip = at + d * size_px;
    let left = at - d * size_px * 0.4 + side * size_px * 0.6;
    let right = at - d * size_px * 0.4 - side * size_px * 0.6;
    vec![[tip, left], [left, right], [right, tip]]
}

/// A screen-space direction from clip-space `xy`. Clip `y` is up, the viewport's is down.
fn to_screen_direction(x: f32, y: f32) -> Vec2 {
    let d = Vec2::new(x, -y).normalize_or_zero();
    if d == Vec2::ZERO { Vec2::X } else { d }
}

/// Where a ray from the middle of `safe` along `direction` leaves it.
fn on_edge(direction: Vec2, safe: Rect) -> Vec2 {
    let half = (safe.max - safe.min) * 0.5;
    let middle = (safe.max + safe.min) * 0.5;
    // The smaller of the two axis crossings is the one the ray actually meets.
    let scale = |d: f32, extent: f32| if d.abs() > 1e-6 { (extent / d).abs() } else { f32::MAX };
    let t = scale(direction.x, half.x).min(scale(direction.y, half.y));
    (middle + direction * t).clamp(safe.min, safe.max)
}

fn clamp_into(centre: Vec2, size: Vec2, safe: Rect) -> Vec2 {
    let half = size * 0.5;
    centre.clamp(safe.min + half, (safe.max - half).max(safe.min + half))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Vec2 = Vec2::new(1280.0, 720.0);

    /// Clip space from a normalised device position and a depth, which is what a projection
    /// would have produced for something in front of the camera.
    fn in_front(ndc_x: f32, ndc_y: f32, w: f32) -> Vec4 {
        Vec4::new(ndc_x * w, ndc_y * w, 0.0, w)
    }

    fn safe() -> Rect {
        safe_rect(VIEW, EDGE_INSET_PX)
    }

    #[test]
    fn something_in_the_middle_of_the_view_is_marked_where_it_is() {
        let Marker::On { at, radius_px } = place(in_front(0.0, 0.0, 5.0), 20.0, VIEW, EDGE_INSET_PX)
        else {
            panic!("the middle of the screen is on screen")
        };
        assert_eq!(at, VIEW * 0.5);
        assert_eq!(radius_px, 20.0);

        // Clip `y` is up and the viewport's is down, so the top of the screen is +1.
        let Marker::On { at, .. } = place(in_front(0.0, 0.9, 5.0), 0.0, VIEW, EDGE_INSET_PX) else {
            panic!("still on screen")
        };
        assert!(at.y < VIEW.y * 0.5, "up in clip space should be up on screen: {at}");
    }

    /// The whole reason this takes clip space, and the sign that is easy to get backwards.
    ///
    /// With the camera down `-Z`, `clip.w` is `-view.z`: behind the camera it is negative while
    /// `clip.x` still carries the sign of `view.x`. So something behind you and to the right
    /// has a *positive* `clip.x` and divides to a negative `ndc.x` — the arrow goes to the left
    /// edge, pointing away from the thing. The first version of this negated as well, and the
    /// first version of this test agreed with it. The numbers below are a worked 45-degree
    /// projection, not a reading of the code.
    #[test]
    fn something_behind_the_camera_points_the_way_it_actually_is() {
        // Five metres behind the camera and two to the right.
        let behind = Vec4::new(2.716, 0.0, -1.0, -5.0);
        let Marker::Off { at, direction } = place(behind, 0.0, VIEW, EDGE_INSET_PX) else {
            panic!("behind the camera is not on screen")
        };
        assert!(direction.x > 0.0, "it is behind and to the right: {direction}");
        assert!((at.x - (VIEW.x - EDGE_INSET_PX)).abs() < 1.0, "on the right edge, not at {at}");

        // Dividing anyway would have sent it to the opposite edge.
        assert!(behind.x / behind.w < 0.0, "the naive reading says left");

        // And its mirror image goes to the other side, as it must.
        let other = Vec4::new(-2.716, 0.0, -1.0, -5.0);
        let Marker::Off { direction, .. } = place(other, 0.0, VIEW, EDGE_INSET_PX) else {
            panic!("still behind")
        };
        assert!(direction.x < 0.0, "{direction}");
    }

    /// An arrow sits inside the border, on every side, and never outside it.
    #[test]
    fn an_arrow_stays_inside_the_safe_border() {
        for (x, y) in [(4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0), (3.0, 3.0)] {
            let Marker::Off { at, .. } = place(in_front(x, y, 1.0), 0.0, VIEW, EDGE_INSET_PX)
            else {
                panic!("{x},{y} is off the screen")
            };
            let safe = safe();
            assert!(safe.contains(at), "{at} is outside {safe:?}");
            // And actually *on* the border, not somewhere in the middle of it.
            let on_border = (at.x - safe.min.x).abs() < 1.0
                || (at.x - safe.max.x).abs() < 1.0
                || (at.y - safe.min.y).abs() < 1.0
                || (at.y - safe.max.y).abs() < 1.0;
            assert!(on_border, "{at} is not on the edge of {safe:?}");
        }
    }

    /// Exactly abeam — a `w` of zero, the camera's own plane — is placed rather than divided
    /// by zero.
    #[test]
    fn something_exactly_abeam_is_still_placed() {
        let to_the_right = Vec4::new(1.0, 0.0, 0.0, 0.0);
        let Marker::Off { at, direction } = place(to_the_right, 0.0, VIEW, EDGE_INSET_PX) else {
            panic!("nothing in the camera's own plane is on screen")
        };
        assert!(direction.x > 0.0 && at.is_finite(), "{direction} {at}");
    }

    #[test]
    fn a_label_sits_above_what_it_names_when_there_is_room() {
        let at = Vec2::new(640.0, 360.0);
        let size = Vec2::new(80.0, 18.0);
        let centre = place_label(at, 30.0, size, safe(), LABEL_GAP_PX, None);
        assert_eq!(centre.x, at.x, "centred over it");
        assert!(centre.y < at.y - 30.0, "above the top of it: {centre}");
        // Clear of the thing by the gap, and no further.
        assert!((at.y - 30.0 - LABEL_GAP_PX - size.y * 0.5 - centre.y).abs() < 1.0e-3);
    }

    /// Against the top of the screen it goes below instead, rather than off the edge.
    #[test]
    fn a_label_with_no_room_above_falls_below() {
        let at = Vec2::new(640.0, 20.0);
        let size = Vec2::new(80.0, 18.0);
        let centre = place_label(at, 12.0, size, safe(), LABEL_GAP_PX, None);
        assert!(centre.y > at.y, "it should have dropped below: {centre}");
        assert!(safe().contains(centre));
    }

    /// In a corner, with neither above nor below available, it goes to the side.
    #[test]
    fn a_label_boxed_in_goes_sideways_and_then_is_clamped() {
        let size = Vec2::new(80.0, 18.0);
        let corner = Vec2::new(60.0, 18.0);
        let centre = place_label(corner, 40.0, size, safe(), LABEL_GAP_PX, None);
        assert!(safe().contains(centre), "{centre} escaped the border");

        // A viewport too small for the label at all still returns something inside it.
        let tiny = safe_rect(Vec2::new(40.0, 30.0), EDGE_INSET_PX);
        let squeezed = place_label(Vec2::new(20.0, 15.0), 5.0, size, tiny, LABEL_GAP_PX, None);
        assert!(squeezed.is_finite());
    }

    /// An off-screen arrow's label goes on the inward side, which is what `preferred` is for.
    #[test]
    fn an_arrow_takes_its_label_inward() {
        let safe = safe();
        let pointing_left = Vec2::new(-1.0, 0.0);
        let at = Vec2::new(safe.min.x, 360.0);
        let size = Vec2::new(90.0, 18.0);
        let centre = place_label(at, 12.0, size, safe, LABEL_GAP_PX, Some(-pointing_left));
        assert!(centre.x > at.x, "the label should be inboard of the arrow: {centre}");
        assert!(safe.contains(centre));
    }

    #[test]
    fn a_ring_closes_and_keeps_its_radius() {
        let at = Vec2::new(100.0, 100.0);
        let segments = ring(at, 40.0, 32);
        assert_eq!(segments.len(), 32);
        for [a, _] in &segments {
            assert!((a.distance(at) - 40.0).abs() < 1.0e-3);
        }
        // Each segment starts where the last ended, all the way round.
        for pair in segments.windows(2) {
            assert!(pair[0][1].distance(pair[1][0]) < 1.0e-3);
        }
        assert!(segments.last().unwrap()[1].distance(segments[0][0]) < 1.0e-3);
        // A point still gets something visible.
        assert!(ring(at, 0.0, 16)[0][0].distance(at) >= MIN_RING_PX - 1.0e-3);
    }

    /// Brackets leave the sides open, which is the whole point of them: the thing stays visible.
    #[test]
    fn brackets_are_four_open_corners() {
        let at = Vec2::new(200.0, 200.0);
        let segments = brackets(at, 50.0, 12.0);
        assert_eq!(segments.len(), 8, "two arms at each of four corners");
        for [a, b] in &segments {
            for point in [a, b] {
                let d = *point - at;
                assert!(d.x.abs() <= 50.0 + 1.0e-3 && d.y.abs() <= 50.0 + 1.0e-3);
            }
        }
        // Nothing crosses the middle of an edge, so the shape is corners and not a box.
        let mid_top = at + Vec2::new(0.0, -50.0);
        assert!(segments.iter().all(|[a, b]| a.distance(mid_top) > 1.0 && b.distance(mid_top) > 1.0));
    }

    #[test]
    fn an_arrow_is_a_closed_triangle_pointing_the_right_way() {
        let at = Vec2::new(10.0, 300.0);
        let segments = arrow(at, Vec2::new(-1.0, 0.0), 12.0);
        assert_eq!(segments.len(), 3);
        let tip = segments[0][0];
        assert!(tip.x < at.x, "the tip should lead: {tip}");
        assert!(segments[2][1].distance(tip) < 1.0e-3, "the triangle should close");
        assert!(arrow(at, Vec2::ZERO, 12.0).is_empty(), "no direction, nothing to draw");
    }
}
