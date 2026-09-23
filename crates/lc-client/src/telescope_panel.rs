//! The telescope window: what the instrument is doing, what has been detected, and where each
//! belief came from.
//!
//! Everything shown comes from [`lc_world::knowledge`], not the catalog: an undetected star has
//! no row, and one with no parallax has no distance. See `lightcone/docs/22-provenance.md`.

use bevy::prelude::*;
use bevy_egui::egui;
use em_spectra::presets;
use lc_world::knowledge::Distance;
use lc_world::knowledge::conclusion::Kind;
use lc_world::knowledge::survey::Duty;
use lc_world::sky::StarId;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::ask;
use crate::plot::CurvePlot;

pub fn telescope(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &mut Game,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
    plot: &mut CurvePlot,
) {
    ui.label("Band mapping");
    ui.horizontal(|ui| {
        for (i, (name, _)) in presets::all().iter().enumerate() {
            if ui.selectable_label(state.preset == i, *name).clicked() {
                ask(out, Action::SetBandPreset(i));
            }
        }
    });
    ui.separator();

    duty(ui, game, out);
    room(ui, game, out);
    ui.separator();

    let order = known(game);
    let seen = game.knowledge.stars().filter(|(_, b)| b.hops == 0).count();
    ui.label(format!("Known: {} stars, {seen} seen by this ship", order.len()));
    // Only the rows on screen are laid out, and only their labels are written.
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show_rows(ui, row_height, order.len(), |ui, rows| {
            for &(_, id) in order.get(rows).unwrap_or_default() {
                if row(ui, game, id, state.selected == Some(id), row_height).clicked() {
                    ask(out, Action::SelectTarget(Some(id)));
                }
            }
        });
    ui.horizontal(|ui| {
        if ui.button("Look at").clicked() {
            ask(out, Action::LookAtSelected);
        }
        if ui.button("Fly there").clicked() {
            ask(out, Action::FlyTo(None));
        }
        if ui.button("Add to watch").clicked() {
            ask(out, Action::WatchSelected);
        }
    });
    ui.separator();

    details(ui, state, game, draft, out);
    conclusion(ui, game, out);
    ui.separator();
    curve(ui, game, out, plot);
}

fn describe(kind: &Kind) -> String {
    let day = 86_400.0;
    match kind {
        Kind::Quiet => "nothing transiting".into(),
        Kind::Unsearched => "a planet this log could not have found yet".into(),
        Kind::Planet { class, transit } => format!(
            "{}, P = {:.4} ± {:.4} d, depth {:.1e}",
            class.name(),
            transit.period_s / day,
            transit.period_sigma_s / day,
            transit.depth,
        ),
        Kind::Swarm(swarm) => {
            let mut text = format!("a swarm covering {:.2e} ± {:.1e}", swarm.coverage.0, swarm.coverage.1);
            if let Some(t) = swarm.crossing_s {
                text += &format!(", crossings of {:.1} h", t / 3600.0);
            }
            if let (Some(area), Some(au)) = (swarm.element_m2, swarm.semi_major_au) {
                text += &format!(", elements of {:.1e} m² at {au:.2} AU", area);
            }
            text
        }
        Kind::Belts { excess: Some((e, sigma)) } => format!("belts only; thermal glow {e:.1e} ± {sigma:.1e}"),
        Kind::Belts { excess: None } => "belts only".into(),
    }
}

/// Storage taken by this ship's raw logs, and the button that frees it.
fn room(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    let capacity = game
        .ship
        .fitting()
        .map_or(lc_world::fitting::ONBOARD_DATA_BYTES, |f| f.balance.data_capacity(&f.loadout_at(now)));
    let used = game.knowledge.bytes();
    let mb = |bytes: f64| bytes / 1_048_576.0;
    ui.horizontal(|ui| {
        ui.label(format!("Data: {:.2} of {:.2} MB", mb(used), mb(capacity)));
        if used >= capacity {
            ui.colored_label(egui::Color32::from_rgb(230, 150, 60), "Data full.");
        }
        match game.knowledge.analyzing() {
            0 => {
                let button = ui.add_enabled(used > 0.0, egui::Button::new("Analyze"));
                if button.on_hover_text("Read every log into a conclusion and free its room").clicked() {
                    ask(out, Action::Analyze);
                }
            }
            n => {
                ui.weak(format!("Analyzing, {n} logs left"));
            }
        }
    });
}

/// A probability with its evidence, never a verdict: see
/// `lightcone/docs/24-standing-instruments.md`.
fn conclusion(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let Some(id) = described(game).filter(|id| game.knowledge.knows(*id)) else { return };
    let day = 86_400.0;
    let owner = game.knowledge.owner;
    let all = game.knowledge.conclusions(id);
    let believed = game.knowledge.believed(id);
    if let Some(planet) = believed.planet.filter(|p| p.probability >= 0.5) {
        ui.label(format!(
            "Believed: a {} on a {:.2}-day orbit — {:.0}%, from {} log",
            planet.class.name(),
            planet.transit.period_s / day,
            planet.probability * 100.0,
            crate::range::who(planet.observer, owner),
        ));
    }
    for c in all {
        let whose = crate::range::who(c.observer, owner);
        let from = c.from_ly.map_or(String::new(), |p| format!(" from {:.2}, {:.2}, {:.2} ly", p.x, p.y, p.z));
        ui.label(format!("{whose}'s log{from}:"));
        for h in c.transits.iter().chain(&c.populations).filter(|h| h.probability >= 0.005) {
            ui.label(format!("  {} — {:.0}%", describe(&h.kind), h.probability * 100.0));
        }
        let searched = match c.evidence.periods_s {
            Some((lo, hi)) => format!("periods {:.1} to {:.1} d searched", lo / day, hi / day),
            None => "too short to search for a period yet".into(),
        };
        ui.weak(format!(
            "  {} samples in {} bands; {searched}; would have found {:.0}% of the transiting planets the galaxy makes.",
            c.evidence.samples,
            c.evidence.bands.count_ones(),
            c.evidence.completeness * 100.0,
        ));
        if c.discarded_s.is_some() {
            ui.weak("  The log behind it has been thrown away; only the conclusion is left.");
        }
    }
    let mut keep = game.knowledge.retained(id);
    if ui.checkbox(&mut keep, "Keep the raw log").changed() {
        ask(out, Action::RetainRaw(id, keep));
    }
}

fn described(game: &Game) -> Option<StarId> {
    game.described.or(game.pointing)
}

fn duty(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    match &game.observatory.duty {
        Duty::Idle => {
            ui.label("Telescope idle").on_hover_text("Nothing is being learned.");
        }
        Duty::Stare(id) => {
            ui.label(format!("Observing {}", game.name_of(*id)))
                .on_hover_text("The whole exposure on one star.");
        }
        Duty::Sweep(sweep) => {
            let (passes, fraction) = sweep.progress(now);
            ui.label(format!(
                "Sweeping {} fields: pass {}, {:.0}% through",
                sweep.fields(),
                passes + 1,
                fraction * 100.0
            ));
            // Stars are found as a pass reaches their field; parallax needs the ship to have
            // moved between passes.
            ui.weak(format!(
                "one pass every {:.1} hours",
                sweep.pass_s() / 3600.0
            ));
        }
        Duty::Watch {
            targets, dwell_s, ..
        } => {
            ui.label(format!(
                "Watching {} stars, {:.0} s each: every one sampled every {:.1} hours",
                targets.len(),
                dwell_s,
                targets.len() as f64 * dwell_s / 3600.0
            ));
        }
        Duty::Survey { star, .. } => {
            // What this craft has found, not what is there: the arena's count is a list of
            // everything the generator made, and a survey that has detected two bodies must
            // not report two hundred.
            let here = game.system.as_ref().filter(|s| s.star == *star).is_some();
            let bodies = match here {
                true => game.knowledge.bodies_of(*star, game.coordinate_time_s()).len(),
                false => 0,
            };
            ui.label(format!("Surveying {}", game.name_of(*star)));
            // How long a body waits for its turn, which is what sets how fast an orbit fills
            // in. Nothing found yet is worth saying rather than showing a zero.
            if bodies > 0 {
                ui.weak(format!(
                    "{bodies} bodies, {:.0} s each: one comes round every {:.1} hours",
                    lc_world::knowledge::survey::SURVEY_DWELL_S,
                    bodies as f64 * lc_world::knowledge::survey::SURVEY_DWELL_S / 3600.0
                ));
            } else if here {
                ui.weak("nothing detected yet");
            } else {
                ui.weak("not in that system");
            }
        }
    }
    if let (Some(on), Some(looking)) = (game.observatory.pointing(), game.described)
        && on != looking
    {
        ui.weak(format!("The telescope is on {}; the panel below describes {}.", game.name_of(on), game.name_of(looking)));
    }
    ui.horizontal(|ui| {
        if ui.button("Observe").on_hover_text("Observe a single star for a period of time").clicked() {
            ask(out, Action::StareSelected);
        }
        if ui.button("Survey Sky").on_hover_text("Sweep sky to discover stars").clicked() {
            ask(out, Action::SurveySky);
        }
        if ui.button("Survey ahead").on_hover_text("Survey cone in direction of travel to discover stars").clicked() {
            ask(out, Action::SurveyAhead);
        }
        let here = game.system.is_some();
        if ui
            .add_enabled(here, egui::Button::new("Survey system"))
            .on_hover_text("Measure the bodies of this system, brightest first")
            .on_disabled_hover_text("Only from inside a system")
            .clicked()
        {
            ask(out, Action::SurveySystem);
        }
        if ui.button("Stop").on_hover_text("Stop all observations").clicked() {
            ask(out, Action::StopSurvey);
        }
    });
}

/// Nearest believed first. No labels: only the rows on screen get those.
fn known(game: &Game) -> Vec<(f64, StarId)> {
    let here = game.ship.motion.position_ly;
    let mut order: Vec<(f64, StarId)> = game
        .knowledge
        .stars()
        .map(|(id, belief)| {
            let key = match belief.distance {
                Distance::Measured { .. } => belief.distance.from(here).unwrap_or(0.0),
                Distance::AtLeast(_) => 1e9,
                Distance::Unknown => 2e9,
            };
            (key, id)
        })
        .collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    order
}

/// Name on the left, range on the right. Where the range came from is shown only in the details.
fn row(ui: &mut egui::Ui, game: &Game, id: StarId, selected: bool, height: f32) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let inset = egui::vec2(ui.spacing().button_padding.x, 0.0);
        let range = crate::range::short(game.knowledge.belief(id), game.ship.motion.position_ly);
        let right = ui.painter().text(
            rect.right_center() - inset,
            egui::Align2::RIGHT_CENTER,
            range,
            font.clone(),
            visuals.text_color(),
        );
        // Clipped short of the range, so a long name never runs under it.
        let left = rect.with_max_x(right.left() - inset.x);
        ui.painter().with_clip_rect(left).text(
            left.left_center() + inset,
            egui::Align2::LEFT_CENTER,
            game.name_of(id),
            font,
            visuals.text_color(),
        );
    }
    response
}

fn details(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
) {
    let Some(id) = described(game).filter(|id| game.knows(*id)) else {
        ui.weak("Nothing detected under the crosshair yet.");
        return;
    };
    for name in names(game, id) {
        ui.strong(name);
    }
    // A name is a record like any other: timestamped and relayed to whoever this ship reports to.
    if state.selected == Some(id) {
        let field = ui.add(egui::TextEdit::singleline(draft).hint_text("Add Name").desired_width(150.0));
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if entered && !draft.trim().is_empty() {
            ask(out, Action::NameSelected(std::mem::take(draft)));
        }
    }
    let Some(belief) = game.belief(id) else { return };
    let here = game.ship.motion.position_ly;
    ui.label(format!("Estimated range: {}", crate::range::short(Some(belief), here)));
    egui::CollapsingHeader::new("Sources").default_open(true).show(ui, |ui| {
        for note in crate::range::sources(belief, game.knowledge.owner) {
            ui.small(note);
        }
    });
}

/// Chosen names first, each once. An unnamed star's discovery designation is one of these.
fn names(game: &Game, id: StarId) -> Vec<String> {
    let mut held: Vec<_> = game.knowledge.file(id).map(|f| f.names().to_vec()).unwrap_or_default();
    held.sort_by_key(|n| !n.kind.chosen());
    let mut names: Vec<String> = Vec::new();
    for naming in held {
        if !names.contains(&naming.name) {
            names.push(naming.name);
        }
    }
    if names.is_empty() {
        names.push(game.name_of(id));
    }
    names
}

fn curve(
    ui: &mut egui::Ui,
    game: &mut Game,
    out: &mut MessageWriter<Requested>,
    plot: &mut CurvePlot,
) {
    let mut curve = game.curve();
    ui.horizontal(|ui| {
        // The shard's figure, which is the one being integrated at.
        ui.label(format!("integration: {:.0} s", game.observatory.integration_s));
        ui.separator();
        ui.label(format!("{} samples", curve.len()));
        if curve.len() > 1 {
            ui.separator();
            ui.label(format!("+/- {:.1e}", curve.uncertainty()));
        }
    });

    let band = game.curve_band;
    ui.horizontal(|ui| {
        ui.label("curve");
        for b in em_spectra::Band::ALL {
            // Shown disabled rather than omitted, so the sensor's limits are visible.
            if !game.telescope.sees(b) {
                ui.weak(format!("{b:?}"));
                continue;
            }
            if ui.selectable_label(band == b, format!("{b:?}")).clicked() {
                ask(out, Action::SetCurveBand(b));
            }
        }
    });
    let width = ui.available_width().max(220.0);
    let samples = curve.samples();
    plot.show(ui, samples, egui::vec2(width, 190.0));
    let last = samples.last().copied();
    if curve.dated() {
        ui.weak(crate::plot::caption(band));
    } else {
        ui.weak("against arrival time: without a distance there is no emission time to plot");
    }

    if let Some((_, last)) = &last {
        // A number survives being read aloud or seen by someone who cannot see the color.
        let (label, value) = if *last >= 0.0 {
            ("deficit", *last)
        } else {
            ("excess", -*last)
        };
        ui.label(format!(
            "last: {label} {value:.4e}   deepest dip {:.3e}",
            curve.deepest()
        ));
    }
}
