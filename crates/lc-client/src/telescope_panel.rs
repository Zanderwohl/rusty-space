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

    let order = known(game);
    let seen = game.knowledge.stars().filter(|(_, b)| b.hops == 0).count();
    ui.label(format!("Known: {} stars, {seen} seen by this ship", order.len()));
    // Only the rows on screen are laid out, and only their labels are written.
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show_rows(ui, row_height, order.len(), |ui, rows| {
            for &(_, id) in order.get(rows).unwrap_or_default() {
                if ui.selectable_label(state.selected == Some(id), label(game, id)).clicked() {
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
    naming(ui, state, game, draft, out);
    ui.separator();

    provenance(ui, game);
    conclusion(ui, game, out);
    curve(ui, game, out, plot);
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
    let Some(id) = described(game).filter(|id| game.knowledge.knows(*id)) else { return };
    let day = 86_400.0;
    let owner = game.knowledge.owner;
    let all = game.knowledge.conclusions(id);
    if all.is_empty() {
        ui.weak("No log of it has been read yet.");
    }
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
    // Planets are read against a stand-in for the generator the game will ship with.
    ui.weak("Planet readings are provisional until the planet generator exists.");
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

/// What the panel describes: the selection, whatever the telescope is doing.
fn described(game: &Game) -> Option<StarId> {
    game.described.or(game.pointing)
}

/// What the telescope is committed to, and how to commit it to something else.
fn duty(ui: &mut egui::Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let now = game.coordinate_time_s();
    match &game.observatory.duty {
        Duty::Idle => {
            ui.label("Telescope idle — nothing is being learned.");
        }
        Duty::Stare(id) => {
            ui.label(format!("Staring at {}: the whole exposure on one star.", game.name_of(*id)));
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
    }
    // The telescope and the selection are two things, and the panel says when they differ.
    if let (Some(on), Some(looking)) = (game.observatory.pointing(), game.described)
        && on != looking
    {
        ui.weak(format!("The telescope is on {}; the panel below describes {}.", game.name_of(on), game.name_of(looking)));
    }
    ui.horizontal(|ui| {
        if ui.button("Stare").clicked() {
            ask(out, Action::StareSelected);
        }
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

/// Every star this ship knows, nearest believed first, as an order to show them in. Cheap: no
/// labels, which only the rows on screen get.
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

/// One row of the list: the name, and the range as far as this ship knows it.
fn label(game: &Game, id: StarId) -> String {
    let range = crate::range::describe(game.knowledge.belief(id), game.knowledge.owner, game.ship.motion.position_ly);
    format!("{} — {range}", game.name_of(id))
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
    let Some(belief) = described(game).and_then(|id| game.belief(id)) else {
        ui.weak("Nothing detected under the crosshair yet.");
        return;
    };
    let owner = game.knowledge.owner;
    match belief.name.as_ref() {
        Some(naming) => {
            ui.label(format!("Called {} by {}.", naming.name, crate::range::who(naming.witness, owner)));
        }
        None => {
            ui.weak("Nobody has called it anything.");
        }
    }
    ui.label(format!("Range: {}", crate::range::describe(Some(belief), owner, game.ship.motion.position_ly)));
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

/// The light curve of whatever the panel describes, in the selected band.
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
