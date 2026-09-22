//! The telescope window: what the instrument is committed to, what has been detected, and
//! where the belief about any of it came from.
//!
//! Everything shown here comes out of [`lc_world::knowledge`] rather than out of the
//! catalogue. A star nobody aboard has detected has no row, and a star nobody has measured a
//! parallax to has a row with no distance in it — see `lightcone/docs/22-provenance.md`.

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
    room(ui, game);
    ui.separator();

    ui.label(format!("Detected: {} stars", game.knowledge.stars().count()));
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            for line in detected(game) {
                if ui
                    .selectable_label(state.selected == Some(line.id), line.label)
                    .clicked()
                {
                    ask(out, Action::SelectTarget(Some(line.id)));
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
    naming(ui, state, game, draft, out);
    ui.separator();

    provenance(ui, game);
    conclusion(ui, game, out);
    curve(ui, state, game, out, plot);
}

/// One hypothesis, in words.
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

/// How much of the room aboard what this ship knows takes.
fn room(ui: &mut egui::Ui, game: &Game) {
    let now = game.coordinate_time_s();
    let capacity = game
        .ship
        .fitting()
        .map_or(lc_world::fitting::ONBOARD_DATA_BYTES, |f| f.balance.data_capacity(&f.loadout_at(now)));
    let used = game.knowledge.bytes();
    let mb = |bytes: f64| bytes / 1_048_576.0;
    ui.label(format!("Data: {:.2} of {:.2} MB", mb(used), mb(capacity)));
    if used >= capacity {
        ui.colored_label(
            egui::Color32::from_rgb(230, 150, 60),
            "Full: the telescope still measures, but nothing keeps the samples. Reading a log into a conclusion frees its room.",
        );
    }
}

/// What this ship has concluded from the log of whatever is under the crosshair.
///
/// A probability with its evidence, never a verdict: see
/// `lightcone/docs/24-standing-instruments.md`.
fn conclusion(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let Some(id) = game.pointing.filter(|id| game.knowledge.knows(*id)) else { return };
    let day = 86_400.0;
    match game.knowledge.conclusion(id) {
        None => {
            ui.weak("The log has not been read yet.");
        }
        Some(c) => {
            ui.label("Transiting:");
            for h in c.transits.iter().filter(|h| h.probability >= 0.005) {
                ui.label(format!("  {} — {:.0}%", describe(&h.kind), h.probability * 100.0));
            }
            if !c.populations.is_empty() {
                ui.label("In orbit:");
                for h in c.populations.iter().filter(|h| h.probability >= 0.005) {
                    ui.label(format!("  {} — {:.0}%", describe(&h.kind), h.probability * 100.0));
                }
            }
            let whose = if c.observer == game.knowledge.owner { "this ship's" } else { "another craft's" };
            let searched = match c.evidence.periods_s {
                Some((lo, hi)) => format!("periods {:.1} to {:.1} d searched", lo / day, hi / day),
                None => "too short to search for a period yet".into(),
            };
            ui.weak(format!(
                "From {} of {whose} samples in {} bands; {searched}; it would have found {:.0}% of the transiting planets the galaxy makes.",
                c.evidence.samples,
                c.evidence.bands.count_ones(),
                c.evidence.completeness * 100.0,
            ));
            if c.discarded_s.is_some() {
                ui.weak("The log behind it has been thrown away; only the conclusion is left.");
            }
        }
    }
    let mut keep = game.knowledge.retained(id);
    if ui.checkbox(&mut keep, "Keep the raw log").changed() {
        ask(out, Action::RetainRaw(id, keep));
    }
}

/// What the telescope is committed to, and how to commit it to something else.
fn duty(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    match &game.observatory.duty {
        Duty::Idle => {
            ui.label("Telescope idle — nothing is being learned.");
        }
        Duty::Stare(_) => {
            ui.label("Staring: the whole exposure on one star.");
        }
        Duty::Sweep(sweep) => {
            let (passes, fraction) = sweep.progress(now);
            ui.label(format!(
                "Sweeping {} fields: pass {}, {:.0}% through",
                sweep.fields(),
                passes + 1,
                fraction * 100.0
            ));
            // A pass is the unit that matters: a star is found when the sweep reaches its
            // field, and its parallax comes from the ship having moved between two passes.
            ui.weak(format!(
                "one pass every {:.1} hours of ship time",
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
    }
    ui.horizontal(|ui| {
        if ui.button("Survey the sky").clicked() {
            ask(out, Action::SurveySky);
        }
        if ui.button("Survey ahead").clicked() {
            ask(out, Action::SurveyAhead);
        }
        if ui.button("Stop").clicked() {
            ask(out, Action::StopSurvey);
        }
    });
}

/// One line of the target list.
struct Detected {
    id: StarId,
    label: String,
}

/// Stars this ship has detected, nearest believed first, with what is believed about them.
///
/// Not the catalogue. A star nobody aboard has seen does not appear, which is the whole point:
/// the list is a record of work done, not a table handed over at the start.
fn detected(game: &Game) -> Vec<Detected> {
    let mut lines: Vec<(f64, Detected)> = game
        .knowledge
        .stars()
        .map(|(id, belief)| {
            let name = game.name_of(id);
            let (order, distance) = match belief.distance {
                Distance::Measured { sigma_ly, .. } => {
                    let ly = belief
                        .distance
                        .from(game.ship.motion.position_ly)
                        .unwrap_or(0.0);
                    (ly, format!("{ly:.2} +/- {sigma_ly:.2} ly"))
                }
                Distance::AtLeast(ly) => (1e9, format!("beyond {ly:.1} ly")),
                Distance::Unknown => (2e9, "bearing only".to_string()),
            };
            (
                order,
                Detected {
                    id,
                    label: format!("{name} — {distance}"),
                },
            )
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    lines.into_iter().map(|(_, line)| line).collect()
}

/// The field that calls a star something.
///
/// A name is a record like any other: this ship's, stamped with when it said so, and carried
/// to anyone it reports to. Nothing has a name before somebody gives it one — what a row shows
/// until then is the designation its own discovery wrote down.
fn naming(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
) {
    let Some(id) = state.selected.filter(|id| game.knows(*id)) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Call it");
        let field = ui.add(
            egui::TextEdit::singleline(draft)
                .hint_text(game.name_of(id))
                .desired_width(150.0),
        );
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (entered || ui.button("Name it").clicked()) && !draft.trim().is_empty() {
            ask(out, Action::NameSelected(std::mem::take(draft)));
        }
    });
}

/// Where the belief about the selected star came from, and how old it is.
fn provenance(ui: &mut egui::Ui, game: &Game) {
    let Some(belief) = game.pointing.and_then(|id| game.belief(id)) else {
        ui.weak("Nothing detected under the crosshair yet.");
        return;
    };
    match belief.name.as_ref() {
        Some(naming) if naming.witness == game.knowledge.owner => {
            ui.label(format!("Called {} by this ship.", naming.name));
        }
        Some(naming) => {
            ui.label(format!("Called {} by whoever charted it.", naming.name));
        }
        None => {
            ui.weak("Nobody has called it anything.");
        }
    }
    ui.label(format!(
        "{} bearings from {} {}",
        belief.sightings,
        belief.witnesses,
        if belief.witnesses == 1 {
            "instrument"
        } else {
            "instruments"
        }
    ));
    match belief.hops {
        0 => ui.label("Seen from this ship."),
        1 => ui.label("Relayed once; somebody else did the looking."),
        n => ui.label(format!("Relayed {n} times.")),
    };
    // A distance worked out from bearings is one this ship can check. A stated one is not,
    // however narrow the error bars on it are.
    match (&belief.distance, belief.triangulated) {
        (Distance::Unknown, _) => ui.weak("No parallax yet: this is a direction."),
        (Distance::AtLeast(ly), _) => ui.weak(format!(
            "No parallax over the baseline so far, so it is past {ly:.1} ly."
        )),
        (Distance::Measured { .. }, true) => ui.weak("Distance solved from bearings held here."),
        (Distance::Measured { .. }, false) => ui.weak("Distance on somebody else's word."),
    };
    // The age is the distance, so a star without a parallax has no age either: what is on the
    // screen is old by an unknown amount, and saying so is more honest than a number.
    match belief.light_age_s() {
        Some(age) => ui.label(format!(
            "The light left {:.2} years ago. Everything below describes the system then.",
            age / crate::flight::JULIAN_YEAR_S
        )),
        None => ui.label("No distance yet, so no light age: this is a direction and a brightness."),
    };
    if let Some(watts) = belief.luminosity_w() {
        ui.weak(format!(
            "implied {:?}-band output: {watts:.2e} W",
            belief.band
        ));
    }
}

/// The light curve of whatever is under the crosshair, in the selected band.
fn curve(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &mut Game,
    out: &mut MessageWriter<Requested>,
    plot: &mut CurvePlot,
) {
    let mut curve = game.curve();
    ui.horizontal(|ui| {
        ui.label(format!("integration: {:.0} s", state.integration_s));
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
            // A band the sensor cannot reach is shown as unavailable rather than omitted, so
            // the instrument's limits are visible instead of merely being enforced.
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
    let samples = curve.samples().to_vec();
    plot.show(ui, &samples, egui::vec2(width, 190.0));
    if curve.dated() {
        ui.weak(crate::plot::caption(band));
    } else {
        ui.weak("against arrival time: without a distance there is no emission time to plot");
    }

    if let Some((_, last)) = samples.last() {
        // The picture is the readout, but a number is the one form of it that survives being
        // read aloud, screenshotted, or looked at by someone who cannot see the color.
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
