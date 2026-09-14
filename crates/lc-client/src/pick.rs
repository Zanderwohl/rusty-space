//! What the cursor is on, and the mark that says so.
//!
//! The rule lives in [`em_ui::picking`] and the shapes in [`em_ui::reticle`]; this is the part
//! that knows what a Lightcone thing is. Its whole job is to reduce what was drawn to
//! candidates — where each thing was put on screen, how big, and what sort it is — and to turn
//! the answer back into the actions a panel row already emits.
//!
//! Everything is projected as a **direction**, not a position. The camera never translates: the
//! ship sits at the render origin and the sky moves around it, so where a thing lands on screen
//! is a function of which way it lies and nothing else.

use bevy::math::Vec4;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::camera::CameraProjection;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, egui};
use em_render::render_space::sim_to_render;
use em_ui::picking::{self, Candidate, rank};
use em_ui::reticle::{self, Frame, Marker};
use glam::DVec3;
use lc_world::navigation::Target;
use lc_world::sky::StarId;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::starfield::{Bodies, radians_per_pixel};
use crate::system::M_PER_LY;

pub const PICK_BUTTON: MouseButton = MouseButton::Left;

/// Radius of the ring drawn round something hovered, when it is smaller than this.
const HOVER_RING_PX: f32 = 14.0;
const BRACKET_ARM_PX: f32 = 8.0;
const ARROW_PX: f32 = 11.0;

/// What a candidate turned out to be, and what to call it.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    Body(String),
    Star(StarId, String),
    /// A population, by its index in the system's list.
    Swarm(usize, String),
}

impl Subject {
    /// Whether this is the same thing as `other`, by identity rather than by name.
    fn is(&self, other: &Subject) -> bool {
        match (self, other) {
            (Subject::Body(a), Subject::Body(b)) => a == b,
            (Subject::Star(a, _), Subject::Star(b, _)) => a == b,
            (Subject::Swarm(a, _), Subject::Swarm(b, _)) => a == b,
            _ => false,
        }
    }

    /// The action that selects it. The same ones the panel rows send, so a click in the view
    /// and a click in the list are the same event as far as everything downstream is concerned.
    fn select(&self) -> Action {
        match self {
            Subject::Body(name) => Action::FocusTarget(Some(Target::Body(name.clone()))),
            Subject::Star(id, _) => Action::SelectTarget(Some(*id)),
            Subject::Swarm(index, _) => Action::FocusTarget(Some(Target::Band(*index))),
        }
    }
}

/// One thing to draw a mark on, still in clip space.
///
/// Placed by the overlay rather than here, because where a mark may go is the part of the
/// window the sky is visible through — which only the interface pass knows, and which changes
/// as panels open.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// Where the bracket, the ring or the arrow goes. For something with extent, the part of
    /// it nearest whatever the mark is *for* — the cursor when hovering.
    pub clip: Vec4,
    pub radius_px: f32,
    pub label: String,
    /// The curve it is drawn along, for anything that is not a point.
    pub outline: Option<Vec<Vec4>>,
}

/// What the overlay draws. Rebuilt every frame, so a mark never survives what it was on.
#[derive(Resource, Default)]
pub struct Picked {
    pub hover: Option<Mark>,
    pub selected: Option<Mark>,
}

pub struct PickPlugin;

impl Plugin for PickPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Picked>()
            .add_systems(Update, survey.run_if(in_state(crate::app::AppState::InGame)))
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                // After the interface, not before it. `available_rect` is built up as panels
                // are added during the pass, so an overlay that runs first sees the whole
                // window and puts its arrows under the head-up display.
                draw.after(crate::panels::hud)
                    .after(crate::panels::open_panels)
                    .run_if(in_state(crate::app::AppState::InGame)),
            );
    }
}

/// A thing on screen, before it is known whether the cursor is on it.
struct Sighted {
    subject: Subject,
    /// Clip space, from a direction: the projection applied and the divide not.
    ///
    /// For something with extent this is filled in once the cursor is known, since where the
    /// nearest part of it is depends on where the cursor is.
    clip: Vec4,
    radius_px: f32,
    rank: u8,
    /// The curve it is drawn along, clip space, for anything that is not a point. A swarm is
    /// picked and marked along this rather than at a centre it does not have.
    outline: Option<Vec<Vec4>>,
}

/// How many points a swarm's circle is sampled at.
///
/// Sixty-four is a fifth of a degree of error against a true circle at the widest, which is far
/// below a pixel at any distance the thing is drawn at.
const SWARM_SAMPLES: usize = 64;

/// Find what the cursor is on, mark what is selected, and act on a click.
#[allow(clippy::too_many_arguments)]
fn survey(
    game: Res<Game>,
    ui: Res<Ui>,
    bodies: Res<Bodies>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Projection, &Transform), With<Camera3d>>,
    buttons: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    mut picked: ResMut<Picked>,
    mut out: MessageWriter<Requested>,
) {
    *picked = Picked::default();
    let Ok(window) = windows.single() else { return };
    let Ok((Projection::Perspective(perspective), camera_at)) = camera.single() else { return };
    let viewport = Vec2::new(window.width(), window.height());
    if viewport.x <= 0.0 || viewport.y <= 0.0 {
        return;
    }

    let clip_from_view = perspective.get_clip_from_view();
    let rad_per_px = radians_per_pixel(perspective.fov, viewport.y);
    let sighted = sight(&game, &bodies, camera_at, clip_from_view, rad_per_px);

    // Against the whole window, because that is where the cursor can be. A thing under a panel
    // is not pickable anyway: egui takes the pointer first, which is checked below.
    let whole = Frame::bare(reticle::safe_rect(viewport, 0.0));
    let cursor = (!egui.wants_any_pointer_input()).then(|| window.cursor_position()).flatten();

    // Only what is actually on screen can be under the cursor. An off-screen thing is placed on
    // the border, and taking that as a position would make the edges pick whatever is out there.
    let mut candidates = Vec::with_capacity(sighted.len());
    for (index, seen) in sighted.iter().enumerate() {
        let at = match (&seen.outline, cursor) {
            // A curve has no one place it is: the nearest part of it to the cursor is what the
            // cursor is on, and that is a different point for every cursor position.
            (Some(outline), Some(cursor)) => {
                let runs = reticle::project_path(outline, viewport);
                picking::nearest_on_path(&runs, cursor).map(|(at, _)| (at, 0.0))
            }
            (Some(_), None) => None,
            (None, _) => match reticle::place(seen.clip, seen.radius_px, viewport, whole) {
                Marker::On { at, radius_px } => Some((at, radius_px)),
                Marker::Off { .. } => None,
            },
        };
        if let Some((at, radius_px)) = at
            && whole.safe.contains(at)
        {
            candidates.push(Candidate { id: index as u64, at, radius_px, rank: seen.rank });
        }
    }

    if let Some(cursor) = cursor
        && let Some(id) = picking::pick(&candidates, cursor, picking::SLACK_PX)
    {
        let seen = &sighted[id as usize];
        picked.hover = Some(mark(seen, viewport, cursor));
        if buttons.just_pressed(PICK_BUTTON) {
            out.write(Requested(seen.subject.select()));
        }
    }

    // The selection is marked whether or not it is on screen: an arrow at the edge is the only
    // way to say where something went.
    if let Some(chosen) = selected(&ui)
        && let Some(seen) = sighted.iter().find(|s| s.subject.is(&chosen))
    {
        // Marked against the middle of the view rather than the cursor: a selection stands
        // whether or not anyone is pointing at it.
        picked.selected = Some(mark(seen, viewport, viewport * 0.5));
    }
}

/// What the interface says is selected, as a subject.
fn selected(ui: &Ui) -> Option<Subject> {
    match &ui.focus {
        Some(Target::Body(name)) => Some(Subject::Body(name.clone())),
        Some(Target::Band(index)) => Some(Subject::Swarm(*index, String::new())),
        // Matched on the identifier alone; the name rides along for the label.
        None => ui.selected.map(|id| Subject::Star(id, String::new())),
    }
}

/// A mark on `seen`, anchored at whatever part of it is nearest `toward`.
fn mark(seen: &Sighted, viewport: Vec2, toward: Vec2) -> Mark {
    let clip = match &seen.outline {
        Some(outline) => nearest_sample(outline, viewport, toward).unwrap_or(seen.clip),
        None => seen.clip,
    };
    Mark {
        clip,
        radius_px: seen.radius_px,
        label: match &seen.subject {
            Subject::Body(name) => name.clone(),
            Subject::Star(_, name) | Subject::Swarm(_, name) => name.clone(),
        },
        outline: seen.outline.clone(),
    }
}

/// The sample of `outline` that lands nearest `toward` on screen.
///
/// Clip space out as well as in, so a marker can be placed from it by the same path as anything
/// else — including the edge arrow, when the whole curve is behind the camera. Sample accuracy
/// is enough for an anchor; picking uses the segments.
fn nearest_sample(outline: &[Vec4], viewport: Vec2, toward: Vec2) -> Option<Vec4> {
    outline
        .iter()
        .filter(|clip| clip.w > 0.0)
        .map(|clip| {
            let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
            let at = Vec2::new((ndc.x + 1.0) * 0.5 * viewport.x, (1.0 - ndc.y) * 0.5 * viewport.y);
            (toward.distance(at), *clip)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, clip)| clip)
}

/// A simulation-space *direction* in clip space.
///
/// A direction and not a position, with `w = 0` on the way in, so the camera's own translation
/// cannot enter: the camera never moves, and where something lands on screen is a function of
/// which way it lies and nothing else. The `w` that comes *out* is `-view.z`, which is what
/// [`reticle::place`] reads to tell in front from behind.
pub fn project_direction(camera_rotation: Quat, clip_from_view: Mat4, direction: DVec3) -> Vec4 {
    let render = sim_to_render(direction).as_vec3();
    let view = camera_rotation.inverse() * render;
    clip_from_view * Vec4::new(view.x, view.y, view.z, 0.0)
}

/// Everything drawn, reduced to where it was drawn.
fn sight(
    game: &Game,
    bodies: &Bodies,
    camera_at: &Transform,
    clip_from_view: Mat4,
    rad_per_px: f32,
) -> Vec<Sighted> {
    let ship = game.ship.motion.position_ly;
    let mut out = Vec::with_capacity(bodies.drawn.len() + game.stars.len());

    let project = |direction: DVec3| project_direction(camera_at.rotation, clip_from_view, direction);

    for body in &bodies.drawn {
        let offset = body.position_ly - ship;
        let distance_m = offset.length() * M_PER_LY;
        if distance_m <= 0.0 {
            continue;
        }
        // The geometric disc, which is what a resolved body looks like. Below a pixel it comes
        // out as nothing and the slack does the work, which is the same thing a point gets.
        let radius_px = (body.radius_m / distance_m) as f32 / rad_per_px.max(f32::MIN_POSITIVE);
        out.push(Sighted {
            subject: Subject::Body(body.name.clone()),
            clip: project(offset.normalize_or_zero()),
            radius_px: radius_px.max(0.0),
            rank: rank::BODY,
            outline: None,
        });
    }

    for star in &game.stars {
        // Where the sky pass draws it, aberration and all. Picking what you see rather than
        // what is there is the whole point of reading this and not the offset.
        let direction = game.apparent_dir(star);
        if direction == DVec3::ZERO {
            continue;
        }
        let name =
            star.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get()));
        out.push(Sighted {
            subject: Subject::Star(star.id, name),
            clip: project(direction),
            radius_px: 0.0,
            rank: rank::STAR,
            outline: None,
        });
    }

    // A swarm is a shell about the star, not a ring, so it has no silhouette to click. What it
    // does have is its own orbit circle: the radius its elements sit at, in its own plane.
    // Seen from outside that projects to an ellipse and from inside it wraps right round, and
    // the same curve serves as the skeleton drawn on hover and as the thing the cursor is
    // measured against.
    if let Some(system) = game.system.as_ref() {
        let star = system.star_position_at(game.coordinate_time_s()).unwrap_or(ship);
        for (index, population) in system.populations.iter().enumerate() {
            if !crate::envelope::visible(population) {
                continue;
            }
            let outline: Vec<Vec4> = swarm_circle(star, ship, population)
                .into_iter()
                .map(&project)
                .collect();
            if outline.len() < 2 {
                continue;
            }
            out.push(Sighted {
                subject: Subject::Swarm(index, swarm_name(population)),
                // Filled in against the cursor; a circle has no one place it is.
                clip: outline[0],
                radius_px: 0.0,
                rank: rank::SWARM,
                outline: Some(outline),
            });
        }
    }

    out
}

/// What the interface calls a population, matching its row in the system list.
fn swarm_name(population: &lc_world::population::Population) -> String {
    let radius = population.thermal_radius();
    format!(
        "{} at {:.1} AU",
        if lc_world::navigation::is_flat(population) { "belt" } else { "cloud" },
        radius / lc_world::navigation::AU,
    )
}

/// The population's own orbit circle, as directions from the ship.
///
/// Sampled in the population's plane at its thermal radius, which is the radius its light
/// comes from. Closed: the last point repeats the first, so the curve has no seam.
fn swarm_circle(
    star_ly: DVec3,
    ship_ly: DVec3,
    population: &lc_world::population::Population,
) -> Vec<DVec3> {
    let radius_m = population.thermal_radius();
    if radius_m <= 0.0 {
        return Vec::new();
    }
    let (u, v) = lc_world::navigation::basis(population.pole);
    let centre = (star_ly - ship_ly) * M_PER_LY;
    (0..=SWARM_SAMPLES)
        .map(|i| {
            let theta = std::f64::consts::TAU * i as f64 / SWARM_SAMPLES as f64;
            centre + (u * theta.cos() + v * theta.sin()) * radius_m
        })
        .collect()
}

/// Paint the marks.
fn draw(mut contexts: EguiContexts, picked: Res<Picked>, windows: Query<&Window, With<PrimaryWindow>>) {
    let Ok(context) = contexts.ctx_mut() else { return };
    let Ok(window) = windows.single() else { return };
    let viewport = Vec2::new(window.width(), window.height());

    // The part of the window the sky is visible through, less the border. Not the whole window:
    // with a panel open, an edge arrow drawn against the window edge sits behind the panel.
    let available = context.available_rect();
    let inset = reticle::EDGE_INSET_PX;
    let safe = bevy::math::Rect::from_corners(
        Vec2::new(available.min.x + inset, available.min.y + inset),
        Vec2::new(available.max.x - inset, available.max.y - inset),
    );
    // What the interface has taken. `available_rect` accounts for docked panels and nothing
    // else, so every floating window and notice has to be named here or a mark lands on one.
    let occupied = occupied_rects(context);
    let frame = Frame::with(safe, &occupied, ARROW_PX);

    // Foreground, not background. Panels paint in `Order::Background` and floating notices in
    // `Order::Middle`, so a mark on either of those layers is covered by whatever the interface
    // happens to have open — which is how the first version lost its edge arrow under a notice
    // box. Keeping the marks inside `available` is what stops this from drawing over a panel.
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("lc_pick_marks"),
    ));

    for (mark, colour, bracketed) in [
        (picked.hover.as_ref(), HOVER, false),
        (picked.selected.as_ref(), SELECTED, true),
    ]
    .into_iter()
    .filter_map(|(mark, colour, bracketed)| Some((mark?, colour, bracketed)))
    {
        paint(&painter, mark, viewport, frame, colour, bracketed);
    }
}

const HOVER: egui::Color32 = egui::Color32::from_rgb(150, 170, 190);
const SELECTED: egui::Color32 = egui::Color32::from_rgb(235, 200, 120);
const LABEL_SIZE: f32 = 12.0;

/// How much fainter a swarm's skeleton is than the mark on it. The curve is long and would
/// otherwise read as the brightest thing in the sky.
const SKELETON_FADE: f32 = 0.45;

/// Every visible floating area, as rectangles to keep marks out of.
///
/// Docked panels are not areas and do not appear here; they are already out of
/// [`egui::Context::available_rect`]. This is the rest of the interface: windows, notices,
/// tooltips — everything that floats over the view and that `available_rect` says nothing
/// about. The overlay's own layer is skipped, or it would exclude itself.
fn occupied_rects(context: &egui::Context) -> Vec<bevy::math::Rect> {
    context.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .iter()
            .filter(|layer| layer.order < egui::Order::Foreground)
            .filter_map(|layer| memory.area_rect(layer.id))
            .map(|rect| {
                bevy::math::Rect::from_corners(
                    Vec2::new(rect.min.x, rect.min.y),
                    Vec2::new(rect.max.x, rect.max.y),
                )
            })
            .collect()
    })
}

fn paint(
    painter: &egui::Painter,
    mark: &Mark,
    viewport: Vec2,
    frame: Frame<'_>,
    colour: egui::Color32,
    bracketed: bool,
) {
    let stroke = egui::Stroke::new(1.0_f32, colour);

    // The skeleton first, under whatever marks the anchor. A swarm is a shell and has no
    // outline of its own on screen; this is the circle its elements are drawn from, which is
    // the only line in it a player can be said to be pointing at.
    if let Some(outline) = &mark.outline {
        let faint = egui::Stroke::new(1.0_f32, colour.gamma_multiply(SKELETON_FADE));
        let runs = reticle::project_path(outline, viewport);
        draw_segments(painter, &reticle::path_segments(&runs), faint);
    }

    let (anchor, radius_px, preferred) =
        match reticle::place(mark.clip, mark.radius_px, viewport, frame) {
        Marker::On { at, radius_px } => {
            let radius = radius_px.max(HOVER_RING_PX);
            let segments = if bracketed {
                reticle::brackets(at, radius, BRACKET_ARM_PX)
            } else {
                reticle::ring(at, radius, 40)
            };
            draw_segments(painter, &segments, stroke);
            (at, radius, None)
        }
        Marker::Off { at, direction } => {
            draw_segments(painter, &reticle::arrow(at, direction, ARROW_PX), stroke);
            (at, ARROW_PX, Some(-direction))
        }
    };

    if mark.label.is_empty() {
        return;
    }
    // Measured before it is placed, because where it fits depends on how wide it is.
    let font = egui::FontId::proportional(LABEL_SIZE);
    let galley = painter.layout_no_wrap(mark.label.clone(), font, colour);
    let size = Vec2::new(galley.size().x, galley.size().y);
    let centre =
        reticle::place_label(anchor, radius_px, size, frame, reticle::LABEL_GAP_PX, preferred);
    painter.galley(
        egui::pos2(centre.x - size.x * 0.5, centre.y - size.y * 0.5),
        galley,
        colour,
    );
}

fn draw_segments(painter: &egui::Painter, segments: &[[Vec2; 2]], stroke: egui::Stroke) {
    for [a, b] in segments {
        painter.line_segment([egui::pos2(a.x, a.y), egui::pos2(b.x, b.y)], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::CameraProjection;

    /// The camera as [`crate::app::aim_camera`] builds it, so the test and the app cannot
    /// disagree about which way the swizzle goes.
    fn looking_along(direction: DVec3) -> (Quat, Mat4) {
        let mut transform = Transform::from_xyz(0.0, 0.0, 0.0);
        transform.look_to(sim_to_render(direction).as_vec3(), sim_to_render(DVec3::Z).as_vec3());
        let projection = PerspectiveProjection::default();
        (transform.rotation, projection.get_clip_from_view())
    }

    /// What the camera is pointed at lands in the middle of the screen, and in front.
    #[test]
    fn straight_ahead_is_the_middle_of_the_screen() {
        let (rotation, clip_from_view) = looking_along(DVec3::X);
        let clip = project_direction(rotation, clip_from_view, DVec3::X);
        assert!(clip.w > 0.0, "ahead is in front of the camera: w = {}", clip.w);
        assert!(clip.x.abs() < 1.0e-5 && clip.y.abs() < 1.0e-5, "{clip:?}");

        let marker = reticle::place(clip, 0.0, Vec2::new(1280.0, 720.0), Frame::bare(reticle::safe_rect(Vec2::new(1280.0, 720.0), 10.0)));
        let Marker::On { at, .. } = marker else { panic!("{marker:?}") };
        assert!((at - Vec2::new(640.0, 360.0)).length() < 0.05, "{at}");
    }

    /// Behind is behind: a negative `w`, which is what an edge arrow is decided by.
    #[test]
    fn straight_back_is_behind_the_camera() {
        let (rotation, clip_from_view) = looking_along(DVec3::X);
        let clip = project_direction(rotation, clip_from_view, -DVec3::X);
        assert!(clip.w < 0.0, "w = {}", clip.w);
    }

    /// Simulation space is Z-up and the renderer is Y-up. Getting the swizzle backwards puts
    /// every mark upside down, which no amount of screen-space testing would catch.
    #[test]
    fn simulation_up_is_up_on_screen() {
        let (rotation, clip_from_view) = looking_along(DVec3::X);
        // A little above the line of sight.
        let above = (DVec3::X + DVec3::Z * 0.2).normalize();
        let clip = project_direction(rotation, clip_from_view, above);
        assert!(clip.w > 0.0);
        assert!(clip.y > 0.0, "clip y is up: {clip:?}");

        let viewport = Vec2::new(1280.0, 720.0);
        let marker = reticle::place(clip, 0.0, viewport, Frame::bare(reticle::safe_rect(viewport, 10.0)));
        let Marker::On { at, .. } = marker else { panic!("{marker:?}") };
        assert!(at.y < 360.0, "up in the world should be up the screen: {at}");
    }

    /// And left is left. The two together pin the whole basis.
    #[test]
    fn simulation_port_is_left_on_screen() {
        let (rotation, clip_from_view) = looking_along(DVec3::X);
        // Looking down +X with +Z up, +Y is to the left.
        let port = (DVec3::X + DVec3::Y * 0.2).normalize();
        let viewport = Vec2::new(1280.0, 720.0);
        let marker = reticle::place(
            project_direction(rotation, clip_from_view, port),
            0.0,
            viewport,
            Frame::bare(reticle::safe_rect(viewport, 10.0)),
        );
        let Marker::On { at, .. } = marker else { panic!("{marker:?}") };
        assert!(at.x < 640.0, "{at}");
    }

    /// A click sends exactly what the panel row sends, so nothing downstream can tell them
    /// apart.
    #[test]
    fn a_click_sends_the_action_the_list_sends() {
        let body = Subject::Body("Earth".into());
        assert_eq!(body.select(), Action::FocusTarget(Some(Target::Body("Earth".into()))));

        let id = StarId::synthesise("test", 7);
        let star = Subject::Star(id, "Sol".into());
        assert_eq!(star.select(), Action::SelectTarget(Some(id)));
    }

    /// The selection is matched by identity, not by the name that rides along for the label.
    #[test]
    fn a_star_is_matched_by_identity_and_not_by_name() {
        let id = StarId::synthesise("test", 7);
        assert!(Subject::Star(id, String::new()).is(&Subject::Star(id, "Sol".into())));
        assert!(!Subject::Star(id, "Sol".into()).is(&Subject::Body("Sol".into())));
        assert!(
            !Subject::Star(id, "Sol".into()).is(&Subject::Star(StarId::synthesise("test", 8), "Sol".into()))
        );
    }
}
