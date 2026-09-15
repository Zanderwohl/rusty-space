//! egui drawing. These systems read [`UiState`] and emit [`Action`]s; they change nothing.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use em_spectra::presets;

use crate::action::{Action, MAX_ACCEL_G, MIN_ACCEL_G};
use crate::flight::JULIAN_YEAR_S;
use crate::app::{Game, Ui};
use crate::hud;
use crate::input::Requested;
use crate::plot::CurvePlot;
use crate::navigation::Target;
use crate::ui::Panel;

fn ask(out: &mut MessageWriter<Requested>, action: Action) {
    out.write(Requested(action));
}

pub fn loading(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            ui.heading("Loading the sky");
            ui.spinner();
        });
    });
}

/// The palette for a connection state. The words are `uplink`'s; only the colour is here.
fn connection_colour(note: crate::uplink::Note) -> egui::Color32 {
    match note {
        crate::uplink::Note::Quiet => egui::Color32::from_rgb(140, 170, 150),
        crate::uplink::Note::Working => egui::Color32::from_rgb(240, 170, 60),
        crate::uplink::Note::Wrong => egui::Color32::from_rgb(235, 110, 100),
    }
}

/// The always-visible readout. Never in a closable panel: it is the premise.
pub fn hud(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    game: Res<Game>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let lines = hud::lines(&game.0, &ui_state.0);

    egui::TopBottomPanel::top("hud").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.strong(&lines.clock);
            ui.separator();
            ui.label(format!("BAND {}", lines.mapping));
            ui.separator();
            ui.label(format!("EXPOSURE {}", lines.exposure));
            if let Some(warning) = &lines.warning {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(240, 170, 60), warning);
            }
            if let Some((note, words)) = crate::uplink::note(&uplink.state) {
                ui.separator();
                ui.colored_label(connection_colour(note), words);
            }
        });
        if let Some(target) = &lines.target {
            ui.colored_label(egui::Color32::from_rgb(240, 190, 110), target);
        }
        ui.horizontal(|ui| {
            ui.weak(&lines.ship_clock);
            if let Some(flight) = &lines.flight {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(130, 200, 250), flight);
                // No confirmation. Cutting the engine is not destructive -- the ship keeps its
                // velocity -- and a dialogue between a player and their own throttle is worse
                // than the mistake it prevents.
                // U+00D7, not U+2715: egui's default font has no glyph for the latter and it
                // came out as a tofu box.
                if ui.small_button("×").on_hover_text("cut the drive").clicked() {
                    ask(&mut out, Action::AbortFlight);
                }
            }
            if let Some(coasting) = &lines.coasting {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(170, 190, 170), coasting);
            }
        });
    });

    if !ui_state.notifications.is_empty() {
        egui::Window::new("notifications")
            .title_bar(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .resizable(false)
            .show(ctx, |ui| {
                for note in &ui_state.notifications {
                    ui.label(&note.text);
                }
            });
    }
}

pub fn open_panels(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    mut game: ResMut<Game>,
    mut out: MessageWriter<Requested>,
    mut curve: Local<CurvePlot>,
    sky: Option<Res<crate::starfield::Starfield>>,
    mut show_all: Local<bool>,
    mut revealed: Local<Option<Target>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    for panel in ui_state.open_panels().to_vec() {
        let mut open = true;
        egui::Window::new(panel.title()).open(&mut open).show(ctx, |ui| match panel {
            Panel::Escape => escape(ui, &mut out),
            Panel::Settings => settings(ui, &ui_state),
            Panel::Debug => debug(ui, &ui_state, &game, sky.as_deref(), &mut out),
            Panel::Telescope => telescope(ui, &ui_state, &mut game, &mut out, &mut curve),
            Panel::System => system(ui, &ui_state, &game, &mut show_all, &mut revealed, &mut out),
            Panel::Flight => flight(ui, &ui_state, &game, &mut out),
            Panel::Tuning => tuning(ui, &ui_state, &mut out),
        });
        if !open {
            ask(&mut out, Action::ClosePanel(panel));
        }
    }
}

fn escape(ui: &mut egui::Ui, out: &mut MessageWriter<Requested>) {
    // Nothing behind this has stopped, so it does not say "paused".
    ui.label("The clock is still running.");
    ui.separator();
    if ui.button("Settings").clicked() {
        ask(out, Action::OpenPanel(Panel::Settings));
    }
    if ui.button("Quit").clicked() {
        ask(out, Action::Quit);
    }
}

fn settings(ui: &mut egui::Ui, state: &Ui) {
    ui.label("Display");
    ui.label(format!("tone window: {:.1} stops", state.0.exposure_offset.abs().max(2.5)));
    ui.separator();
    ui.label("Settings apply immediately; there is no resume to apply them on.");
}

fn debug(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    sky: Option<&crate::starfield::Starfield>,
    out: &mut MessageWriter<Requested>,
) {
    ui.label(format!("stars: {}", game.stars.len()));
    if let Some(sky) = sky {
        ui.label(format!(
            "drawn: {} background, {} local, {} bodies",
            sky.distant.count, sky.local.count, sky.bodies.count
        ));
    }
    match game.system.as_ref() {
        Some(system) => ui.label(format!("in {} — {} bodies loaded", system.star_name, system.len())),
        None => ui.label("between systems"),
    };
    ui.label(format!("curve samples: {}", game.curve.len()));
    ui.label(format!("coordinate time: {:.3} s", game.coordinate_time_s()));
    ui.separator();

    ui.label("clock rate (development only; the server owns this)");
    ui.label(crate::ui::rate_label(state.time_rate));
    for (rate, name) in crate::ui::RATE_LADDER {
        if ui.selectable_label(state.time_rate == rate, name).clicked() {
            ask(out, Action::SetTimeRate(rate));
        }
    }
    ui.weak("or , and . while flying");
    ui.separator();

    if cfg!(feature = "godview") {
        let mut god = state.god_view;
        if ui.checkbox(&mut god, "god view").changed() {
            ask(out, Action::ToggleGodView);
        }
    } else {
        ui.weak("god view is not compiled into this build");
    }
    if ui.button("write snapshot").clicked() {
        ask(out, Action::WriteSnapshot);
    }
}

fn telescope(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &mut Game,
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

    ui.label("Target");
    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
        for star in game.stars.iter().take(40) {
            let label = star.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get()));
            let distance = game.distance_to(star);
            if ui
                .selectable_label(state.selected == Some(star.id), format!("{label} — {distance:.2} ly"))
                .clicked()
            {
                ask(out, Action::SelectTarget(Some(star.id)));
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
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.label(format!("integration: {:.0} s", state.integration_s));
        ui.separator();
        ui.label(format!("{} samples", game.curve.len()));
        if game.curve.len() > 1 {
            ui.separator();
            ui.label(format!("+/- {:.1e}", game.curve.uncertainty()));
        }
    });

    let band = game.curve.band();
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
    let samples = game.curve.samples().to_vec();
    plot.show(ui, &samples, egui::vec2(width, 190.0));
    ui.weak(crate::plot::caption(band));

    if let Some((_, last)) = samples.last() {
        // The picture is the readout, but a number is the one form of it that survives being
        // read aloud, screenshotted, or looked at by someone who cannot see the colour.
        let (label, value) = if *last >= 0.0 {
            ("deficit", *last)
        } else {
            ("excess", -*last)
        };
        ui.label(format!("last: {label} {value:.4e}   deepest dip {:.3e}", game.curve.deepest()));
    }
}

/// Every starfield knob, driven from the table in `starfield` so a knob cannot exist without a
/// slider. Each drag emits one action per frame carrying the whole style; nothing here mutates.
fn tuning(ui: &mut egui::Ui, state: &Ui, out: &mut MessageWriter<Requested>) {
    use crate::starfield::Which;
    for (which, label) in
        [(Which::Local, "Local star"), (Which::Bodies, "Planets and moons"), (Which::Distant, "Background")]
    {
        let current = crate::starfield::style_for(state, which);
        let corona_pass = which == Which::Local;
        egui::CollapsingHeader::new(label).default_open(corona_pass).show(ui, |ui| {
            let mut style = current;
            let mut changed = false;
            for (name, field, lo, hi) in crate::starfield::KNOBS {
                let corona = name.starts_with("corona")
                    || name.starts_with("reach")
                    || name.starts_with("tip");
                // A corona knob on a pass with no corona would do nothing.
                let live = corona_pass || !corona;
                let slider = egui::Slider::new(field(&mut style), lo..=hi).text(name);
                changed |= ui.add_enabled(live, slider).changed();
            }
            if changed {
                ask(out, Action::SetPointStyle { which, style });
            }
            if ui.button("Reset").clicked() {
                ask(out, Action::ResetPointStyle { which });
            }
        });
    }
    ui.separator();
    let mut gain = state.envelope_gain;
    if ui.add(egui::Slider::new(&mut gain, 0.0..=60.0).text("envelope opacity")).changed() {
        ask(out, Action::SetEnvelopeGain(gain));
    }
    ui.weak("How far a population's covering fraction is amplified. A belt really does block\nabout a millionth of a millionth of the light.");
    ui.separator();
    ui.weak("Values apply as they are dragged. Nothing here is saved.");
}

fn flight(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    ui.label(format!("drive: {:.0} g, cap {:.3}c", game.ship.motion.drive.accel_g, game.ship.motion.drive.max_beta));
    ui.horizontal(|ui| {
        for g in [1.0, 5.0, 20.0, 100.0] {
            if ui.button(format!("{g:.0} g")).clicked() {
                ask(out, Action::SetDriveAccel(g));
            }
        }
    });
    ui.weak(format!("between {MIN_ACCEL_G} and {MAX_ACCEL_G} g"));
    ui.separator();

    match &game.cruise() {
        Some(cruise) => {
            let now = game.coordinate_time_s();
            let state = cruise.at(now);
            ui.add(egui::ProgressBar::new(cruise.progress(now) as f32).show_percentage());
            ui.label(format!("{:?}", state.phase));
            ui.label(format!("speed: {:.6}c", state.beta.length()));
            ui.label(format!("peak: {:.6}c", cruise.peak_beta()));
            ui.label(format!(
                "crossing: {:.2} years, {:.2} aboard",
                cruise.duration_s() / JULIAN_YEAR_S,
                cruise.proper_duration_s() / JULIAN_YEAR_S
            ));
            if ui.button("Cut the drive").clicked() {
                ask(out, Action::AbortFlight);
            }
        }
        None => {
            ui.label("At rest.");
            match state.selected.and_then(|id| game.star(id)) {
                Some(star) => {
                    let name = star.name.clone().unwrap_or_else(|| "unnamed".into());
                    ui.label(format!("{name} — {:.2} ly", game.distance_to(star)));
                    if ui.button("Fly there").clicked() {
                        ask(out, Action::FlyTo(None));
                    }
                }
                None => {
                    ui.label("Select a target in the telescope panel first.");
                }
            }
        }
    }
    ui.separator();
    let p = game.ship.motion.position_ly;
    ui.weak(format!("at {:.3}, {:.3}, {:.3} ly", p.x, p.y, p.z));
}

/// The system window: what is here, and where the ship can be sent.
///
/// Two sections. The inventory runs outward from the star with each body's satellites behind
/// it; picking one opens its courses. Nothing is flown until Go, so a player can read the
/// options without committing to one.
fn system(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    show_all: &mut bool,
    revealed: &mut Option<Target>,
    out: &mut MessageWriter<Requested>,
) {
    let Some(system) = game.system.as_ref() else {
        ui.label("Between systems. There is nothing local to go to.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(format!("{} — {} bodies", system.star_name, system.len()));
        ui.checkbox(show_all, "all");
    });
    station(ui, state, game, out);
    ui.separator();

    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        for entry in system.inventory().iter().filter(|e| *show_all || e.major) {
            let picked = state.focus.as_ref() == Some(&entry.target);
            ui.horizontal(|ui| {
                ui.add_space(entry.depth as f32 * 12.0);
                let row = ui.selectable_label(picked, &entry.designation);
                if row.clicked() {
                    let next = (!picked).then(|| entry.target.clone());
                    ask(out, Action::FocusTarget(next));
                }
                // Once, when the focus changes. Every frame would fight the player's own
                // scrolling, and never would hide a body picked from anywhere but this list.
                if picked && revealed.as_ref() != Some(&entry.target) {
                    *revealed = Some(entry.target.clone());
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                ui.weak(span_m(entry.orbit_radius_m));
            });
        }
    });
    ui.separator();

    let Some(target) = state.focus.as_ref() else {
        ui.label("Pick something to go to.");
        return;
    };
    let Some(entry) = system.inventory().iter().find(|e| &e.target == target) else { return };
    ui.heading(&entry.designation);
    ui.weak(match entry.orbit_radius_m {
        r if r > 0.0 => format!(
            "{} — {} out, {} away",
            entry.kind.label(),
            span_m(r),
            span(range_to(game, system, target)),
        ),
        _ => format!("{} — {} away", entry.kind.label(), span(range_to(game, system, target))),
    });

    for (label, course) in crate::navigation::options_for(system, target) {
        let armed = state.course.as_ref() == Some(&course);
        if ui.selectable_label(armed, &label).clicked() {
            ask(out, Action::ChooseCourse((!armed).then_some(course)));
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        let ready = state.course.is_some();
        if ui.add_enabled(ready, egui::Button::new("Go")).clicked() {
            if let Some(course) = state.course.clone() {
                ask(out, Action::SetCourse(course));
            }
        }
        ui.weak(format!("brachistochrone at {:.0} g", game.ship.motion.drive.accel_g));
    });
}

/// Where the ship is holding, if it is holding anywhere.
fn station(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let Some(system) = game.system.as_ref() else { return };
    let Some(waypoint) = game.station() else {
        match game.coast() {
            // Not "adrift": the ship is on something, and which conic it is on is the first
            // thing a player needs after cutting the engine.
            Some(coast) => {
                ui.label(format!("coasting: {}", crate::hud::arc(coast)));
                if let Some(period) = coast.period_s() {
                    ui.weak(format!("one turn in {}", duration(period)));
                }
            }
            None => {
                ui.weak("adrift");
            }
        }
        return;
    };
    ui.label(format!("holding: {}", waypoint.label()));
    if let Some(period) = waypoint.period_s(system, game.coordinate_time_s()) {
        ui.weak(format!("one turn in {}", duration(period)));
        // The clock outruns an orbit by default and the view is then a strobe. Say so where the
        // decision is made rather than leaving it to be discovered.
        if period < state.time_rate * crate::session::TIME_RATE * 4.0 {
            ui.colored_label(
                egui::Color32::from_rgb(220, 170, 90),
                "faster than the clock — slow time down to watch it",
            );
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Look at it").clicked() {
            ask(out, Action::LookAtStation);
        }
        if ui.button("Cut the drive").clicked() {
            ask(out, Action::AbortFlight);
        }
    });
}

/// How far the ship is from a target, light-years.
fn range_to(game: &Game, system: &crate::system::LocalSystem, target: &Target) -> f64 {
    // At the ship's own time. Read out of the arena, which no longer advances, a three-radii
    // orbit of Earth read as five hundred thousand kilometres after five hours -- which is
    // exactly how far Earth had gone in the meantime.
    let now = game.coordinate_time_s();
    let at = match target {
        Target::Body(name) => system.body_position_at(name, now),
        Target::Band(_) => system.star_position_at(now),
    };
    at.map(|at| at.distance(game.ship.motion.position_ly)).unwrap_or(0.0)
}


/// A duration in whatever unit makes it readable.
fn duration(seconds: f64) -> String {
    match seconds {
        s if s < 120.0 => format!("{s:.0} s"),
        s if s < 7200.0 => format!("{:.1} minutes", s / 60.0),
        s if s < 172_800.0 => format!("{:.1} hours", s / 3600.0),
        s if s < 63_115_200.0 => format!("{:.1} days", s / 86_400.0),
        s => format!("{:.1} years", s / 31_557_600.0),
    }
}

/// A distance in whatever unit makes it readable.
fn span(light_years: f64) -> String {
    span_m(light_years * crate::system::M_PER_LY)
}

fn span_m(metres: f64) -> String {
    match metres {
        // The primary orbits nothing, and "0 thousand km" reads as a measurement.
        m if m <= 0.0 => "the centre".to_string(),
        m if m < 1.0e6 => format!("{:.0} km", m / 1.0e3),
        m if m < 1.0e9 => format!("{:.0} thousand km", m / 1.0e6),
        m if m < 1.0e11 => format!("{:.2} million km", m / 1.0e9),
        m if m < 1.0e14 => format!("{:.2} AU", m / crate::navigation::AU),
        m => format!("{:.2} ly", m / crate::system::M_PER_LY),
    }
}
