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

/// Where marks may go, and what is in the way.
///
/// The outer bound alone is not enough. A docked panel is excluded from it — that is what an
/// interface's "available rect" means — but a floating window or a notice is not, and an arrow
/// or a label placed against the outer edge lands behind one. Those go in
/// [`occupied`](Self::occupied), and an arrow is drawn back along its own ray until it is clear
/// of them: still pointing the same way, just further in.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The outer limit, already inset by whatever border it should keep.
    pub safe: Rect,
    /// Rectangles a mark must stay out of, in the same coordinates.
    pub occupied: &'a [Rect],
    /// How much room a mark needs around its anchor, pixels. An arrow's own length, so the
    /// whole head clears rather than its tip alone.
    pub clearance_px: f32,
}

impl<'a> Frame<'a> {
    /// A frame with nothing in the way.
    pub fn bare(safe: Rect) -> Self {
        Self { safe, occupied: &[], clearance_px: 0.0 }
    }

    pub fn with(safe: Rect, occupied: &'a [Rect], clearance_px: f32) -> Self {
        Self { safe, occupied, clearance_px }
    }

    /// Whether `rect` is clear of everything taken.
    pub fn is_clear(&self, rect: Rect) -> bool {
        self.occupied.iter().all(|taken| taken.intersect(rect).is_empty())
    }
}

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
/// `viewport` maps the projection onto pixels; `safe` is where a mark is allowed to be, which is
/// not the same rectangle. It is the part of the window the view is actually visible through,
/// less a border — so with a panel open, an edge arrow sits at the edge of the *sky* rather than
/// behind the panel. [`safe_rect`] builds the whole-window case.
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
pub fn place(clip: Vec4, radius_px: f32, viewport: Vec2, frame: Frame<'_>) -> Marker {
    let center = (frame.safe.max + frame.safe.min) * 0.5;

    if clip.w <= 0.0 {
        // Behind the camera, or exactly abeam. The projected point is meaningless; the way to
        // turn is not, and it is the sign `clip.xy` already carries.
        let direction = to_screen_direction(clip.x, clip.y);
        return Marker::Off { at: on_edge(center, direction, frame), direction };
    }

    let at = to_screen(clip, viewport);
    if frame.safe.contains(at) {
        return Marker::On { at, radius_px };
    }
    let direction = (at - center).normalize_or_zero();
    let direction = if direction == Vec2::ZERO { Vec2::X } else { direction };
    Marker::Off { at: on_edge(center, direction, frame), direction }
}

/// Where a label of `size` goes for a thing at `anchor` of `radius_px`, in the safe rect.
///
/// Above the thing by preference — clear of it, and clear of whatever is drawn on it. When that
/// would not fit, or would land on something the interface has taken, below, then right, then
/// left; and if none of them fit the first is clamped in. `preferred` is tried before all of those, which is how an off-screen arrow gets its
/// label on the inward side rather than stacked above it.
///
/// Returns the label's **center**.
pub fn place_label(
    anchor: Vec2,
    radius_px: f32,
    size: Vec2,
    frame: Frame<'_>,
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
    let fits = |center: Vec2| {
        let half = size * 0.5;
        let box_ = Rect::from_corners(center - half, center + half);
        (center - half).cmpge(frame.safe.min).all()
            && (center + half).cmple(frame.safe.max).all()
            // "Does not fit" covers "is covered", which is what keeps a label off a notice box.
            && frame.is_clear(box_)
    };

    let first = preferred.map(offset).unwrap_or_else(|| offset(LADDER[0]));
    preferred
        .into_iter()
        .chain(LADDER)
        .map(offset)
        .find(|center| fits(*center))
        .unwrap_or_else(|| clamp_into(first, size, frame.safe))
}

/// How far past the camera plane a clipped vertex is placed, as a fraction of the visible
/// end's `w`.
///
/// A line crossing the camera plane goes to infinity on screen, so the crossing vertex has to
/// stop somewhere. Two per cent puts it about fifty screen-widths out, which reads as "off the
/// edge" at any zoom and stays a finite number.
const CLIP_FRACTION: f32 = 0.02;

/// How far outside the viewport a projected vertex may land before it is pulled in, in
/// viewport widths. Purely to keep a renderer from being handed an astronomical coordinate.
const MAX_OVERSHOOT: f32 = 60.0;

/// A path of clip-space points as runs of screen positions, cut where it crosses behind the
/// camera.
///
/// One run per stretch that is in front of the camera; a closed curve seen from inside comes
/// back as two open runs, or as none when it is entirely behind. This is the part that cannot
/// be done by projecting each point and discarding the failures: dropping the vertices that
/// are behind leaves a segment joining the two survivors *across* the view, which draws a
/// chord through a ring that has no chord.
pub fn project_path(clips: &[Vec4], viewport: Vec2) -> Vec<Vec<Vec2>> {
    let mut runs: Vec<Vec<Vec2>> = Vec::new();
    let mut run: Vec<Vec2> = Vec::new();

    for pair in clips.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        match (a.w > 0.0, b.w > 0.0) {
            (true, true) => {
                if run.is_empty() {
                    run.push(to_screen(a, viewport));
                }
                run.push(to_screen(b, viewport));
            }
            // Leaving: carry the line out to where it crosses, then end the run.
            (true, false) => {
                if run.is_empty() {
                    run.push(to_screen(a, viewport));
                }
                run.push(to_screen(cross(a, b), viewport));
                runs.push(std::mem::take(&mut run));
            }
            // Arriving: start a fresh run at the crossing.
            (false, true) => {
                if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
                run.push(to_screen(cross(b, a), viewport));
                run.push(to_screen(b, viewport));
            }
            (false, false) => {
                if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs.retain(|run| run.len() > 1);
    runs
}

/// A run of screen positions as line segments, for a caller that draws in pairs.
pub fn path_segments(runs: &[Vec<Vec2>]) -> Vec<[Vec2; 2]> {
    runs.iter().flat_map(|run| run.windows(2).map(|p| [p[0], p[1]])).collect()
}

/// Where the segment from `visible` to `behind` crosses the camera plane, as a clip-space
/// point just in front of it.
fn cross(visible: Vec4, behind: Vec4) -> Vec4 {
    let target = visible.w * CLIP_FRACTION;
    let span = behind.w - visible.w;
    let t = if span.abs() > f32::MIN_POSITIVE { (target - visible.w) / span } else { 0.0 };
    visible + (behind - visible) * t.clamp(0.0, 1.0)
}

/// Clip space to viewport pixels. Only meaningful for a positive `w`.
fn to_screen(clip: Vec4, viewport: Vec2) -> Vec2 {
    let w = clip.w.max(f32::MIN_POSITIVE);
    let ndc = Vec2::new(clip.x / w, clip.y / w);
    let at = Vec2::new((ndc.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc.y) * 0.5 * viewport.y);
    let bound = viewport * MAX_OVERSHOOT;
    at.clamp(-bound, bound)
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

/// An arrowhead pointing along `direction`, with its **tip at `at`**.
///
/// The tip, not the middle. `at` comes from [`place`], which puts it on the safe rect, and the
/// whole reason for that inset is that the arrow be visible: a triangle centered there reaches
/// `size_px` past the border and loses its point off the edge of the window.
pub fn arrow(at: Vec2, direction: Vec2, size_px: f32) -> Vec<[Vec2; 2]> {
    let d = direction.normalize_or_zero();
    if d == Vec2::ZERO {
        return Vec::new();
    }
    let side = Vec2::new(-d.y, d.x);
    let back = at - d * size_px;
    let left = back + side * size_px * 0.5;
    let right = back - side * size_px * 0.5;
    vec![[at, left], [left, right], [right, at]]
}

/// A screen-space direction from clip-space `xy`. Clip `y` is up, the viewport's is down.
fn to_screen_direction(x: f32, y: f32) -> Vec2 {
    let d = Vec2::new(x, -y).normalize_or_zero();
    if d == Vec2::ZERO { Vec2::X } else { d }
}

/// How far out along `direction` a mark may sit: the safe edge, or short of it when something
/// the interface has taken is in the way.
///
/// Drawn back along its own ray rather than slid sideways, so the arrow keeps pointing where
/// the thing actually is — it only moves closer to the middle.
fn on_edge(center: Vec2, direction: Vec2, frame: Frame<'_>) -> Vec2 {
    let half = (frame.safe.max - frame.safe.min) * 0.5;
    // The smaller of the two axis crossings is the one the ray actually meets.
    let scale = |d: f32, extent: f32| if d.abs() > 1e-6 { (extent / d).abs() } else { f32::MAX };
    let mut reach = scale(direction.x, half.x).min(scale(direction.y, half.y));

    let clearance = frame.clearance_px.max(0.0);
    for taken in frame.occupied {
        let grown =
            Rect::from_corners(taken.min - Vec2::splat(clearance), taken.max + Vec2::splat(clearance));
        if let Some(entry) = entry_along(center, direction, grown) {
            reach = reach.min(entry);
        }
    }
    (center + direction * reach.max(0.0)).clamp(frame.safe.min, frame.safe.max)
}

/// Where a ray from `origin` along `direction` first enters `rect`, if it does ahead of the
/// origin.
///
/// `None` when it misses, when the rectangle is behind, and — deliberately — when the origin is
/// already inside it. Something covering the middle of the view cannot be escaped by moving
/// inward, and collapsing every arrow onto the center is worse than drawing over it.
fn entry_along(origin: Vec2, direction: Vec2, rect: Rect) -> Option<f32> {
    let slab = |o: f32, d: f32, lo: f32, hi: f32| -> Option<(f32, f32)> {
        if d.abs() < 1.0e-6 {
            return (o >= lo && o <= hi).then_some((f32::NEG_INFINITY, f32::INFINITY));
        }
        let (a, b) = ((lo - o) / d, (hi - o) / d);
        Some((a.min(b), a.max(b)))
    };
    let (x0, x1) = slab(origin.x, direction.x, rect.min.x, rect.max.x)?;
    let (y0, y1) = slab(origin.y, direction.y, rect.min.y, rect.max.y)?;
    let near = x0.max(y0);
    let far = x1.min(y1);
    (near <= far && near > 0.0).then_some(near)
}

fn clamp_into(center: Vec2, size: Vec2, safe: Rect) -> Vec2 {
    let half = size * 0.5;
    center.clamp(safe.min + half, (safe.max - half).max(safe.min + half))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Vec2 = Vec2::new(1280.0, 720.0);

    /// Clip space from a normalized device position and a depth, which is what a projection
    /// would have produced for something in front of the camera.
    fn in_front(ndc_x: f32, ndc_y: f32, w: f32) -> Vec4 {
        Vec4::new(ndc_x * w, ndc_y * w, 0.0, w)
    }

    /// An arrow's own length, so the whole head clears rather than its tip alone.
    const ARROW_CLEARANCE: f32 = 11.0;

    fn safe() -> Rect {
        safe_rect(VIEW, EDGE_INSET_PX)
    }

    #[test]
    fn something_in_the_middle_of_the_view_is_marked_where_it_is() {
        let Marker::On { at, radius_px } = place(in_front(0.0, 0.0, 5.0), 20.0, VIEW, Frame::bare(safe()))
        else {
            panic!("the middle of the screen is on screen")
        };
        assert_eq!(at, VIEW * 0.5);
        assert_eq!(radius_px, 20.0);

        // Clip `y` is up and the viewport's is down, so the top of the screen is +1.
        let Marker::On { at, .. } = place(in_front(0.0, 0.9, 5.0), 0.0, VIEW, Frame::bare(safe())) else {
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
        // Five meters behind the camera and two to the right.
        let behind = Vec4::new(2.716, 0.0, -1.0, -5.0);
        let Marker::Off { at, direction } = place(behind, 0.0, VIEW, Frame::bare(safe())) else {
            panic!("behind the camera is not on screen")
        };
        assert!(direction.x > 0.0, "it is behind and to the right: {direction}");
        assert!((at.x - (VIEW.x - EDGE_INSET_PX)).abs() < 1.0, "on the right edge, not at {at}");

        // Dividing anyway would have sent it to the opposite edge.
        assert!(behind.x / behind.w < 0.0, "the naive reading says left");

        // And its mirror image goes to the other side, as it must.
        let other = Vec4::new(-2.716, 0.0, -1.0, -5.0);
        let Marker::Off { direction, .. } = place(other, 0.0, VIEW, Frame::bare(safe())) else {
            panic!("still behind")
        };
        assert!(direction.x < 0.0, "{direction}");
    }

    /// An arrow sits inside the border, on every side, and never outside it.
    #[test]
    fn an_arrow_stays_inside_the_safe_border() {
        for (x, y) in [(4.0, 0.0), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0), (3.0, 3.0)] {
            let Marker::Off { at, .. } = place(in_front(x, y, 1.0), 0.0, VIEW, Frame::bare(safe()))
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
        let Marker::Off { at, direction } = place(to_the_right, 0.0, VIEW, Frame::bare(safe())) else {
            panic!("nothing in the camera's own plane is on screen")
        };
        assert!(direction.x > 0.0 && at.is_finite(), "{direction} {at}");
    }

    #[test]
    fn a_label_sits_above_what_it_names_when_there_is_room() {
        let at = Vec2::new(640.0, 360.0);
        let size = Vec2::new(80.0, 18.0);
        let center = place_label(at, 30.0, size, Frame::bare(safe()), LABEL_GAP_PX, None);
        assert_eq!(center.x, at.x, "centered over it");
        assert!(center.y < at.y - 30.0, "above the top of it: {center}");
        // Clear of the thing by the gap, and no further.
        assert!((at.y - 30.0 - LABEL_GAP_PX - size.y * 0.5 - center.y).abs() < 1.0e-3);
    }

    /// Against the top of the screen it goes below instead, rather than off the edge.
    #[test]
    fn a_label_with_no_room_above_falls_below() {
        let at = Vec2::new(640.0, 20.0);
        let size = Vec2::new(80.0, 18.0);
        let center = place_label(at, 12.0, size, Frame::bare(safe()), LABEL_GAP_PX, None);
        assert!(center.y > at.y, "it should have dropped below: {center}");
        assert!(safe().contains(center));
    }

    /// In a corner, with neither above nor below available, it goes to the side.
    #[test]
    fn a_label_boxed_in_goes_sideways_and_then_is_clamped() {
        let size = Vec2::new(80.0, 18.0);
        let corner = Vec2::new(60.0, 18.0);
        let center = place_label(corner, 40.0, size, Frame::bare(safe()), LABEL_GAP_PX, None);
        assert!(safe().contains(center), "{center} escaped the border");

        // A viewport too small for the label at all still returns something inside it.
        let tiny = safe_rect(Vec2::new(40.0, 30.0), EDGE_INSET_PX);
        let squeezed = place_label(Vec2::new(20.0, 15.0), 5.0, size, Frame::bare(tiny), LABEL_GAP_PX, None);
        assert!(squeezed.is_finite());
    }

    /// An off-screen arrow's label goes on the inward side, which is what `preferred` is for.
    #[test]
    fn an_arrow_takes_its_label_inward() {
        let safe = safe();
        let pointing_left = Vec2::new(-1.0, 0.0);
        let at = Vec2::new(safe.min.x, 360.0);
        let size = Vec2::new(90.0, 18.0);
        let center = place_label(at, 12.0, size, Frame::bare(safe), LABEL_GAP_PX, Some(-pointing_left));
        assert!(center.x > at.x, "the label should be inboard of the arrow: {center}");
        assert!(safe.contains(center));
    }

    /// The reported problem: a strip across the top of the window is not part of the outer
    /// bound — it is a panel, or a notice, and an arrow placed against the window edge lands on
    /// it. Given the strip as a taken rectangle, the arrow comes back down its own ray until it
    /// is clear.
    #[test]
    fn an_arrow_stops_short_of_an_interface_strip() {
        let strip = Rect::from_corners(Vec2::ZERO, Vec2::new(VIEW.x, 42.0));
        let occupied = [strip];
        let frame = Frame::with(safe(), &occupied, ARROW_CLEARANCE);

        // Something straight up, which is exactly where the strip is.
        let above = in_front(0.0, 4.0, 1.0);
        let Marker::Off { at, direction } = place(above, 0.0, VIEW, frame) else {
            panic!("above the view is off screen")
        };
        assert!(direction.y < 0.0, "it still points up: {direction}");
        assert!(
            at.y >= strip.max.y + ARROW_CLEARANCE - 1.0e-3,
            "{at} is on the strip, which reaches {}",
            strip.max.y,
        );

        // Without the strip it would have gone right to the border, so the test is about the
        // strip and not about the safe inset.
        let Marker::Off { at: unobstructed, .. } = place(above, 0.0, VIEW, Frame::bare(safe()))
        else {
            panic!("still off screen")
        };
        assert!(unobstructed.y < at.y, "{unobstructed} against {at}");
    }

    /// Pushed toward the middle, not slid along the edge: an arrow that moved sideways would
    /// point somewhere the thing is not.
    #[test]
    fn an_obstructed_arrow_keeps_its_direction() {
        let strip = Rect::from_corners(Vec2::ZERO, Vec2::new(VIEW.x, 120.0));
        let occupied = [strip];
        let up_and_right = in_front(1.5, 4.0, 1.0);

        let Marker::Off { at: free, direction: free_dir } =
            place(up_and_right, 0.0, VIEW, Frame::bare(safe()))
        else {
            panic!("off screen")
        };
        let Marker::Off { at: pushed, direction } =
            place(up_and_right, 0.0, VIEW, Frame::with(safe(), &occupied, ARROW_CLEARANCE))
        else {
            panic!("off screen")
        };

        assert_eq!(direction, free_dir, "the direction is the message; it may not change");
        let center = (safe().max + safe().min) * 0.5;
        assert!(
            center.distance(pushed) < center.distance(free),
            "it should have come inward: {pushed} against {free}",
        );
        // And it is on the same ray, not merely nearer.
        let along = (pushed - center).normalize_or_zero();
        assert!(along.distance(direction) < 1.0e-3, "{along} left the ray {direction}");
    }

    /// Something covering the middle cannot be escaped by moving inward, so it is ignored
    /// rather than collapsing every arrow onto the center.
    #[test]
    fn a_covering_rectangle_does_not_collapse_the_arrow() {
        let everything = [Rect::from_corners(Vec2::ZERO, VIEW)];
        let frame = Frame::with(safe(), &everything, ARROW_CLEARANCE);
        let Marker::Off { at, .. } = place(in_front(4.0, 0.0, 1.0), 0.0, VIEW, frame) else {
            panic!("off screen")
        };
        let center = (safe().max + safe().min) * 0.5;
        assert!(center.distance(at) > 100.0, "{at} collapsed onto {center}");
    }

    /// A label goes where it is not covered, which is the same ladder doing more work. This is
    /// the collision that put "Earth" on top of a notice box.
    #[test]
    fn a_label_steps_around_what_is_already_drawn() {
        let size = Vec2::new(90.0, 18.0);
        let at = Vec2::new(640.0, 360.0);
        // A box occupying exactly where the label would go first.
        let above = Rect::from_corners(Vec2::new(500.0, 280.0), Vec2::new(800.0, 345.0));
        let occupied = [above];

        let clear = place_label(at, 20.0, size, Frame::bare(safe()), LABEL_GAP_PX, None);
        assert!(clear.y < at.y, "it goes above when it can: {clear}");

        let stepped =
            place_label(at, 20.0, size, Frame::with(safe(), &occupied, 0.0), LABEL_GAP_PX, None);
        assert!(stepped.y > at.y, "it should have dropped below: {stepped}");
        let half = size * 0.5;
        assert!(
            above.intersect(Rect::from_corners(stepped - half, stepped + half)).is_empty(),
            "{stepped} still overlaps {above:?}",
        );
    }

    /// A closed curve the camera sits inside comes back as runs that stop at the camera plane,
    /// not as one run with a chord across the view.
    ///
    /// Projecting every point and discarding the failures is the obvious thing and is wrong:
    /// the two survivors either side of the gap get joined, and a ring acquires a straight line
    /// through the middle of it that no ring has.
    #[test]
    fn a_path_that_passes_behind_the_camera_is_cut_rather_than_joined() {
        // Half in front, half behind, in order — a ring seen from inside it.
        let ring: Vec<Vec4> = (0..=16)
            .map(|i| {
                let theta = std::f32::consts::TAU * i as f32 / 16.0;
                // `w` is `-view.z`: positive ahead, negative behind.
                Vec4::new(theta.sin(), 0.0, 0.0, theta.cos())
            })
            .collect();

        let runs = project_path(&ring, VIEW);
        assert_eq!(runs.len(), 2, "two arcs in front, cut at the two crossings: {runs:?}");
        for run in &runs {
            assert!(run.len() > 1);
            assert!(run.iter().all(|p| p.is_finite()), "{run:?}");
        }

        // Every drawn point is finite and bounded, however close to the plane it was cut.
        let bound = VIEW * MAX_OVERSHOOT;
        for [a, b] in path_segments(&runs) {
            assert!(a.abs().cmple(bound).all() && b.abs().cmple(bound).all(), "{a} {b}");
        }
    }

    /// Entirely in front is one run; entirely behind is none.
    #[test]
    fn a_path_wholly_on_one_side_is_all_or_nothing() {
        let ahead: Vec<Vec4> =
            (0..5).map(|i| Vec4::new(i as f32 * 0.1, 0.0, 0.0, 1.0)).collect();
        assert_eq!(project_path(&ahead, VIEW).len(), 1);

        let behind: Vec<Vec4> =
            (0..5).map(|i| Vec4::new(i as f32 * 0.1, 0.0, 0.0, -1.0)).collect();
        assert!(project_path(&behind, VIEW).is_empty());

        assert!(project_path(&[], VIEW).is_empty());
        assert!(project_path(&[Vec4::new(0.0, 0.0, 0.0, 1.0)], VIEW).is_empty(), "one point is not a path");
    }

    /// The cut lands well off screen, so the line reads as leaving the view rather than
    /// stopping in the middle of it.
    #[test]
    fn a_cut_end_is_carried_off_the_edge() {
        let crossing = [Vec4::new(0.1, 0.0, 0.0, 1.0), Vec4::new(0.1, 0.0, 0.0, -1.0)];
        let runs = project_path(&crossing, VIEW);
        assert_eq!(runs.len(), 1);
        let end = *runs[0].last().unwrap();
        assert!(
            end.x > VIEW.x * 2.0,
            "the cut should be far outside the viewport, not at {end}",
        );
        assert!(end.is_finite());
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

    /// The tip sits on the anchor and the body trails behind it, so an arrow placed on the
    /// safe rect is entirely inside the window. Drawn the other way round it reaches a whole
    /// arrow-length past the border and its point is clipped away.
    #[test]
    fn an_arrow_keeps_its_point_on_the_anchor() {
        let at = Vec2::new(10.0, 300.0);
        let pointing_left = Vec2::new(-1.0, 0.0);
        let segments = arrow(at, pointing_left, 12.0);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0][0], at, "the tip is the anchor");
        assert!(segments[2][1].distance(at) < 1.0e-3, "the triangle should close");

        // Nothing reaches past the anchor in the direction it points.
        for [a, b] in &segments {
            for point in [a, b] {
                assert!(point.x >= at.x - 1.0e-3, "{point} is outboard of {at}");
            }
        }
        assert!(arrow(at, Vec2::ZERO, 12.0).is_empty(), "no direction, nothing to draw");
    }
}
