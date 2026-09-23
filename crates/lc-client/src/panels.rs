//! egui drawing. These systems read [`UiState`] and emit [`Action`]s; they change nothing.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::action::{Action, MAX_ACCEL_G, MIN_ACCEL_G};
use crate::flight::JULIAN_YEAR_S;
use crate::app::{Game, Ui};
use crate::hud;
use crate::input::Requested;
use crate::plot::CurvePlot;
use crate::navigation::Target;
use crate::ui::Panel;

pub(crate) fn ask(out: &mut MessageWriter<Requested>, action: Action) {
    out.write(Requested(action));
}

/// A root `Ui` covering the whole viewport.
///
/// egui 0.36 shows panels inside a `Ui` rather than straight onto the `Context`, so a
/// top-level panel needs one of these to sit in.
fn viewport_ui(ctx: &egui::Context) -> egui::Ui {
    egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    )
}

pub fn loading(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let mut root = viewport_ui(ctx);
    egui::CentralPanel::default().show(&mut root, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            ui.heading("Loading the sky");
            ui.spinner();
        });
    });
}

/// The browser build's dead end. Nothing is behind it, so it says what happened and the one
/// thing that helps.
pub fn unreachable(
    mut contexts: EguiContexts,
    uplink: Res<crate::uplink::Uplink>,
    address: Res<crate::uplink::ServerAddress>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let why = crate::uplink::out_of_reach(&uplink.state, address.0.as_deref());
    let mut root = viewport_ui(ctx);
    egui::CentralPanel::default().show(&mut root, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            ui.heading("Not connected to the server");
            if let Some(why) = why {
                ui.colored_label(connection_color(crate::uplink::Note::Wrong), why);
            }
            ui.add_space(12.0);
            ui.label("Reload the page to try again.");
        });
    });
}

/// The palette for a connection state. The words are `uplink`'s; only the color is here.
fn connection_color(note: crate::uplink::Note) -> egui::Color32 {
    match note {
        crate::uplink::Note::Quiet => egui::Color32::from_rgb(140, 170, 150),
        crate::uplink::Note::Working => egui::Color32::from_rgb(240, 170, 60),
        crate::uplink::Note::Wrong => egui::Color32::from_rgb(235, 110, 100),
    }
}

/// Where the readout ends, in points from the top of the window.
///
/// egui panels stack inside one `Ui` and every system here builds its own root, so a second
/// strip cannot find the first by asking. [`crate::map_panel`] needs it: the map's own strip
/// hangs under this one.
#[derive(Resource, Default)]
pub struct HudFoot(pub f32);

/// The always-visible readout. Never in a closable panel: it is the premise.
pub fn hud(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    game: Res<Game>,
    uplink: Res<crate::uplink::Uplink>,
    mut foot: ResMut<HudFoot>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let lines = hud::lines(&game.0, &ui_state.0);

    let mut root = viewport_ui(ctx);
    let strip = egui::Panel::top("hud").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.strong(&lines.clock);
            ui.separator();
            ui.strong(&lines.ship_clock);
            ui.separator();
            ui.label(format!("BAND {}", lines.mapping));
            ui.separator();
            ui.label(format!("EXPOSURE {}", lines.exposure));
            if let Some(energy) = &lines.energy {
                ui.separator();
                ui.label("ENERGY");
                ui.add(egui::ProgressBar::new(energy.fraction).desired_width(80.0));
                ui.label(&energy.amount);
            }
            if let Some(warning) = &lines.warning {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(240, 170, 60), warning);
            }
            if let Some((note, words)) =
                crate::uplink::note(&uplink.state, uplink.round_trip_s)
            {
                ui.separator();
                ui.colored_label(connection_color(note), words);
            }
        });
        if let Some(target) = &lines.target {
            ui.colored_label(egui::Color32::from_rgb(240, 190, 110), target);
        }
        ui.horizontal(|ui| {
            // A pursuit takes the crossing's place and its button: × is no further corrections,
            // which for a pursuit means giving up the policy as well as cutting the drive.
            if let Some(pursuit) = uplink.chasing {
                let quarry = uplink.contacts.iter().find(|c| c.ship_id == pursuit.quarry);
                ui.colored_label(
                    egui::Color32::from_rgb(130, 200, 250),
                    hud::pursuit(&game.0, pursuit, quarry),
                );
                if ui.small_button("×").on_hover_text("break off: no further corrections").clicked() {
                    ask(&mut out, Action::BreakOff);
                }
                let (label, hint, next) = closer(pursuit.closeness);
                if ui.small_button(label).on_hover_text(hint).clicked() {
                    ask(&mut out, Action::Intercept(pursuit.quarry, next));
                }
            } else if let Some(flight) = &lines.flight {
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
                // The row's first item since the ship clock moved up, unless something is flying.
                if uplink.chasing.is_some() || lines.flight.is_some() {
                    ui.separator();
                }
                ui.colored_label(egui::Color32::from_rgb(170, 190, 170), coasting);
            }
        });
    });
    foot.0 = strip.response.rect.max.y;

    if !ui_state.notifications.is_empty() {
        egui::Window::new("notifications")
            .title_bar(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .resizable(false)
            .show(ctx, |ui| {
                // A width of its own, so "the whole row" means something. Left to size itself
                // the box is as wide as its widest notice, and a row that then asked for all of
                // it would be asking the box how wide to be while the box asked the row.
                ui.set_min_width(NOTICE_WIDTH);
                for note in &ui_state.notifications {
                    // Everything else in this box is the interface reporting on itself and has
                    // nowhere to go. A transmission is the one kind of event with somewhere to
                    // go, so it is the one kind that is a link.
                    match note.from {
                        Some(from) => {
                            if notice_link(ui, &note.text)
                                .on_hover_text("open the conversation")
                                .clicked()
                            {
                                ask(&mut out, Action::OpenChat(from));
                            }
                        }
                        None => {
                            ui.label(&note.text);
                        }
                    }
                }
            });
    }
}


/// How wide the events box is. See [`notice_link`] for why it is stated rather than measured.
const NOTICE_WIDTH: f32 = 300.0;

/// One notice that goes somewhere: the whole row clickable, lit while the cursor is on it.
///
/// **Not a button**, and the whole row rather than the words. A button's frame in a list of
/// notices makes the list read as a row of controls, and a click target the width of its own
/// text is one a cursor slides off — these arrive unasked for, so hitting one should not need
/// aim. What marks it out is the color it is already drawn in and the background under it
/// while it is hovered.
///
/// Laid out by hand rather than as a widget because the background has to be painted *under*
/// the text, and anything added after a label paints over it.
fn notice_link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let width = ui.available_width();
    let galley = ui.painter().layout(
        text.to_string(),
        egui::TextStyle::Body.resolve(ui.style()),
        crate::radio_panel::RADIO,
        (width - padding.x * 2.0).max(1.0),
    );
    let size = egui::vec2(width, galley.size().y + padding.y * 2.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                ui.visuals().widgets.hovered.corner_radius,
                ui.visuals().widgets.hovered.weak_bg_fill,
            );
        }
        ui.painter().galley(rect.min + padding, galley, crate::radio_panel::RADIO);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The button that changes a pursuit's closeness: what it says, what it does, and what it asks
/// for.
fn closer(now: lc_proto::Closeness) -> (&'static str, &'static str, lc_proto::Closeness) {
    match now {
        lc_proto::Closeness::Company => {
            ("close in", "within sight: a kilometer between hulls", lc_proto::Closeness::Intimate)
        }
        lc_proto::Closeness::Intimate => {
            ("stand off", "back to formation distance", lc_proto::Closeness::Company)
        }
    }
}

/// Which half of the System window is showing.
///
/// The two lists answer different questions — what is here, and who is here — and they are
/// different lengths and change at different rates. One scrolling list holding both would put
/// a ship that arrived a second ago below two hundred moons.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum SystemTab {
    #[default]
    Bodies,
    Ships,
}

#[allow(clippy::too_many_arguments)]
pub fn open_panels(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    mut game: ResMut<Game>,
    mut beliefs: ResMut<crate::beliefs::Beliefs>,
    uplink: Res<crate::uplink::Uplink>,
    mut out: MessageWriter<Requested>,
    mut curve: Local<CurvePlot>,
    sky: Option<Res<crate::starfield::Starfield>>,
    mut show_all: Local<bool>,
    mut revealed: Local<Option<Target>>,
    mut tab: Local<SystemTab>,
    mut draft: Local<String>,
    mut name_draft: Local<String>,
    mut aimed: Local<crate::radio_panel::Aimed>,
    mut seal: Local<bool>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let panels = ui_state.open_panels().to_vec();
    // Once for every panel drawn this frame, and the map already asked for the same second,
    // so this is a read.
    let wants = panels.iter().any(|p| matches!(p, Panel::System | Panel::Telescope));
    let held = beliefs.held_if(wants, &game.0);
    for panel in panels {
        // The book brings its own window: it is not a readout, and egui's chrome around it
        // would be a dark title bar over a white page. See `crate::reader`.
        if panel == Panel::Reader {
            continue;
        }
        let mut open = true;
        egui::Window::new(panel.title()).open(&mut open).show(ctx, |ui| match panel {
            Panel::Escape => escape(ui, &mut out),
            Panel::Settings => settings(ui, &ui_state),
            Panel::Debug => debug(ui, &ui_state, &game, sky.as_deref(), &mut out),
                Panel::Telescope => crate::telescope_panel::telescope(
                    ui,
                    &ui_state,
                    &mut game,
                    held,
                    &mut name_draft,
                    &mut out,
                    &mut curve,
                ),
            Panel::System => crate::system_panel::system(
                ui,
                &ui_state,
                &game,
                held,
                &uplink,
                &mut tab,
                &mut show_all,
                &mut revealed,
                &mut out,
            ),
            Panel::Flight => flight(ui, &ui_state, &game, &mut out),
            Panel::Tuning => tuning(ui, &ui_state, &mut out),
            Panel::Scenarios => {
                crate::demos::scenarios(ui, &uplink, ui_state.0.perspective, &mut out)
            }
            Panel::Reader => unreachable!("drawn by crate::reader"),
            Panel::Refit => crate::refit_panel::refit(ui, &ui_state.0, &game, &mut out),
            Panel::DevActions => crate::refit_panel::dev_actions(ui, &game, &mut out),
            Panel::Chat => crate::radio_panel::chat(
                ui,
                &ui_state,
                &game,
                &uplink,
                &mut draft,
                &mut aimed,
                &mut seal,
                &mut out,
            ),
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
    // A browser tab has no menu to return to and no process to quit; closing it is the
    // browser's.
    if crate::app::HAS_MAIN_MENU && ui.button("Quit").clicked() {
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
        Some(system) => ui.label(format!("in {} — {} bodies loaded", game.name_of(system.star), system.len())),
        None => ui.label("between systems"),
    };
    ui.label(format!("stars detected: {}", game.knowledge.stars().count()));
    ui.label(format!("curve samples: {}", game.curve().len()));
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
    let rated = game.ship.rated_drive(game.coordinate_time_s()).accel_g;
    if game.ship.fitting().is_some() {
        ui.weak(format!("engines rated for {rated:.1} g at this mass"));
    }
    ui.horizontal(|ui| {
        for g in [1.0, 5.0, 20.0, 100.0].into_iter().filter(|g| *g <= rated.max(1.0)) {
            if ui.button(format!("{g:.0} g")).clicked() {
                ask(out, Action::SetDriveAccel(g));
            }
        }
        // The rating rarely lands on a preset, so without this a drive set lower had no way back
        // to all of it.
        if ui.button("Max").on_hover_text(format!("All the engines are rated for: {rated:.1} g")).clicked() {
            ask(out, Action::SetDriveAccel(rated));
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
            // A transfer's numbers are in its body's frame, and saying which is the difference
            // between "ten kilometers a second" and "ten kilometers a second *past Earth*".
            let frame = match game.flown_about() {
                Some(body) => format!(" past {}", game.body_label(body)),
                None => String::new(),
            };
            ui.label(format!("speed: {:.6}c{frame}", state.beta.length()));
            ui.label(format!("peak: {:.6}c{frame}", cruise.peak_beta()));
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
                    let name = game.name_of(star.id);
                    let range = crate::range::short(game.knowledge.belief(star.id), game.ship.motion.position_ly);
                    ui.label(format!("{name} — {range}"));
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


/// Who else is here, and how old the news of them is.
///
/// Every row is a *sighting*, and the age of the light is a column rather than a footnote:
/// across a system it runs from seconds to hours, and a range read as though it were current
/// is the one mistake this list exists to stop a player making.
pub(crate) fn ships(
    ui: &mut egui::Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    out: &mut MessageWriter<Requested>,
) {
    if uplink.contacts.is_empty() {
        ui.weak(match game.remote {
            true => "Nobody else is in this system.",
            // Not the same statement at all, and saying the first would be a lie.
            false => "No server, so nobody to see.",
        });
        return;
    }
    let here = game.ship.motion.position_ly;
    let mut rows: Vec<&crate::uplink::Contact> = uplink.contacts.iter().collect();
    rows.sort_by(|a, b| {
        here.distance_squared(a.position_ly).total_cmp(&here.distance_squared(b.position_ly))
    });

    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for contact in rows {
            let range = here.distance(contact.position_ly);
            let chasing = uplink.chasing.filter(|p| p.quarry == contact.ship_id);
            ui.horizontal(|ui| {
                ui.label(&contact.name);
                ui.weak(span(range));
                // Inline rather than right-aligned: a right-to-left layout claims the whole
                // available width, and the panel grew to a third of the screen to hold one
                // button.
                match chasing {
                    Some(pursuit) => {
                        if ui.button("break off").clicked() {
                            ask(out, Action::BreakOff);
                        }
                        let (label, hint, next) = closer(pursuit.closeness);
                        if ui.button(label).on_hover_text(hint).clicked() {
                            ask(out, Action::Intercept(contact.ship_id, next));
                        }
                    }
                    None => {
                        if ui.button("intercept").clicked() {
                            ask(out, Action::Intercept(contact.ship_id, lc_proto::Closeness::Company));
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                if chasing.is_some() {
                    // What the ship is *doing* rather than what was asked for: a standing
                    // order and the approach it most recently produced are different facts,
                    // and only the second one says where the ship will actually be.
                    ui.weak(match game.ship.motion.still_closing(game.coordinate_time_s()) {
                        true => "closing",
                        false => "alongside",
                    });
                }
                ui.weak(format!(
                    "{} hull — {:.4}c — light is {} old",
                    span_m(contact.length_m),
                    contact.beta.length(),
                    // From the range, not from the clock. A light-year is a year of travel by
                    // definition, so the distance to where the light left *is* its age — and
                    // taking it that way needs no agreement with the server about what time it
                    // is. Differencing the timestamps instead measured the clock skew, which
                    // at a frozen client rate put a ship eight kilometers away five minutes in
                    // the past.
                    duration(range * lc_world::flight::JULIAN_YEAR_S),
                ));
            });
        }
    });
}

/// Where the ship is holding, if it is holding anywhere.
pub(crate) fn station(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    let Some(system) = game.system.as_ref() else { return };
    let Some(waypoint) = game.station() else {
        match game.coast() {
            // Not "adrift": the ship is on something, and which conic it is on is the first
            // thing a player needs after cutting the engine.
            Some(coast) => {
                ui.label(format!("coasting: {}", crate::hud::arc(coast, &game.body_label(&coast.primary))));
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
pub(crate) fn range_to(game: &Game, system: &crate::system::LocalSystem, target: &Target) -> f64 {
    // At the ship's own time. Read out of the arena, which no longer advances, a three-radii
    // orbit of Earth read as five hundred thousand kilometers after five hours -- which is
    // exactly how far Earth had gone in the meantime.
    let now = game.coordinate_time_s();
    let at = match target {
        Target::Body(name) => system.body_position_at(name, now),
        Target::Band(_) => system.star_position_at(now),
    };
    at.map(|at| at.distance(game.ship.motion.position_ly)).unwrap_or(0.0)
}


/// A duration in whatever unit makes it readable.
pub(crate) fn duration(seconds: f64) -> String {
    match seconds {
        s if s < 120.0 => format!("{s:.0} s"),
        s if s < 7200.0 => format!("{:.1} minutes", s / 60.0),
        s if s < 172_800.0 => format!("{:.1} hours", s / 3600.0),
        s if s < 63_115_200.0 => format!("{:.1} days", s / 86_400.0),
        s => format!("{:.1} years", s / 31_557_600.0),
    }
}

/// A distance in whatever unit makes it readable.
pub(crate) fn span(light_years: f64) -> String {
    span_m(light_years * crate::system::M_PER_LY)
}

pub(crate) fn span_m(meters: f64) -> String {
    match meters {
        // The primary orbits nothing, and "0 thousand km" reads as a measurement.
        m if m <= 0.0 => "the center".to_string(),
        // Meters below a kilometer, because a hull is measured in them and "0 km" is not a
        // size. Nothing that reads as an orbit radius is ever this small.
        m if m < 1.0e3 => format!("{m:.0} m"),
        m if m < 1.0e6 => format!("{:.0} km", m / 1.0e3),
        m if m < 1.0e9 => format!("{:.0} thousand km", m / 1.0e6),
        m if m < 1.0e11 => format!("{:.2} million km", m / 1.0e9),
        m if m < 1.0e14 => format!("{:.2} AU", m / crate::navigation::AU),
        m => format!("{:.2} ly", m / crate::system::M_PER_LY),
    }
}
