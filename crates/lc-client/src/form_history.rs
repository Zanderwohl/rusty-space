//! The editor's history: every settled edit to the draft, before and after, and a cursor into
//! them, with its panel and its keys. See `lightcone/docs/29-ship-form.md` §Undo and redo.
//!
//! Only a settled edit is recorded, and a held handle's settled edit carries the part as the drag
//! began, so a drag is one entry. Stepping writes an entry's `before` or `after` straight onto
//! the draft, past the storage gate a new edit meets: every state it returns to is one the player
//! had.

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use em_ui::{MenuTheme, MenuUi, Reached};
use lc_world::fitting::Balance;
use lc_world::form::{Form, Part};

use crate::action::Action;
use crate::app::Ui;
use crate::draft::{Draft, Edit, Refused, What, figure, kind_name, primitive_name};
use crate::form_view::FormView;
use crate::input::Requested;
use crate::ui::ViewMode;

pub const MAX_HISTORY: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub edit: Edit,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    /// Oldest first.
    entries: VecDeque<Entry>,
    /// How many entries the draft has had done to it.
    cursor: usize,
}

impl History {
    pub fn entries(&self) -> &VecDeque<Entry> {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// An edit that changed nothing is not recorded, nor one still being made.
    pub fn record(&mut self, edit: &Edit, ship: &Form) {
        if !edit.settled || sorted(&edit.before) == sorted(&edit.after) {
            return;
        }
        self.entries.truncate(self.cursor);
        self.entries.push_back(Entry { edit: edit.clone(), label: label(edit, ship) });
        if self.entries.len() > MAX_HISTORY {
            self.entries.pop_front();
        }
        self.cursor = self.entries.len();
    }

    /// Undo or redo one entry at a time until `to` are done, stopping at the first the draft
    /// refuses.
    pub fn go(&mut self, to: usize, draft: &mut Draft) -> Result<(), Refused> {
        let to = to.min(self.entries.len());
        while self.cursor != to {
            let back = to < self.cursor;
            let edit = if back { self.entries[self.cursor - 1].edit.inverse() } else { self.entries[self.cursor].edit.clone() };
            draft.apply(&edit, &Balance::DEFAULT)?;
            self.cursor = if back { self.cursor - 1 } else { self.cursor + 1 };
        }
        Ok(())
    }
}

fn sorted(parts: &[Part]) -> Vec<Part> {
    let mut parts = parts.to_vec();
    parts.sort_by_key(|p| p.id);
    parts
}

fn named(part: &Part) -> String {
    format!("{} {}", part.id.0, kind_name(part.kind))
}

/// What the entry says in the panel: what was done, in the tree's names for parts.
pub fn label(edit: &Edit, ship: &Form) -> String {
    let (was, now) = (edit.before.iter().find(|p| p.id == edit.part), edit.after.iter().find(|p| p.id == edit.part));
    match (edit.what, was, now) {
        (What::Whole, ..) if sorted(&edit.after) == sorted(&ship.parts) => "reset to the ship".into(),
        (What::Whole, ..) => "replaced the draft".into(),
        (What::Add, _, Some(now)) => format!("added {}, {}", named(now), primitive_name(&now.primitive)),
        (What::Remove, Some(was), _) => match edit.before.len() - 1 {
            0 => format!("removed {}", named(was)),
            1 => format!("removed {} and the part on it", named(was)),
            n => format!("removed {} and the {n} parts on it", named(was)),
        },
        (What::Resize, Some(was), Some(now)) => {
            format!("resized {}, {} to {} m3", named(was), figure(was.volume_m3), figure(now.volume_m3))
        }
        (What::Reshape, Some(was), Some(now)) if was.kind != now.kind => {
            format!("made {} {}", named(was), kind_name(now.kind))
        }
        (What::Reshape, Some(was), Some(now)) if primitive_name(&was.primitive) != primitive_name(&now.primitive) => {
            format!("reshaped {}, {} to {}", named(was), primitive_name(&was.primitive), primitive_name(&now.primitive))
        }
        (What::Reshape, Some(was), _) => format!("reshaped {}", named(was)),
        (What::Move, Some(was), _) => format!("moved {}", named(was)),
        (What::Mirror, _, Some(now)) if now.placement.is_some_and(|p| p.mirror) => format!("mirrored {}", named(now)),
        (What::Mirror, _, Some(now)) => format!("unmirrored {}", named(now)),
        _ => format!("edited part {}", edit.part.0),
    }
}

/// `Cmd` or `Ctrl` with `Z` undoes, and `Shift` with them redoes.
pub fn keyed(z: bool, command: bool, shift: bool) -> Option<Action> {
    match (z && command, shift) {
        (true, false) => Some(Action::Undo),
        (true, true) => Some(Action::Redo),
        (false, _) => None,
    }
}

/// Silent while a part is carried or a handle held: their edits are still being made.
pub fn read_keys(
    keys: Res<ButtonInput<KeyCode>>,
    typing: em_ui::Typing,
    egui: Res<EguiWantsInput>,
    carried: Res<crate::form_carry::Carried>,
    grabbed: Res<crate::form_handles::Grabbed>,
    mut out: MessageWriter<Requested>,
) {
    if typing.active() || egui.wants_any_keyboard_input() || carried.is_carrying() || grabbed.is_holding() {
        return;
    }
    let command = keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight, KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(action) = keyed(keys.just_pressed(KeyCode::KeyZ), command, shift) {
        out.write(Requested(action));
    }
}

/// A row of the panel: the cursor put where this many entries are done.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct GoTo(usize);

/// What the panel was built for.
#[derive(Component, PartialEq)]
struct Built(Vec<String>, usize);

/// Newest first, over the start, so what a short window cuts off is the oldest.
fn rows(form: &FormView) -> Vec<(String, Reached, usize)> {
    let history = &form.history;
    let reached = |done: usize| match done.cmp(&history.cursor) {
        std::cmp::Ordering::Less => Reached::Behind,
        std::cmp::Ordering::Equal => Reached::At,
        std::cmp::Ordering::Greater => Reached::Ahead,
    };
    let mut out: Vec<(String, Reached, usize)> =
        history.entries.iter().enumerate().rev().map(|(i, entry)| (entry.label.clone(), reached(i + 1), i + 1)).collect();
    out.push(("start".into(), reached(0), 0));
    out
}

pub struct FormHistoryPlugin;

impl Plugin for FormHistoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            lay_out
                .in_set(crate::app::Stage::Scene)
                .after(crate::form_view::place)
                .run_if(in_state(crate::app::AppState::InGame)),
        );
    }
}

/// Last in the left column, under the palette, which is put first whenever it is rebuilt.
fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    assets: Res<AssetServer>,
    panels: Query<(Entity, &Built)>,
    columns: Query<Entity, With<crate::form_panel::LeftColumn>>,
) {
    let column = columns.iter().next().filter(|_| ui.view == ViewMode::Form && ui.form.draft.is_some());
    let key = Built(ui.form.history.entries.iter().map(|e| e.label.clone()).collect(), ui.form.history.cursor);
    let mut current = false;
    for (panel, built) in &panels {
        if column.is_some() && *built == key {
            current = true;
        } else {
            commands.entity(panel).try_despawn();
        }
    }
    let (Some(column), false) = (column, current) else { return };
    let mut menu = MenuUi::new(&mut commands, MenuTheme::VFD).font(assets.load(crate::faces::UI_FILE));
    let panel = menu.strip(column);
    let node = Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Stretch,
        flex_grow: 1.0,
        min_height: Val::Px(0.0),
        overflow: Overflow::clip_y(),
        padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
        border: UiRect::all(Val::Px(1.0)),
        row_gap: Val::Px(2.0),
        ..default()
    };
    menu.insert(panel, (key, node));
    menu.inline(panel, "HISTORY", 15.0, em_ui::vfd::TEXT);
    for (text, reached, done) in rows(&ui.form) {
        menu.cursor_row(panel, &text, reached, GoTo(done));
    }
}

pub fn press(
    carried: Res<crate::form_carry::Carried>,
    rows: Query<(&Interaction, &GoTo), Changed<Interaction>>,
    mut out: MessageWriter<Requested>,
) {
    if carried.is_carrying() || carried.just_dropped() {
        return;
    }
    for (interaction, go) in &rows {
        if *interaction == Interaction::Pressed {
            out.write(Requested(Action::GoToEdit(go.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_world::form::{Kind, PartId};

    use super::*;
    use crate::action::apply;
    use crate::draft::PRIMITIVES;
    use crate::session::Session;
    use crate::ui::UiState;

    const B: Balance = Balance::DEFAULT;

    fn editing() -> (UiState, Session) {
        let (mut ui, mut s) = (UiState::default(), Session::new(&lc_world::sky::AuthoredStars::sample(), 3));
        apply(Action::StartDraft(Form::starting()), &mut ui, &mut s);
        (ui, s)
    }

    fn draft(ui: &UiState) -> &Draft {
        ui.form.draft.as_ref().expect("a draft")
    }

    fn edit(ui: &mut UiState, s: &mut Session, make: impl Fn(&Draft) -> Result<Edit, Refused>) {
        let edit = make(draft(ui));
        apply(Action::EditForm(edit), ui, s);
    }

    fn by_kind(d: &Draft, kind: Kind) -> PartId {
        d.form.parts.iter().find(|p| p.kind == kind).unwrap().id
    }

    fn twist(ui: &mut UiState, s: &mut Session, radians: f64) {
        edit(ui, s, |d| d.twist(by_kind(d, Kind::Living), radians));
    }

    /// The done-when: a removal with children comes back with the same ids, in the same places.
    #[test]
    fn undoing_a_removal_restores_the_same_part_with_its_children() {
        let (mut ui, mut s) = editing();
        let storage = by_kind(draft(&ui), Kind::Storage);
        edit(&mut ui, &mut s, |d| d.add(storage, Kind::Bay, PRIMITIVES[3], DVec3::NEG_Y, &B));
        let before = draft(&ui).form.clone();
        let subtree: Vec<Part> = draft(&ui).subtree(storage);
        assert!(subtree.len() > 2, "storage carries children: {subtree:?}");
        edit(&mut ui, &mut s, |d| d.remove(storage));
        assert!(draft(&ui).part(storage).is_none());
        assert_eq!(ui.form.history.entries().back().unwrap().label, format!("removed {} storage and the {} parts on it", storage.0, subtree.len() - 1));
        apply(Action::Undo, &mut ui, &mut s);
        assert_eq!(draft(&ui).form, before, "the same parts, ids and placements");
        assert_eq!(draft(&ui).subtree(storage), subtree);
        apply(Action::Redo, &mut ui, &mut s);
        assert!(draft(&ui).part(storage).is_none(), "and redo takes it away again");
    }

    /// A handle sends an unsettled edit every frame it moves, and one settled on release.
    #[test]
    fn a_drag_is_one_entry() {
        let (mut ui, mut s) = editing();
        let start = draft(&ui).form.clone();
        let living = by_kind(draft(&ui), Kind::Living);
        let grip = *draft(&ui).part(living).unwrap();
        let mut last = None;
        for frame in 1..=20 {
            let mut e = draft(&ui).twist(living, 0.01 * f64::from(frame)).unwrap();
            e.before = vec![grip];
            e.settled = false;
            apply(Action::EditForm(Ok(e.clone())), &mut ui, &mut s);
            last = Some(e);
        }
        assert!(ui.form.history.entries().is_empty(), "nothing is recorded while held");
        apply(Action::EditForm(Ok(Edit { settled: true, ..last.unwrap() })), &mut ui, &mut s);
        assert_eq!(ui.form.history.entries().len(), 1);
        apply(Action::Undo, &mut ui, &mut s);
        assert_eq!(draft(&ui).form, start, "the whole gesture undoes at once");
    }

    #[test]
    fn a_new_edit_behind_the_end_drops_what_was_undone() {
        let (mut ui, mut s) = editing();
        for k in 1..=3 {
            twist(&mut ui, &mut s, 0.1 * f64::from(k));
        }
        apply(Action::Undo, &mut ui, &mut s);
        apply(Action::Undo, &mut ui, &mut s);
        assert_eq!((ui.form.history.entries().len(), ui.form.history.cursor()), (3, 1));
        twist(&mut ui, &mut s, 0.9);
        assert_eq!((ui.form.history.entries().len(), ui.form.history.cursor()), (2, 2));
        let twisted = draft(&ui).form.clone();
        apply(Action::Redo, &mut ui, &mut s);
        assert_eq!(draft(&ui).form, twisted, "nothing ahead to redo");
    }

    #[test]
    fn going_to_an_entry_undoes_or_redoes_everything_between() {
        let (mut ui, mut s) = editing();
        let mut states = vec![draft(&ui).form.clone()];
        let storage = by_kind(draft(&ui), Kind::Storage);
        edit(&mut ui, &mut s, |d| d.add(storage, Kind::Bay, PRIMITIVES[3], DVec3::NEG_Y, &B));
        states.push(draft(&ui).form.clone());
        twist(&mut ui, &mut s, 0.4);
        states.push(draft(&ui).form.clone());
        edit(&mut ui, &mut s, |d| d.remove(by_kind(d, Kind::Data)));
        states.push(draft(&ui).form.clone());
        edit(&mut ui, &mut s, |d| d.mirror(by_kind(d, Kind::Living), true));
        states.push(draft(&ui).form.clone());
        for to in [1, 4, 0, 5, 2] {
            apply(Action::GoToEdit(to), &mut ui, &mut s);
            assert_eq!(ui.form.history.cursor(), to);
            assert_eq!(draft(&ui).form, states[to], "at {to}");
        }
        assert_eq!(ui.form.selected, None, "the added bay, selected when added, went with its undoing");
    }

    #[test]
    fn the_oldest_entry_goes_past_max_history() {
        let (mut ui, mut s) = editing();
        for k in 0..MAX_HISTORY + 5 {
            twist(&mut ui, &mut s, 0.001 * (k + 1) as f64);
        }
        let history = &ui.form.history;
        assert_eq!((history.entries().len(), history.cursor()), (MAX_HISTORY, MAX_HISTORY));
        let first = &history.entries()[0].edit;
        assert!((first.after[0].placement.unwrap().twist - 0.006).abs() < 1e-12, "the first five went");
        apply(Action::GoToEdit(0), &mut ui, &mut s);
        assert!((draft(&ui).part(first.part).unwrap().placement.unwrap().twist - 0.005).abs() < 1e-12);
    }

    #[test]
    fn the_history_survives_leaving_the_editor() {
        let (mut ui, mut s) = editing();
        apply(Action::SetView(ViewMode::Form), &mut ui, &mut s);
        twist(&mut ui, &mut s, 0.3);
        apply(Action::ToggleView(ViewMode::Form), &mut ui, &mut s);
        assert_ne!(ui.view, ViewMode::Form);
        apply(Action::StartDraft(Form::starting()), &mut ui, &mut s);
        apply(Action::ToggleView(ViewMode::Form), &mut ui, &mut s);
        assert_eq!(ui.form.history.entries().len(), 1);
        apply(Action::Undo, &mut ui, &mut s);
        assert_eq!(draft(&ui).form, draft(&ui).ship);
    }

    #[test]
    fn the_keys_are_undo_and_redo() {
        assert_eq!(keyed(true, true, false), Some(Action::Undo));
        assert_eq!(keyed(true, true, true), Some(Action::Redo));
        assert_eq!(keyed(true, false, false), None, "a bare Z");
        assert_eq!(keyed(false, true, true), None);
    }

    #[test]
    fn a_reset_is_one_entry_for_the_whole_draft_and_a_no_op_is_none() {
        let (mut ui, mut s) = editing();
        apply(Action::EditForm(Ok(draft(&ui).reset())), &mut ui, &mut s);
        assert!(ui.form.history.entries().is_empty(), "the draft was the ship already");
        twist(&mut ui, &mut s, 0.3);
        edit(&mut ui, &mut s, |d| d.remove(by_kind(d, Kind::Data)));
        let edited = draft(&ui).form.clone();
        apply(Action::EditForm(Ok(draft(&ui).reset())), &mut ui, &mut s);
        assert_eq!(ui.form.history.entries().back().unwrap().label, "reset to the ship");
        apply(Action::Undo, &mut ui, &mut s);
        assert_eq!(draft(&ui).form, edited);
    }

    #[test]
    fn the_panel_lists_newest_first_with_the_cursor_marked() {
        let (mut ui, mut s) = editing();
        twist(&mut ui, &mut s, 0.3);
        edit(&mut ui, &mut s, |d| d.resize(by_kind(d, Kind::Engine), d.part(by_kind(d, Kind::Engine)).unwrap().volume_m3 * 0.5));
        apply(Action::Undo, &mut ui, &mut s);
        let rows = rows(&ui.form);
        let reached: Vec<(Reached, usize)> = rows.iter().map(|(_, r, to)| (*r, *to)).collect();
        assert_eq!(reached, vec![(Reached::Ahead, 2), (Reached::At, 1), (Reached::Behind, 0)]);
        assert!(rows[0].0.starts_with("resized 2 engine, ") && rows[0].0.ends_with(" m3"), "{}", rows[0].0);
        assert_eq!(rows[1].0, "moved 4 living");
    }
}
