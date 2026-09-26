//! The editor's preview panel, and the one [`Preview`] a frame that it and the budget beside Apply
//! both read. The draft's geometry is measured off the frame whenever the draft changes, since a
//! grid takes tens of milliseconds, and a drag would stutter waiting for it; until it arrives the
//! panel keeps the figures it had.

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_ui::{Edge, MenuTheme, MenuUi};
use lc_world::fitting::Balance;
use lc_world::form::Form;

use crate::app::Ui;
use crate::draft::figure;
use crate::preview::{Measured, Preview};
use crate::ui::ViewMode;

const WIDTH: f32 = 250.0;
const INSET: f32 = 12.0;
const TEXT: f32 = 13.0;

#[derive(Resource, Default)]
pub struct Previewed(pub Option<Preview>);

#[derive(Resource, Default)]
struct Measuring {
    done: Option<Measured>,
    /// The last form sent to be measured, so one that cannot be is not tried every frame.
    asked: Option<Form>,
    running: Option<Task<Option<Measured>>>,
}

pub struct FormPreviewPlugin;

impl Plugin for FormPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Previewed>().init_resource::<Measuring>().add_systems(
            Update,
            (measure, refresh, lay_out, show)
                .chain()
                .in_set(crate::app::Stage::Scene)
                .after(crate::form_view::place)
                .run_if(in_state(crate::app::AppState::InGame)),
        );
    }
}

fn measure(ui: Res<Ui>, game: Res<crate::app::Game>, mut measuring: ResMut<Measuring>) {
    let Some(draft) = ui.form.draft.as_ref().filter(|_| ui.view == ViewMode::Form) else { return };
    if let Some(task) = measuring.running.as_mut() {
        let Some(result) = check_ready(task) else { return };
        measuring.running = None;
        if let Some(measured) = result {
            measuring.done = Some(measured);
        }
    }
    if measuring.asked.as_ref() == Some(&draft.form) {
        return;
    }
    let form = draft.form.clone();
    let balance = game.0.ship.fitting().map_or(Balance::DEFAULT, |f| *f.balance());
    measuring.asked = Some(form.clone());
    measuring.running = Some(AsyncComputeTaskPool::get().spawn(async move { Measured::of(&form, &balance).ok() }));
}

pub fn refresh(ui: Res<Ui>, game: Res<crate::app::Game>, measuring: Res<Measuring>, mut previewed: ResMut<Previewed>) {
    previewed.0 = (ui.view == ViewMode::Form).then(|| Preview::of(&game.0, &ui.0, measuring.done.as_ref())).flatten();
}

/// The preview's rows: always the same labels in the same order, so the panel is built once and
/// its figures written in place. `None` for a figure that waits on the grid.
pub fn rows(preview: &Preview, module_j: f64) -> Vec<(&'static str, Option<String>)> {
    let me = |j: f64| format!("{} ME", figure(j / module_j));
    let per_year = |w: f64| format!("{} ME/yr", figure(w * lc_world::flight::JULIAN_YEAR_S / module_j));
    let c = &preview.capacities;
    let shape = preview.shape;
    vec![
        ("storage", Some(me(c.storage_j))),
        ("building", Some(per_year(c.building_w))),
        ("drain", Some(per_year(c.drain_w))),
        ("data", Some(format!("{} MB", figure(c.data_b / 1.0e6)))),
        ("acceleration, full", Some(format!("{} g", figure(preview.accel_g)))),
        ("broadside", shape.map(|s| format!("{} m2", figure(s.broadside_m2)))),
        ("envelope", shape.map(|s| format!("{} m2", figure(s.envelope_m2)))),
        ("flip", shape.map(|s| format!("{:.0} s", lc_world::attitude::flip_time_s(s.slew_rad_s)))),
        ("rated load", shape.map(|s| per_year(s.rated_load_w))),
        ("headroom", shape.map(|s| me(s.headroom_j))),
        ("starlight here", shape.map(|s| per_year(s.starlight_w))),
        ("round", Some(preview.duration_s.map_or_else(|| "none planned".into(), crate::ledger::span))),
    ]
}

#[derive(Component)]
struct PreviewPanel;

#[derive(Component)]
struct Row(usize);

fn lay_out(mut commands: Commands, previewed: Res<Previewed>, game: Res<crate::app::Game>, assets: Res<AssetServer>, panels: Query<Entity, With<PreviewPanel>>) {
    let module_j = game.0.ship.fitting().map_or(Balance::DEFAULT, |f| *f.balance()).module_energy_j();
    match (&previewed.0, panels.iter().next()) {
        (None, _) => {
            for entity in &panels {
                commands.entity(entity).despawn();
            }
        }
        (Some(preview), None) => {
            let mut ui = MenuUi::new(&mut commands, MenuTheme::VFD).font(assets.load(crate::faces::UI_FILE));
            let root = ui.docked(PreviewPanel, Edge::Left, INSET);
            ui.insert(root, Node { position_type: PositionType::Absolute, left: Val::Px(INSET), bottom: Val::Px(INSET), width: Val::Px(WIDTH), flex_direction: FlexDirection::Column, align_items: AlignItems::Stretch, ..default() });
            let panel = ui.strip(root);
            ui.insert(panel, Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Stretch, padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)), border: UiRect::all(Val::Px(1.0)), row_gap: Val::Px(2.0), ..default() });
            ui.inline(panel, "PREVIEW", 15.0, em_ui::vfd::TEXT);
            for (index, (label, _)) in rows(preview, module_j).into_iter().enumerate() {
                let row = ui.row(panel);
                ui.insert(row, Node { flex_direction: FlexDirection::Row, justify_content: JustifyContent::SpaceBetween, ..default() });
                ui.inline(row, label, TEXT, em_ui::vfd::TEXT_DIM);
                let figure = ui.inline(row, "", TEXT, em_ui::vfd::TEXT);
                ui.insert(figure, Row(index));
            }
        }
        (Some(_), Some(_)) => {}
    }
}

fn show(previewed: Res<Previewed>, game: Res<crate::app::Game>, mut figures: Query<(&Row, &mut Text)>) {
    let Some(preview) = &previewed.0 else { return };
    let module_j = game.0.ship.fitting().map_or(Balance::DEFAULT, |f| *f.balance()).module_energy_j();
    let rows = rows(preview, module_j);
    for (row, mut text) in &mut figures {
        if let Some((_, Some(value))) = rows.get(row.0)
            && text.0 != *value
        {
            text.0.clone_from(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_world::fitting::Fitting;
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::draft::Draft;

    #[test]
    fn the_rows_keep_their_places_while_the_grid_is_measured() {
        let b = Balance::DEFAULT;
        let mut s = crate::session::Session::new(&AuthoredStars::sample(), 3);
        s.ship.fit(Some(Fitting::full(Form::starting(), b, 0.0)));
        let mut ui = crate::ui::UiState::default();
        ui.form.draft = Some(Draft::new(Form::starting()));
        let waiting = rows(&Preview::of(&s, &ui, None).unwrap(), b.module_energy_j());
        let measured = Measured::of(&Form::starting(), &b).unwrap();
        let done = rows(&Preview::of(&s, &ui, Some(&measured)).unwrap(), b.module_energy_j());
        let labels = |r: &[(&str, Option<String>)]| r.iter().map(|(l, _)| *l).collect::<Vec<_>>();
        assert_eq!(labels(&waiting), labels(&done));
        assert!(waiting.iter().any(|(_, v)| v.is_none()) && done.iter().all(|(_, v)| v.is_some()));
        assert_eq!(done[0].1.as_deref(), Some("25 ME"));
    }
}
