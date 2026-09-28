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
        menu.text_field(row, "", lc_proto::form::PRESET_NAME_LIMIT, NameField);
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

/// [`Effect::Copy`].
pub fn copy(clipboard: Option<&mut bevy_egui::EguiClipboard>, text: &str) -> Option<String> {
    match clipboard {
        Some(clipboard) => {
            clipboard.set_text(text);
            None
        }
        None => Some("there is no clipboard here".into()),
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
