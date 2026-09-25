//! The editor's side panels: the tree of parts on the left, and the selected part's fields on the
//! right, in [`em_ui`]'s widgets like the handles. **Every handle has a field**, so anything done
//! with the mouse can be typed exactly, and a typed number is not snapped.
//!
//! Both are rebuilt only when what they list changes, since Bevy UI is retained and a button
//! rebuilt every frame never shows a hover; the numbers in the fields are written in place.

use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use em_ui::{Committed, Edge, MenuTheme, MenuUi, NumberField};
use lc_world::fitting::Balance;
use lc_world::form::{Kind, Mount, PartId, SparMode};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{self, Draft, Field, Mark};
use crate::input::Requested;
use crate::ui::ViewMode;

const WIDTH: f32 = 250.0;
/// Below the editor's own strip across the top.
const BELOW_STRIP: f32 = 44.0;

/// What a panel button asks for, turned into an action against the draft as it is when pressed.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Tap {
    Select(PartId),
    NextKind,
    NextPrimitive,
    ToggleMount,
    SparMode(SparMode),
    Mirror(bool),
    Delete,
    /// Arm the add, or disarm it.
    Add,
    NextAddKind,
    NextAddPrimitive,
    Reset,
}

/// The action `tap` asks for, given the draft and what is selected and armed.
pub fn action_of(tap: Tap, draft: &Draft, selected: Option<PartId>, adding: Option<(Kind, lc_world::form::Primitive)>) -> Option<Action> {
    let part = selected.and_then(|id| draft.part(id));
    let b = Balance::DEFAULT;
    let armed = adding.unwrap_or((draft::KINDS[0], draft::PRIMITIVES[0]));
    Some(match tap {
        Tap::Select(id) => Action::SelectPart((selected != Some(id)).then_some(id)),
        Tap::NextKind => Action::EditForm(draft.set_kind(part?.id, draft::next_kind(part?.kind))),
        Tap::NextPrimitive => Action::EditForm(draft.reshape(part?.id, draft::next_primitive(&part?.primitive))),
        Tap::ToggleMount => Action::EditForm(draft.toggle_mount(part?.id)),
        Tap::SparMode(mode) => Action::EditForm(draft.spar_mode(part?.id, mode)),
        Tap::Mirror(on) => Action::EditForm(draft.mirror(part?.id, on)),
        Tap::Delete => Action::EditForm(draft.remove(part?.id, &b)),
        Tap::Add => Action::ArmAdd(adding.is_none().then_some(armed)),
        Tap::NextAddKind => Action::ArmAdd(Some((draft::next_kind(armed.0), armed.1))),
        Tap::NextAddPrimitive => Action::ArmAdd(Some((armed.0, draft::next_primitive(&armed.1)))),
        Tap::Reset => Action::EditForm(Ok(draft.reset())),
    })
}

/// Which number a field is.
#[derive(Component, Clone, Copy)]
pub struct FieldOf(pub Field);

#[derive(Component)]
struct TreePanel;

#[derive(Component)]
struct FieldsPanel;

/// What a panel was built for, compared with what it would be built for now.
#[derive(Component, PartialEq)]
struct Built(Vec<String>);

pub struct FormPanelPlugin;

impl Plugin for FormPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (lay_out, show_numbers)
                .chain()
                .in_set(crate::app::Stage::Scene)
                .after(crate::form_view::place)
                .run_if(in_state(crate::app::AppState::InGame)),
        )
        .add_systems(OnExit(crate::app::AppState::InGame), put_away);
    }
}

/// The tree's lines: each part under its parent, with its mark in words as well as color.
fn tree_lines(draft: &Draft, marks: &std::collections::BTreeMap<PartId, Mark>) -> Vec<(usize, PartId, String)> {
    draft
        .tree()
        .into_iter()
        .map(|(depth, part)| {
            let mark = marks.get(&part.id).map(|m| format!("  [{}]", m.word())).unwrap_or_default();
            let mirror = if part.placement.is_some_and(|p| p.mirror) { " x2" } else { "" };
            let what = format!("{} {}, {}{mirror}{mark}", part.id.0, draft::kind_name(part.kind), draft::primitive_name(&part.primitive));
            (depth, part.id, what)
        })
        .collect()
}

fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    shown: Res<crate::form_view::Shown>,
    foot: Res<crate::panels::HudFoot>,
    assets: Res<AssetServer>,
    mut trees: Query<(Entity, &Built, &mut Node), (With<TreePanel>, Without<FieldsPanel>)>,
    mut fields: Query<(Entity, &Built, &mut Node), (With<FieldsPanel>, Without<TreePanel>)>,
) {
    let draft = ui.form.draft.as_ref().filter(|_| ui.view == ViewMode::Form);
    let top = foot.0 + BELOW_STRIP;
    let font = || assets.load(crate::faces::UI_FILE);

    let tree_key = draft.map(|d| {
        let mut key: Vec<String> = tree_lines(d, shown.marks()).into_iter().map(|(depth, id, what)| format!("{depth}{id}{what}")).collect();
        key.push(format!("{:?} {:?}", ui.form.selected, ui.form.adding));
        key
    });
    rebuild(&mut commands, trees.iter_mut(), tree_key, top, |commands, key| {
        let draft = draft.expect("keyed on the draft");
        build_tree(commands, draft, shown.marks(), ui.form.selected, ui.form.adding, key, top, font());
    });

    let fields_key = draft.map(|d| {
        let part = ui.form.selected.and_then(|id| d.part(id));
        vec![format!("{:?}", part.map(|p| (p.id, p.kind, draft::primitive_name(&p.primitive), draft::fields(p), p.placement.map(|pl| (matches!(pl.mount, Mount::Enclosing), pl.mirror)))))]
    });
    rebuild(&mut commands, fields.iter_mut(), fields_key, top, |commands, key| {
        let draft = draft.expect("keyed on the draft");
        build_fields(commands, draft, ui.form.selected, key, top, font());
    });
}

/// Build a panel when what it would show differs from what it shows, and keep it under the
/// readout otherwise.
fn rebuild<'a>(
    commands: &mut Commands,
    panels: impl Iterator<Item = (Entity, &'a Built, Mut<'a, Node>)>,
    key: Option<Vec<String>>,
    top: f32,
    build: impl FnOnce(&mut Commands, Built),
) {
    let mut current = false;
    for (entity, built, mut node) in panels {
        if key.as_ref() == Some(&built.0) {
            current = true;
            if node.top != Val::Px(top) {
                node.top = Val::Px(top);
            }
        } else {
            commands.entity(entity).despawn();
        }
    }
    if let (false, Some(key)) = (current, key) {
        build(commands, Built(key));
    }
}

#[allow(clippy::too_many_arguments)]
fn build_tree(
    commands: &mut Commands,
    draft: &Draft,
    marks: &std::collections::BTreeMap<PartId, Mark>,
    selected: Option<PartId>,
    adding: Option<(Kind, lc_world::form::Primitive)>,
    built: Built,
    top: f32,
    font: Handle<Font>,
) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let root = ui.docked((TreePanel, built), Edge::Left, crate::map_panel::CORNER_INSET);
    ui.insert(root, Node { top: Val::Px(top), width: Val::Px(WIDTH), ..column(Edge::Left) });
    let panel = ui.strip(root);
    ui.insert(panel, Node { align_items: AlignItems::Stretch, ..panel_node() });
    ui.inline(panel, "PARTS", 15.0, em_ui::vfd::TEXT);
    for (depth, id, what) in tree_lines(draft, marks) {
        let chosen = selected == Some(id);
        let text = if chosen { format!("> {what}") } else { what };
        let row = ui.tree_row(panel, depth, &text, chosen, Tap::Select(id));
        if let Some(mark) = marks.get(&id) {
            ui.insert(row, BorderColor::all(mark.color()));
        }
    }
    let (kind, primitive) = adding.unwrap_or((draft::KINDS[0], draft::PRIMITIVES[0]));
    ui.inline(panel, "ADD", 15.0, em_ui::vfd::TEXT);
    let row = ui.row(panel);
    ui.small_button(row, draft::kind_name(kind), Tap::NextAddKind);
    ui.small_button(row, draft::primitive_name(&primitive), Tap::NextAddPrimitive);
    let arm = if adding.is_some() { "click a part" } else { "add" };
    ui.chosen_button(row, arm, adding.is_some(), Tap::Add);
    let row = ui.row(panel);
    ui.small_button(row, "reset to the ship", Tap::Reset);
}

fn build_fields(commands: &mut Commands, draft: &Draft, selected: Option<PartId>, built: Built, top: f32, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let root = ui.docked((FieldsPanel, built), Edge::Right, crate::map_panel::CORNER_INSET);
    ui.insert(root, Node { top: Val::Px(top), width: Val::Px(WIDTH), ..column(Edge::Right) });
    let panel = ui.strip(root);
    ui.insert(panel, Node { align_items: AlignItems::Stretch, row_gap: Val::Px(3.0), ..panel_node() });
    let Some(part) = selected.and_then(|id| draft.part(id)) else {
        ui.inline(panel, "Select a part to edit it.", 14.0, em_ui::vfd::TEXT_DIM);
        return;
    };
    ui.inline(panel, &format!("PART {}", part.id.0), 15.0, em_ui::vfd::TEXT);
    let row = ui.row(panel);
    ui.small_button(row, draft::kind_name(part.kind), Tap::NextKind);
    ui.small_button(row, draft::primitive_name(&part.primitive), Tap::NextPrimitive);
    if let Some(placement) = part.placement {
        let row = ui.row(panel);
        let mount = match placement.mount {
            Mount::Attached { .. } => "attached",
            Mount::Enclosing => "enclosing",
        };
        ui.small_button(row, mount, Tap::ToggleMount);
        let mirror = if placement.mirror { "mirror: on" } else { "mirror: off" };
        ui.chosen_button(row, mirror, placement.mirror, Tap::Mirror(!placement.mirror));
        if let Kind::Spar(mode) = part.kind {
            let row = ui.row(panel);
            ui.chosen_button(row, "saddle", mode == SparMode::Saddle, Tap::SparMode(SparMode::Saddle));
            ui.chosen_button(row, "strap", mode == SparMode::Strap, Tap::SparMode(SparMode::Strap));
        }
    }
    for (field, label) in draft::fields(part) {
        let text = draft::value(part, field).map(draft::shown).unwrap_or_default();
        ui.field(panel, label, &text, FieldOf(field));
    }
    if part.kind != Kind::Mind {
        let row = ui.row(panel);
        ui.small_button(row, "delete", Tap::Delete);
    }
}

fn column(edge: Edge) -> Node {
    let mut node = Node { position_type: PositionType::Absolute, flex_direction: FlexDirection::Column, align_items: AlignItems::Stretch, ..default() };
    match edge {
        Edge::Left => node.left = Val::Px(crate::map_panel::CORNER_INSET),
        _ => node.right = Val::Px(crate::map_panel::CORNER_INSET),
    }
    node
}

fn panel_node() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
        border: UiRect::all(Val::Px(1.0)),
        row_gap: Val::Px(2.0),
        ..default()
    }
}

/// Write each field's number in place, except the one being typed in.
fn show_numbers(
    ui: Res<Ui>,
    focus: Option<Res<InputFocus>>,
    mut fields: Query<(Entity, &FieldOf, &mut NumberField, &mut EditableText)>,
) {
    let Some(part) = ui.form.selected.and_then(|id| ui.form.draft.as_ref()?.part(id)) else { return };
    let focused = focus.and_then(|f| f.get());
    for (entity, of, mut field, mut editable) in &mut fields {
        let text = draft::value(part, of.0).map(draft::shown).unwrap_or_default();
        field.show(&mut editable, focused == Some(entity), &text);
    }
}

/// Panel buttons and committed fields, as actions.
pub fn press(
    ui: Res<Ui>,
    taps: Query<(&Interaction, &Tap), Changed<Interaction>>,
    fields: Query<&FieldOf>,
    mut committed: MessageReader<Committed>,
    mut out: MessageWriter<Requested>,
) {
    let Some(draft) = ui.form.draft.as_ref() else { return };
    for (interaction, tap) in &taps {
        if *interaction == Interaction::Pressed
            && let Some(action) = action_of(*tap, draft, ui.form.selected, ui.form.adding)
        {
            out.write(Requested(action));
        }
    }
    for commit in committed.read() {
        if let (Ok(of), Some(id)) = (fields.get(commit.field), ui.form.selected) {
            out.write(Requested(Action::EditForm(draft.set_field(id, of.0, commit.value))));
        }
    }
}

fn put_away(mut commands: Commands, panels: Query<Entity, Or<(With<TreePanel>, With<FieldsPanel>)>>) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::Form;

    use super::*;

    #[test]
    fn a_tap_on_the_selected_row_lets_it_go() {
        let d = Draft::new(Form::starting());
        assert_eq!(action_of(Tap::Select(PartId(2)), &d, None, None), Some(Action::SelectPart(Some(PartId(2)))));
        assert_eq!(action_of(Tap::Select(PartId(2)), &d, Some(PartId(2)), None), Some(Action::SelectPart(None)));
    }

    #[test]
    fn a_part_button_needs_a_part_and_the_add_cycles_what_it_adds() {
        let d = Draft::new(Form::starting());
        assert_eq!(action_of(Tap::Delete, &d, None, None), None);
        let Some(Action::ArmAdd(Some((kind, _)))) = action_of(Tap::Add, &d, None, None) else { panic!() };
        assert_eq!(kind, draft::KINDS[0]);
        assert_eq!(action_of(Tap::Add, &d, None, Some((kind, draft::PRIMITIVES[0]))), Some(Action::ArmAdd(None)));
        let Some(Action::ArmAdd(Some((next, _)))) = action_of(Tap::NextAddKind, &d, None, None) else { panic!() };
        assert_eq!(next, draft::KINDS[1]);
    }

    #[test]
    fn the_tree_names_each_part_and_its_mark_in_words() {
        let mut d = Draft::new(Form::starting());
        d.apply(&d.twist(PartId(4), 0.3).unwrap(), &Balance::DEFAULT).unwrap();
        let lines = tree_lines(&d, &d.marks(&Balance::DEFAULT));
        assert_eq!(lines[0].2, "0 Mind, slab");
        let living = lines.iter().find(|(_, id, _)| *id == PartId(4)).unwrap();
        assert!(living.2.ends_with("[move]"), "{}", living.2);
        assert_eq!(living.0, 2, "under the storage, under the Mind");
    }
}
