//! Apply, on the editor's title bar after Back: the button, the live budget beside it, why it
//! cannot be pressed, the shard's refusal, the round under way once it is accepted, and the second
//! question when the round would collapse the field. Bevy UI in [`em_ui`]'s widgets, as the rest of the
//! editor's controls are. The ledger itself is the refit window's.
//!
//! Also keeps the draft edited against the right form of the ship's; see [`crate::ledger::base`].

use bevy::prelude::*;
use em_ui::{MenuTheme, MenuUi};

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::ledger::{self, Blocked};
use crate::preview::Preview;
use crate::ui::ViewMode;

const TEXT: f32 = 14.0;

pub struct FormApplyPlugin;

impl Plugin for FormApplyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                follow_ship,
                (lay_out, show_standing, show_budget, ask)
                    .chain()
                    .in_set(crate::app::Stage::Scene)
                    .after(crate::form_view::place)
                    .after(crate::form_preview::refresh),
            )
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

#[derive(Component)]
struct BudgetFigure(usize);

/// Apply's second question, and its answers.
#[derive(Component)]
struct Asking;

#[derive(Component, Clone, Copy)]
pub(crate) struct Answer(bool);

/// The live budget as it reads beside Apply: what is available, what is spent, the peak in storage,
/// and what is vented, if anything. Always four, so they are written in place; a round that
/// cannot be planned says why in the first.
pub fn budget_figures(preview: &Preview, module_j: f64) -> [String; 4] {
    use lc_world::refit::rounds::Refusal;
    // Plus zero, so nothing spent reads 0 rather than -0.
    let me = |j: f64| format!("{} ME", crate::draft::figure(j / module_j + 0.0));
    let budget = match &preview.budget {
        Ok(budget) => budget,
        Err(refusal) => {
            let why = match *refusal {
                Refusal::Energy { short_j } => format!("short by {}", me(short_j)),
                Refusal::Form(fault) => fault.to_string(),
                Refusal::NoDrones { part } => format!("no drones left to work on {part}"),
                Refusal::Mind(part) => format!("{part} is not this ship's Mind"),
            };
            return [why, String::new(), String::new(), String::new()];
        }
    };
    // Red when it would collapse the field; Apply's second question says the rest.
    let vent = if budget.vented_j > 0.0 { format!("vents {}", me(budget.vented_j)) } else { String::new() };
    [
        format!("available {}", me(budget.available_j)),
        format!("spent {}", me(budget.spent_j)),
        format!("peak {} of {}", me(budget.peak_j), me(budget.peak_capacity_j)),
        vent,
    ]
}

fn follow_ship(ui: Res<Ui>, game: Res<crate::app::Game>, mut out: MessageWriter<Requested>) {
    let (Some(draft), Some(fitting)) = (ui.form.draft.as_ref(), game.0.ship.fitting()) else { return };
    let base = ledger::base(fitting);
    if !draft.is_based_on(base) {
        out.write(Requested(Action::RebaseDraft(base.clone())));
    }
}

fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    game: Res<crate::app::Game>,
    assets: Res<AssetServer>,
    built: Query<(Entity, &Built)>,
    title: Query<Entity, With<crate::form_view::TitleRow>>,
) {
    let draft = ui.form.draft.as_ref().filter(|_| ui.view == ViewMode::Form);
    let (Some(draft), Some(title)) = (draft, title.iter().next()) else {
        for (entity, _) in &built {
            commands.entity(entity).try_despawn();
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
            commands.entity(entity).try_despawn();
        }
    }
    if !current {
        build(&mut commands, title, want, assets.load(crate::faces::UI_FILE));
    }
}

/// Apply and the budget on the title's line, and under them whatever says why.
fn build(commands: &mut Commands, title: Entity, built: Built, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let root = ui.row(title);
    ui.insert(root, (Node { flex_direction: FlexDirection::Column, align_items: AlignItems::FlexStart, ..default() }, built.clone()));
    let row = ui.row(root);
    match built.gate {
        Ok(()) => {
            ui.small_button(row, "Apply", ApplyButton);
        }
        Err(_) => {
            ui.disabled_button(row, "Apply");
        }
    }
    for index in 0..4 {
        let figure = ui.inline(row, "", TEXT, em_ui::vfd::TEXT);
        ui.insert(figure, BudgetFigure(index));
    }
    // A running round says what it is doing instead, on the standing line.
    if let Err(blocked) = built.gate
        && blocked != Blocked::Refitting
    {
        ui.inline(root, blocked.reason(), TEXT, em_ui::vfd::TEXT_DIM);
    }
    if let Some(why) = &built.refusal {
        ui.inline(root, &format!("refused: {why}"), TEXT, em_ui::vfd::TEXT);
    }
    let line = ui.inline(root, "", TEXT, em_ui::vfd::TEXT);
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

fn show_budget(
    previewed: Res<crate::form_preview::Previewed>,
    game: Res<crate::app::Game>,
    mut figures: Query<(&BudgetFigure, &mut Text, &mut TextColor, &mut Node)>,
) {
    let Some(preview) = &previewed.0 else { return };
    let module_j = game.0.ship.fitting().map_or(1.0, |f| f.balance().module_energy_j());
    let said = budget_figures(preview, module_j);
    let hazard = if preview.collapses() { crate::ui::HAZARD } else { em_ui::vfd::TEXT };
    for (figure, mut text, mut color, mut node) in &mut figures {
        let line = &said[figure.0];
        if text.0 != *line {
            text.0.clone_from(line);
        }
        let tone = if figure.0 == 3 { hazard } else { em_ui::vfd::TEXT };
        if color.0 != tone {
            color.0 = tone;
        }
        let display = if line.is_empty() { Display::None } else { Display::Flex };
        if node.display != display {
            node.display = display;
        }
    }
}

fn ask(
    mut commands: Commands,
    ui: Res<Ui>,
    previewed: Res<crate::form_preview::Previewed>,
    game: Res<crate::app::Game>,
    assets: Res<AssetServer>,
    shown: Query<Entity, With<Asking>>,
) {
    let asked = ui.form.draft.as_ref().is_some_and(|d| ui.form.asking.as_ref() == Some(&d.form));
    let wanted = asked && ui.view == ViewMode::Form;
    if !wanted {
        for entity in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }
    if !shown.is_empty() {
        return;
    }
    let module_j = game.0.ship.fitting().map_or(1.0, |f| f.balance().module_energy_j());
    let said = match previewed.0.as_ref().and_then(|p| Some((p.budget.as_ref().ok()?, p.heat?))) {
        Some((budget, heat)) => format!(
            "This refit vents {} ME into the field, taking it to {:.0} K, past {:.0} K where it fails.",
            crate::draft::figure(budget.vented_j / module_j),
            heat.peak_k,
            heat.max_k,
        ),
        None => "This refit collapses the field.".into(),
    };
    let mut menu = MenuUi::new(&mut commands, MenuTheme::VFD).panel_width(460.0).font(assets.load(crate::faces::UI_FILE)).warning(crate::ui::HAZARD);
    menu.confirm(Asking, "COLLAPSE THE FIELD?", &said, ("Apply anyway", Answer(true)), ("Back", Answer(false)));
}

pub(crate) fn press(
    buttons: Query<&Interaction, (Changed<Interaction>, With<ApplyButton>)>,
    answers: Query<(&Interaction, &Answer), Changed<Interaction>>,
    carried: Res<crate::form_carry::Carried>,
    mut out: MessageWriter<Requested>,
) {
    if carried.is_carrying() {
        return;
    }
    if let Some((_, answer)) = answers.iter().find(|(i, _)| **i == Interaction::Pressed) {
        out.write(Requested(Action::ApplyPastCollapse(answer.0)));
    } else if buttons.iter().any(|i| *i == Interaction::Pressed) {
        out.write(Requested(Action::ApplyDraft));
    }
}

fn put_away(mut commands: Commands, built: Query<Entity, Or<(With<Built>, With<Asking>)>>) {
    for entity in &built {
        commands.entity(entity).try_despawn();
    }
}

#[cfg(test)]
mod tests {
    use lc_world::fitting::{Balance, Fitting};
    use lc_world::form::{Form, Kind};
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::draft::Draft;

    fn previewed(draft: Draft, stored_me: f64) -> (Preview, f64) {
        let b = Balance::DEFAULT;
        let mut s = crate::session::Session::new(&AuthoredStars::sample(), 3);
        s.ship.fit(Some(Fitting::full(Form::starting(), b, 0.0)));
        let full = s.ship.fitting().unwrap().capacity_j_at(0.0);
        s.ship.drain(full - stored_me * b.module_energy_j(), 0.0);
        let mut ui = crate::ui::UiState::default();
        ui.form.draft = Some(draft);
        (Preview::of(&s, &ui, None).unwrap(), b.module_energy_j())
    }

    fn resized(kind: Kind, by: f64) -> Draft {
        let mut d = Draft::new(Form::starting());
        let part = *d.form.parts.iter().find(|p| p.kind == kind).unwrap();
        d.apply(&d.resize(part.id, part.volume_m3 * by).unwrap(), &Balance::DEFAULT).unwrap();
        d
    }

    #[test]
    fn the_budget_says_what_is_vented_and_a_shortfall() {
        let (unchanged, me) = previewed(Draft::new(Form::starting()), 30.0);
        assert_eq!(budget_figures(&unchanged, me)[1], "spent 0 ME");
        assert_eq!(budget_figures(&unchanged, me)[3], "", "nothing vented, nothing said");
        let (vent, me) = previewed(resized(Kind::Storage, 1.0 / 3.0), 30.0);
        let said = budget_figures(&vent, me);
        assert!(vent.collapses() && said[3].starts_with("vents ") && said[3].ends_with(" ME"), "{said:?}");
        let (short, me) = previewed(resized(Kind::Engine, 2.0), 0.0);
        let said = budget_figures(&short, me);
        assert!(said[0].starts_with("short by ") && said[1..].iter().all(String::is_empty), "{said:?}");
    }
}
