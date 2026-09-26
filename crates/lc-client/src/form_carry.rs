//! Carrying a part, as a spaceplane hangar does: a click picks up an attached part, it hangs from
//! whatever part the pointer meets, and the next click puts it down. New parts come off the
//! palette the same way, and a drop on the palette deletes. Each move is an unsettled edit from
//! where it was picked up, so the whole carry is one entry in the history.

use std::collections::BTreeSet;

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use glam::{DMat3, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::place::{Pose, Side};
use lc_world::form::sdf::{Piece, Sdf};
use lc_world::form::{Kind, Mount, Part, PartId, Placement, Primitive};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Draft, Edit, What};
use crate::form_handles::{Lens, Modifiers, hit_except, lens, pick_part};
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
        Some(Carry { was: Some(part), part, hung: true, subtree, float: Float { pieces, rotation: about.rotation, depth_m } })
    }

    pub fn new_part(draft: &Draft, kind: Kind, primitive: Primitive, balance: &Balance) -> Option<Carry> {
        let mind = draft.form.parts.iter().find(|p| p.kind == Kind::Mind)?.id;
        let part = *draft.add(mind, kind, primitive, DVec3::X, balance).ok()?.after.first()?;
        let piece = Piece { part: part.id, side: Side::Original, kind, shape: part.shape(balance.min_part_m3), pose: Pose::IDENTITY };
        let float = Float { pieces: vec![piece], rotation: DMat3::IDENTITY, depth_m: None };
        Some(Carry { was: None, part, hung: false, subtree: BTreeSet::from([part.id]), float })
    }

    /// Hangs it from the part under `at`, or floats it. The edit, if anything changed.
    pub fn follow(&mut self, draft: &Draft, sdf: &Sdf, lens: &Lens, at: Vec2, keys: Modifiers, balance: &Balance) -> Option<Edit> {
        let (origin, direction) = lens.ray(at);
        let Some((index, point)) = hit_except(sdf, origin, direction, 10.0 * lens.extent.size_m(), &self.subtree) else {
            return self.unhang();
        };
        let piece = &sdf.pieces()[index];
        let anchor = snap::anchor(piece.pose.to_local(point), keys.fine);
        let part = match self.was {
            Some(was) => {
                let placement = was.placement?;
                let Mount::Attached { standoff, .. } = placement.mount else { return None };
                let mount = Mount::Attached { anchor, standoff };
                Part { placement: Some(Placement { parent: piece.part, mount, ..placement }), ..was }
            }
            // Sized against what it hangs from.
            None => Part { id: self.part.id, ..*draft.add(piece.part, self.part.kind, self.part.primitive, anchor, balance).ok()?.after.first()? },
        };
        if self.hung && part == self.part {
            return None;
        }
        self.part = part;
        self.hung = true;
        Some(self.edit(false))
    }

    /// Floats it, taking it off whatever it hung from.
    pub fn unhang(&mut self) -> Option<Edit> {
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
    game: Res<crate::app::Game>,
    mut seen: Local<Option<Vec2>>,
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
        *seen = Some(at);
        return;
    }
    let Some(carry) = carried.0.as_mut() else { return };
    let Some(at) = cursor.filter(|at| Some(*at) != *seen) else { return };
    let start = crate::preview::Start::of(&game.0);
    *seen = Some(at);
    if controls.under_pointer() || !crate::form_view::on_picture(&surface, at) {
        return;
    }
    if let Some(edit) = carry.follow(draft, sdf, &lens, at, Modifiers::of(&keys), &balance) {
        // Sized against what it hangs from, so a part may be paid for on one parent and not another.
        let edit = match start.as_ref().is_none_or(|s| s.allows(draft, &edit)) {
            true => Some(edit),
            false => carry.unhang(),
        };
        out.write_batch(edit.map(|edit| Requested(Action::EditForm(Ok(edit)))));
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
    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z));
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

    #[test]
    fn a_part_picked_up_and_put_down_is_one_move() {
        let mut draft = Draft::new(Form::starting());
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
        assert!(anchor.z > 0.0 && anchor == snap::anchor(anchor, false), "on the deck's top: {anchor}");
        assert_eq!(standoff, -0.3, "the standoff comes along");
        let down = carry.put_down().unwrap();
        assert!(down.settled && down.what == What::Move);
        assert_eq!(down.before, vec![was], "measured from where it was picked up");
    }

    #[test]
    fn a_carried_part_hangs_from_the_part_under_the_pointer_but_not_from_itself() {
        let draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let mut deck = Carry::pick_up(&draft, &sdf, &lens, PartId(4)).unwrap();
        // It looks through itself to the storage, where it already hangs.
        assert!(deck.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).is_none());
        let engine = lens.project(sdf.pieces().iter().find(|p| p.part == PartId(2)).unwrap().pose.position).unwrap().0;
        let edit = deck.follow(&draft, &sdf, &lens, engine, Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(2), "onto the engine");
        let edit = deck.follow(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(1), "and back");
        assert!(Carry::pick_up(&draft, &sdf, &lens, PartId(0)).is_none() && Carry::pick_up(&draft, &sdf, &lens, PartId(1)).is_none(), "the Mind and an enclosing part stay put");
    }

    #[test]
    fn a_part_taken_from_the_list_is_added_where_it_is_put_down() {
        let mut draft = Draft::new(Form::starting());
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
        let mut draft = Draft::new(Form::starting());
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
        let mut draft = Draft::new(Form::starting());
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
        let mut draft = Draft::new(Form::starting());
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
        let mut draft = Draft::new(Form::starting());
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
