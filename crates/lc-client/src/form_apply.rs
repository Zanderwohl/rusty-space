//! Apply, in the editor's chrome: the button, why it cannot be pressed, the shard's refusal beside
//! it, and the round under way once it is accepted. Bevy UI in [`em_ui`]'s widgets, as the rest of
//! the editor's controls are. The ledger itself is the refit window's.
//!
//! Also keeps the draft edited against the right form of the ship's; see [`crate::ledger::base`].

use bevy::prelude::*;
use em_ui::{Edge, MenuTheme, MenuUi};

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::ledger::{self, Blocked};
use crate::ui::ViewMode;

const INSET: f32 = 12.0;
const TEXT: f32 = 14.0;

pub struct FormApplyPlugin;

impl Plugin for FormApplyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (follow_ship, (lay_out, show_standing).chain().in_set(crate::app::Stage::Scene).after(crate::form_view::place))
                .run_if(in_state(crate::app::AppState::InGame)),
        )
        .add_systems(OnExit(crate::app::AppState::InGame), put_away);
    }
}

/// What the strip is built for. The round's standing is written in place, since it moves every
/// frame and a rebuilt button never shows a hover.
#[derive(Component, Clone, Debug, PartialEq)]
struct Built {
    gate: Result<(), Blocked>,
    refusal: Option<String>,
}

#[derive(Component)]
pub(crate) struct ApplyButton;

#[derive(Component)]
struct StandingLine;

fn follow_ship(ui: Res<Ui>, game: Res<crate::app::Game>, mut out: MessageWriter<Requested>) {
    let (Some(draft), Some(fitting)) = (ui.form.draft.as_ref(), game.0.ship.fitting()) else { return };
    let mut base = ledger::base(fitting).clone();
    base.parts.sort_by_key(|p| p.id);
    if base != draft.ship {
        out.write(Requested(Action::RebaseDraft(base)));
    }
}

fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    game: Res<crate::app::Game>,
    assets: Res<AssetServer>,
    built: Query<(Entity, &Built)>,
) {
    let draft = ui.form.draft.as_ref().filter(|_| ui.view == ViewMode::Form);
    let Some(draft) = draft else {
        for (entity, _) in &built {
            commands.entity(entity).despawn();
        }
        return;
    };
    let want = Built {
        gate: ledger::gate(draft, &ui.form.applying, ledger::Situation::of(&game.0)),
        refusal: ui.form.applying.refusal(draft).map(str::to_owned),
    };
    let mut current = false;
    for (entity, was) in &built {
        if *was == want && !current {
            current = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    if !current {
        build(&mut commands, want, assets.load(crate::faces::UI_FILE));
    }
}

fn build(commands: &mut Commands, built: Built, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let root = ui.docked(built.clone(), Edge::Bottom, INSET);
    let strip = ui.strip(root);
    let row = ui.row(strip);
    match built.gate {
        Ok(()) => {
            ui.small_button(row, "Apply", ApplyButton);
        }
        Err(blocked) => {
            ui.disabled_button(row, "Apply");
            // A running round says what it is doing instead, on the line below.
            if blocked != Blocked::Refitting {
                ui.inline(row, blocked.reason(), TEXT, em_ui::vfd::TEXT_DIM);
            }
        }
    }
    if let Some(why) = &built.refusal {
        ui.inline(strip, &format!("refused: {why}"), TEXT, crate::draft::Mark::Dismantle.color());
    }
    let line = ui.inline(strip, "", TEXT, em_ui::vfd::TEXT);
    ui.insert(line, StandingLine);
}

fn show_standing(game: Res<crate::app::Game>, mut lines: Query<(&mut Text, &mut Node), With<StandingLine>>) {
    let now = game.0.coordinate_time_s();
    let running = game.0.ship.fitting().and_then(|f| f.refit()).filter(|_| game.0.ship.is_refitting(now));
    let said = running.map(|plan| ledger::standing(plan, now).line()).unwrap_or_default();
    for (mut text, mut node) in &mut lines {
        if text.0 != said {
            text.0.clone_from(&said);
        }
        let display = if said.is_empty() { Display::None } else { Display::Flex };
        if node.display != display {
            node.display = display;
        }
    }
}

pub(crate) fn press(
    buttons: Query<&Interaction, (Changed<Interaction>, With<ApplyButton>)>,
    carried: Res<crate::form_carry::Carried>,
    mut out: MessageWriter<Requested>,
) {
    if carried.is_carrying() {
        return;
    }
    if buttons.iter().any(|i| *i == Interaction::Pressed) {
        out.write(Requested(Action::ApplyDraft));
    }
}

fn put_away(mut commands: Commands, built: Query<Entity, With<Built>>) {
    for entity in &built {
        commands.entity(entity).despawn();
    }
}
