//! The editor's handles: picking a part where it is drawn, and knobs that edit it there.
//!
//! Every knob is a Bevy UI button over the picture, so [`em_ui::Controls`] keeps the camera off a
//! drag that starts on one, as [`form_view::drag_of`] keeps it off a press on a part. A drag sends
//! an edit each frame it moves, every one carrying the part as it was when the drag began, and a
//! settled one on release: one gesture, one entry in the history. The lines between the knobs are
//! gizmos on the editor's own layer. See `lightcone/docs/29-ship-form.md` §Handles.

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use bevy_egui::input::EguiWantsInput;
use em_ui::picking::{Candidate, SLACK_PX, pick, rank};
use em_ui::{MenuTheme, MenuUi};
use glam::{DMat3, DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::place::Side;
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Mount, Part, PartId, Placement};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Edit, Refused, What, grown, stretched};
use crate::form_view::{Extent, FORM_FOV, FORM_LAYER, FormOrbit, FormSurface, Shown};
use crate::input::Requested;
use crate::snap;
use crate::ui::ViewMode;

/// Which edit a knob makes.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// Volume at fixed proportions.
    Size,
    /// One dimension, along the part's own axis. The volume goes with it, or with the
    /// constant-volume modifier the others give way instead.
    Axis(usize),
    Twist,
    Standoff,
}

impl Grip {
    fn label(self) -> &'static str {
        match self {
            Grip::Size => "SIZE",
            Grip::Axis(0) => "X",
            Grip::Axis(1) => "Y",
            Grip::Axis(_) => "Z",
            Grip::Twist => "TWIST",
            Grip::Standoff => "OUT",
        }
    }
}

/// Pixels of drag for volume to grow by a factor of e.
const SIZE_PX: f64 = 80.0;
/// Pixels of drag for a ratio to grow by a factor of e.
const STRETCH_PX: f64 = 120.0;
/// Past the part's own half-extent, how far out the axis knobs and the twist ring sit.
const REACH_OUT: f64 = 1.3;
const KNOB: Vec2 = Vec2::new(46.0, 20.0);

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

/// Where each knob of `part` goes, ship frame, or `None` for one placed on screen from the
/// part's middle: the size knob, which has no direction of its own.
fn anchors(part: &Part, piece: &Piece) -> Vec<(Grip, Option<DVec3>)> {
    if part.placement.is_none() {
        return Vec::new();
    }
    let h = half(piece);
    let pose = &piece.pose;
    let mut out = vec![(Grip::Size, None)];
    for i in 0..3 {
        out.push((Grip::Axis(i), Some(pose.to_outer(DVec3::AXES[i] * h[i] * REACH_OUT))));
    }
    out.push((Grip::Twist, Some(pose.to_outer(DVec3::Z * h.y.max(h.z) * REACH_OUT * 1.2))));
    if matches!(part.placement.map(|p| p.mount), Some(Mount::Attached { .. })) {
        out.push((Grip::Standoff, Some(pose.to_outer(-DVec3::X * piece.shape.reach()))));
    }
    out
}

/// Every knob of the selected part, on screen.
pub fn knobs(part: &Part, piece: &Piece, lens: &Lens) -> Vec<(Grip, Vec2)> {
    let Some((center, depth)) = lens.project(piece.pose.position) else { return Vec::new() };
    let radius_px = (half(piece).max_element() / lens.m_per_px(depth)) as f32;
    anchors(part, piece)
        .into_iter()
        .filter_map(|(grip, at)| match at {
            Some(at) => lens.project(at).map(|(px, _)| (grip, px)),
            None => Some((grip, center + Vec2::new(0.8, -0.8) * radius_px.max(30.0))),
        })
        .collect()
}

/// A drag under way: what was grabbed, and everything about the part it is measured against, as
/// it was when the drag began.
#[derive(Clone, Copy, Debug)]
pub struct Held {
    pub grip: Grip,
    pub part: Part,
    pub from: Vec2,
    /// The part's middle on screen, and the knob.
    pub center: Vec2,
    pub knob: Vec2,
    /// The part's axes on screen, unit, and whether its own x points away from the eye.
    pub axes: [Vec2; 3],
    pub away: bool,
    pub m_per_px: f64,
    pub reach_m: f64,
}

impl Held {
    pub fn new(grip: Grip, part: Part, sdf: &Sdf, lens: &Lens, from: Vec2) -> Option<Held> {
        let (_, piece) = original(sdf, part.id)?;
        let (center, depth) = lens.project(piece.pose.position)?;
        let knob = knobs(&part, piece, lens).into_iter().find(|(g, _)| *g == grip).map_or(from, |(_, at)| at);
        let axes = [0, 1, 2].map(|i| {
            let tip = piece.pose.position + piece.pose.rotation.col(i) * half(piece)[i].max(1.0);
            lens.project(tip).map_or(Vec2::X, |(px, _)| (px - center).normalize_or(Vec2::X))
        });
        let away = piece.pose.axis().dot(lens.orbit.basis()[0]) > 0.0;
        Some(Held {
            grip,
            part,
            from,
            center,
            knob,
            axes,
            away,
            m_per_px: lens.m_per_px(depth),
            reach_m: piece.shape.reach().max(f64::MIN_POSITIVE),
        })
    }

    /// The edit a drag to `at` makes.
    pub fn edit(&self, at: Vec2, keys: Modifiers, balance: &Balance) -> Result<Edit, Refused> {
        let fine = keys.fine;
        let drag = at - self.from;
        let along = |direction: Vec2| drag.dot(direction.normalize_or(Vec2::X)) as f64;
        let placement = self.part.placement.ok_or(Refused::Mind)?;
        let (what, after) = match self.grip {
            Grip::Size => {
                let factor = (along(self.knob - self.center) / SIZE_PX).exp();
                let volume_m3 = snap::volume(self.part.volume_m3 * factor, balance.min_part_m3, fine);
                (What::Resize, Part { volume_m3, ..self.part })
            }
            Grip::Axis(i) => {
                let factor = (along(self.axes[i]) / STRETCH_PX).exp();
                let after = match keys.keep_volume {
                    true => Part { primitive: stretched(self.part.primitive, i, factor, fine), ..self.part },
                    false => grown(&self.part, i, factor, fine),
                };
                (What::Reshape, after)
            }
            Grip::Twist => {
                let (a, b) = (self.from - self.center, at - self.center);
                // Clockwise on a screen whose y is down, which turns a part pointing away from
                // the eye the right-handed way about its axis.
                let turned = (a.perp_dot(b) as f64).atan2(a.dot(b) as f64);
                let sign = if self.away { 1.0 } else { -1.0 };
                let twist = snap::angle(placement.twist + sign * turned, fine);
                (What::Move, Part { placement: Some(Placement { twist, ..placement }), ..self.part })
            }
            Grip::Standoff => {
                let Mount::Attached { anchor, standoff } = placement.mount else { return Err(Refused::Mind) };
                let moved = snap::standoff(standoff + along(self.axes[0]) * self.m_per_px / self.reach_m, fine);
                let mount = Mount::Attached { anchor, standoff: moved };
                (What::Move, Part { placement: Some(Placement { mount, ..placement }), ..self.part })
            }
        };
        Ok(Edit { what, part: self.part.id, before: vec![self.part], after: vec![after], settled: false })
    }
}

pub struct FormHandlesPlugin;

impl Plugin for FormHandlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<FormGizmos>()
            .init_resource::<crate::form_carry::Carried>()
            .add_systems(Startup, configure_gizmos)
            .add_systems(
                Update,
                (lay_out, crate::form_carry::show_tag, crate::form_carry::drop_on_leaving)
                    .in_set(crate::app::Stage::Scene)
                    .after(crate::form_view::place)
                    .run_if(in_state(crate::app::AppState::InGame)),
            )
            .add_systems(OnExit(crate::app::AppState::InGame), put_away);
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
struct FormGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<FormGizmos>();
    config.render_layers = RenderLayers::layer(FORM_LAYER);
    config.line.width = 2.0;
    // In front of the parts, so a ring through a hull is seen whole.
    config.depth_bias = -1.0;
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

/// A drag on a knob, as edits. Each frame the pointer moves sends one measured from the press,
/// and the release sends it settled.
#[allow(clippy::too_many_arguments)]
pub fn drag_knobs(
    ui: Res<Ui>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    knobs: Query<(&Interaction, &Grip)>,
    window: Single<&Window, With<PrimaryWindow>>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    carried: Res<crate::form_carry::Carried>,
    mut held: Local<Option<(Held, Option<Edit>, Vec2)>>,
    mut out: MessageWriter<Requested>,
) {
    let cursor = window.cursor_position();
    // A press while carrying a part puts the part down, whatever it lands on.
    let (Some(lens), Some(sdf), false) = (lens(&ui, &shown, &surface), shown.sdf(), carried.is_carrying()) else {
        *held = None;
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        let grabbed = knobs.iter().find(|(i, _)| **i == Interaction::Pressed).map(|(_, g)| *g);
        let part = ui.form.selected.and_then(|id| ui.form.draft.as_ref()?.part(id).copied());
        *held = match (grabbed, part, cursor) {
            (Some(grip), Some(part), Some(at)) => Held::new(grip, part, sdf, &lens, at).map(|h| (h, None, at)),
            _ => None,
        };
        return;
    }
    let Some((grip, last, seen)) = held.as_mut() else { return };
    if !buttons.pressed(MouseButton::Left) {
        if let Some(edit) = last.take() {
            out.write(Requested(Action::EditForm(Ok(Edit { settled: true, ..edit }))));
        }
        *held = None;
        return;
    }
    let Some(at) = cursor.filter(|at| at != seen) else { return };
    *seen = at;
    match grip.edit(at, Modifiers::of(&keys), &Balance::DEFAULT) {
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
        out.write(Requested(Action::EditForm(draft.remove(id, &Balance::DEFAULT))));
    }
}

/// Everything laid over the picture that is the handles'.
#[derive(Component)]
struct Layer;

#[derive(Component)]
struct RoleLabel;

#[derive(Component)]
struct Badge(PartId);

/// What the layer was built for: the selected part and which knobs it has, and the marks.
#[derive(Component, PartialEq)]
struct Built(Option<PartId>, Vec<Grip>, Vec<(PartId, crate::draft::Mark)>);

#[allow(clippy::too_many_arguments)]
fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    assets: Res<AssetServer>,
    layers: Query<(Entity, &Built), With<Layer>>,
    mut placed: Query<(&mut Node, Option<&Grip>, Option<&Badge>, Option<&mut Text>, Has<RoleLabel>), Without<Layer>>,
    mut gizmos: Gizmos<FormGizmos>,
) {
    let (Some(lens), Some(sdf), Some(draft)) = (lens(&ui, &shown, &surface), shown.sdf(), ui.form.draft.as_ref()) else {
        for (entity, _) in &layers {
            commands.entity(entity).despawn();
        }
        return;
    };
    let selected = ui.form.selected.and_then(|id| Some((*draft.part(id)?, original(sdf, id)?.1)));
    let grips: Vec<Grip> = selected.map(|(part, piece)| anchors(&part, piece).into_iter().map(|(g, _)| g).collect()).unwrap_or_default();
    let marks: Vec<(PartId, crate::draft::Mark)> = shown.marks().iter().map(|(id, m)| (*id, *m)).collect();
    let wanted = Built(selected.map(|(p, _)| p.id), grips, marks);
    if !layers.iter().any(|(_, built)| *built == wanted) {
        for (entity, _) in &layers {
            commands.entity(entity).despawn();
        }
        build(&mut commands, wanted, assets.load(crate::faces::UI_FILE));
        return;
    }

    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z));
    let to_render = |p: DVec3| turn * p.as_vec3();
    let knob_at = selected.map(|(part, piece)| knobs(&part, piece, &lens)).unwrap_or_default();
    for (mut node, grip, badge, text, role) in &mut placed {
        let at = if let Some(grip) = grip {
            knob_at.iter().find(|(g, _)| g == grip).map(|(_, at)| *at - KNOB * 0.5)
        } else if let Some(Badge(id)) = badge {
            let piece = original(sdf, *id).map(|(_, p)| p).or_else(|| shown.ghost().and_then(|g| original(g, *id).map(|(_, p)| p)));
            piece.and_then(|p| lens.project(p.pose.position)).map(|(at, _)| at + Vec2::new(-24.0, 14.0))
        } else if role {
            if let (Some(mut text), Some((part, _))) = (text, selected) {
                let said = draft.role(part.id, &Balance::DEFAULT);
                if text.0 != said {
                    text.0 = said;
                }
            }
            selected.and_then(|(_, piece)| lens.project(piece.pose.position)).map(|(at, _)| at + Vec2::new(-60.0, -46.0))
        } else {
            continue;
        };
        let (display, left, top) = match at {
            Some(at) => (Display::Flex, Val::Px(at.x), Val::Px(at.y)),
            None => (Display::None, node.left, node.top),
        };
        if node.display != display || node.left != left || node.top != top {
            node.display = display;
            node.left = left;
            node.top = top;
        }
    }

    let Some((part, piece)) = selected else { return };
    let color = em_ui::vfd::TEXT;
    let center = piece.pose.position;
    for (grip, at) in anchors(&part, piece) {
        match (grip, at) {
            (Grip::Axis(_) | Grip::Standoff, Some(at)) => {
                gizmos.line(to_render(center), to_render(at), color);
            }
            (Grip::Twist, Some(at)) => {
                let radius = (at - center).length() as f32;
                let axis = turn * piece.pose.axis().as_vec3();
                gizmos.circle(Isometry3d::new(to_render(center), Quat::from_rotation_arc(Vec3::Z, axis)), radius, color);
            }
            _ => {}
        }
    }
}

fn build(commands: &mut Commands, built: Built, font: Handle<Font>) {
    let layer = commands
        .spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
            Layer,
        ))
        .id();
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let absolute = |node: Node| Node { position_type: PositionType::Absolute, display: Display::None, ..node };
    for &grip in &built.1 {
        let knob = ui.small_button(layer, grip.label(), grip);
        ui.insert(knob, absolute(Node {
            width: Val::Px(KNOB.x),
            height: Val::Px(KNOB.y),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        }));
    }
    if built.0.is_some() {
        let role = ui.inline(layer, "", 14.0, em_ui::vfd::TEXT);
        ui.insert(role, absolute(Node::default()));
        ui.insert(role, RoleLabel);
    }
    for &(id, mark) in &built.2 {
        let text = format!("{} {}", sign(mark), mark.word());
        let badge = ui.inline(layer, &text, 12.0, mark.color());
        ui.insert(badge, absolute(Node::default()));
        ui.insert(badge, Badge(id));
    }
    ui.insert(layer, built);
}

/// A second signal beside the color.
fn sign(mark: crate::draft::Mark) -> &'static str {
    use crate::draft::Mark;
    match mark {
        Mark::Build => "+",
        Mark::Dismantle => "-",
        Mark::Move => ">",
        Mark::Rebuild => "*",
    }
}

fn put_away(mut commands: Commands, layers: Query<Entity, With<Layer>>) {
    for entity in &layers {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::Form;

    use crate::draft::Draft;

    use super::*;

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

    /// Dragging the size knob outward grows the part on the R10 ladder, and the edit carries the
    /// part as it was at the press.
    #[test]
    fn the_size_knob_grows_the_part_on_the_ladder() {
        let (draft, sdf, lens) = scene();
        let part = *draft.part(PartId(3)).unwrap();
        let held = Held::new(Grip::Size, part, &sdf, &lens, Vec2::ZERO).unwrap();
        let out = (held.knob - held.center).normalize() * 40.0;
        let edit = held.edit(held.from + out, Modifiers::default(), &B).unwrap();
        assert_eq!(edit.before, vec![part]);
        let grown = edit.after[0].volume_m3;
        assert!(grown > part.volume_m3);
        assert!((snap::volume(grown, B.min_part_m3, false) - grown).abs() < 1e-6 * grown, "{grown} is on the ladder");
        let shrunk = held.edit(held.from - out, Modifiers::default(), &B).unwrap().after[0].volume_m3;
        assert!(shrunk < part.volume_m3);
        assert!(!edit.settled, "a drag is settled on release");
    }

    /// An axis knob grows that dimension and the volume with it; with Shift the volume is kept
    /// and the other dimensions give way.
    #[test]
    fn an_axis_knob_grows_the_part_unless_the_volume_is_held() {
        let (draft, sdf, lens) = scene();
        let part = *draft.part(PartId(3)).unwrap();
        let held = Held::new(Grip::Axis(0), part, &sdf, &lens, Vec2::ZERO).unwrap();
        let at = held.from + held.axes[0] * 100.0;
        let free = held.edit(at, Modifiers::default(), &B).unwrap().after[0];
        assert!(free.volume_m3 > part.volume_m3 * 1.2, "{} to {}", part.volume_m3, free.volume_m3);
        let kept = held.edit(at, Modifiers { keep_volume: true, ..Modifiers::default() }, &B).unwrap().after[0];
        assert_eq!(kept.volume_m3, part.volume_m3);
        assert_eq!(kept.primitive, free.primitive, "the same stretch either way");
        assert_ne!(kept.primitive, part.primitive);
    }

    #[test]
    fn a_twist_drag_goes_by_fifteen_degrees() {
        let (draft, sdf, lens) = scene();
        let part = *draft.part(PartId(5)).unwrap();
        let held = Held::new(Grip::Twist, part, &sdf, &lens, Vec2::ZERO).unwrap();
        let from = held.center + Vec2::new(100.0, 0.0);
        let held = Held { from, ..held };
        let quarter = held.edit(held.center + Vec2::new(0.0, 100.0), Modifiers::default(), &B).unwrap();
        let twist = quarter.after[0].placement.unwrap().twist;
        assert!((twist.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{twist}");
        let step = snap::SNAPS.coarse.angle_rad;
        assert!((twist / step - (twist / step).round()).abs() < 1e-9);
    }

    #[test]
    fn the_standoff_knob_goes_by_tenths_and_an_enclosing_part_has_none() {
        let (draft, sdf, lens) = scene();
        let part = *draft.part(PartId(2)).unwrap();
        let held = Held::new(Grip::Standoff, part, &sdf, &lens, Vec2::ZERO).unwrap();
        let edit = held.edit(held.from + held.axes[0] * 200.0, Modifiers::default(), &B).unwrap();
        let Mount::Attached { standoff, .. } = edit.after[0].placement.unwrap().mount else { panic!() };
        assert!(standoff > -0.2 && (standoff * 10.0 - (standoff * 10.0).round()).abs() < 1e-9, "{standoff}");
        let storage = *draft.part(PartId(1)).unwrap();
        let grips: Vec<Grip> = anchors(&storage, original(&sdf, PartId(1)).unwrap().1).into_iter().map(|(g, _)| g).collect();
        assert!(!grips.contains(&Grip::Standoff));
        assert!(anchors(draft.part(PartId(0)).unwrap(), original(&sdf, PartId(0)).unwrap().1).is_empty(), "the Mind has none");
    }
}
