//! The editor's handles: picking a part where it is drawn, and the handles that edit it there.
//!
//! Drawn after newseum's editor controls: a line per axis of the part in red, green and blue with a
//! square end, a line along its diagonal for size, one ring for its twist, and an arrow into its
//! parent for its standoff. They are meshes on their own layer, drawn by a camera of their own
//! after the parts so they are never buried in a hull, and sized in pixels so they read the same
//! on a 500 m ship and a 50 km one.
//!
//! A line drags by the point on it nearest the pointer's ray, and the ring by where the ray meets
//! its plane, so a handle stays under the pointer however the camera is turned. Both are measured
//! against the handle as it stood when the drag began. Picking is in screen space over the same
//! geometry that is drawn, and a press on a handle is neither the camera's nor a part's. A drag
//! sends an edit each frame it moves and a settled one on release: one gesture, one entry in the
//! history. See `lightcone/docs/29-ship-form.md` §Handles.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use bevy_egui::input::EguiWantsInput;
use em_ui::picking::{Candidate, SLACK_PX, nearest_on_path, pick, rank};
use glam::{DMat3, DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::place::Side;
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Mount, Part, PartId, Placement};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Edit, Refused, What, grown, stretched};
use crate::form_view::{Extent, FORM_FOV, FormOrbit, FormSurface, Shown};
use crate::input::Requested;
use crate::snap;
use crate::ui::ViewMode;

/// The handles' own layer, drawn by the handle camera over the parts.
pub const FORM_HANDLE_LAYER: usize = 6;

/// Which edit a handle makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// Volume at fixed proportions, along the part's diagonal.
    Size,
    /// One dimension, along the part's own axis. The volume goes with it, or with the
    /// constant-volume modifier the others give way instead.
    Axis(usize),
    Twist,
    /// Into the parent, and back out to resting on it.
    Standoff,
}

/// Past the part's half-extent, how far out a handle reaches.
const REACH_OUT: f64 = 1.3;
/// However small the part is drawn, a handle is at least this long.
const MIN_ARM_PX: f64 = 50.0;
const LINE_PX: f64 = 3.0;
const END_PX: f64 = 13.0;
const HEAD_PX: (f64, f64) = (18.0, 7.0);
/// A ball the width of a square reads smaller than it.
const BALL_OVER_END: f64 = 1.2;
const RING_SEGMENTS: usize = 48;
/// Of its start, the least a line handle may be pulled in to, so a part cannot be dragged through
/// nothing.
const LEAST_PULL: f64 = 0.05;

/// The camera the handles are laid against: its orbit over the draft, into the picture.
#[derive(Clone, Copy, Debug)]
pub struct Lens {
    pub orbit: FormOrbit,
    pub extent: Extent,
    /// The picture, logical pixels.
    pub rect: egui::Rect,
}

impl Lens {
    fn half_tan(&self) -> f64 {
        (FORM_FOV as f64 * 0.5).tan()
    }

    fn aspect(&self) -> f64 {
        (self.rect.width() / self.rect.height().max(1.0)) as f64
    }

    /// Where `p`, ship frame, is drawn, logical pixels, and how far in front of the eye. `None`
    /// behind it.
    pub fn project(&self, p: DVec3) -> Option<(Vec2, f64)> {
        let [forward, right, up] = self.orbit.basis();
        let v = p - self.orbit.eye_m(&self.extent);
        let depth = v.dot(forward);
        if depth <= 0.0 {
            return None;
        }
        let ndc = DVec2::new(v.dot(right) / (depth * self.half_tan() * self.aspect()), v.dot(up) / (depth * self.half_tan()));
        let x = self.rect.min.x as f64 + (ndc.x + 1.0) * 0.5 * self.rect.width() as f64;
        let y = self.rect.min.y as f64 + (1.0 - ndc.y) * 0.5 * self.rect.height() as f64;
        Some((Vec2::new(x as f32, y as f32), depth))
    }

    /// The ray under `at`, logical pixels: from the eye, unit, ship frame.
    pub fn ray(&self, at: Vec2) -> (DVec3, DVec3) {
        let ndc = DVec2::new(
            ((at.x - self.rect.min.x) / self.rect.width() * 2.0 - 1.0) as f64,
            (1.0 - (at.y - self.rect.min.y) / self.rect.height() * 2.0) as f64,
        );
        self.orbit.ray(&self.extent, FORM_FOV as f64, self.aspect(), ndc)
    }

    /// Meters across a pixel at `depth` meters.
    pub fn m_per_px(&self, depth: f64) -> f64 {
        depth * 2.0 * self.half_tan() / self.rect.height().max(1.0) as f64
    }
}

/// Where the ray first meets a part as drawn, the part's primitive uncut and unblended, as the
/// editor's meshes are. The piece's index and the point.
pub fn hit(sdf: &Sdf, origin: DVec3, direction: DVec3, limit_m: f64) -> Option<(usize, DVec3)> {
    hit_except(sdf, origin, direction, limit_m, &std::collections::BTreeSet::new())
}

/// [`hit`], looking through the parts in `skip`.
pub fn hit_except(
    sdf: &Sdf,
    origin: DVec3,
    direction: DVec3,
    limit_m: f64,
    skip: &std::collections::BTreeSet<PartId>,
) -> Option<(usize, DVec3)> {
    const STEPS: usize = 256;
    let (min, max) = sdf.bounds();
    let tolerance = (max - min).length() * 1.0e-4;
    let mut t = 0.0;
    for _ in 0..STEPS {
        let p = origin + direction * t;
        let (piece, d) = (0..sdf.pieces().len())
            .filter(|&i| !skip.contains(&sdf.pieces()[i].part))
            .map(|i| (i, sdf.primitive(i, p)))
            .fold((0, f64::INFINITY), |best, next| if next.1 < best.1 { next } else { best });
        if d < tolerance {
            return Some((piece, p));
        }
        t += d.max(tolerance);
        if t > limit_m {
            return None;
        }
    }
    None
}

/// The part under `at`: the one a ray meets outright, else the nearest drawn within
/// [`SLACK_PX`], so a part a few pixels across can still be taken. Both are built from the pieces
/// as drawn, so picking agrees with the picture.
pub fn pick_part(sdf: &Sdf, lens: &Lens, at: Vec2) -> Option<PartId> {
    let (origin, direction) = lens.ray(at);
    let limit = 10.0 * lens.extent.size_m() * lens.orbit.distance.max(1.0);
    let mut candidates: Vec<Candidate> = Vec::new();
    if let Some((piece, _)) = hit(sdf, origin, direction, limit) {
        candidates.push(Candidate { id: piece as u64, at, radius_px: 0.0, rank: rank::CRAFT });
    }
    for (i, piece) in sdf.pieces().iter().enumerate() {
        let Some((center, depth)) = lens.project(piece.pose.position) else { continue };
        // The least half-extent, so the disc stays inside the part.
        let half = piece.shape.extent(DMat3::IDENTITY, 0.0).min_element();
        candidates.push(Candidate { id: i as u64, at: center, radius_px: (half / lens.m_per_px(depth)) as f32, rank: rank::BODY });
    }
    pick(&candidates, at, SLACK_PX).map(|i| sdf.pieces()[i as usize].part)
}

pub(crate) fn original(sdf: &Sdf, part: PartId) -> Option<(usize, &Piece)> {
    sdf.pieces().iter().enumerate().find(|(_, p)| p.part == part && p.side == Side::Original)
}

/// The part's own half-extents along its own axes.
fn half(piece: &Piece) -> DVec3 {
    piece.shape.extent(DMat3::IDENTITY, 0.0)
}

/// Where the selected part's handles are, ship frame, meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handles {
    pub center: DVec3,
    /// The part's axes, unit.
    pub axes: DMat3,
    /// Each axis line's length.
    pub arms: DVec3,
    /// The size line's end.
    pub diagonal: DVec3,
    pub ring: Ring,
    /// The standoff arrow's root and tip, for an attached part: along the parent's normal at
    /// the anchor, pointing into the parent.
    pub sink: Option<(DVec3, DVec3)>,
    /// Meters across a pixel at the part.
    pub m_per_px: f64,
    /// The parent's shape and the part's, for where a standoff would leave it touching.
    pub shapes: (lc_world::form::primitive::Shape, lc_world::form::primitive::Shape),
}

/// What a part's twist turns it about: its parent's surface normal through its foot, or its
/// parent's axis when it encloses. Not the part's own axis, which a tilt takes off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub center: DVec3,
    pub axis: DVec3,
    /// Across the axis, with `across.0 × across.1 = axis`, so an angle measured from the first
    /// toward the second is a right-handed turn, as a twist is.
    pub across: (DVec3, DVec3),
    pub radius: f64,
}

impl Ring {
    fn about(axis: DVec3, through: DVec3, part: DVec3, h: DVec3, least: f64) -> Ring {
        let axis = axis.normalize();
        let (a, _) = axis.any_orthonormal_pair();
        // Level with the part, and wide enough to go round it where a tilt swings it off the axis.
        let center = through + axis * (part - through).dot(axis);
        let off = (part - center).length();
        Ring { center, axis, across: (a, axis.cross(a)), radius: (off + h.y.max(h.z) * REACH_OUT * 1.1).max(least) }
    }
}

impl Handles {
    pub fn of(part: &Part, piece: &Piece, sdf: &Sdf, lens: &Lens) -> Option<Handles> {
        let placement = part.placement?;
        let (_, parent) = original(sdf, placement.parent)?;
        let (_, depth) = lens.project(piece.pose.position)?;
        let m_per_px = lens.m_per_px(depth);
        let least = MIN_ARM_PX * m_per_px;
        let h = half(piece);
        let arms = (h * REACH_OUT).max(DVec3::splat(least));
        let axes = piece.pose.rotation;
        let center = piece.pose.position;
        let (ring, sink) = match placement.mount {
            Mount::Attached { anchor, .. } => {
                // As placement finds it: where a ray from the parent's middle along the anchor
                // leaves it.
                let exit = parent.shape.exit((anchor / anchor.abs().max_element()).normalize());
                let normal = (parent.pose.rotation * exit.normal).normalize();
                let foot = center - axes.x_axis * piece.shape.reach();
                let ring = Ring::about(normal, foot, center, h, least);
                let tip = ring.center - normal * (piece.shape.reach() * REACH_OUT).max(least);
                (ring, Some((ring.center, tip)))
            }
            Mount::Enclosing => (Ring::about(parent.pose.rotation.x_axis, parent.pose.position, center, h, least), None),
        };
        Some(Handles {
            center,
            axes,
            arms,
            diagonal: center + axes * (h.normalize_or(DVec3::ONE) * (h.length() * REACH_OUT).max(least)),
            ring,
            sink,
            shapes: (parent.shape, piece.shape),
            m_per_px,
        })
    }

    pub fn grips(&self) -> Vec<Grip> {
        let mut out = vec![Grip::Axis(0), Grip::Axis(1), Grip::Axis(2), Grip::Size, Grip::Twist];
        if self.sink.is_some() {
            out.push(Grip::Standoff);
        }
        out
    }

    /// A line handle, from its root to its end.
    fn line(&self, grip: Grip) -> Option<(DVec3, DVec3)> {
        match grip {
            Grip::Axis(i) => Some((self.center, self.center + self.axes.col(i) * self.arms[i])),
            Grip::Size => Some((self.center, self.diagonal)),
            Grip::Standoff => self.sink,
            Grip::Twist => None,
        }
    }

    fn ring(&self) -> Vec<DVec3> {
        let Ring { center, across: (a, b), radius, .. } = self.ring;
        (0..=RING_SEGMENTS)
            .map(|k| {
                let angle = k as f64 * std::f64::consts::TAU / RING_SEGMENTS as f64;
                center + (a * angle.cos() + b * angle.sin()) * radius
            })
            .collect()
    }

    /// The handle under `at`, by what is drawn: a line along its length, its end as a square, the
    /// ring all round.
    pub fn under(&self, lens: &Lens, at: Vec2) -> Option<Grip> {
        let grips = self.grips();
        let mut candidates = Vec::new();
        for (index, grip) in grips.iter().enumerate() {
            let points: Vec<DVec3> = match self.line(*grip) {
                Some((from, to)) => {
                    if let Some((end, _)) = lens.project(to) {
                        // Ends before lines, so a square where a line crosses it is the square's.
                        candidates.push(Candidate { id: index as u64, at: end, radius_px: (END_PX * 0.5) as f32, rank: rank::CRAFT });
                    }
                    vec![from, to]
                }
                None => self.ring(),
            };
            let run: Option<Vec<Vec2>> = points.iter().map(|p| lens.project(*p).map(|(px, _)| px)).collect();
            if let Some((nearest, _)) = run.and_then(|run| nearest_on_path(&[run], at)) {
                candidates.push(Candidate { id: index as u64, at: nearest, radius_px: 0.0, rank: rank::BODY });
            }
        }
        pick(&candidates, at, (LINE_PX as f32).max(6.0)).map(|i| grips[i as usize])
    }
}

/// The parameter along the line through `origin` in unit `direction` of its point nearest the ray.
/// `None` for a ray nearly along the line.
fn along_line(origin: DVec3, direction: DVec3, ray: (DVec3, DVec3)) -> Option<f64> {
    let (from, toward) = ray;
    let b = toward.dot(direction);
    let w = from - origin;
    let denom = 1.0 - b * b;
    (denom.abs() > 1e-9).then(|| (w.dot(direction) - b * w.dot(toward)) / denom)
}

/// The angle about `handles`' axis of where the ray meets the ring's plane. `None` for a ray
/// along the plane.
fn angle_on_ring(handles: &Handles, ray: (DVec3, DVec3)) -> Option<f64> {
    let (from, toward) = ray;
    let Ring { center, axis, across: (a, b), .. } = handles.ring;
    let denom = toward.dot(axis);
    if denom.abs() < 1e-9 {
        return None;
    }
    let v = from + toward * (center - from).dot(axis) / denom - center;
    Some(v.dot(b).atan2(v.dot(a)))
}

/// A drag under way: what was grabbed, the part and its handles as they were when it began, and
/// where on the handle the pointer took hold.
#[derive(Clone, Copy, Debug)]
pub struct Held {
    pub grip: Grip,
    pub part: Part,
    pub handles: Handles,
    /// Along a line from its root, meters, or the angle round the ring.
    pub start: f64,
    pub reach_m: f64,
}

impl Held {
    pub fn new(grip: Grip, part: Part, handles: Handles, piece: &Piece, lens: &Lens, at: Vec2) -> Option<Held> {
        let start = Self::measure(grip, &handles, lens, at)?;
        Some(Held { grip, part, handles, start, reach_m: piece.shape.reach().max(f64::MIN_POSITIVE) })
    }

    fn measure(grip: Grip, handles: &Handles, lens: &Lens, at: Vec2) -> Option<f64> {
        let ray = lens.ray(at);
        match handles.line(grip) {
            Some((from, to)) => along_line(from, (to - from).normalize(), ray),
            None => angle_on_ring(handles, ray),
        }
    }

    /// The edit a drag to `at` makes.
    pub fn edit(&self, lens: &Lens, at: Vec2, keys: Modifiers, balance: &Balance) -> Result<Edit, Refused> {
        let fine = keys.fine;
        let placement = self.part.placement.ok_or(Refused::Mind)?;
        let now = Self::measure(self.grip, &self.handles, lens, at).ok_or(Refused::NoSuchPart(self.part.id))?;
        // How far out a line was pulled, as a ratio of where it was taken hold of.
        let pulled = (now / self.start).max(LEAST_PULL);
        let (what, after) = match self.grip {
            Grip::Size => {
                let volume_m3 = snap::volume(self.part.volume_m3 * pulled.powi(3), balance.min_part_m3, fine);
                (What::Resize, Part { volume_m3, ..self.part })
            }
            Grip::Axis(i) => {
                let after = match keys.keep_volume {
                    true => Part { primitive: stretched(self.part.primitive, i, pulled, fine), ..self.part },
                    false => grown(&self.part, i, pulled, fine),
                };
                (What::Reshape, after)
            }
            Grip::Twist => {
                let turned = (now - self.start + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
                let twist = snap::angle(placement.twist + turned, fine);
                (What::Move, Part { placement: Some(Placement { twist, ..placement }), ..self.part })
            }
            Grip::Standoff => {
                let Mount::Attached { anchor, standoff } = placement.mount else { return Err(Refused::Mind) };
                // The arrow points into the parent, so pulling along it sinks the part. Never out
                // past where it would come away from the parent.
                let wanted = snap::standoff(standoff - (now - self.start) / self.reach_m, fine);
                let at = |s: f64| Placement { mount: Mount::Attached { anchor, standoff: s }, ..placement };
                let (parent, child) = self.handles.shapes;
                let sunk = if wanted <= standoff || crate::draft::touches(&parent, &child, &at(wanted)) {
                    wanted
                } else {
                    furthest_touching(standoff, wanted, fine, |s| crate::draft::touches(&parent, &child, &at(s)))
                };
                (What::Move, Part { placement: Some(at(sunk)), ..self.part })
            }
        };
        Ok(Edit { what, part: self.part.id, before: vec![self.part], after: vec![after], settled: false })
    }
}

/// Between `from`, which touches, and `to`, which does not, the furthest standoff that still
/// touches, on the snapping steps.
fn furthest_touching(from: f64, to: f64, fine: bool, touches: impl Fn(f64) -> bool) -> f64 {
    let (mut near, mut far) = (from, to);
    for _ in 0..32 {
        let mid = (near + far) / 2.0;
        if touches(mid) { near = mid } else { far = mid }
    }
    let step = snap::SNAPS.steps(fine).standoff;
    let snapped = (near / step).floor() * step;
    if snapped >= from && touches(snapped) { snapped } else { from }
}

pub struct FormHandlesPlugin;

impl Plugin for FormHandlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::form_carry::Carried>()
            .init_resource::<Grabbed>()
            .add_systems(Startup, make_looks)
            .add_systems(
                Update,
                (draw, crate::form_carry::drop_on_leaving, crate::form_carry::draw_float)
                    .in_set(crate::app::Stage::Scene)
                    .after(crate::form_view::place)
                    .run_if(in_state(crate::app::AppState::InGame)),
            )
            .add_systems(OnExit(crate::app::AppState::InGame), (put_away, crate::form_carry::put_away));
    }
}

/// The lens for this frame, while the editor is the view and has something to draw.
pub fn lens(ui: &Ui, shown: &Shown, surface: &FormSurface) -> Option<Lens> {
    let extent = shown.extent()?;
    let rect = crate::form_view::picture(surface)?;
    (ui.view == ViewMode::Form).then(|| Lens { orbit: ui.form.orbit.held_to(&extent), extent, rect })
}

/// The modifiers a drag reads: Alt for the finer snapping steps, and Shift to stretch an axis
/// at fixed volume.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Modifiers {
    pub fine: bool,
    pub keep_volume: bool,
}

impl Modifiers {
    pub fn of(keys: &ButtonInput<KeyCode>) -> Self {
        Self {
            fine: keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
            keep_volume: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        }
    }
}

/// The handle being dragged, and the last edit it sent. A press that takes hold of one is
/// neither the slide's nor a part's.
#[derive(Resource, Default)]
pub struct Grabbed(Option<(Held, Option<Edit>, Vec2)>);

impl Grabbed {
    pub fn is_holding(&self) -> bool {
        self.0.is_some()
    }
}

/// The selected part and its handles this frame.
fn selected_handles<'a>(ui: &Ui, sdf: &'a Sdf, lens: &Lens) -> Option<(Part, &'a Piece, Handles)> {
    let part = *ui.form.draft.as_ref()?.part(ui.form.selected?)?;
    let (_, piece) = original(sdf, part.id)?;
    Some((part, piece, Handles::of(&part, piece, sdf, lens)?))
}

/// A drag on a handle, as edits. Each frame the pointer moves sends one measured from the press,
/// and the release sends it settled.
#[allow(clippy::too_many_arguments)]
pub fn drag_handles(
    ui: Res<Ui>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    controls: em_ui::Controls,
    window: Single<&Window, With<PrimaryWindow>>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    carried: Res<crate::form_carry::Carried>,
    mut grabbed: ResMut<Grabbed>,
    mut out: MessageWriter<Requested>,
) {
    let cursor = window.cursor_position();
    // A press while carrying a part puts the part down, whatever it lands on.
    let (Some(lens), Some(sdf), false) = (lens(&ui, &shown, &surface), shown.sdf(), carried.is_carrying()) else {
        grabbed.0 = None;
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        let free = !egui.wants_any_pointer_input() && !controls.under_pointer();
        grabbed.0 = cursor.filter(|at| free && crate::form_view::on_picture(&surface, *at)).and_then(|at| {
            let (part, piece, handles) = selected_handles(&ui, sdf, &lens)?;
            let grip = handles.under(&lens, at)?;
            Some((Held::new(grip, part, handles, piece, &lens, at)?, None, at))
        });
        return;
    }
    let Some((held, last, seen)) = grabbed.0.as_mut() else { return };
    if !buttons.pressed(MouseButton::Left) {
        if let Some(edit) = last.take() {
            out.write(Requested(Action::EditForm(Ok(Edit { settled: true, ..edit }))));
        }
        grabbed.0 = None;
        return;
    }
    let Some(at) = cursor.filter(|at| at != seen) else { return };
    *seen = at;
    match held.edit(&lens, at, Modifiers::of(&keys), &Balance::DEFAULT) {
        Ok(edit) if last.as_ref() != Some(&edit) => {
            *last = Some(edit.clone());
            out.write(Requested(Action::EditForm(Ok(edit))));
        }
        _ => {}
    }
}

/// Delete takes the selected part and its subtree, unless the part is one that may not go.
pub fn delete_key(
    ui: Res<Ui>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: em_ui::Typing,
    egui: Res<EguiWantsInput>,
    carried: Res<crate::form_carry::Carried>,
    mut out: MessageWriter<Requested>,
) {
    if typing.active() || carried.is_carrying() || egui.wants_any_keyboard_input() || !keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
        return;
    }
    if let (Some(id), Some(draft)) = (ui.form.selected, ui.form.draft.as_ref()) {
        out.write(Requested(Action::EditForm(draft.remove(id))));
    }
}

/// Each handle's paint at rest and under the pointer.
#[derive(Resource)]
struct Looks {
    axes: [(Handle<StandardMaterial>, Handle<StandardMaterial>); 3],
    neutral: (Handle<StandardMaterial>, Handle<StandardMaterial>),
    line: Handle<Mesh>,
    end: Handle<Mesh>,
    ball: Handle<Mesh>,
    head: Handle<Mesh>,
}

/// 18 §Handles: the usual red, green and blue for the part's axes, brighter under the pointer.
const AXIS_COLORS: [Color; 3] = [Color::srgb(0.95, 0.25, 0.22), Color::srgb(0.30, 0.85, 0.30), Color::srgb(0.30, 0.50, 1.0)];
const NEUTRAL: Color = Color::srgb(0.85, 0.85, 0.80);

fn make_looks(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mut paint = |color: Color| {
        let lit = color.mix(&Color::WHITE, 0.55);
        let at = |c: Color| StandardMaterial { base_color: c, unlit: true, ..default() };
        (materials.add(at(color)), materials.add(at(lit)))
    };
    let axes = AXIS_COLORS.map(&mut paint);
    let neutral = paint(NEUTRAL);
    commands.insert_resource(Looks {
        axes,
        neutral,
        line: meshes.add(Cylinder::new(0.5, 1.0)),
        end: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ball: meshes.add(Sphere::new(0.5).mesh().uv(24, 12)),
        head: meshes.add(Cone::new(0.5, 1.0)),
    });
}

impl Looks {
    fn of(&self, grip: Grip, lit: bool) -> Handle<StandardMaterial> {
        let (rest, hover) = match grip {
            Grip::Axis(i) => &self.axes[i],
            // The ring turns the part about its own x.
            Grip::Twist => &self.axes[0],
            Grip::Size | Grip::Standoff => &self.neutral,
        };
        if lit { hover.clone() } else { rest.clone() }
    }
}

/// The ship's frame for the handles.
#[derive(Component)]
struct HandleRoot(Option<PartId>);

/// One mesh of a handle, and which: a line's shaft, its end, or a stretch of the ring.
#[derive(Component, Clone, Copy, PartialEq)]
enum Bit {
    Shaft(Grip),
    End(Grip),
    Arc(usize),
}

impl Bit {
    fn grip(self) -> Grip {
        match self {
            Bit::Shaft(g) | Bit::End(g) => g,
            Bit::Arc(_) => Grip::Twist,
        }
    }
}

/// A unit mesh along +y, laid from `from` to `to` at `width` meters across.
fn laid(from: DVec3, to: DVec3, width: f64) -> Transform {
    let span = to - from;
    Transform {
        translation: ((from + to) * 0.5).as_vec3(),
        rotation: Quat::from_rotation_arc(Vec3::Y, span.normalize_or(DVec3::Y).as_vec3()),
        scale: Vec3::new(width as f32, span.length() as f32, width as f32),
    }
}

impl Bit {
    fn place(self, handles: &Handles, ring: &[DVec3]) -> Transform {
        let px = handles.m_per_px;
        match self {
            Bit::Shaft(grip) => {
                let (from, to) = handles.line(grip).unwrap_or_default();
                // The arrow's shaft stops where its head begins.
                let back = if grip == Grip::Standoff { (to - from).normalize_or(DVec3::X) * HEAD_PX.0 * px } else { DVec3::ZERO };
                laid(from, to - back, LINE_PX * px)
            }
            Bit::End(Grip::Standoff) => {
                let (from, to) = handles.line(Grip::Standoff).unwrap_or_default();
                let root = to - (to - from).normalize_or(DVec3::X) * HEAD_PX.0 * px;
                let mut head = laid(root, to, HEAD_PX.1 * 2.0 * px);
                head.scale.y = (HEAD_PX.0 * px) as f32;
                head
            }
            Bit::End(grip) => {
                let (_, to) = handles.line(grip).unwrap_or_default();
                Transform {
                    translation: to.as_vec3(),
                    rotation: Quat::from_mat3(&handles.axes.as_mat3()),
                    scale: Vec3::splat((END_PX * if grip == Grip::Size { BALL_OVER_END } else { 1.0 } * px) as f32),
                }
            }
            Bit::Arc(k) => laid(ring[k], ring[k + 1], LINE_PX * px),
        }
    }
}

/// The selected part's handles, rebuilt when the selection or its set of handles changes and
/// moved every frame; lit under the pointer and while held.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    ui: Res<Ui>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    looks: Res<Looks>,
    grabbed: Res<Grabbed>,
    carried: Res<crate::form_carry::Carried>,
    window: Single<&Window, With<PrimaryWindow>>,
    roots: Query<(Entity, &HandleRoot)>,
    mut bits: Query<(&Bit, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let found = lens(&ui, &shown, &surface).zip(shown.sdf()).filter(|_| !carried.is_carrying());
    let selected = found.and_then(|(lens, sdf)| Some((lens, selected_handles(&ui, sdf, &lens)?)));
    let wanted = selected.as_ref().map(|(_, (part, _, handles))| (part.id, handles.sink.is_some()));
    let built = roots.iter().next();
    let current = built.is_some_and(|(_, root)| root.0 == wanted.map(|(id, _)| id)) && bits.iter().any(|(b, ..)| *b == Bit::End(Grip::Standoff)) == wanted.is_some_and(|(_, sink)| sink);
    if !current || wanted.is_none() {
        for (entity, _) in &roots {
            commands.entity(entity).despawn();
        }
        if let Some((_, (part, _, handles))) = &selected {
            build(&mut commands, &looks, part.id, handles);
        }
        return;
    }
    let Some((lens, (_, _, handles))) = selected else { return };
    let ring = handles.ring();
    let lit = grabbed.0.as_ref().map(|(held, ..)| held.grip).or_else(|| window.cursor_position().and_then(|at| handles.under(&lens, at)));
    for (bit, mut transform, mut material) in &mut bits {
        transform.set_if_neq(bit.place(&handles, &ring));
        let paint = looks.of(bit.grip(), lit == Some(bit.grip()));
        if material.0 != paint {
            material.0 = paint;
        }
    }
}

fn build(commands: &mut Commands, looks: &Looks, part: PartId, handles: &Handles) {
    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z));
    let root = commands.spawn((Transform::from_rotation(turn), Visibility::default(), HandleRoot(Some(part)))).id();
    let mut bits = Vec::new();
    for grip in handles.grips() {
        bits.push((Bit::Shaft(grip), looks.line.clone()));
        match grip {
            Grip::Twist => bits.extend((0..RING_SEGMENTS).map(|k| (Bit::Arc(k), looks.line.clone()))),
            Grip::Standoff => bits.push((Bit::End(grip), looks.head.clone())),
            // Round, so the one that scales everything reads apart from the three that scale one way.
            Grip::Size => bits.push((Bit::End(grip), looks.ball.clone())),
            _ => bits.push((Bit::End(grip), looks.end.clone())),
        }
    }
    let ring = handles.ring();
    for (bit, mesh) in bits.into_iter().filter(|(b, _)| *b != Bit::Shaft(Grip::Twist)) {
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(looks.of(bit.grip(), false)),
            bit.place(handles, &ring),
            bit,
            NoFrustumCulling,
            RenderLayers::layer(FORM_HANDLE_LAYER),
            ChildOf(root),
        ));
    }
}

fn put_away(mut commands: Commands, roots: Query<Entity, With<HandleRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::Form;

    use super::*;
    use crate::draft::Draft;

    const B: Balance = Balance::DEFAULT;

    fn scene() -> (Draft, Sdf, Lens) {
        let draft = Draft::new(Form::starting());
        let sdf = Sdf::new(&draft.form, &B).unwrap();
        let (min, max) = sdf.bounds();
        let extent = Extent { min, max };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 60.0), egui::vec2(1280.0, 660.0));
        let orbit = FormOrbit::default().held_to(&extent);
        (draft, sdf, Lens { orbit, extent, rect })
    }

    fn handles(draft: &Draft, sdf: &Sdf, lens: &Lens, id: u16) -> (Part, Piece, Handles) {
        let part = *draft.part(PartId(id)).unwrap();
        let (_, piece) = original(sdf, part.id).unwrap();
        (part, piece.clone(), Handles::of(&part, piece, sdf, lens).unwrap())
    }

    fn px(lens: &Lens, p: DVec3) -> Vec2 {
        lens.project(p).unwrap().0
    }

    /// A projected point comes back along the ray through the pixel it was drawn at.
    #[test]
    fn a_point_projects_to_the_pixel_whose_ray_passes_through_it() {
        let (_, _, lens) = scene();
        let p = DVec3::new(120.0, -40.0, 30.0);
        let (at, _) = lens.project(p).unwrap();
        let (origin, direction) = lens.ray(at);
        let off = (p - origin) - direction * (p - origin).dot(direction);
        assert!(off.length() < 1e-3 * (p - origin).length(), "{off}");
    }

    /// **Picking agrees with what is drawn**: a pixel on a part picks that part, and a pixel far
    /// from the ship picks nothing.
    #[test]
    fn the_part_drawn_under_a_pixel_is_the_part_picked() {
        let (_, sdf, lens) = scene();
        for (i, piece) in sdf.pieces().iter().enumerate() {
            let Some((at, _)) = lens.project(piece.pose.position) else { continue };
            let (origin, direction) = lens.ray(at);
            let Some((first, _)) = hit(&sdf, origin, direction, 1e7) else { continue };
            assert_eq!(pick_part(&sdf, &lens, at), Some(sdf.pieces()[first].part), "piece {i}");
        }
        assert_eq!(pick_part(&sdf, &lens, Vec2::new(5.0, 700.0)), None);
    }

    /// Each handle is picked where it is drawn: its end, and anywhere along it.
    #[test]
    fn a_handle_is_picked_where_it_is_drawn() {
        let (draft, sdf, lens) = scene();
        let (_, _, h) = handles(&draft, &sdf, &lens, 5);
        for grip in [Grip::Axis(0), Grip::Axis(1), Grip::Axis(2), Grip::Size, Grip::Standoff] {
            let (from, to) = h.line(grip).unwrap();
            assert_eq!(h.under(&lens, px(&lens, to)), Some(grip), "{grip:?} at its end");
            let along = px(&lens, from + (to - from) * 0.8);
            assert_eq!(h.under(&lens, along), Some(grip), "{grip:?} along it");
        }
        assert_eq!(h.under(&lens, px(&lens, h.ring()[5])), Some(Grip::Twist));
        assert_eq!(h.under(&lens, Vec2::new(5.0, 700.0)), None);
        let (storage, _, h) = handles(&draft, &sdf, &lens, 1);
        assert!(h.sink.is_none() && !h.grips().contains(&Grip::Standoff), "{:?} encloses, so no standoff", storage.id);
    }

    /// However small the part is drawn, a handle is long enough to take hold of.
    #[test]
    fn a_handle_is_never_shorter_than_a_grip() {
        let (draft, sdf, mut lens) = scene();
        lens.orbit.distance = 4.0;
        let (_, _, h) = handles(&draft, &sdf, &lens, 5);
        for i in 0..3 {
            assert!(h.arms[i] / h.m_per_px >= MIN_ARM_PX - 1e-9);
        }
    }

    /// Pulled out to twice where it was taken hold of, the size line grows the part eightfold,
    /// on the ladder.
    #[test]
    fn the_size_line_scales_the_part_as_far_as_it_is_pulled() {
        let (draft, sdf, lens) = scene();
        let (part, piece, h) = handles(&draft, &sdf, &lens, 3);
        let (from, to) = h.line(Grip::Size).unwrap();
        let held = Held::new(Grip::Size, part, h, &piece, &lens, px(&lens, to)).unwrap();
        let edit = held.edit(&lens, px(&lens, from + (to - from) * 2.0), Modifiers::default(), &B).unwrap();
        assert_eq!(edit.before, vec![part]);
        let grown = edit.after[0].volume_m3;
        assert!((grown / part.volume_m3 - 8.0).abs() < 1.5, "{}", grown / part.volume_m3);
        assert!((snap::volume(grown, B.min_part_m3, false) - grown).abs() < 1e-6 * grown, "on the ladder");
        let shrunk = held.edit(&lens, px(&lens, from + (to - from) * 0.5), Modifiers::default(), &B).unwrap();
        assert!(shrunk.after[0].volume_m3 < part.volume_m3);
        assert!(!edit.settled, "a drag is settled on release");
    }

    /// An axis line grows that dimension and the volume with it; with Shift the volume is kept
    /// and the other dimensions give way.
    #[test]
    fn an_axis_line_grows_the_part_unless_the_volume_is_held() {
        let (draft, sdf, lens) = scene();
        let (part, piece, h) = handles(&draft, &sdf, &lens, 3);
        let (from, to) = h.line(Grip::Axis(0)).unwrap();
        let held = Held::new(Grip::Axis(0), part, h, &piece, &lens, px(&lens, to)).unwrap();
        let at = px(&lens, from + (to - from) * 1.6);
        let free = held.edit(&lens, at, Modifiers::default(), &B).unwrap().after[0];
        assert!(free.volume_m3 > part.volume_m3 * 1.2, "{} to {}", part.volume_m3, free.volume_m3);
        let kept = held.edit(&lens, at, Modifiers { keep_volume: true, ..Modifiers::default() }, &B).unwrap().after[0];
        assert_eq!(kept.volume_m3, part.volume_m3);
        assert_eq!(kept.primitive, free.primitive, "the same stretch either way");
        assert_ne!(kept.primitive, part.primitive);
    }

    /// A quarter turn round the ring is a quarter turn of the part about what twist turns it
    /// about, the way the pointer went: for the drone pod, tilted to lie under the keel, that is
    /// the hull's normal and not the pod's own axis.
    #[test]
    fn the_ring_turns_the_part_about_its_twist_axis_as_far_as_the_pointer_goes() {
        let (draft, sdf, lens) = scene();
        for id in [5, 3] {
            let (part, piece, h) = handles(&draft, &sdf, &lens, id);
            let Ring { center, axis, across: (a, b), radius } = h.ring;
            let held = Held::new(Grip::Twist, part, h, &piece, &lens, px(&lens, center + a * radius)).unwrap();
            let edit = held.edit(&lens, px(&lens, center + b * radius), Modifiers::default(), &B).unwrap();
            let twist = edit.after[0].placement.unwrap().twist;
            assert!((twist - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "part {id}: {twist}");
            let mut moved = draft.clone();
            moved.apply(&edit, &B).unwrap();
            let turned = original(&Sdf::new(&moved.form, &B).unwrap(), PartId(id)).unwrap().1.pose.rotation;
            let expected = glam::DQuat::from_axis_angle(axis, std::f64::consts::FRAC_PI_2) * piece.pose.rotation.x_axis;
            assert!(turned.x_axis.dot(expected) > 0.999, "part {id}: {} against {expected}", turned.x_axis);
        }
        let (_, piece, pod) = handles(&draft, &sdf, &lens, 3);
        assert!(pod.ring.axis.dot(piece.pose.rotation.x_axis).abs() < 0.9, "the pod is tilted off the normal");
    }

    /// The arrow runs along the parent's normal at the anchor, into the parent, whatever the tilt.
    #[test]
    fn the_standoff_arrow_is_the_parents_normal() {
        let (draft, sdf, lens) = scene();
        let (part, piece, pod) = handles(&draft, &sdf, &lens, 3);
        let (root, tip) = pod.sink.unwrap();
        let before = piece.pose.position;
        let sunk = draft.standoff(PartId(3), -1.0, &B).unwrap();
        let mut moved = draft.clone();
        moved.apply(&sunk, &B).unwrap();
        let after = original(&Sdf::new(&moved.form, &B).unwrap(), PartId(3)).unwrap().1.pose.position;
        let Mount::Attached { standoff, .. } = part.placement.unwrap().mount else { panic!() };
        assert!(standoff > -1.0);
        assert!((after - before).normalize().dot((tip - root).normalize()) > 0.999, "sinking moves it along the arrow");
    }

    /// The arrow points into the parent: pulled along it the part sinks, by tenths, and pushed
    /// back it stops resting on the parent.
    #[test]
    fn the_standoff_arrow_sinks_the_part_and_stops_where_it_would_come_away() {
        let (draft, sdf, lens) = scene();
        let (part, piece, h) = handles(&draft, &sdf, &lens, 2);
        let (from, to) = h.line(Grip::Standoff).unwrap();
        let held = Held::new(Grip::Standoff, part, h, &piece, &lens, px(&lens, to)).unwrap();
        let deeper = held.edit(&lens, px(&lens, from + (to - from) * 1.4), Modifiers::default(), &B).unwrap();
        let Mount::Attached { standoff, .. } = deeper.after[0].placement.unwrap().mount else { panic!() };
        assert!(standoff < -0.2 && (standoff * 10.0 - (standoff * 10.0).round()).abs() < 1e-9, "{standoff}");
        let out = held.edit(&lens, px(&lens, from - (to - from) * 3.0), Modifiers::default(), &B).unwrap();
        let Mount::Attached { standoff, .. } = out.after[0].placement.unwrap().mount else { panic!() };
        let placed = Placement { mount: Mount::Attached { anchor: DVec3::NEG_X, standoff }, ..part.placement.unwrap() };
        assert!(crate::draft::touches(&h.shapes.0, &h.shapes.1, &placed), "it stops touching: {standoff}");
        let further = Placement { mount: Mount::Attached { anchor: DVec3::NEG_X, standoff: standoff + 0.1 }, ..placed };
        assert!(!crate::draft::touches(&h.shapes.0, &h.shapes.1, &further), "and no further than that: {standoff}");
    }

    #[test]
    fn the_mind_has_no_handles() {
        let (draft, sdf, lens) = scene();
        let mind = *draft.part(PartId(0)).unwrap();
        assert!(Handles::of(&mind, original(&sdf, PartId(0)).unwrap().1, &sdf, &lens).is_none());
    }
}
