//! Carrying a part, as a spaceplane hangar does: a click picks up an attached part, it hangs from
//! whatever part the pointer meets, and the next click puts it down. New parts come off the
//! palette the same way, and a drop on the palette deletes. Each move is an unsettled edit from
//! where it was picked up, so the whole carry is one entry in the history.
//!
//! It goes where the pointer meets the parent, and Shift holds it to the parent's midlines. A part
//! lying along its parent keeps its heading in the ship's frame and the side it lies on, and is
//! centered on the pointer, so an engine firing aft still fires aft on another face. Space turns
//! it through [`Orient`]'s cycle.

use std::collections::BTreeSet;

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use glam::{DMat3, DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::place::{Pose, Side, normal_frame, twist_tilt_of};
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Kind, Mount, Part, PartId, Placement, Primitive};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Draft, Edit, What};
use crate::form_handles::{Lens, Modifiers, hit_except, lens, original, pick_part};
use crate::form_view::{FormSurface, Shown};
use crate::input::Requested;
use crate::snap;

#[derive(Resource, Default)]
pub struct Carried(Option<Carry>, bool);

impl Carried {
    pub fn is_carrying(&self) -> bool {
        self.0.is_some()
    }

    /// Whether this frame's press put a part down, so a button under it does not also fire.
    pub fn just_dropped(&self) -> bool {
        self.1
    }

    pub fn carry(&self) -> Option<&Carry> {
        self.0.as_ref()
    }

    pub fn take(&mut self, carry: Carry) {
        self.0 = Some(carry);
    }
}

/// A drop on it, buttons included, deletes what is carried.
#[derive(Component)]
pub struct DropZone;

#[derive(Clone, Debug, PartialEq)]
pub struct Carry {
    /// `None` for a new part.
    pub was: Option<Part>,
    pub part: Part,
    /// Otherwise the draft is as it was before the carry, and the part floats at the pointer.
    pub hung: bool,
    /// What it cannot hang from: itself and what hangs from it.
    pub subtree: BTreeSet<PartId>,
    pub float: Float,
    pub orient: Orient,
    /// The midlines it is held to, while Shift holds it.
    pub guide: Option<Guide>,
}

/// How a carried part is turned as it moves from face to face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Orient {
    /// Its axis out of the surface, turned as its placement says.
    Standing { twist: f64, tilt: DVec2 },
    /// Its axis along the surface, as near `axis` (ship frame) as the surface allows, with its own
    /// `down` (a unit axis of its frame) against the surface. `rest` seats it on the surface
    /// rather than at the standoff it was carried with.
    Lying { axis: DVec3, down: DVec3, rest: bool },
}

/// Past this the axis is nearer along the surface than out of it.
const STANDING_COS: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Normals closer than this, about 10°, are one face.
const SAME_FACE_COS: f64 = 0.985;

/// A heading the surface turns nearly square to is no heading on it.
const LEAST_ALONG: f64 = 1e-6;

impl Orient {
    /// What Space turns it to: standing, lying fore and aft either way, lying across.
    pub fn next(self) -> Orient {
        let lying = |axis| Orient::Lying { axis, down: DVec3::NEG_Z, rest: true };
        match self {
            Orient::Standing { .. } => lying(DVec3::X),
            Orient::Lying { axis, .. } if axis.x > STANDING_COS => lying(DVec3::NEG_X),
            Orient::Lying { axis, .. } if axis.x < -STANDING_COS => lying(DVec3::Y),
            Orient::Lying { .. } => Orient::Standing { twist: 0.0, tilt: DVec2::ZERO },
        }
    }

    /// As a part already hangs: `rotation` its own, `normal` its parent's at its anchor, both in
    /// the ship's frame.
    fn of(placement: &Placement, rotation: DMat3, normal: DVec3) -> Orient {
        let axis = rotation.x_axis;
        if axis.dot(normal).abs() >= STANDING_COS {
            return Orient::Standing { twist: placement.twist, tilt: placement.tilt };
        }
        let down = [DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z]
            .into_iter()
            .max_by(|a, b| (rotation * *a).dot(-normal).total_cmp(&(rotation * *b).dot(-normal)))
            .unwrap_or(DVec3::NEG_Z);
        Orient::Lying { axis, down, rest: false }
    }
}

/// A parent's midlines: the center planes across its axes `axes`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub host: PartId,
    pub axes: [usize; 2],
}

/// Where a part goes on `host` (its original copy) for a point there, `target` in its frame.
struct Seat {
    anchor: DVec3,
    twist: f64,
    tilt: DVec2,
    standoff: Option<f64>,
}

impl Seat {
    /// `heading` is a lying part's axis already in the host's frame.
    fn on(host: &Piece, target: DVec3, child: &lc_world::form::primitive::Shape, orient: Orient, heading: DVec3) -> Seat {
        let standing = |twist, tilt| Seat { anchor: target.normalize_or(DVec3::X), twist, tilt, standoff: None };
        let Orient::Lying { down, rest, .. } = orient else {
            let Orient::Standing { twist, tilt } = orient else { unreachable!() };
            return standing(twist, tilt);
        };
        let along = |normal: DVec3| {
            let v = heading - normal * normal.dot(heading);
            (v.length() > LEAST_ALONG).then(|| v.normalize())
        };
        let under = host.shape.exit(target.normalize_or(DVec3::X)).normal;
        let Some(first) = along(under) else { return standing(0.0, DVec2::ZERO) };
        // Its center over the target, so its foot a reach back along the heading, unless that is
        // over an edge, where it hangs by its foot at the target instead.
        let reach = child.reach();
        let back = target - first * reach;
        let over = host.shape.exit(back.normalize_or(DVec3::X));
        let (foot, exit) = if over.normal.dot(under) >= SAME_FACE_COS {
            (back, over)
        } else {
            (target, host.shape.exit(target.normalize_or(DVec3::X)))
        };
        let axis = along(exit.normal).unwrap_or(first);
        let own = DMat3::from_cols(DVec3::X, -down, DVec3::X.cross(-down));
        let wanted = DMat3::from_cols(axis, exit.normal, axis.cross(exit.normal)) * own.transpose();
        let (twist, tilt) = twist_tilt_of(normal_frame(exit.normal).transpose() * wanted);
        let half = child.extent(DMat3::IDENTITY, 0.0);
        let standoff = rest.then(|| (half * down).abs().max_element() / reach);
        Seat { anchor: foot.normalize_or(DVec3::X), twist, tilt, standoff }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Float {
    /// The subtree, about the carried part's middle.
    pub pieces: Vec<Piece>,
    pub rotation: DMat3,
    /// How far in front of the eye it was taken, meters. `None` floats at the focus.
    pub depth_m: Option<f64>,
}

impl Float {
    pub fn at(&self, lens: &Lens, at: Vec2) -> Vec<Piece> {
        let (origin, direction) = lens.ray(at);
        let forward = lens.orbit.basis()[0];
        let depth = self.depth_m.unwrap_or(lens.orbit.distance * lens.extent.size_m());
        let middle = origin + direction * depth / direction.dot(forward).max(1e-6);
        let turn = Pose { position: middle, rotation: self.rotation };
        self.pieces.iter().map(|p| Piece { pose: turn.then(&p.pose), ..p.clone() }).collect()
    }
}

impl Carry {
    /// Attached parts only.
    pub fn pick_up(draft: &Draft, sdf: &Sdf, lens: &Lens, id: PartId) -> Option<Carry> {
        let part = *draft.part(id)?;
        if !matches!(part.placement?.mount, Mount::Attached { .. }) {
            return None;
        }
        let subtree: BTreeSet<PartId> = draft.subtree(id).iter().map(|p| p.id).collect();
        let (_, own) = crate::form_handles::original(sdf, id)?;
        let about = own.pose;
        let inverse = Pose { position: about.rotation.transpose() * -about.position, rotation: about.rotation.transpose() };
        let pieces = sdf.pieces().iter().filter(|p| subtree.contains(&p.part)).map(|p| Piece { pose: inverse.then(&p.pose), ..p.clone() }).collect();
        let depth_m = lens.project(about.position).map(|(_, depth)| depth);
        let placement = part.placement?;
        let Mount::Attached { anchor, .. } = placement.mount else { return None };
        let (_, host) = original(sdf, placement.parent)?;
        let normal = host.pose.rotation * host.shape.exit(anchor.normalize_or(DVec3::X)).normal;
        let orient = Orient::of(&placement, about.rotation, normal);
        let float = Float { pieces, rotation: about.rotation, depth_m };
        Some(Carry { was: Some(part), part, hung: true, subtree, float, orient, guide: None })
    }

    pub fn new_part(draft: &Draft, kind: Kind, primitive: Primitive, balance: &Balance) -> Option<Carry> {
        let mind = draft.form.parts.iter().find(|p| p.kind == Kind::Mind)?.id;
        let part = *draft.add(mind, kind, primitive, DVec3::X, balance).ok()?.after.first()?;
        let piece = Piece { part: part.id, side: Side::Original, kind, shape: part.shape(balance.min_part_m3), pose: Pose::IDENTITY };
        let float = Float { pieces: vec![piece], rotation: DMat3::IDENTITY, depth_m: None };
        let orient = Orient::Standing { twist: 0.0, tilt: DVec2::ZERO };
        Some(Carry { was: None, part, hung: false, subtree: BTreeSet::from([part.id]), float, orient, guide: None })
    }

    /// Space: on to the next of [`Orient::next`].
    pub fn turn(&mut self) {
        self.orient = self.orient.next();
    }

    /// Hangs it from the part under `at`, or floats it. The edit, if anything changed.
    pub fn follow(&mut self, draft: &Draft, sdf: &Sdf, lens: &Lens, at: Vec2, keys: Modifiers, balance: &Balance) -> Option<Edit> {
        let (origin, direction) = lens.ray(at);
        let Some((index, point)) = hit_except(sdf, origin, direction, 10.0 * lens.extent.size_m(), &self.subtree) else {
            return self.unhang();
        };
        let hit = &sdf.pieces()[index];
        // An anchor names a point on the original, and a mirror's copy is its reflection.
        let flip = |v: DVec3| if hit.side == Side::Mirror { v * DVec3::new(1.0, -1.0, 1.0) } else { v };
        let (_, host) = original(sdf, hit.part)?;
        let mut target = host.pose.to_local(flip(point));
        let normal = host.shape.exit(target.normalize_or(DVec3::X)).normal;
        self.guide = keys.align.then(|| Guide { host: hit.part, axes: snap::across(normal) });
        if keys.align {
            target = snap::midline(target, normal, host.shape.extent(DMat3::IDENTITY, 0.0));
        }
        let unturned = match self.was {
            Some(was) => {
                let placement = was.placement?;
                Part { placement: Some(Placement { parent: hit.part, ..placement }), ..was }
            }
            // Sized against what it hangs from.
            None => Part { id: self.part.id, ..*draft.add(hit.part, self.part.kind, self.part.primitive, target, balance).ok()?.after.first()? },
        };
        let heading = match self.orient {
            Orient::Lying { axis, .. } => host.pose.rotation.transpose() * flip(axis),
            Orient::Standing { .. } => DVec3::ZERO,
        };
        let seat = Seat::on(host, target, &unturned.shape(balance.min_part_m3), self.orient, heading);
        let mut placement = unturned.placement?;
        let Mount::Attached { standoff, .. } = placement.mount else { return None };
        placement.mount = Mount::Attached { anchor: seat.anchor, standoff: seat.standoff.unwrap_or(standoff) };
        placement.twist = seat.twist;
        placement.tilt = seat.tilt;
        let part = Part { placement: Some(placement), ..unturned };
        if self.hung && part == self.part {
            return None;
        }
        self.part = part;
        self.hung = true;
        Some(self.edit(false))
    }

    /// Floats it, taking it off whatever it hung from.
    fn unhang(&mut self) -> Option<Edit> {
        self.guide = None;
        if !self.hung {
            return None;
        }
        // Before it stops being hung, which is what says a new part is in the draft to take out.
        let back = self.cancel();
        self.hung = false;
        if let Some(was) = self.was {
            self.part = was;
        }
        back
    }

    fn edit(&self, settled: bool) -> Edit {
        let (what, before) = match self.was {
            Some(was) => (What::Move, vec![was]),
            None => (What::Add, Vec::new()),
        };
        Edit { what, part: self.part.id, before, after: vec![self.part], settled }
    }

    /// `None` when there is nothing to record, or it floats.
    pub fn put_down(&self) -> Option<Edit> {
        match self.was {
            _ if !self.hung => None,
            Some(was) if was == self.part => None,
            _ => Some(self.edit(true)),
        }
    }

    /// Unsettled, so nothing is recorded.
    pub fn cancel(&self) -> Option<Edit> {
        match self.was {
            Some(was) if was != self.part => {
                Some(Edit { what: What::Move, part: was.id, before: vec![self.part], after: vec![was], settled: false })
            }
            None if self.hung => {
                Some(Edit { what: What::Remove, part: self.part.id, before: vec![self.part], after: Vec::new(), settled: false })
            }
            _ => None,
        }
    }

    /// The removal records a picked-up part as it was before the carry, so undo puts it back
    /// there.
    pub fn discard(&self, draft: &Draft) -> Vec<Action> {
        match self.was {
            Some(was) => {
                let removal = draft.remove(self.part.id).map(|mut removal| {
                    for part in removal.before.iter_mut().filter(|p| p.id == was.id) {
                        *part = was;
                    }
                    removal
                });
                vec![Action::EditForm(removal)]
            }
            None => self.cancel().map(|out| Action::EditForm(Ok(out))).into_iter().collect(),
        }
    }
}


#[allow(clippy::too_many_arguments)]
pub fn carry(
    ui: Res<Ui>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    controls: em_ui::Controls,
    zones: Query<(Entity, &Interaction), With<DropZone>>,
    children: Query<&Children>,
    inside: Query<&Interaction, Without<DropZone>>,
    window: Single<&Window, With<PrimaryWindow>>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    mut carried: ResMut<Carried>,
    grabbed: Res<crate::form_handles::Grabbed>,
    typing: em_ui::Typing,
    // Where the pointer was and what was held when it last followed.
    mut seen: Local<(Option<Vec2>, Option<Modifiers>)>,
    mut out: MessageWriter<Requested>,
) {
    let balance = Balance::DEFAULT;
    if grabbed.is_holding() {
        return;
    }
    let (Some(lens), Some(sdf), Some(draft)) = (lens(&ui, &shown, &surface), shown.sdf(), ui.form.draft.as_ref()) else {
        return;
    };
    let cursor = window.cursor_position();
    carried.1 = false;
    if buttons.just_pressed(MouseButton::Left) && !egui.wants_any_pointer_input() {
        if let Some(carry) = carried.0.clone() {
            carried.1 = true;
            // A button holds the pointer rather than the list under it.
            let over_list = zones.iter().any(|(zone, own)| {
                *own != Interaction::None || children.iter_descendants(zone).flat_map(|b| inside.get(b)).any(|i| *i != Interaction::None)
            });
            let actions: Vec<Action> = match (over_list, carry.hung) {
                (true, _) => carry.discard(draft),
                (false, true) => carry.put_down().map(|edit| Action::EditForm(Ok(edit))).into_iter().collect(),
                (false, false) => return,
            };
            out.write_batch(actions.into_iter().map(Requested));
            carried.0 = None;
            return;
        }
        let Some(at) = cursor.filter(|at| !controls.under_pointer() && crate::form_view::on_picture(&surface, *at)) else { return };
        let Some(id) = pick_part(sdf, &lens, at) else { return };
        out.write(Requested(Action::SelectPart(Some(id))));
        carried.0 = Carry::pick_up(draft, sdf, &lens, id);
        seen.0 = Some(at);
        return;
    }
    let Some(carry) = carried.0.as_mut() else { return };
    let modifiers = Modifiers::of(&keys);
    let turned = keys.just_pressed(KeyCode::Space) && !typing.active();
    if turned {
        carry.turn();
    }
    let Some(at) = cursor.filter(|at| turned || Some(*at) != seen.0 || Some(modifiers) != seen.1) else { return };
    *seen = (Some(at), Some(modifiers));
    if controls.under_pointer() || !crate::form_view::on_picture(&surface, at) {
        return;
    }
    if let Some(edit) = carry.follow(draft, sdf, &lens, at, modifiers, &balance) {
        out.write(Requested(Action::EditForm(Ok(edit))));
    }
}

/// Before the key bindings, which would otherwise take Escape to close a window or the editor.
pub fn cancel_on_escape(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    typing: em_ui::Typing,
    mut carried: ResMut<Carried>,
    mut out: MessageWriter<Requested>,
) {
    if typing.active() || !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    let Some(carry) = carried.0.take() else { return };
    keys.clear_just_pressed(KeyCode::Escape);
    if let Some(back) = carry.cancel() {
        out.write(Requested(Action::EditForm(Ok(back))));
    }
}

pub fn drop_on_leaving(ui: Res<Ui>, mut carried: ResMut<Carried>, mut out: MessageWriter<Requested>) {
    if ui.view == crate::ui::ViewMode::Form {
        return;
    }
    let Some(carry) = carried.0.take() else { return };
    if let Some(back) = carry.cancel() {
        out.write(Requested(Action::EditForm(Ok(back))));
    }
}

#[derive(Component)]
pub struct FloatRoot;

/// Which of the float's pieces, and its mesh's scale.
#[derive(Component)]
pub struct Floating(usize, Vec3);


#[allow(clippy::too_many_arguments)]
pub fn draw_float(
    mut commands: Commands,
    ui: Res<Ui>,
    carried: Res<Carried>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut drawn: Query<(&crate::form_view::DrawnPart, &mut Visibility)>,
    mut floating: Query<(Entity, &mut Transform, &Floating)>,
    roots: Query<Entity, With<FloatRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<em_render::body_surface_material::BodySurfaceMaterial>>,
    surfaces: Res<crate::surfaces::Surfaces>,
) {
    let float = carried.carry().filter(|c| !c.hung && ui.view == crate::ui::ViewMode::Form);
    let hidden = float.filter(|c| c.was.is_some()).map(|c| &c.subtree);
    for (part, mut visibility) in &mut drawn {
        let wanted = if hidden.is_some_and(|h| h.contains(&part.0)) { Visibility::Hidden } else { Visibility::Inherited };
        visibility.set_if_neq(wanted);
    }
    let placed = float.zip(lens(&ui, &shown, &surface)).zip(window.cursor_position()).map(|((c, lens), at)| c.float.at(&lens, at));
    let Some(pieces) = placed else {
        for entity in &roots {
            commands.entity(entity).despawn();
        }
        return;
    };
    if floating.iter().count() == pieces.len() {
        for (_, mut transform, floating) in &mut floating {
            if let Some(piece) = pieces.get(floating.0) {
                transform.set_if_neq(crate::parts::local(piece, floating.1));
            }
        }
        return;
    }
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z), 0.0);
    let root = commands.spawn((Transform::from_rotation(turn), Visibility::default(), FloatRoot)).id();
    for (i, piece) in pieces.iter().enumerate() {
        let (mesh, scale) = crate::parts::solid(&piece.shape);
        let paint = crate::parts::paint(piece.kind);
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(surfaces.flat.material(crate::form_view::hangar(paint, DVec3::Z)))),
            crate::parts::local(piece, scale),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::camera::visibility::RenderLayers::layer(crate::form_view::FORM_LAYER),
            crate::form_view::HangarPaint(paint),
            Floating(i, scale),
            ChildOf(root),
        ));
    }
}

pub fn put_away(mut commands: Commands, mut carried: ResMut<Carried>, roots: Query<Entity, With<FloatRoot>>) {
    carried.0 = None;
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::Form;

    use super::*;
    use crate::form_view::{Extent, FormOrbit};

    const B: Balance = Balance::DEFAULT;

    fn scene(draft: &Draft) -> (Sdf, Lens) {
        let sdf = Sdf::new(&draft.form, &B).unwrap();
        let (min, max) = sdf.bounds();
        let extent = Extent { min, max };
        let rect = bevy_egui::egui::Rect::from_min_size(bevy_egui::egui::pos2(0.0, 60.0), bevy_egui::egui::vec2(1280.0, 660.0));
        (sdf, Lens { orbit: FormOrbit::default().held_to(&extent), extent, rect })
    }

    fn apply(draft: &mut Draft, edit: &Edit) {
        draft.apply(edit, &B).unwrap();
    }

    /// On the storage's top, under the deck, which a carried deck looks through.
    fn top(lens: &Lens) -> Vec2 {
        lens.project(DVec3::new(0.0, 0.0, 33.0)).unwrap().0
    }

    /// The starting form's storage top, toward the bow and port of the habitat on it.
    fn on_deck(x: f64, y: f64) -> DVec3 {
        let Some(lc_world::form::primitive::Shape::Slab { edges, .. }) = Form::starting().parts.iter().find(|p| p.id == PartId(1)).map(|p| p.shape(B.min_part_m3)) else { panic!() };
        DVec3::new(x, y, edges.z / 2.0)
    }

    fn placed(draft: &Draft, id: PartId) -> Piece {
        let sdf = Sdf::new(&draft.form, &B).unwrap();
        original(&sdf, id).unwrap().1.clone()
    }

    #[test]
    fn a_part_goes_where_the_pointer_meets_its_parent_and_keeps_its_heading() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let data = PartId(5);
        let before = placed(&draft, data);
        let mut carry = Carry::pick_up(&draft, &sdf, &lens, data).unwrap();
        assert!(matches!(carry.orient, Orient::Lying { .. }), "slung fore and aft under the keel");
        let point = on_deck(150.0, 60.0);
        let at = lens.project(point).unwrap().0;
        let edit = carry.follow(&draft, &sdf, &lens, at, Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        assert_eq!(draft.part(data).unwrap().placement.unwrap().parent, PartId(1));
        let after = placed(&draft, data);
        let center = after.pose.position;
        assert!((center.x - point.x).abs() < 2.0 && (center.y - point.y).abs() < 2.0, "centered on the pointer: {center}");
        assert!(center.z > point.z, "on top of the deck: {center}");
        let (was, now) = (before.pose.rotation.x_axis, after.pose.rotation.x_axis);
        assert!(now.dot(was) > 1.0 - 1e-9, "still fore and aft the same way: {now} against {was}");
    }

    #[test]
    fn shift_holds_a_carried_part_to_its_parents_midline() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let data = PartId(5);
        let mut carry = Carry::pick_up(&draft, &sdf, &lens, data).unwrap();
        let at = lens.project(on_deck(150.0, 60.0)).unwrap().0;
        let shift = Modifiers { align: true, ..Modifiers::default() };
        let edit = carry.follow(&draft, &sdf, &lens, at, shift, &B).unwrap();
        assert_eq!(carry.guide, Some(Guide { host: PartId(1), axes: [0, 1] }));
        apply(&mut draft, &edit);
        let center = placed(&draft, data).pose.position;
        assert!(center.y.abs() < 1e-6 && (center.x - 150.0).abs() < 2.0, "on the keel line: {center}");
        let edit = carry.follow(&draft, &sdf, &lens, at, Modifiers::default(), &B).unwrap();
        assert_eq!(carry.guide, None);
        apply(&mut draft, &edit);
        assert!((placed(&draft, data).pose.position.y - 60.0).abs() < 2.0, "and free again");
    }

    #[test]
    fn space_lays_a_part_down_on_its_side_fore_and_aft_then_across() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let at = lens.project(on_deck(150.0, 60.0)).unwrap().0;
        // Far enough toward the bow that lying aft its foot is over the edge.
        let mut carry = Carry::new_part(&draft, Kind::Engine, crate::draft::PRIMITIVES[3], &B).unwrap();
        let lay = |carry: &mut Carry, draft: &mut Draft| {
            carry.turn();
            let edit = carry.follow(draft, &sdf, &lens, at, Modifiers::default(), &B).unwrap();
            apply(draft, &edit);
            let piece = placed(draft, carry.part.id);
            apply(draft, &carry.cancel().unwrap());
            piece
        };
        for axis in [DVec3::X, DVec3::NEG_X, DVec3::Y] {
            let piece = lay(&mut carry, &mut draft);
            assert!(piece.pose.rotation.x_axis.dot(axis) > 1.0 - 1e-9, "along {axis}: {}", piece.pose.rotation.x_axis);
            let low = piece.pose.position.z - piece.shape.extent(piece.pose.rotation, 0.0).z;
            assert!((low - on_deck(0.0, 0.0).z).abs() < 1e-6, "resting on the deck: {low}");
        }
        let piece = lay(&mut carry, &mut draft);
        assert!(piece.pose.rotation.x_axis.z > 1.0 - 1e-9, "and standing again");
    }

    #[test]
    fn a_part_picked_up_and_put_down_is_one_move() {
        let mut draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let data = PartId(5);
        let mut carry = Carry::pick_up(&draft, &sdf, &lens, data).unwrap();
        let was = *draft.part(data).unwrap();
        let edit = carry.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        assert!(!edit.settled);
        apply(&mut draft, &edit);
        let placement = draft.part(data).unwrap().placement.unwrap();
        assert_eq!(placement.parent, PartId(4));
        let Mount::Attached { anchor, standoff } = placement.mount else { panic!() };
        assert!(anchor.z > 0.0 && (anchor.length() - 1.0).abs() < 1e-12, "on the deck's top: {anchor}");
        assert_eq!(standoff, -0.3, "the standoff comes along");
        let down = carry.put_down().unwrap();
        assert!(down.settled && down.what == What::Move);
        assert_eq!(down.before, vec![was], "measured from where it was picked up");
    }

    #[test]
    fn a_carried_part_hangs_from_the_part_under_the_pointer_but_not_from_itself() {
        let draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let mut deck = Carry::pick_up(&draft, &sdf, &lens, PartId(4)).unwrap();
        // It looks through itself to the storage, where it already hangs.
        let through = deck.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B);
        assert!(through.is_none_or(|e| e.after[0].placement.unwrap().parent == PartId(1)));
        let engine = lens.project(sdf.pieces().iter().find(|p| p.part == PartId(2)).unwrap().pose.position).unwrap().0;
        let edit = deck.follow(&draft, &sdf, &lens, engine, Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(2), "onto the engine");
        let edit = deck.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(1), "and back");
        assert!(Carry::pick_up(&draft, &sdf, &lens, PartId(0)).is_none() && Carry::pick_up(&draft, &sdf, &lens, PartId(1)).is_none(), "the Mind and an enclosing part stay put");
    }

    #[test]
    fn a_part_taken_from_the_list_is_added_where_it_is_put_down() {
        let mut draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let mut carry = Carry::new_part(&draft, Kind::Bay, crate::draft::PRIMITIVES[3], &B).unwrap();
        assert_eq!(carry.put_down(), None, "not hung anywhere, so nothing to add");
        assert!(carry.discard(&draft).is_empty());
        let edit = carry.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        assert_eq!(draft.part(carry.part.id).unwrap().kind, Kind::Bay);
        let down = carry.put_down().unwrap();
        assert!(down.settled && down.what == What::Add && down.before.is_empty());
        let [Action::EditForm(Ok(out))] = &carry.discard(&draft)[..] else { panic!() };
        assert!(!out.settled);
        apply(&mut draft, out);
        assert_eq!(draft.form, draft.ship);
    }


    #[test]
    fn nothing_hangs_from_a_dismantled_part() {
        let mut draft = Draft::new(Form::reference());
        let (ship, lens) = scene(&draft);
        let nose = ship.pieces().iter().find(|p| p.part == PartId(5)).unwrap().pose.to_outer(DVec3::X * 40.0);
        let at = lens.project(nose).unwrap().0;
        let mut before = Carry::new_part(&draft, Kind::Living, crate::draft::PRIMITIVES[0], &B).unwrap();
        let edit = before.follow(&draft, &ship, &lens, at, Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(5), "while it is there, it is hung from");
        let removal = draft.remove(PartId(5)).unwrap();
        apply(&mut draft, &removal);
        let (drawn, _) = scene(&draft);
        let mut after = Carry::new_part(&draft, Kind::Living, crate::draft::PRIMITIVES[0], &B).unwrap();
        let parent = after.follow(&draft, &drawn, &lens, at, Modifiers::default(), &B).map(|e| e.after[0].placement.unwrap().parent);
        assert_ne!(parent, Some(PartId(5)));
    }


    #[test]
    fn off_every_part_it_floats_at_the_pointer() {
        let mut draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let mut deck = Carry::pick_up(&draft, &sdf, &lens, PartId(4)).unwrap();
        let engine = lens.project(sdf.pieces().iter().find(|p| p.part == PartId(2)).unwrap().pose.position).unwrap().0;
        let edit = deck.follow(&draft, &sdf, &lens, engine, Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        assert_ne!(draft.form, draft.ship);
        let empty = Vec2::new(40.0, 120.0);
        let back = deck.follow(&draft, &sdf, &lens, empty, Modifiers::default(), &B).unwrap();
        apply(&mut draft, &back);
        assert_eq!(draft.form, draft.ship, "floating, the draft is as it was");
        assert!(!deck.hung && deck.put_down().is_none(), "and a click puts nothing down");
        assert!(deck.follow(&draft, &sdf, &lens, empty + Vec2::X, Modifiers::default(), &B).is_none());

        let taken = lens.project(sdf.pieces().iter().find(|p| p.part == PartId(4)).unwrap().pose.position).unwrap().1;
        let floating = deck.float.at(&lens, empty);
        let own = floating.iter().find(|p| p.part == PartId(4)).unwrap();
        let (at, depth) = lens.project(own.pose.position).unwrap();
        assert!((at - empty).length() < 0.5, "its middle is on the pointer: {at}");
        assert!((depth - taken).abs() < 1e-6 * taken, "at the depth it was taken from");
        let before = sdf.pieces().iter().find(|p| p.part == PartId(4)).unwrap().pose.rotation;
        assert!((own.pose.rotation - before).abs_diff_eq(glam::DMat3::ZERO, 1e-9), "turned as it was");
    }

    #[test]
    fn a_cancel_puts_the_draft_back() {
        let mut draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let mut data = Carry::pick_up(&draft, &sdf, &lens, PartId(5)).unwrap();
        assert_eq!(data.cancel(), None, "not moved, nothing to put back");
        let edit = data.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        let back = data.cancel().unwrap();
        assert!(!back.settled);
        apply(&mut draft, &back);
        assert_eq!(draft.form, draft.ship);
        let mut bay = Carry::new_part(&draft, Kind::Bay, crate::draft::PRIMITIVES[3], &B).unwrap();
        let edit = bay.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        apply(&mut draft, &bay.cancel().unwrap());
        assert_eq!(draft.form, draft.ship);
    }


    #[test]
    fn a_part_dropped_on_the_list_is_deleted_as_it_was() {
        let mut draft = Draft::new(Form::reference());
        let (sdf, lens) = scene(&draft);
        let mut carry = Carry::pick_up(&draft, &sdf, &lens, PartId(5)).unwrap();
        let edit = carry.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        let [Action::EditForm(Ok(removal))] = &carry.discard(&draft)[..] else { panic!() };
        assert!(removal.settled && removal.what == What::Remove);
        apply(&mut draft, removal);
        assert!(draft.part(PartId(5)).is_none());
        apply(&mut draft, &removal.inverse());
        assert_eq!(draft.form, draft.ship);
        let drones = Carry::pick_up(&draft, &sdf, &lens, PartId(3)).unwrap();
        let [Action::EditForm(Ok(removal))] = &drones.discard(&draft)[..] else { panic!() };
        apply(&mut draft, removal);
        assert!(draft.form.parts.iter().all(|p| p.kind != Kind::Drone));
    }
}
