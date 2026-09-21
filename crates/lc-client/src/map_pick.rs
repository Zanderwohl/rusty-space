//! What the cursor is on over the map, and the mark that says so.
//!
//! [`crate::pick`] over the map's surface instead of the sky. The rule is [`em_ui::picking`],
//! the shapes are [`em_ui::reticle`], and what a thing is is [`crate::pick::Subject`]; this
//! reduces the frame the map drew to candidates. A click sends the action a click on the sky
//! sends, and what is marked is what [`crate::pick::selected`] reads, so one selection serves
//! both modes.
//!
//! Coordinates are the surface's own, its top left at the origin. `origin` is what puts them
//! back in the window when they reach the painter.

use bevy::math::{Vec2, Vec4};
use bevy::prelude::MessageWriter;
use bevy_egui::egui;
use em_map::{ItemKey, ItemKind, MapFrame, Placement};
use em_ui::picking::{self, Candidate, rank};
use em_ui::reticle::{self, Frame, Marker};
use glam::DVec3;

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::map::{MAP_FOV, Map};
use crate::panels::ask;
use crate::pick::{self, Mark, Subject};
use crate::ui::{MapFocus, MapView};

/// What the overlay draws, and which names it is drawing itself.
#[derive(Default)]
pub(crate) struct Picked {
    hover: Option<Mark>,
    selected: Option<Mark>,
    /// Keys this is naming, so [`crate::map_panel`]'s layout does not name them again a few
    /// pixels away.
    pub named: Vec<ItemKey>,
}

/// One placement on the surface, before it is known whether the cursor is on it.
struct Seen<'a> {
    key: ItemKey,
    subject: &'a Subject,
    label: &'a str,
    clip: Vec4,
    radius_px: f32,
    rank: u8,
    /// The curves a population is drawn along. It has no center to be pointed at, so it is
    /// picked and marked along these.
    outline: Option<Vec<Vec<Vec4>>>,
}

/// Find what the cursor is on, work out what is selected, and act on a click.
pub(crate) fn survey(
    response: &egui::Response,
    rect: egui::Rect,
    state: &Ui,
    map: &Map,
    out: &mut MessageWriter<Requested>,
) -> Picked {
    let mut picked = Picked::default();
    let Some(frame) = map.frame.as_ref() else {
        return picked;
    };
    let viewport = Vec2::new(rect.width(), rect.height());
    if viewport.x <= 0.0 || viewport.y <= 0.0 {
        return picked;
    }
    let origin = Vec2::new(rect.min.x, rect.min.y);
    let seen = sight(state, map, frame, viewport);

    // egui gives the pointer to whatever is on top, so `hover_pos` is already `None` over a
    // panel or the corner square and nothing has to be excluded here.
    let whole = Frame::bare(reticle::safe_rect(viewport, 0.0));
    let cursor = response
        .hover_pos()
        .map(|at| Vec2::new(at.x, at.y) - origin);

    // Only what is on the surface can be under the cursor. Something off it is placed on the
    // border, and taking that as a position would make the edges pick whatever is out there.
    let mut candidates = Vec::with_capacity(seen.len());
    for (index, thing) in seen.iter().enumerate() {
        let at = match (&thing.outline, cursor) {
            // The nearest part of the curve, which is a different point for every cursor
            // position.
            (Some(outline), Some(cursor)) => {
                let runs = pick::screen_runs(outline, viewport);
                picking::nearest_on_path(&runs, cursor).map(|(at, _)| (at, 0.0))
            }
            (Some(_), None) => None,
            (None, _) => match reticle::place(thing.clip, thing.radius_px, viewport, whole) {
                Marker::On { at, radius_px } => Some((at, radius_px)),
                Marker::Off { .. } => None,
            },
        };
        if let Some((at, radius_px)) = at
            && whole.safe.contains(at)
        {
            candidates.push(Candidate {
                id: index as u64,
                at,
                radius_px,
                rank: thing.rank,
            });
        }
    }

    if let Some(cursor) = cursor
        && let Some(id) = picking::pick(&candidates, cursor, picking::SLACK_PX)
    {
        let thing = &seen[id as usize];
        act(response, thing, out);
        picked.named.push(thing.key);
        picked.hover = Some(mark(thing, viewport, cursor));
    }

    // Marked whether or not it is on the surface: an arrow at the edge is the only way to say
    // where something went. Anchored against the middle, since a selection stands with nobody
    // pointing at it.
    if let Some(chosen) = pick::selected(state)
        && let Some(thing) = seen.iter().find(|s| s.subject.is(&chosen))
    {
        picked.named.push(thing.key);
        picked.selected = Some(mark(thing, viewport, viewport * 0.5));
    }
    picked
}

/// Paint the marks, over the names.
///
/// `hole` is the corner square the world's camera draws into and `over` is what the interface
/// floats across the surface. A mark in either is a mark on something else.
pub(crate) fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    hole: egui::Rect,
    over: &[egui::Rect],
    picked: &Picked,
) {
    if picked.hover.is_none() && picked.selected.is_none() {
        return;
    }
    let viewport = Vec2::new(rect.width(), rect.height());
    let origin = Vec2::new(rect.min.x, rect.min.y);
    let occupied: Vec<bevy::math::Rect> = over
        .iter()
        .chain(std::iter::once(&hole))
        // The background layer is one of these and it is the whole window. A box covering the
        // surface is what the surface is drawn on, not something floating over it.
        .filter(|taken| taken.is_positive() && !taken.contains_rect(rect))
        .map(|taken| local(*taken, origin))
        .collect();
    let bounds = Frame::with(
        reticle::safe_rect(viewport, reticle::EDGE_INSET_PX),
        &occupied,
        pick::ARROW_PX,
    );

    // The selection first, so hovering what is already selected shows both marks.
    for (mark, color, bracketed) in [
        (picked.selected.as_ref(), pick::SELECTED, true),
        (picked.hover.as_ref(), pick::HOVER, false),
    ]
    .into_iter()
    .filter_map(|(mark, color, bracketed)| Some((mark?, color, bracketed)))
    {
        pick::paint(painter, mark, origin, viewport, bounds, color, bracketed);
    }
}

/// A single click selects, as it does on the sky. A double click also centers the map, which
/// is the only way to center on a body the control strip does not name.
fn act(response: &egui::Response, thing: &Seen<'_>, out: &mut MessageWriter<Requested>) {
    if response.clicked()
        && let Some(action) = thing.subject.select()
    {
        ask(out, action);
    }
    if response.double_clicked() {
        ask(out, Action::FocusMap(MapFocus::Item(thing.key)));
    }
}

/// A mark on `thing`, anchored at whatever part of it is nearest `toward`.
fn mark(thing: &Seen<'_>, viewport: Vec2, toward: Vec2) -> Mark {
    let clip = match &thing.outline {
        Some(outline) => nearest_sample(outline, viewport, toward).unwrap_or(thing.clip),
        None => thing.clip,
    };
    Mark {
        clip,
        radius_px: thing.radius_px,
        label: thing.label.to_string(),
        outline: thing.outline.clone(),
    }
}

/// The sample of `outline` that lands nearest `toward` on the surface.
///
/// Clip space out as well as in, so an edge arrow can be placed from it when the whole curve
/// is behind the camera.
fn nearest_sample(outline: &[Vec<Vec4>], viewport: Vec2, toward: Vec2) -> Option<Vec4> {
    outline
        .iter()
        .flatten()
        .filter(|clip| clip.w > 0.0)
        .map(|clip| {
            let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
            let at = Vec2::new(
                (ndc.x + 1.0) * 0.5 * viewport.x,
                (1.0 - ndc.y) * 0.5 * viewport.y,
            );
            (toward.distance(at), *clip)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, clip)| clip)
}

/// Everything the map drew, reduced to where it was drawn.
///
/// A placement with no subject is left out rather than picked and ignored: the reader's own
/// craft is the one of those, and it outranks every body while answering no click.
fn sight<'a>(state: &Ui, map: &'a Map, frame: &'a MapFrame, viewport: Vec2) -> Vec<Seen<'a>> {
    let view = state.map;
    let aspect = (viewport.x / viewport.y) as f64;
    // Marks are sized against the texture and drawn on the surface showing it. The two differ
    // on a display that scales, and on the frame after a resize.
    let per_pixel = map.points_per_pixel(viewport.y);

    let mut out = Vec::with_capacity(frame.placements.len());
    for placement in &frame.placements {
        let Some((_, subject)) = map.subjects.iter().find(|(key, _)| *key == placement.key) else {
            continue;
        };
        let outline = outline_of(placement, &view, aspect);
        let clip = match &outline {
            Some(curves) => match curves.iter().flatten().next().copied() {
                Some(first) => first,
                None => continue,
            },
            None => clip_of(&view, placement.at.as_dvec3(), aspect),
        };
        out.push(Seen {
            key: placement.key,
            subject,
            label: &placement.label,
            clip,
            radius_px: match outline.is_some() {
                true => 0.0,
                false => map.drawn_radius_px(placement) * per_pixel,
            },
            rank: rank_of(placement.kind),
            outline,
        });
    }
    out
}

/// What sort of thing a placement is, on the scale both products share.
fn rank_of(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Ship | ItemKind::Station | ItemKind::Observer => rank::CRAFT,
        ItemKind::Planet | ItemKind::Moon | ItemKind::Minor => rank::BODY,
        ItemKind::Star => rank::STAR,
        ItemKind::Population => rank::SWARM,
    }
}

/// The curves a population is drawn along, in clip space.
///
/// Render units, which is what the placement is already in: `em_map::outline` works in
/// whatever unit it is handed and the projection divides the scale out again.
fn outline_of(placement: &Placement, view: &MapView, aspect: f64) -> Option<Vec<Vec<Vec4>>> {
    let annulus = placement.annulus?;
    let curves = em_map::outline::torus(
        placement.at.as_dvec3(),
        placement.pole.as_dvec3(),
        em_map::outline::Extent {
            inner: annulus.inner as f64,
            outer: annulus.outer as f64,
            half_angle_rad: annulus.half_angle_rad as f64,
        },
    );
    Some(
        curves
            .into_iter()
            .map(|curve| {
                curve
                    .into_iter()
                    .map(|at| clip_of(view, at, aspect))
                    .collect()
            })
            .collect(),
    )
}

/// A camera-relative offset in clip space: the projection applied, the divide not, so
/// something behind the camera still says which way it lies.
fn clip_of(view: &MapView, offset: DVec3, aspect: f64) -> Vec4 {
    view.orbit
        .clip(view.plane, offset, MAP_FOV as f64, aspect)
        .as_vec4()
}

/// A window rectangle in the surface's own coordinates.
fn local(rect: egui::Rect, origin: Vec2) -> bevy::math::Rect {
    bevy::math::Rect::from_corners(
        Vec2::new(rect.min.x, rect.min.y) - origin,
        Vec2::new(rect.max.x, rect.max.y) - origin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map item's rank has to be the one the sky gives the same thing, or picking a moon
    /// off the map picks the planet behind it.
    #[test]
    fn a_map_item_ranks_as_the_sky_ranks_it() {
        assert_eq!(rank_of(ItemKind::Ship), rank::CRAFT);
        assert_eq!(rank_of(ItemKind::Observer), rank::CRAFT);
        assert_eq!(rank_of(ItemKind::Planet), rank::BODY);
        assert_eq!(rank_of(ItemKind::Moon), rank::BODY);
        assert_eq!(rank_of(ItemKind::Minor), rank::BODY);
        assert_eq!(rank_of(ItemKind::Star), rank::STAR);
        assert_eq!(rank_of(ItemKind::Population), rank::SWARM);
        assert!(rank::CRAFT < rank::BODY && rank::BODY < rank::STAR && rank::STAR < rank::SWARM);
    }

    /// A belt is picked along the outline the geometry is built from: two edge circles and
    /// four cross-sections.
    #[test]
    fn a_population_is_picked_along_the_curves_it_is_drawn_as() {
        let placement = Placement {
            key: ItemKey(1),
            kind: ItemKind::Population,
            label: "belt".into(),
            weight: 0.0,
            symbol_scale: 1.0,
            at: bevy::math::Vec3::ZERO,
            foot: bevy::math::Vec3::ZERO,
            radius: 0.0,
            angular_radius: 0.0,
            annulus: Some(em_map::Annulus {
                inner: 2.0,
                outer: 3.0,
                half_angle_rad: 0.2,
            }),
            pole: bevy::math::Vec3::Z,
        };
        let view = MapView {
            orbit: em_map::Orbit::framing(DVec3::ZERO, em_map::snapshot::M_PER_AU),
            ..Default::default()
        };
        let curves = outline_of(&placement, &view, 1.6).expect("a belt has an outline");
        assert_eq!(curves.len(), 2 + em_map::outline::CROSS_SECTIONS);
        assert!(curves.iter().all(|curve| curve.len() > 2));

        let body = Placement {
            annulus: None,
            kind: ItemKind::Planet,
            ..placement
        };
        assert!(outline_of(&body, &view, 1.6).is_none());
    }

    /// Something off the surface still says which way it lies. Dividing through a negative
    /// `w` sends the arrow to the opposite edge.
    #[test]
    fn something_behind_the_camera_is_marked_at_an_edge() {
        let view = MapView {
            orbit: em_map::Orbit::framing(DVec3::ZERO, em_map::snapshot::M_PER_AU),
            ..Default::default()
        };
        let (forward, right, _) = view.orbit.view_basis(view.plane);
        let clip = clip_of(&view, (-forward * 3.0 + right) * 1.0e-6, 1.6);
        assert!(clip.w < 0.0, "{clip:?}");

        let viewport = Vec2::new(1280.0, 720.0);
        let frame = Frame::bare(reticle::safe_rect(viewport, reticle::EDGE_INSET_PX));
        match reticle::place(clip, 0.0, viewport, frame) {
            Marker::Off { at, direction } => {
                assert!(
                    direction.x > 0.0,
                    "the arrow points away from the thing: {direction:?}"
                );
                assert!(at.x > viewport.x * 0.5, "{at:?}");
            }
            other => panic!("behind the camera should be an edge marker, got {other:?}"),
        }
    }
}
