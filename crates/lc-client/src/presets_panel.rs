//! The editor's presets: the built-ins and the account's own, saved from the draft by name,
//! applied to it as a layout or a design, deleted, and carried between accounts as RON on the
//! clipboard. See `lightcone/docs/29-ship-form.md` §Your own presets.
//!
//! An account's list is the shard's, as [`crate::session::Session::presets`] last heard it, so
//! saving and deleting only ask. The limits are checked here as well, by the same
//! [`lc_world::form::presets::may_keep`] the shard runs, so a refusal is named before it is sent.

use bevy::prelude::*;
use bevy::text::EditableText;
use em_ui::{MenuTheme, MenuUi};
use lc_world::fitting::Balance;
use lc_world::form::presets::{Builtin, may_keep};
use lc_world::form::{Form, Kind, PartId};

use crate::action::{Action, Effect};
use crate::app::Ui;
use crate::draft::kind_name;
use crate::form_history::Named;
use crate::input::Requested;
use crate::session::Session;
use crate::ui::{UiState, ViewMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    Builtin(Builtin),
    Own(String),
}

impl Chosen {
    fn name(&self) -> &str {
        match self {
            Chosen::Builtin(b) => b.name(),
            Chosen::Own(name) => name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Layout,
    Design,
}

fn form_of(chosen: &Chosen, session: &Session) -> Result<Form, String> {
    match chosen {
        Chosen::Builtin(b) => Ok(b.form()),
        Chosen::Own(name) => session
            .presets
            .iter()
            .find(|p| &p.name == name)
            .map(|p| Form::from(&p.form))
            .ok_or_else(|| format!("there is no preset named {name}")),
    }
}

/// The preset as the clipboard carries it: its name and its form with its part ids.
pub fn export(chosen: &Chosen, session: &Session) -> Result<String, String> {
    let preset = lc_proto::Preset { name: chosen.name().to_owned(), form: (&form_of(chosen, session)?).into() };
    ron::to_string(&preset).map_err(|e| e.to_string())
}

/// A preset read from the clipboard, if the account could keep it.
pub fn import(text: &str, session: &Session) -> Result<lc_proto::Preset, String> {
    let preset: lc_proto::Preset = ron::from_str(text.trim()).map_err(|e| format!("refused: not a preset: {}", e.code))?;
    keepable(&preset.name, &preset.form, session)?;
    Ok(preset)
}

fn keepable(name: &str, form: &lc_proto::Form, session: &Session) -> Result<(), String> {
    may_keep(name, form, session.presets.iter().map(|p| p.name.as_str())).map_err(|r| format!("refused: {}", crate::uplink::refused(r)))
}

/// The draft as `chosen` applied to it, and what the history calls that.
pub fn applied(chosen: &Chosen, how: How, draft: &crate::draft::Draft, session: &Session) -> Result<(Form, Named), String> {
    let preset = form_of(chosen, session)?;
    let min = Balance::DEFAULT.min_part_m3;
    let refused = |e| format!("refused: {e}");
    let (form, note) = match how {
        How::Design => (preset.as_design(min).map_err(refused)?, None),
        How::Layout => {
            let layout = preset.as_layout(&draft.ship, min).map_err(refused)?;
            (layout.form, left_out(&preset, &layout.emptied, &layout.unplaced))
        }
    };
    let how = if how == How::Layout { "a layout" } else { "a design" };
    Ok((form, Named { label: format!("applied {} as {how}", chosen.name()), note }))
}

/// The parts a layout gave nothing, by the preset's ids, and the ship's kinds it had no part for.
pub fn left_out(preset: &Form, emptied: &[PartId], unplaced: &[Kind]) -> Option<String> {
    let parts: Vec<String> = emptied
        .iter()
        .filter_map(|id| preset.parts.iter().find(|p| p.id == *id))
        .map(|p| format!("{} {}", p.id.0, kind_name(p.kind)))
        .collect();
    let kinds: Vec<&str> = unplaced.iter().map(|&k| kind_name(k)).collect();
    let mut said = Vec::new();
    if !parts.is_empty() {
        said.push(format!("given nothing: {}", parts.join(", ")));
    }
    if !kinds.is_empty() {
        said.push(format!("no part for: {}", kinds.join(", ")));
    }
    (!said.is_empty()).then(|| said.join("; "))
}

/// The preset actions, which [`crate::action::apply`] hands here whole.
pub fn act(action: Action, ui: &mut UiState, session: &Session) -> Vec<Effect> {
    let kept = |name: String, form: Option<lc_proto::Form>| match session.remote {
        true => Effect::Keep { name, form },
        false => Effect::Notify("no server, so nowhere to keep presets".into()),
    };
    let said = match action {
        Action::ChoosePreset(chosen) => {
            ui.form.preset = chosen;
            return Vec::new();
        }
        Action::SavePreset(name) => match &ui.form.draft {
            None => Err("there is no draft to save".into()),
            Some(draft) => {
                let form = lc_proto::Form::from(&draft.form);
                keepable(&name, &form, session).map(|()| kept(name, Some(form)))
            }
        },
        Action::DeletePreset(Chosen::Builtin(b)) => Err(format!("{} is built in", b.name())),
        Action::DeletePreset(Chosen::Own(name)) => match session.presets.iter().any(|p| p.name == name) {
            true => Ok(kept(name, None)),
            false => Err(format!("there is no preset named {name}")),
        },
        Action::ExportPreset(chosen) => export(&chosen, session).map(Effect::Copy),
        Action::ImportPreset(text) => import(&text, session).map(|p| kept(p.name, Some(p.form))),
        Action::ApplyPreset(chosen, how) => {
            let Some(draft) = &ui.form.draft else { return vec![Effect::Notify("there is no draft to apply it to".into())] };
            match applied(&chosen, how, draft, session) {
                Ok((form, named)) => {
                    let edit = Ok(draft.replace(form));
                    return crate::form_history::edit(&mut ui.form, session, edit, Some(named)).map(Effect::Notify).into_iter().collect();
                }
                Err(why) => Err(why),
            }
        }
        _ => return Vec::new(),
    };
    vec![said.unwrap_or_else(Effect::Notify)]
}

/// `--preset`'s spelling: a built-in's name, any case, a colon, and `layout` or `design`.
pub fn staged(spec: &str) -> Option<(Builtin, How)> {
    let (name, how) = spec.split_once(':')?;
    let builtin = Builtin::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(name))?;
    let how = match how {
        "layout" => How::Layout,
        "design" => How::Design,
        _ => return None,
    };
    Some((builtin, how))
}

/// A panel button.
#[derive(Component, Clone, Debug, PartialEq)]
pub enum Press {
    Choose(Chosen),
    Save,
    Apply(How),
    Export,
    Delete,
    Import,
    Fold,
}

#[derive(Component)]
pub struct NameField;

/// What the panel was built for.
#[derive(Component, PartialEq)]
struct Built(Vec<String>);

fn rows(session: &Session) -> Vec<Chosen> {
    let own = session.presets.iter().map(|p| Chosen::Own(p.name.clone()));
    Builtin::ALL.into_iter().map(Chosen::Builtin).chain(own).collect()
}

pub struct PresetsPanelPlugin;

impl Plugin for PresetsPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            lay_out
                .in_set(crate::app::Stage::Scene)
                .after(crate::form_panel::lay_out)
                .before(crate::form_history::lay_out)
                .run_if(in_state(crate::app::AppState::InGame)),
        );
        #[cfg(target_arch = "wasm32")]
        app.init_resource::<Pasted>().add_systems(Update, take_paste.before(crate::app::dispatch));
    }
}

/// In the left column under the palette, which is its first child whenever it is rebuilt.
fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    game: Res<crate::app::Game>,
    assets: Res<AssetServer>,
    panels: Query<(Entity, &Built)>,
    names: Query<&EditableText, With<NameField>>,
    columns: Query<Entity, With<crate::form_panel::LeftColumn>>,
) {
    let form = &ui.form;
    let column = columns.iter().next().filter(|_| ui.view == ViewMode::Form && form.draft.is_some());
    let rows = rows(&game.0);
    let chosen = form.preset.clone().filter(|c| rows.contains(c));
    let note = form.history.current().and_then(|e| e.note.clone());
    let open = !form.folded.presets;
    let mut key: Vec<String> = rows.iter().map(|c| format!("{c:?}")).collect();
    key.push(format!("{chosen:?} {note:?} {open}"));
    let key = Built(key);
    let mut current = false;
    for (panel, built) in &panels {
        if column.is_some() && *built == key {
            current = true;
        } else {
            commands.entity(panel).try_despawn();
        }
    }
    let (Some(column), false) = (column, current) else { return };
    // A rebuild, such as choosing a row, keeps what was typed.
    let typed = names.iter().next().map(|e| e.value().to_string()).unwrap_or_default();
    let mut menu = MenuUi::new(&mut commands, MenuTheme::VFD).font(assets.load(crate::faces::UI_FILE));
    let panel = menu.strip(column);
    let node = Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Stretch,
        padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
        border: UiRect::all(Val::Px(1.0)),
        row_gap: Val::Px(2.0),
        ..default()
    };
    menu.insert(panel, (key, node));
    menu.fold_heading(panel, "PRESETS", open, Press::Fold);
    if let Some(note) = &note {
        menu.inline(panel, note, 13.0, em_ui::vfd::TEXT);
    }
    if open {
        let row = menu.row(panel);
        menu.insert(row, Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() });
        menu.text_field(row, &typed, lc_proto::form::PRESET_NAME_LIMIT, NameField);
        menu.small_button(row, "save", Press::Save);
        for row in rows {
            let here = chosen.as_ref() == Some(&row);
            let text = if here { format!("> {}", row.name()) } else { row.name().to_owned() };
            menu.tree_row(panel, 0, &text, here, Press::Choose(row));
        }
        if let Some(chosen) = &chosen {
            let row = menu.row(panel);
            menu.small_button(row, "as layout", Press::Apply(How::Layout));
            menu.small_button(row, "as design", Press::Apply(How::Design));
            let row = menu.row(panel);
            menu.small_button(row, "export", Press::Export);
            if matches!(chosen, Chosen::Own(_)) {
                menu.small_button(row, "delete", Press::Delete);
            }
        }
        let row = menu.row(panel);
        menu.small_button(row, "import", Press::Import);
    }
    commands.entity(column).insert_child(1, panel);
}

/// A button's action, or `None` for one that needs more than the interface state.
fn action_of(press: &Press, ui: &UiState) -> Option<Action> {
    let chosen = ui.form.preset.clone();
    Some(match press {
        Press::Choose(c) => Action::ChoosePreset((chosen.as_ref() != Some(c)).then(|| c.clone())),
        Press::Apply(how) => Action::ApplyPreset(chosen?, *how),
        Press::Export => Action::ExportPreset(chosen?),
        Press::Delete => Action::DeletePreset(chosen?),
        Press::Fold => Action::Fold(crate::form_view::Fold::Presets),
        Press::Save | Press::Import => return None,
    })
}

pub fn press(
    ui: Res<Ui>,
    carried: Res<crate::form_carry::Carried>,
    buttons: Query<(&Interaction, &Press), Changed<Interaction>>,
    names: Query<&EditableText, With<NameField>>,
    mut entered: MessageReader<em_ui::Entered>,
    #[cfg(not(target_arch = "wasm32"))] mut clipboard: Option<ResMut<bevy_egui::EguiClipboard>>,
    #[cfg(target_arch = "wasm32")] pasted: Res<Pasted>,
    mut out: MessageWriter<Requested>,
) {
    for said in entered.read() {
        if names.contains(said.field) {
            out.write(Requested(Action::SavePreset(said.text.trim().to_owned())));
        }
    }
    if carried.is_carrying() || carried.just_dropped() {
        return;
    }
    for (interaction, press) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match press {
            Press::Save => {
                let name = names.iter().next().map(|e| e.value().to_string()).unwrap_or_default();
                out.write(Requested(Action::SavePreset(name.trim().to_owned())));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Press::Import => {
                let text = clipboard.as_mut().and_then(|c| c.get_text()).unwrap_or_default();
                out.write(Requested(Action::ImportPreset(text)));
            }
            #[cfg(target_arch = "wasm32")]
            Press::Import => pasted.ask(),
            other => {
                if let Some(action) = action_of(other, &ui.0) {
                    out.write(Requested(action));
                }
            }
        }
    }
}

/// [`Effect::Copy`], and what to tell the player.
pub fn copy(clipboard: Option<&mut bevy_egui::EguiClipboard>, text: &str) -> String {
    match clipboard {
        Some(clipboard) => {
            clipboard.set_text(text);
            "copied to clipboard".into()
        }
        None => "there is no clipboard here".into(),
    }
}

/// The browser's clipboard, read on the button's press, which is the gesture it asks for. The
/// answer comes later.
#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
pub struct Pasted(std::sync::Arc<std::sync::Mutex<Option<Option<String>>>>);

#[cfg(target_arch = "wasm32")]
impl Pasted {
    fn ask(&self) {
        let slot = self.0.clone();
        let Some(read) = web_sys::window().map(|w| w.navigator().clipboard().read_text()) else { return };
        wasm_bindgen_futures::spawn_local(async move {
            let text = wasm_bindgen_futures::JsFuture::from(read).await.ok().and_then(|v| v.as_string());
            *slot.lock().unwrap() = Some(text);
        });
    }
}

/// A refused read falls back on the last text pasted into the page.
#[cfg(target_arch = "wasm32")]
fn take_paste(pasted: Res<Pasted>, mut clipboard: Option<ResMut<bevy_egui::EguiClipboard>>, mut out: MessageWriter<Requested>) {
    let Some(text) = pasted.0.lock().unwrap().take() else { return };
    let text = text.or_else(|| clipboard.as_mut().and_then(|c| c.get_text())).unwrap_or_default();
    out.write(Requested(Action::ImportPreset(text)));
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::Refusal;
    use lc_proto::form::{MAX_PRESETS, PRESET_NAME_LIMIT};
    use lc_server::presets::Presets;
    use lc_world::form::{MAX_PARTS, Part};

    use super::*;
    use crate::action::apply;
    use crate::draft::{Draft, PRIMITIVES};

    const B: Balance = Balance::DEFAULT;

    struct Player {
        account: &'static str,
        ui: UiState,
        session: Session,
    }

    impl Player {
        fn new(account: &'static str, ship: Form) -> Self {
            let mut session = Session::new(&lc_world::sky::AuthoredStars::sample(), 3);
            session.remote = true;
            let mut ui = UiState::default();
            apply(Action::StartDraft(ship), &mut ui, &mut session);
            Player { account, ui, session }
        }

        fn draft(&self) -> &Draft {
            self.ui.form.draft.as_ref().unwrap()
        }

        /// The action's effects, with each [`Effect::Keep`] carried to the shard and its list back.
        fn does(&mut self, action: Action, shard: &mut Presets) -> Vec<Effect> {
            let effects = apply(action, &mut self.ui, &mut self.session);
            for effect in &effects {
                if let Effect::Keep { name, form } = effect {
                    match form {
                        Some(form) => shard.save(self.account, name.clone(), form.clone()).expect("the shard keeps it"),
                        None => assert!(shard.delete(self.account, name)),
                    }
                    self.session.presets = shard.for_account(self.account);
                }
            }
            effects
        }

        fn edit(&mut self, make: impl Fn(&Draft) -> Result<crate::draft::Edit, crate::draft::Refused>) {
            let edit = make(self.draft());
            apply(Action::EditForm(edit), &mut self.ui, &mut self.session);
        }
    }

    fn id_of(form: &Form, kind: Kind) -> PartId {
        form.parts.iter().find(|p| p.kind == kind).unwrap().id
    }

    /// The starting ship with more engine than storage, so a layout of it is not any preset's
    /// own volumes.
    fn odd_ship() -> Form {
        let mut ship = Form::starting();
        for part in ship.parts.iter_mut().filter(|p| p.placement.is_some()) {
            part.volume_m3 *= if part.kind == Kind::Engine { 3.0 } else { 2.0 };
        }
        ship
    }

    /// A draft worth sharing: reshaped, resized, and a bay added, so ids are not in a fresh order.
    fn designed(player: &mut Player) {
        player.edit(|d| d.twist(id_of(&d.form, Kind::Living), 0.3));
        player.edit(|d| d.resize(id_of(&d.form, Kind::Engine), d.part(id_of(&d.form, Kind::Engine)).unwrap().volume_m3 * 0.75));
        player.edit(|d| d.add(id_of(&d.form, Kind::Storage), Kind::Bay, PRIMITIVES[3], DVec3::NEG_Y, &B));
        player.edit(|d| d.remove(id_of(&d.form, Kind::Data)));
    }

    fn said(effects: &[Effect]) -> Vec<&str> {
        effects.iter().filter_map(|e| if let Effect::Notify(m) = e { Some(m.as_str()) } else { None }).collect()
    }

    fn refusal(reason: Refusal) -> String {
        format!("refused: {}", crate::uplink::refused(reason))
    }

    /// The done-when.
    #[test]
    fn an_exported_preset_imported_on_another_account_applies_to_the_same_draft() {
        let mut shard = Presets::default();
        let mut alice = Player::new("acct-a", Form::starting());
        designed(&mut alice);
        let saved = alice.draft().form.clone();
        alice.does(Action::SavePreset("Long".into()), &mut shard);
        assert_eq!(alice.session.presets.len(), 1);
        let long = Chosen::Own("Long".into());
        let copied = alice.does(Action::ExportPreset(long.clone()), &mut shard);
        let [Effect::Copy(text)] = copied.as_slice() else { panic!("{copied:?}") };

        let mut bob = Player::new("acct-b", odd_ship());
        assert!(shard.for_account("acct-b").is_empty());
        bob.does(Action::ImportPreset(format!("\n  {text}\n")), &mut shard);
        assert_eq!(bob.session.presets, alice.session.presets, "the same name and form, ids and all");

        let mut carol = Player::new("acct-a", odd_ship());
        carol.session.presets = shard.for_account("acct-a");
        for how in [How::Layout, How::Design] {
            carol.does(Action::ApplyPreset(long.clone(), how), &mut shard);
            bob.does(Action::ApplyPreset(long.clone(), how), &mut shard);
            assert_eq!(bob.draft().form, carol.draft().form, "{how:?}");
        }
        assert_eq!(bob.draft().form, saved, "as a design it is the draft that was saved");
    }

    #[test]
    fn applying_is_one_entry_named_for_the_preset_and_undo_restores_the_draft() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", odd_ship());
        designed(&mut player);
        let before = player.draft().form.clone();
        let done = player.ui.form.history.entries().len();
        player.does(Action::ApplyPreset(Chosen::Builtin(Builtin::Plate), How::Layout), &mut shard);
        let history = &player.ui.form.history;
        assert_eq!(history.entries().len(), done + 1);
        assert_eq!(history.current().unwrap().label, "applied Plate as a layout");
        assert_ne!(player.draft().form, before);
        player.does(Action::Undo, &mut shard);
        assert_eq!(player.draft().form, before);
        player.does(Action::Redo, &mut shard);
        player.does(Action::ApplyPreset(Chosen::Builtin(Builtin::Spindle), How::Design), &mut shard);
        assert_eq!(player.ui.form.history.current().unwrap().label, "applied Spindle as a design");
    }

    #[test]
    fn a_layout_and_a_design_are_as_layout_and_as_design() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", odd_ship());
        let min = B.min_part_m3;
        for b in Builtin::ALL {
            let chosen = Chosen::Builtin(b);
            player.does(Action::ApplyPreset(chosen.clone(), How::Layout), &mut shard);
            assert_eq!(player.draft().form, b.form().as_layout(&odd_ship(), min).unwrap().form, "{b:?}");
            player.does(Action::ApplyPreset(chosen, How::Design), &mut shard);
            assert_eq!(player.draft().form, b.form().as_design(min).unwrap(), "{b:?}");
        }
        assert_ne!(Builtin::Plate.form().as_layout(&odd_ship(), min).unwrap().form, Builtin::Plate.form());
    }

    /// No storage, and a bay the spindle has no part for.
    #[test]
    fn a_layout_names_the_parts_it_gave_nothing_and_the_kinds_it_had_no_part_for() {
        let mut ship = Form::starting();
        ship.parts.retain(|p| p.kind != Kind::Storage);
        for placement in ship.parts.iter_mut().filter_map(|p| p.placement.as_mut()) {
            placement.parent = PartId(0);
        }
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", ship);
        player.does(Action::ApplyPreset(Chosen::Builtin(Builtin::Cluster), How::Layout), &mut shard);
        let note = player.ui.form.history.current().unwrap().note.clone();
        assert_eq!(note.as_deref(), Some("given nothing: 1 storage, 11 storage"));

        player.edit(|d| d.add(id_of(&d.form, Kind::Engine), Kind::Bay, PRIMITIVES[3], DVec3::Y, &B));
        let bay = player.draft().form.clone();
        player.ui.form.draft = Some(Draft::new(bay));
        player.does(Action::ApplyPreset(Chosen::Builtin(Builtin::Spindle), How::Layout), &mut shard);
        let note = player.ui.form.history.current().unwrap().note.clone();
        assert_eq!(note.as_deref(), Some("given nothing: 1 storage; no part for: bay"));
        player.does(Action::Undo, &mut shard);
        assert_eq!(player.ui.form.history.current().and_then(|e| e.note.clone()), None, "the note is the entry's");
    }

    #[test]
    fn saving_under_a_kept_name_replaces_it() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", Form::starting());
        player.does(Action::SavePreset("Mine".into()), &mut shard);
        designed(&mut player);
        player.does(Action::SavePreset("Mine".into()), &mut shard);
        let kept = &player.session.presets;
        assert_eq!(kept.len(), 1);
        assert_eq!(Form::from(&kept[0].form), player.draft().form);
    }

    fn full(player: &mut Player) {
        let form = lc_proto::Form::from(&Form::starting());
        player.session.presets = (0..MAX_PRESETS).map(|k| lc_proto::Preset { name: format!("p{k}"), form: form.clone() }).collect();
    }

    #[test]
    fn each_limit_is_refused_by_name_on_save_and_on_import() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", Form::starting());
        let export = |name: &str, form: &Form| ron::to_string(&lc_proto::Preset { name: name.into(), form: form.into() }).unwrap();
        let starting = Form::starting();
        let mut huge = starting.clone();
        let spare = *huge.parts.last().unwrap();
        huge.parts.extend((0..MAX_PARTS as u16).map(|k| Part { id: PartId(1000 + k), ..spare }));

        let long = "x".repeat(PRESET_NAME_LIMIT + 1);
        for (name, form, why) in [
            ("", &starting, Refusal::PresetName),
            (long.as_str(), &starting, Refusal::PresetName),
            ("Huge", &huge, Refusal::Form(lc_proto::FormFault::TooManyParts { found: huge.parts.len() as u32 })),
        ] {
            let effects = player.does(Action::ImportPreset(export(name, form)), &mut shard);
            assert_eq!(said(&effects), [refusal(why)], "import {name:?}");
            player.ui.form.draft.as_mut().unwrap().form = form.clone();
            let effects = player.does(Action::SavePreset(name.into()), &mut shard);
            assert_eq!(said(&effects), [refusal(why)], "save {name:?}");
        }
        player.ui.form.draft = Some(Draft::new(starting.clone()));

        full(&mut player);
        for action in [Action::SavePreset("new".into()), Action::ImportPreset(export("new", &starting))] {
            assert_eq!(said(&player.does(action, &mut shard)), [refusal(Refusal::TooManyPresets)]);
        }
        let effects = apply(Action::SavePreset("p3".into()), &mut player.ui, &mut player.session);
        assert!(matches!(effects.as_slice(), [Effect::Keep { .. }]), "a full list still replaces: {effects:?}");

        let effects = player.does(Action::ImportPreset("not a preset".into()), &mut shard);
        assert!(said(&effects)[0].starts_with("refused: not a preset"), "{effects:?}");
        assert!(shard.for_account("acct-a").is_empty(), "nothing refused reached the shard");
    }

    #[test]
    fn a_built_in_cannot_be_deleted_and_an_own_one_can() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", Form::starting());
        let effects = player.does(Action::DeletePreset(Chosen::Builtin(Builtin::Plate)), &mut shard);
        assert_eq!(said(&effects), ["Plate is built in"]);
        player.does(Action::SavePreset("Plate".into()), &mut shard);
        player.does(Action::DeletePreset(Chosen::Own("Plate".into())), &mut shard);
        assert!(player.session.presets.is_empty());

        player.ui.form.preset = Some(Chosen::Builtin(Builtin::Plate));
        assert_eq!(action_of(&Press::Delete, &player.ui), Some(Action::DeletePreset(Chosen::Builtin(Builtin::Plate))));
        assert!(rows(&player.session).starts_with(&Builtin::ALL.map(Chosen::Builtin)), "every built-in is listed");
    }

    #[test]
    fn with_no_server_there_is_nowhere_to_keep_one() {
        let mut shard = Presets::default();
        let mut player = Player::new("acct-a", Form::starting());
        player.session.remote = false;
        let effects = player.does(Action::SavePreset("Mine".into()), &mut shard);
        assert_eq!(said(&effects), ["no server, so nowhere to keep presets"]);
        player.does(Action::ApplyPreset(Chosen::Builtin(Builtin::Cluster), How::Layout), &mut shard);
        assert_eq!(player.ui.form.history.current().unwrap().label, "applied Cluster as a layout", "a built-in needs no server");
    }
}
