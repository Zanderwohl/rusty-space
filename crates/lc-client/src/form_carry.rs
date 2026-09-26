//! Carrying a part, as a spaceplane hangar does. A click on an attached part picks it up; it hangs
//! wherever the pointer meets another part, re-parenting to it, and the next click puts it down. A
//! part taken from the list of kinds is carried the same way, and anything dropped on that list
//! is deleted.
//!
//! While carried, each move is an unsettled edit measured from where the part was picked up, and
//! putting it down sends it settled, so the whole carry is one entry in the history.

use std::collections::BTreeSet;

use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use glam::DVec3;
use lc_world::fitting::Balance;
use lc_world::form::sdf::Sdf;
use lc_world::form::{Kind, Mount, Part, PartId, Placement, Primitive};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Draft, Edit, What};
use crate::form_handles::{Lens, Modifiers, hit_except, lens, pick_part};
use crate::form_view::{FormSurface, Shown};
use crate::input::Requested;
use crate::snap;

/// The part on the pointer, if any.
#[derive(Resource, Default)]
pub struct Carried(Option<Carry>, bool);

impl Carried {
    pub fn is_carrying(&self) -> bool {
        self.0.is_some()
    }

    /// Whether this frame's press put a part down. The press is the drop's, so a button under it
    /// is not pressed: a part dropped on the list is deleted, not traded for a new one.
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

/// Where a drop on it deletes what is carried: the list new parts are taken from, buttons and all.
#[derive(Component)]
pub struct DropZone;

#[derive(Clone, Debug, PartialEq)]
pub struct Carry {
    /// As it was when picked up, or `None` for a part taken from the list.
    pub was: Option<Part>,
    /// As it last hung. A new part's placement means nothing until it has hung anywhere.
    pub part: Part,
    pub hung: bool,
    /// The part and everything under it, which it cannot hang from.
    pub subtree: BTreeSet<PartId>,
}

impl Carry {
    /// An attached part, as it is. The Mind and an enclosing part stay where they are.
    pub fn pick_up(draft: &Draft, id: PartId) -> Option<Carry> {
        let part = *draft.part(id)?;
        matches!(part.placement?.mount, Mount::Attached { .. }).then(|| Carry {
            was: Some(part),
            part,
            hung: true,
            subtree: draft.subtree(id).iter().map(|p| p.id).collect(),
        })
    }

    /// A new part of `kind`, not yet hung anywhere.
    pub fn new_part(draft: &Draft, kind: Kind, primitive: Primitive, balance: &Balance) -> Option<Carry> {
        let mind = draft.form.parts.iter().find(|p| p.kind == Kind::Mind)?.id;
        let part = *draft.add(mind, kind, primitive, DVec3::X, balance).ok()?.after.first()?;
        Some(Carry { was: None, part, hung: false, subtree: BTreeSet::from([part.id]) })
    }

    /// Hung from whatever part is under `at`, other than itself and what hangs from it, and the
    /// edit that shows it there. `None` over empty space, where it stays as it last hung.
    pub fn hang(&mut self, draft: &Draft, sdf: &Sdf, lens: &Lens, at: Vec2, keys: Modifiers, balance: &Balance) -> Option<Edit> {
        let (origin, direction) = lens.ray(at);
        let (index, point) = hit_except(sdf, origin, direction, 10.0 * lens.extent.size_m(), &self.subtree)?;
        let piece = &sdf.pieces()[index];
        let anchor = snap::anchor(piece.pose.to_local(point), keys.fine);
        let part = match self.was {
            Some(was) => {
                let placement = was.placement?;
                let standoff = match placement.mount {
                    Mount::Attached { standoff, .. } => standoff,
                    Mount::Enclosing => return None,
                };
                let mount = Mount::Attached { anchor, standoff };
                Part { placement: Some(Placement { parent: piece.part, mount, ..placement }), ..was }
            }
            // Sized against what it hangs from, so it arrives in proportion wherever it goes.
            None => Part { id: self.part.id, ..*draft.add(piece.part, self.part.kind, self.part.primitive, anchor, balance).ok()?.after.first()? },
        };
        if self.hung && part == self.part {
            return None;
        }
        self.part = part;
        self.hung = true;
        Some(self.edit(false))
    }

    fn edit(&self, settled: bool) -> Edit {
        let (what, before) = match self.was {
            Some(was) => (What::Move, vec![was]),
            None => (What::Add, Vec::new()),
        };
        Edit { what, part: self.part.id, before, after: vec![self.part], settled }
    }

    /// Put down where it last hung: the settled edit, or `None` when there is nothing to record.
    pub fn put_down(&self) -> Option<Edit> {
        match self.was {
            Some(was) if was == self.part => None,
            None if !self.hung => None,
            _ => Some(self.edit(true)),
        }
    }

    /// Dropped on the list. A part picked up is deleted with its subtree, the removal recording
    /// it as it was before it was picked up; a new one is taken back out, with nothing recorded.
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
            None if self.hung => {
                let out = Edit { what: What::Remove, part: self.part.id, before: vec![self.part], after: Vec::new(), settled: false };
                vec![Action::EditForm(Ok(out))]
            }
            None => Vec::new(),
        }
    }
}

/// A click on the picture while carrying puts the part down, or deletes it over the list. A click
/// on a part otherwise selects it and picks it up. Between clicks the carried part follows the
/// pointer.
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
    mut seen: Local<Option<Vec2>>,
    mut out: MessageWriter<Requested>,
) {
    let balance = Balance::DEFAULT;
    let (Some(lens), Some(sdf), Some(draft)) = (lens(&ui, &shown, &surface), shown.sdf(), ui.form.draft.as_ref()) else {
        return;
    };
    let cursor = window.cursor_position();
    carried.1 = false;
    if buttons.just_pressed(MouseButton::Left) && !egui.wants_any_pointer_input() {
        if let Some(carry) = carried.0.take() {
            carried.1 = true;
            // A button in the list holds the pointer rather than the list, so over either.
            let over_list = zones.iter().any(|(zone, own)| {
                *own != Interaction::None || children.iter_descendants(zone).flat_map(|b| inside.get(b)).any(|i| *i != Interaction::None)
            });
            let actions = match over_list {
                true => carry.discard(draft),
                false => carry.put_down().map(|edit| Action::EditForm(Ok(edit))).into_iter().collect(),
            };
            out.write_batch(actions.into_iter().map(Requested));
            return;
        }
        let Some(at) = cursor.filter(|at| !controls.under_pointer() && crate::form_view::on_picture(&surface, *at)) else { return };
        let Some(id) = pick_part(sdf, &lens, at) else { return };
        out.write(Requested(Action::SelectPart(Some(id))));
        carried.0 = Carry::pick_up(draft, id);
        *seen = Some(at);
        return;
    }
    let Some(carry) = carried.0.as_mut() else { return };
    let Some(at) = cursor.filter(|at| Some(*at) != *seen) else { return };
    *seen = Some(at);
    if controls.under_pointer() || !crate::form_view::on_picture(&surface, at) {
        return;
    }
    if let Some(edit) = carry.hang(draft, sdf, &lens, at, Modifiers::of(&keys), &balance) {
        out.write(Requested(Action::EditForm(Ok(edit))));
    }
}

/// The words on the pointer while something is carried.
#[derive(Component)]
pub struct Tag;

pub fn show_tag(
    mut commands: Commands,
    ui: Res<Ui>,
    carried: Res<Carried>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    mut tags: Query<(Entity, &mut Node, &mut Text), With<Tag>>,
) {
    let wanted = carried.carry().filter(|_| ui.view == crate::ui::ViewMode::Form).zip(window.cursor_position());
    let Some((carry, at)) = wanted else {
        for (entity, ..) in &tags {
            commands.entity(entity).despawn();
        }
        return;
    };
    let what = format!("{}, {}", crate::draft::kind_name(carry.part.kind), crate::draft::primitive_name(&carry.part.primitive));
    let said = match carry.hung {
        true => format!("{what}: click to put down, or drop on the list to delete"),
        false => format!("{what}: point at a part to hang it there"),
    };
    let (left, top) = (Val::Px(at.x + 16.0), Val::Px(at.y + 12.0));
    if let Some((_, mut node, mut text)) = tags.iter_mut().next() {
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
        if text.0 != said {
            text.0 = said;
        }
        return;
    }
    commands.spawn((
        Text::new(said),
        TextFont { font: FontSource::Handle(assets.load(crate::faces::UI_FILE)), font_size: FontSize::Px(13.0), ..default() },
        TextColor(em_ui::vfd::TEXT),
        Node { position_type: PositionType::Absolute, left, top, ..default() },
        GlobalZIndex(em_ui::widgets::OVERLAY_Z),
        Pickable::IGNORE,
        Tag,
    ));
}

/// Out of the editor, nothing is carried: a part picked up goes back where it was.
pub fn drop_on_leaving(ui: Res<Ui>, mut carried: ResMut<Carried>, mut out: MessageWriter<Requested>) {
    if ui.view == crate::ui::ViewMode::Form {
        return;
    }
    let Some(carry) = carried.0.take() else { return };
    let restore = match carry.was {
        Some(was) => Edit { what: What::Move, part: was.id, before: vec![carry.part], after: vec![was], settled: false },
        None => Edit { what: What::Remove, part: carry.part.id, before: vec![carry.part], after: Vec::new(), settled: false },
    };
    out.write(Requested(Action::EditForm(Ok(restore))));
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

    /// **Picked up, hung somewhere else, put down: one settled move** from where it was.
    #[test]
    fn a_part_picked_up_and_put_down_is_one_move() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let data = PartId(5);
        let mut carry = Carry::pick_up(&draft, data).unwrap();
        let was = *draft.part(data).unwrap();
        let edit = carry.hang(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        assert!(!edit.settled);
        apply(&mut draft, &edit);
        // Onto the deck, which is what is drawn there.
        let placement = draft.part(data).unwrap().placement.unwrap();
        assert_eq!(placement.parent, PartId(4));
        let Mount::Attached { anchor, standoff } = placement.mount else { panic!() };
        assert!(anchor.z > 0.0 && anchor == snap::anchor(anchor, false), "on the deck's top: {anchor}");
        assert_eq!(standoff, -0.3, "the standoff comes along");
        let down = carry.put_down().unwrap();
        assert!(down.settled && down.what == What::Move);
        assert_eq!(down.before, vec![was], "measured from where it was picked up");
    }

    /// It re-parents to whatever it hangs on, as a hangar does, but never onto itself.
    #[test]
    fn a_carried_part_hangs_from_the_part_under_the_pointer_but_not_from_itself() {
        let draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let mut deck = Carry::pick_up(&draft, PartId(4)).unwrap();
        // The deck is drawn over this point and looks through itself to the storage, where it
        // already hangs, so there is nothing to show.
        assert!(deck.hang(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).is_none());
        let engine = lens.project(sdf.pieces().iter().find(|p| p.part == PartId(2)).unwrap().pose.position).unwrap().0;
        let edit = deck.hang(&draft, &sdf, &lens, engine, Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(2), "onto the engine");
        let edit = deck.hang(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        assert_eq!(edit.after[0].placement.unwrap().parent, PartId(1), "and back");
        assert!(deck.hang(&draft, &sdf, &lens, Vec2::new(5.0, 700.0), Modifiers::default(), &B).is_none(), "over nothing it stays");
        assert!(Carry::pick_up(&draft, PartId(0)).is_none() && Carry::pick_up(&draft, PartId(1)).is_none(), "the Mind and an enclosing part stay put");
    }

    #[test]
    fn a_part_taken_from_the_list_is_added_where_it_is_put_down() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let mut carry = Carry::new_part(&draft, Kind::Bay, crate::draft::PRIMITIVES[3], &B).unwrap();
        assert_eq!(carry.put_down(), None, "not hung anywhere, so nothing to add");
        assert!(carry.discard(&draft).is_empty());
        let edit = carry.hang(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        assert_eq!(draft.part(carry.part.id).unwrap().kind, Kind::Bay);
        let down = carry.put_down().unwrap();
        assert!(down.settled && down.what == What::Add && down.before.is_empty());
        // Dropped on the list instead, it goes again and nothing is recorded.
        let [Action::EditForm(Ok(out))] = &carry.discard(&draft)[..] else { panic!() };
        assert!(!out.settled);
        apply(&mut draft, out);
        assert_eq!(draft.form, draft.ship);
    }

    /// Dropped on the list, a part picked up is deleted, and the deletion remembers it as it was
    /// before it was picked up, so undoing it puts it back there.
    #[test]
    fn a_part_dropped_on_the_list_is_deleted_as_it_was() {
        let mut draft = Draft::new(Form::starting());
        let (sdf, lens) = scene(&draft);
        let mut carry = Carry::pick_up(&draft, PartId(5)).unwrap();
        let edit = carry.hang(&draft, &sdf, &lens, top(&lens), Modifiers::default(), &B).unwrap();
        apply(&mut draft, &edit);
        let [Action::EditForm(Ok(removal))] = &carry.discard(&draft)[..] else { panic!() };
        assert!(removal.settled && removal.what == What::Remove);
        apply(&mut draft, removal);
        assert!(draft.part(PartId(5)).is_none());
        apply(&mut draft, &removal.inverse());
        assert_eq!(draft.form, draft.ship);
        // The last drones too: a draft may have none on the way to a design.
        let drones = Carry::pick_up(&draft, PartId(3)).unwrap();
        let [Action::EditForm(Ok(removal))] = &drones.discard(&draft)[..] else { panic!() };
        apply(&mut draft, removal);
        assert!(draft.form.parts.iter().all(|p| p.kind != Kind::Drone));
    }
}
