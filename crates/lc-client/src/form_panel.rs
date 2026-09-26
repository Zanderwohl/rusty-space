//! The editor's side panels: the palette on the left, and on the right the tree of parts and the
//! selected part's detail, whose numbers include a field for every handle.
//!
//! Each is rebuilt only when what it lists changes, since a Bevy UI button rebuilt every frame
//! never shows a hover; numbers are written in place.

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
const GAP: f32 = 6.0;
const INSET: f32 = 12.0;
/// Of the window's height, the most the tree takes before it is cut off.
const TREE_SHARE: f32 = 45.0;

/// A panel button, turned into an action against the draft as it is when pressed.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Tap {
    Select(PartId),
    NextKind,
    NextPrimitive,
    ToggleMount,
    SparMode(SparMode),
    Mirror(bool),
    Delete,
    Take(Kind),
    NextShape,
    Reset,
    /// Hovered, it shows the ship as it is; pressed, it keeps it shown.
    ShowCurrent,
    Advanced,
}

/// `None` for [`Tap::Take`], which puts a part on the pointer instead.
pub fn action_of(tap: Tap, draft: &Draft, form: &crate::form_view::FormView) -> Option<Action> {
    let (selected, new_shape) = (form.selected, form.new_shape);
    let part = selected.and_then(|id| draft.part(id));
    Some(match tap {
        Tap::Select(id) => Action::SelectPart((selected != Some(id)).then_some(id)),
        Tap::NextKind => Action::EditForm(draft.set_kind(part?.id, draft::next_kind(part?.kind))),
        Tap::NextPrimitive => Action::EditForm(draft.reshape(part?.id, draft::next_primitive(&part?.primitive))),
        Tap::ToggleMount => Action::EditForm(draft.toggle_mount(part?.id)),
        Tap::SparMode(mode) => Action::EditForm(draft.spar_mode(part?.id, mode)),
        Tap::Mirror(on) => Action::EditForm(draft.mirror(part?.id, on)),
        Tap::Delete => Action::EditForm(draft.remove(part?.id)),
        Tap::Take(_) => return None,
        Tap::NextShape => Action::SetNewShape(new_shape + 1),
        Tap::Reset => Action::EditForm(Ok(draft.reset())),
        Tap::ShowCurrent => Action::ShowCurrent(!form.show_current),
        Tap::Advanced => Action::ShowAdvanced(!form.advanced),
    })
}

/// Which of the part's stats a figure is.
#[derive(Component)]
struct Stat(usize);

#[derive(Component, Clone, Copy)]
pub struct FieldOf(pub Field);

/// What a panel was built for.
#[derive(Component, PartialEq)]
struct Built(Vec<String>);

pub struct FormPanelPlugin;

impl Plugin for FormPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (lay_out, show_numbers, reveal)
                .chain()
                .in_set(crate::app::Stage::Scene)
                .after(crate::form_view::place)
                .run_if(in_state(crate::app::AppState::InGame)),
        )
        .add_systems(OnExit(crate::app::AppState::InGame), put_away);
    }
}

/// Marks in words as well as color.
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
    panels: Query<(Entity, &Built, &Side)>,
    mut columns: Query<(Entity, &mut Node), With<RightColumn>>,
) {
    let draft = ui.form.draft.as_ref().filter(|_| ui.view == ViewMode::Form);
    let top = foot.0 + GAP;
    let font = || assets.load(crate::faces::UI_FILE);
    let Some(draft) = draft else {
        for (entity, ..) in &panels {
            commands.entity(entity).despawn();
        }
        for (entity, _) in &columns {
            commands.entity(entity).despawn();
        }
        return;
    };

    let palette_key = vec![format!("{}", ui.form.new_shape)];
    let mut tree_key: Vec<String> = tree_lines(draft, shown.marks()).into_iter().map(|(depth, id, what)| format!("{depth}{id}{what}")).collect();
    tree_key.push(format!("{:?} {}", ui.form.selected, ui.form.show_current));
    let fields_key = vec![format!("{:?}", ui.form.selected.and_then(|id| draft.part(id)).map(|p| {
        let placement = p.placement.map(|pl| (matches!(pl.mount, Mount::Enclosing), pl.mirror));
        (p.id, p.kind, draft::primitive_name(&p.primitive), draft::fields(p), placement, ui.form.advanced)
    }))];

    let mut current = [false; 3];
    for (entity, built, side) in &panels {
        let key = match side {
            Side::Palette => &palette_key,
            Side::Tree => &tree_key,
            Side::Fields => &fields_key,
        };
        if *key == built.0 {
            current[*side as usize] = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    // A rebuilt tree takes its detail with it, so the detail stays under it.
    if !current[Side::Tree as usize] && current[Side::Fields as usize] {
        for (entity, _, side) in &panels {
            if *side == Side::Fields {
                commands.entity(entity).despawn();
            }
        }
        current[Side::Fields as usize] = false;
    }
    let column = match columns.iter_mut().next() {
        Some((entity, mut node)) => {
            if node.top != Val::Px(top) {
                node.top = Val::Px(top);
            }
            entity
        }
        None => commands
            .spawn((Node { top: Val::Px(top), width: Val::Px(WIDTH), row_gap: Val::Px(GAP), ..column(Edge::Right) }, RightColumn))
            .id(),
    };
    if !current[Side::Palette as usize] {
        build_palette(&mut commands, ui.form.new_shape, Built(palette_key), top, font());
    }
    if !current[Side::Tree as usize] {
        build_tree(&mut commands, column, draft, shown.marks(), &ui.form, Built(tree_key), font());
    }
    if !current[Side::Fields as usize] {
        build_fields(&mut commands, column, draft, &ui.form, Built(fields_key), font());
    }
}

#[derive(Component)]
struct RightColumn;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Side {
    Palette,
    Tree,
    Fields,
}

fn root(ui: &mut MenuUi, side: Side, built: Built, place: Node) -> Entity {
    let root = ui.docked((side, built), Edge::Left, INSET);
    ui.insert(root, Node { width: Val::Px(WIDTH), ..place });
    let panel = ui.strip(root);
    ui.insert(panel, Node { align_items: AlignItems::Stretch, flex_grow: 1.0, ..panel_node() });
    panel
}

fn stacked(ui: &mut MenuUi, column: Entity, side: Side, built: Built) -> Entity {
    let panel = ui.strip(column);
    ui.insert(panel, (Node { align_items: AlignItems::Stretch, ..panel_node() }, side, built));
    panel
}

fn column(edge: Edge) -> Node {
    let mut node = Node { position_type: PositionType::Absolute, flex_direction: FlexDirection::Column, align_items: AlignItems::Stretch, ..default() };
    match edge {
        Edge::Left => node.left = Val::Px(INSET),
        _ => node.right = Val::Px(INSET),
    }
    node
}

fn build_palette(commands: &mut Commands, new_shape: usize, built: Built, top: f32, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    // Its own height, leaving the bottom of the column to the preview.
    let place = Node { top: Val::Px(top), ..column(Edge::Left) };
    let panel = root(&mut ui, Side::Palette, built, place);
    ui.insert(panel, (Interaction::None, crate::form_carry::DropZone));
    ui.inline(panel, "ADD A PART", 15.0, em_ui::vfd::TEXT);
    let shape = draft::PRIMITIVES[new_shape % draft::PRIMITIVES.len()];
    let row = ui.row(panel);
    ui.inline(row, "shape", 13.0, em_ui::vfd::TEXT_DIM);
    ui.small_button(row, draft::primitive_name(&shape), Tap::NextShape);
    for kind in draft::KINDS {
        ui.tree_row(panel, 0, draft::kind_name(kind), false, Tap::Take(kind));
    }
}

fn build_tree(
    commands: &mut Commands,
    column: Entity,
    draft: &Draft,
    marks: &std::collections::BTreeMap<PartId, Mark>,
    form: &crate::form_view::FormView,
    built: Built,
    font: Handle<Font>,
) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let panel = stacked(&mut ui, column, Side::Tree, built);
    // So the detail under it always has room.
    ui.insert(panel, Node { max_height: Val::Vh(TREE_SHARE), overflow: Overflow::clip_y(), align_items: AlignItems::Stretch, ..panel_node() });
    ui.inline(panel, "PARTS", 15.0, em_ui::vfd::TEXT);
    for (depth, id, what) in tree_lines(draft, marks) {
        let chosen = form.selected == Some(id);
        let text = if chosen { format!("> {what}") } else { what };
        let row = ui.tree_row(panel, depth, &text, chosen, Tap::Select(id));
        if let Some(mark) = marks.get(&id) {
            ui.insert(row, BorderColor::all(mark.color()));
        }
    }
    let row = ui.row(panel);
    ui.small_button(row, "reset to the ship", Tap::Reset);
    let on = if form.show_current { "on" } else { "off" };
    let row = ui.row(panel);
    ui.chosen_button(row, &format!("show current: {on}"), form.show_current, Tap::ShowCurrent);
}

fn build_fields(commands: &mut Commands, column: Entity, draft: &Draft, form: &crate::form_view::FormView, built: Built, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let panel = stacked(&mut ui, column, Side::Fields, built);
    let Some(part) = form.selected.and_then(|id| draft.part(id)) else {
        // Kept, empty, so its key is still there to compare.
        ui.insert(panel, Node { display: Display::None, ..default() });
        return;
    };
    ui.insert(panel, Node { align_items: AlignItems::Stretch, row_gap: Val::Px(3.0), ..panel_node() });
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
    for (index, (label, value)) in draft.stats(part.id, &Balance::DEFAULT).into_iter().enumerate() {
        let row = ui.row(panel);
        ui.insert(row, Node { flex_direction: FlexDirection::Row, justify_content: JustifyContent::SpaceBetween, ..default() });
        ui.inline(row, label, 13.0, em_ui::vfd::TEXT_DIM);
        let figure = ui.inline(row, &value, 13.0, em_ui::vfd::TEXT);
        ui.insert(figure, Stat(index));
    }
    let row = ui.row(panel);
    ui.chosen_button(row, if form.advanced { "advanced: on" } else { "advanced: off" }, form.advanced, Tap::Advanced);
    if part.kind != Kind::Mind {
        ui.small_button(row, "delete", Tap::Delete);
    }
    if form.advanced {
        for (label, line) in draft::fields(part) {
            let cells = line.into_iter().map(|field| (draft::value(part, field).map(draft::shown).unwrap_or_default(), FieldOf(field)));
            ui.fields(panel, label, cells);
        }
    }
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

fn reveal(ui: Res<Ui>, taps: Query<(&Interaction, &Tap)>, mut revealed: ResMut<crate::form_view::Revealed>) {
    let held = taps.iter().any(|(i, t)| *t == Tap::ShowCurrent && *i != Interaction::None);
    let wanted = ui.view == ViewMode::Form && (ui.form.show_current || held);
    if revealed.0 != wanted {
        revealed.0 = wanted;
    }
}

fn show_numbers(
    ui: Res<Ui>,
    focus: Option<Res<InputFocus>>,
    mut fields: Query<(Entity, &FieldOf, &mut NumberField, &mut EditableText)>,
    mut figures: Query<(&Stat, &mut Text)>,
) {
    let Some(draft) = ui.form.draft.as_ref() else { return };
    let Some(part) = ui.form.selected.and_then(|id| draft.part(id)) else { return };
    if !figures.is_empty() {
        let stats = draft.stats(part.id, &Balance::DEFAULT);
        for (stat, mut text) in &mut figures {
            if let Some((_, value)) = stats.get(stat.0)
                && text.0 != *value
            {
                text.0 = value.clone();
            }
        }
    }
    let focused = focus.and_then(|f| f.get());
    for (entity, of, mut field, mut editable) in &mut fields {
        let text = draft::value(part, of.0).map(draft::shown).unwrap_or_default();
        field.show(&mut editable, focused == Some(entity), &text);
    }
}

pub fn press(
    ui: Res<Ui>,
    mut carried: ResMut<crate::form_carry::Carried>,
    taps: Query<(&Interaction, &Tap), Changed<Interaction>>,
    fields: Query<&FieldOf>,
    mut committed: MessageReader<Committed>,
    mut out: MessageWriter<Requested>,
) {
    let Some(draft) = ui.form.draft.as_ref() else { return };
    if carried.just_dropped() {
        return;
    }
    for (interaction, tap) in &taps {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Tap::Take(kind) = *tap
            && let Some(carry) = crate::form_carry::Carry::new_part(draft, kind, draft::PRIMITIVES[ui.form.new_shape % draft::PRIMITIVES.len()], &Balance::DEFAULT)
        {
            carried.take(carry);
        } else if let Some(action) = action_of(*tap, draft, &ui.form) {
            out.write(Requested(action));
        }
    }
    for commit in committed.read() {
        if let (Ok(of), Some(id)) = (fields.get(commit.field), ui.form.selected) {
            out.write(Requested(Action::EditForm(draft.set_field(id, of.0, commit.value))));
        }
    }
}

fn put_away(mut commands: Commands, panels: Query<Entity, Or<(With<Side>, With<RightColumn>)>>) {
    for entity in &panels {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::form::Form;

    use super::*;
    use crate::form_view::FormView;

    fn view(selected: Option<PartId>, new_shape: usize) -> FormView {
        FormView { selected, new_shape, ..FormView::default() }
    }

    #[test]
    fn the_current_toggle_flips() {
        let d = Draft::new(Form::starting());
        assert_eq!(action_of(Tap::ShowCurrent, &d, &view(None, 0)), Some(Action::ShowCurrent(true)));
        let on = FormView { show_current: true, ..view(None, 0) };
        assert_eq!(action_of(Tap::ShowCurrent, &d, &on), Some(Action::ShowCurrent(false)));
    }

    #[test]
    fn a_tap_on_the_selected_row_lets_it_go() {
        let d = Draft::new(Form::starting());
        assert_eq!(action_of(Tap::Select(PartId(2)), &d, &view(None, 0)), Some(Action::SelectPart(Some(PartId(2)))));
        assert_eq!(action_of(Tap::Select(PartId(2)), &d, &view(Some(PartId(2)), 0)), Some(Action::SelectPart(None)));
    }

    #[test]
    fn a_part_button_needs_a_part_and_the_shape_button_cycles() {
        let d = Draft::new(Form::starting());
        assert_eq!(action_of(Tap::Delete, &d, &view(None, 0)), None);
        assert_eq!(action_of(Tap::NextShape, &d, &view(None, 5)), Some(Action::SetNewShape(6)));
        assert_eq!(action_of(Tap::Take(Kind::Bay), &d, &view(None, 0)), None, "it goes on the pointer, not into an action");
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
