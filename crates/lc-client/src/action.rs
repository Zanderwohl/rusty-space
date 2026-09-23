//! Every UI action, as a value.
//!
//! No system acts directly. A key press, a button, a menu item and a test all produce the
//! same [`Action`], and [`apply`] is the only thing that changes [`crate::ui::UiState`] or
//! reaches into a [`Session`]. Rebinding is then a table rather than a rewrite, and a UI can
//! be driven from a test with no window.

use em_spectra::{Band, presets};
use lc_world::knowledge::survey::{Duty, Sweep};
use lc_world::sky::StarId;

use crate::navigation::{Course, Target};
use crate::session::Session;
use crate::starfield::{PointStyle, Which};
use crate::ui::{Look, MenuPage, Panel, UiState, ViewMode};

/// Everything the interface can be asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    // --- navigation -------------------------------------------------------------------
    OpenPanel(Panel),
    ClosePanel(Panel),
    TogglePanel(Panel),
    CloseTopPanel,
    GoToMenuPage(MenuPage),
    StartGame,
    Quit,
    /// Which mode of play the main view shows. The map is one of two, not a window.
    SetView(ViewMode),
    ToggleView,
    // --- the map ----------------------------------------------------------------------
    /// Turn the map's camera by a relative amount, radians.
    TurnMap { azimuth: f64, elevation: f64 },
    /// In or out, in notches. Positive is closer.
    ///
    /// Not [`Action::Zoom`], which is the ship's boom in hull lengths and is clamped by two
    /// angles. The map's is a stand-off in meters across fifteen orders of magnitude, and one
    /// name for both would mean `--zoom` moving whichever happened to be on top.
    ///
    /// `anchor_ly` is a place on the reference plane to hold still while the camera comes in —
    /// what the cursor is over. `None` zooms about the middle of the view.
    ZoomMap { notches: f64, anchor_ly: Option<glam::DVec3> },
    /// Slide the map's focus across the reference plane, in fractions of the stand-off.
    PanMap { right: f64, ahead: f64 },
    SetMapPlane(em_map::Plane),
    ToggleMapPlane,
    /// What the map is centered on.
    FocusMap(crate::ui::MapFocus),
    SetMapSource(crate::map_source::Source),

    /// Begin the desktop sign-in: open the browser and listen for the answer.
    SignIn,
    /// Give up on one in progress.
    CancelSignIn,
    /// Sign in with the local password provider, from the modal's own form.
    SignInWithPassword { email: String, password: String },
    /// Forget the device grant.
    SignOut,

    // --- reading -----------------------------------------------------------------------
    /// Open a book by the name of its file on the shelf. Nothing in the world changes.
    OpenBook(String),
    CloseBook,
    /// Pages, forward or back. Spent by the reader, which is the only thing that can lay one out.
    TurnPage(i32),
    /// Spine document and character offset: a chapter, or a saved place.
    GoTo(usize, usize),
    ToggleContents,

    // --- instruments ------------------------------------------------------------------
    SetBandPreset(usize),
    NextBandPreset,
    PreviousBandPreset,
    ExposureUp,
    ExposureDown,
    ExposureAuto,

    // --- observing --------------------------------------------------------------------
    SelectTarget(Option<StarId>),
    /// Point at the nearest star that is actually interstellar.
    SelectNearest,
    SetIntegration(f64),
    /// Which band the light curve measures. Independent of the display mapping.
    SetCurveBand(Band),
    /// Sweep the whole sky, field by field, until told otherwise.
    SurveySky,
    /// Sweep the patch of sky the view is pointed at, which comes round far more often.
    SurveyAhead,
    /// Put the whole exposure on the selected star.
    StareSelected,
    /// Add the selected star to the watch rotation, or drop it from one.
    WatchSelected,
    /// Call the selected star something. A name is this ship's, not the star's.
    NameSelected(String),
    /// Keep a star's raw log whatever is concluded from it, or let it go once it has been read.
    RetainRaw(StarId, bool),
    /// Read every log this ship holds into a conclusion and free its room.
    Analyze,
    /// Send what this ship has learned since it last reported to `to`.
    SendReport {
        to: Option<lc_proto::ShipId>,
        aim: lc_proto::Aim,
        secrecy: lc_proto::Secrecy,
    },
    /// Stop whatever the telescope is committed to.
    StopSurvey,

    // --- looking ----------------------------------------------------------------------
    /// Turn by a relative amount, radians.
    Look { yaw: f64, pitch: f64 },
    /// Move the orbit camera in or out, in notches. Positive is closer.
    Zoom(f64),
    LookAtSelected,
    /// Face whatever the station is about: the body below, or the star.
    LookAtStation,

    // --- flight -----------------------------------------------------------------------
    /// Cross to a star. `None` means whatever is selected.
    FlyTo(Option<StarId>),
    /// Cross to the nearest star that is actually interstellar.
    FlyToNearest,
    /// Cut the engine. Not a stop: the ship keeps its velocity and coasts on whatever conic
    /// that puts it on.
    AbortFlight,
    /// Pick something out of the local system's inventory. Clears whatever course was armed
    /// for the last one.
    FocusTarget(Option<Target>),
    /// Arm one of the focused target's courses, without flying it.
    ChooseCourse(Option<Course>),
    /// Go somewhere in the local system, and hold there once arrived. What Go sends.
    SetCourse(Course),
    /// Proper acceleration for the next crossing, in g.
    SetDriveAccel(f64),
    /// Close on another ship, match its velocity, and hold station alongside it at this
    /// closeness. Sent again for the same ship, it closes in or stands off.
    Intercept(lc_proto::ShipId, lc_proto::Closeness),
    /// Give up a standing intercept, with no further corrections: the drive is cut and the ship
    /// keeps whatever velocity it has.
    BreakOff,

    // --- appearance -------------------------------------------------------------------
    /// Replace a starfield pass's drawing parameters. Carries the whole style rather than one
    /// field, so a slider being dragged is one action a frame and the panel stays stateless.
    SetPointStyle { which: Which, style: PointStyle },
    ResetPointStyle { which: Which },
    /// How far a population envelope's covering fraction is amplified for display.
    SetEnvelopeGain(f32),

    // --- development ------------------------------------------------------------------
    ToggleGodView,
    SetTimeRate(f64),
    /// Step one rung along [`crate::ui::RATE_LADDER`].
    TimeRateUp,
    TimeRateDown,
    WriteSnapshot,
    /// Ask the shard to put a scene in the world, by name. See `lc_world::scenario`.
    StageDemo(String),
    /// Watch from another craft. `None` is back to one's own.
    WatchFrom(Option<lc_proto::ShipId>),
    /// Put energy in the ship. Development only; a shard refuses it.
    GrantEnergy(f64),
    /// Grant whatever storage has room for.
    FillStorage,

    // --- fitting ----------------------------------------------------------------------
    /// What the refit panel's sliders say. Nothing is built until [`Action::ApplyRefit`].
    DraftRefit(lc_world::fitting::Loadout),
    /// Put the sliders back to the ship as it is.
    ResetRefitDraft,
    ApplyRefit,
    CancelRefit,

    // --- radio ------------------------------------------------------------------------
    /// Show this craft's conversation, opening the window if it is closed. What a green line
    /// in the events box does when it is clicked.
    OpenChat(lc_proto::ShipId),
    /// Change what the window is showing: one craft, or the public log.
    ChatWith(crate::ui::Channel),
    /// Put a message on the air. `idem` is `None` for something newly typed, which mints one,
    /// and `Some` for a resend, which repeats the message rather than saying a second thing.
    Say {
        /// `None` broadcasts: the public channel, addressed to nobody.
        to: Option<lc_proto::ShipId>,
        aim: lc_proto::Aim,
        secrecy: lc_proto::Secrecy,
        body: String,
        idem: Option<lc_proto::MessageKey>,
    },
    /// Put this ship's public key on the air, so `to` — or anyone at all, for `None` — can
    /// encrypt messages back.
    OfferKey { to: Option<lc_proto::ShipId>, aim: lc_proto::Aim },
    /// Answer this craft automatically, or stop.
    AutoAck { with: lc_proto::ShipId, on: bool },

    // --- console ----------------------------------------------------------------------
    /// A line for the shard, as typed. See [`crate::console`].
    RunCommand(String),
    // A resend is [`Action::Say`] with the original's `idem`, not an action of its own: it is
    // the same message, said again, and the only thing that makes it one is the key.
}

/// What an action needs from outside: the few things the core cannot do itself.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Quit,
    StartGame,
    WriteSnapshot,
    Notify(String),
    /// The desktop sign-in, which needs a browser, a socket and the network — none of which
    /// belong in the action fold.
    SignIn,
    CancelSignIn,
    SignInWithPassword { email: String, password: String },
    SignOut,
    /// An order for the server. Emitted instead of a local change when a server is
    /// authoritative over the ship: see [`crate::session::Session::remote`].
    Send(lc_proto::Order),
    /// Ask for a scene. Not an [`Effect::Send`], because an order is something a *ship* does
    /// and this is not: it is a request to the thing that owns the world.
    Stage(String),
    /// Ask for energy, for the reason [`Effect::Stage`] is not an order.
    Grant(f64),
    /// A console line for the shard, which is the only thing that reads one.
    Command(String),
}

/// Where a scene says to stand, as the interface's own state.
///
/// `None` for a scene watched from the player's ship, which is what the camera has always done
/// and is what every scene but one asks for.
pub fn watching(scene: &lc_world::scenario::Scenario) -> Option<crate::ui::CameraPerspective> {
    let id = lc_world::scenario::Scenario::craft_for(scene.watch)?;
    Some(crate::ui::CameraPerspective::Pov(lc_proto::ShipId(id)))
}

/// Stops of exposure per keypress.
pub const EXPOSURE_STEP: f32 = 0.5;

/// Below this a "star" is the one the ship is already at, not a destination.
pub const INTERSTELLAR_LY: f64 = 0.01;

/// Radians per keypress of look.
pub const LOOK_STEP: f64 = 0.05;

/// Half-angle of the patch a targeted survey covers, radians. Twenty degrees is a few hundred
/// fields, so it comes round in hours where an all-sky pass takes weeks.
pub const SURVEY_CONE_RAD: f64 = 0.35;

/// What the drive will accept. The low end is a burn a crew could live in for decades; the
/// high end is where a crossing stops being something you watch happen.
pub const MIN_ACCEL_G: f64 = 0.1;
pub const MAX_ACCEL_G: f64 = 1000.0;

/// Apply an action. The only path that mutates UI state.
pub fn apply(action: Action, ui: &mut UiState, session: &mut Session) -> Vec<Effect> {
    let mut effects = Vec::new();
    match action {
        Action::OpenPanel(p) => ui.open(p),
        Action::ClosePanel(p) => ui.close(p),
        Action::TogglePanel(p) => ui.toggle(p),
        Action::CloseTopPanel => {
            // Nothing is modal, so "back" closes the most recently opened panel and opens
            // the escape menu only when there is nothing left to close.
            if ui.close_top().is_none() {
                ui.open(Panel::Escape);
            }
        }
        Action::GoToMenuPage(page) => ui.menu_page = page,
        Action::StartGame => effects.push(Effect::StartGame),
        Action::SignIn => effects.push(Effect::SignIn),
        Action::CancelSignIn => effects.push(Effect::CancelSignIn),
        Action::SignInWithPassword { email, password } => {
            effects.push(Effect::SignInWithPassword { email, password })
        }
        Action::SignOut => effects.push(Effect::SignOut),
        Action::Quit => effects.push(Effect::Quit),

        Action::OpenBook(file) => {
            if ui.reading.book.as_deref() != Some(file.as_str()) {
                ui.reading = crate::ui::Reading { book: Some(file), ..Default::default() };
            }
            ui.open(Panel::Reader);
        }
        // A hierarchy rather than two actions: the device shows a book or it shows the shelf,
        // and the way out of a book is the shelf. Closing the book is also what writes down
        // where it was left — see `crate::library::report_place`.
        Action::CloseBook => {
            ui.reading.contents = false;
            if ui.reading.book.is_some() {
                ui.reading.book = None;
            } else {
                ui.close(Panel::Reader);
            }
        }
        Action::TurnPage(by) => ui.reading.turn += by,
        Action::GoTo(spine, offset) => {
            ui.reading.goto = Some((spine, offset));
            ui.reading.contents = false;
        }
        Action::ToggleContents => ui.reading.contents = !ui.reading.contents,

        Action::SetBandPreset(i) => set_preset(ui, session, i, &mut effects),
        Action::NextBandPreset => {
            let next = (ui.preset + 1) % presets::all().len();
            set_preset(ui, session, next, &mut effects);
        }
        Action::PreviousBandPreset => {
            let count = presets::all().len();
            let prev = (ui.preset + count - 1) % count;
            set_preset(ui, session, prev, &mut effects);
        }

        Action::ExposureUp => adjust_exposure(ui, session, EXPOSURE_STEP),
        Action::ExposureDown => adjust_exposure(ui, session, -EXPOSURE_STEP),
        Action::ExposureAuto => {
            ui.exposure_offset = 0.0;
            session.auto_expose();
        }

        // What the panels describe, and nothing else: a click to read a star's provenance must
        // not end a week-long sweep. Staring is `StareSelected`.
        Action::SelectTarget(id) => {
            ui.selected = id;
            session.describe(id);
        }
        Action::StareSelected => match ui.selected {
            Some(id) => {
                let name = session.name_of(id);
                set_duty(ui, session, Duty::Stare(id), &mut effects);
                effects.push(Effect::Notify(format!("staring at {name}")));
            }
            None => effects.push(Effect::Notify("nothing selected to stare at".into())),
        },
        Action::SelectNearest => match nearest_interstellar(session) {
            Some(id) => apply_to(ui, session, Action::SelectTarget(Some(id)), &mut effects),
            None => effects.push(Effect::Notify("nothing interstellar in range".into())),
        },
        Action::SetIntegration(seconds) => {
            ui.integration_s = seconds.max(0.0);
            // The exposure is part of the duty, so a new one is a new order for the shard; the
            // panel shows the shard's figure until it says it has taken it.
            if session.observatory.duty != Duty::Idle {
                let duty = session.observatory.duty.clone();
                set_duty(ui, session, duty, &mut effects);
            }
        }
        Action::SurveySky | Action::SurveyAhead => {
            let now = session.coordinate_time_s();
            let sweep = if action == Action::SurveySky {
                Sweep::all_sky(now)
            } else {
                Sweep::region(ui.look.forward(), SURVEY_CONE_RAD, now)
            };
            let hours = sweep.pass_s() / 3600.0;
            set_duty(ui, session, Duty::Sweep(sweep), &mut effects);
            effects.push(Effect::Notify(format!(
                "surveying: a pass every {hours:.1} hours"
            )));
        }
        Action::NameSelected(name) => match ui.selected {
            // A name is the shard's to record, like a course: sent, and back in what the craft
            // is told it knows.
            Some(id) if session.remote && session.knows(id) => {
                let name = name.trim().to_string();
                effects.push(Effect::Send(lc_proto::Order::NameIt { subject: lc_proto::Subject::Star(id.get()), name }));
            }
            Some(id) if !session.remote && session.name_star(id, &name) => {
                effects.push(Effect::Notify(format!("noted: {}", session.name_of(id))));
            }
            Some(_) => effects.push(Effect::Notify("nothing detected there to name".into())),
            None => effects.push(Effect::Notify("nothing selected to name".into())),
        },
        // Only a shard reads logs, so with none there is nothing to analyze.
        Action::Analyze => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::Analyze));
            } else {
                effects.push(Effect::Notify("no server, so nothing reads the logs".into()));
            }
        }
        Action::RetainRaw(id, keep) => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::RetainRaw { subject: lc_proto::Subject::Star(id.get()), keep }));
            } else {
                session.knowledge.retain_raw(id, keep);
            }
        }
        Action::WatchSelected => match ui.selected {
            Some(id) => {
                let mut targets = match &session.observatory.duty {
                    Duty::Watch { targets, .. } => targets.clone(),
                    _ => Vec::new(),
                };
                let dropped = targets.iter().position(|t| *t == id);
                match dropped {
                    Some(at) => {
                        targets.remove(at);
                    }
                    None => targets.push(id),
                }
                let count = targets.len();
                let duty = if targets.is_empty() {
                    Duty::Idle
                } else {
                    let started_s = session.coordinate_time_s();
                    let dwell_s = ui.integration_s.clamp(lc_proto::DWELL_MIN_S, lc_proto::DWELL_MAX_S);
                    Duty::Watch { targets, dwell_s, started_s }
                };
                set_duty(ui, session, duty, &mut effects);
                effects.push(Effect::Notify(format!("watching {count} stars")));
            }
            None => effects.push(Effect::Notify("nothing selected to watch".into())),
        },
        Action::StopSurvey => {
            set_duty(ui, session, Duty::Idle, &mut effects);
            effects.push(Effect::Notify("telescope idle".into()));
        }
        Action::SetCurveBand(band) => {
            if !session.telescope.sees(band) {
                effects.push(Effect::Notify(format!("the sensor cannot reach {band:?}")));
            } else if session.curve_band != band {
                session.set_curve_band(band);
                effects.push(Effect::Notify(format!("curve: {band:?}")));
            }
        }

        Action::Look { yaw, pitch } => ui.look.turn(yaw, pitch),

    Action::SetView(view) => ui.view = view,
    Action::ToggleView => ui.view = ui.view.other(),
    Action::TurnMap { azimuth, elevation } => ui.map.orbit.turn(azimuth, elevation),
    Action::ZoomMap { notches, anchor_ly } => match anchor_ly {
        Some(anchor) => {
            // Moving the focus is a pan by another name, so it gives up following — but only
            // if it moved. The wheel over a locked center scales about the center itself,
            // which changes nothing and must not cost the lock.
            if ui.map.orbit.zoom_about(anchor, notches) {
                ui.map.focus = crate::ui::MapFocus::Free;
            }
        }
        None => ui.map.orbit.zoom(notches),
    },
    Action::PanMap { right, ahead } => {
        let plane = ui.map.plane;
        ui.map.orbit.pan(plane, right, ahead);
        // A pan is a statement about where to look, so it gives up following anything.
        ui.map.focus = crate::ui::MapFocus::Free;
    }
    Action::SetMapPlane(plane) => ui.map.plane = plane,
    Action::ToggleMapPlane => ui.map.plane = ui.map.plane.other(),
    Action::FocusMap(key) => ui.map.focus = key,
    Action::SetMapSource(source) => {
        #[cfg(feature = "godview")]
        if source == crate::map_source::Source::God && !ui.may_see_everything {
            effects.push(Effect::Notify("god view needs an administrative account".into()));
            return effects;
        }
        ui.map.source = source;
    }

        // Multiplicative, because the range is two and a half decades: a fixed step is either
        // imperceptible at the far end or the whole range in one notch at the near one. Left
        // unclamped here and clamped against the viewport by `hull::place_eye`, which is the
        // only thing that knows how wide a pixel is.
        Action::Zoom(notches) => {
            ui.boom_lengths = (ui.boom_lengths * crate::hull::ZOOM_STEP.powf(-notches))
                .clamp(f64::MIN_POSITIVE, 1.0e9);
        }
        Action::LookAtSelected => match aim(ui, session) {
            Some(look) => ui.look = look,
            None => effects.push(Effect::Notify("nothing is selected to look at".into())),
        },

        Action::LookAtStation => {
            let now = session.coordinate_time_s();
            let at = session
                .station()
                .zip(session.system.as_ref())
                .and_then(|(station, system)| station.focus(system, now));
            match at.and_then(|at| Look::aimed_at(at - session.ship.motion.position_ly)) {
                Some(look) => ui.look = look,
                None => effects.push(Effect::Notify("not on a station".into())),
            }
        }

        Action::FlyTo(id) => fly(ui, session, id, &mut effects),
        Action::FlyToNearest => {
            apply_to(ui, session, Action::SelectNearest, &mut effects);
            let id = ui.selected;
            if id.is_some() {
                fly(ui, session, id, &mut effects);
            }
        }
        Action::AbortFlight => {
            if session.remote {
                if session.cruise().is_some() || session.station().is_some() {
                    effects.push(Effect::Send(lc_proto::Order::CutDrive));
                    effects.push(Effect::Notify("cut sent".into()));
                }
            } else if session.cruise().is_some() || session.station().is_some() {
                let note = match session.cancel() {
                    // What it says is where the ship ended up, because canceling does not
                    // stop it: it keeps its velocity and that velocity is now an orbit.
                    Some(coast) => format!("drive cut — {}", crate::hud::arc(&coast, &session.body_label(&coast.primary))),
                    None => "drive cut".to_string(),
                };
                effects.push(Effect::Notify(note));
            }
        }
        Action::FocusTarget(target) => {
            ui.course = None;
            ui.focus = target;
        }
        Action::ChooseCourse(course) => ui.course = course,
        // Both of these only exist against a server. An intercept is a *standing* order and
        // what makes it stand is the authority re-solving it against sightings; a client
        // holding the policy itself would be a client steering by a quarry it can only see
        // the past of, which is the one thing the design will not have.
        Action::Intercept(ship_id, closeness) => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::Intercept { ship_id, closeness }));
            } else {
                effects.push(Effect::Notify("no server, so nobody to close on".into()));
            }
        }
        Action::BreakOff => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::BreakOff));
            }
        }

        Action::OpenChat(ship_id) => {
            ui.chat_with = crate::ui::Channel::With(ship_id);
            ui.open(Panel::Chat);
        }
        Action::ChatWith(channel) => ui.chat_with = channel,
        // Sent and never applied locally, for the same reason a course is: what a transmission
        // becomes is an event with an identifier, and the identifier is the server's to mint.
        // The client learns of its own message when the acceptance comes back.
        Action::Say { to, aim, secrecy, body, idem } => {
            let body = body.trim().to_string();
            // Somebody pressing send on an empty field. Acknowledging is the server's, not this.
            if body.is_empty() {
            } else if !session.remote {
                effects.push(Effect::Notify("no server, so nobody to talk to".into()));
            } else if body.len() > lc_proto::MESSAGE_LIMIT {
                effects.push(Effect::Notify(format!(
                    "too long by {} characters",
                    body.len() - lc_proto::MESSAGE_LIMIT
                )));
            } else {
                // Minted here and never by the server, because only the sender knows that a
                // resend *is* one. A hash of who, when and what: unique to this ship without
                // a counter either end would have to reconcile.
                let idem = idem.unwrap_or_else(|| {
                    lc_world::rng::hash(&[
                        to.map_or(0, |t| t.0 as u64),
                        (session.coordinate_time_s() * 1.0e6) as u64,
                        body.len() as u64,
                        body.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)),
                    ])
                    // Zero means "not keyed" to every reader, so it is the one value a real
                    // key may not take.
                    .max(1)
                });
                effects.push(Effect::Send(lc_proto::Order::Say { to, aim, secrecy, body, idem }));
            }
        }
        Action::SendReport { to, aim, secrecy } => {
            if session.remote {
                // The shard writes the report from what it holds for this craft, and says
                // `NothingNew` if there is nothing to send: this client does not keep the marks.
                let now_us = (session.coordinate_time_s() * 1.0e6) as u64;
                let idem = lc_world::rng::hash(&[to.map_or(0, |t| t.0 as u64), now_us]).max(1);
                effects.push(Effect::Send(lc_proto::Order::SendReport { to, aim, secrecy, idem }));
            } else {
                effects.push(Effect::Notify("no server, so nobody to report to".into()));
            }
        }
        // A standing order the server keeps, so it answers with nobody flying the ship. The
        // checkbox shows what the server says back, not what was clicked.
        Action::AutoAck { with, on } => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::AutoAck { with, on }));
            } else {
                effects.push(Effect::Notify("no server, so nobody to answer".into()));
            }
        }
        Action::OfferKey { to, aim } => {
            if session.remote {
                effects.push(Effect::Send(lc_proto::Order::OfferKey { to, aim }));
            } else {
                effects.push(Effect::Notify("no server, so nobody to give a key to".into()));
            }
        }
        Action::SetCourse(course) => {
            if session.remote {
                // Sent, not applied. What the server does with it comes back as `Accepted`,
                // carrying the acceleration it actually flew and the time it actually used.
                effects.push(Effect::Send(lc_proto::Order::SetCourse {
                    course: (&course).into(),
                    accel_g: session.ship.motion.drive.accel_g,
                    max_beta: session.ship.motion.drive.max_beta,
                }));
                effects.push(Effect::Notify("course sent".into()));
            } else {
                let note = match session.set_course(&course) {
                    Some(label) => format!("course: {label}"),
                    // Not an error dialogue: the interface offers what the system has, so this
                    // is reachable only by a stale panel or a test.
                    None => "nothing there to go to".to_string(),
                };
                effects.push(Effect::Notify(note));
            }
        }

        Action::SetDriveAccel(g) => {
            session.ship.motion.drive.accel_g = g.clamp(MIN_ACCEL_G, MAX_ACCEL_G);
            effects.push(Effect::Notify(format!("drive set to {:.0} g", session.ship.motion.drive.accel_g)));
        }

        Action::SetPointStyle { which, style } => match which {
            Which::Distant => ui.distant = style,
            Which::Local => ui.local = style,
            Which::Bodies => ui.bodies = style,
        },
        Action::SetEnvelopeGain(gain) => ui.envelope_gain = gain.max(0.0),
        Action::ResetPointStyle { which } => {
            match which {
                Which::Distant => ui.distant = crate::starfield::DISTANT,
                Which::Local => ui.local = crate::starfield::LOCAL,
                Which::Bodies => ui.bodies = crate::starfield::BODIES,
            }
            effects.push(Effect::Notify("starfield reset".into()));
        }

        Action::ToggleGodView => {
            if cfg!(feature = "godview") {
                ui.god_view = !ui.god_view;
            } else {
                // Not merely hidden: the code is not in this build.
                effects.push(Effect::Notify("god view is not compiled into this build".into()));
            }
        }
        // **The server owns the rate**, which `lightcone/docs/13-client-shell.md` calls dev
        // only and says why: a client that can change it is a client that can cheat. It is also
        // the client that suffers — its clock runs away from the server's, so an order comes
        // back stamped in its own past and folds as a maneuvere that already finished. The ship
        // appears to teleport, and the server goes on refusing orders about a system it does
        // not believe the ship has reached.
        Action::SetTimeRate(_) | Action::TimeRateUp | Action::TimeRateDown if session.remote => {
            effects.push(Effect::Notify("the server keeps the clock".into()));
        }
        Action::SetTimeRate(rate) => ui.time_rate = rate.max(0.0),
        Action::TimeRateUp | Action::TimeRateDown => {
            ui.time_rate = crate::ui::rate_step(ui.time_rate, action == Action::TimeRateUp);
            effects.push(Effect::Notify(format!("clock: {}", crate::ui::rate_label(ui.time_rate))));
        }
        Action::WriteSnapshot => effects.push(Effect::WriteSnapshot),
        // Only a shard can do this, and only one started for it will. Offline there is no
        // authority to ask and nothing that could honor the answer.
        Action::StageDemo(name) if !session.remote => {
            let _ = name;
            effects.push(Effect::Notify("no server, so nowhere to stage a scene".into()));
        }
        Action::StageDemo(name) => {
            // The scene says where to stand, so staging one moves the camera to wherever it is
            // about. Set here rather than waiting for the shard to answer: the perspective is
            // the interface's own state and there is nothing to ask anybody.
            if let Some(scene) = lc_world::scenario::Scenario::named(&name) {
                ui.perspective = watching(scene);
            }
            effects.push(Effect::Stage(name));
        }
        // The eye only. What the client works out about light is still solved from the ship
        // the session owns — see [`crate::ui::CameraPerspective::Pov`], which says what that
        // costs and where it would start to show.
        Action::WatchFrom(ship_id) => {
            ui.perspective = ship_id.map(crate::ui::CameraPerspective::Pov);
            let said = match ship_id {
                Some(_) => "watching from another craft",
                None => "back aboard your own ship",
            };
            effects.push(Effect::Notify(said.into()));
        }

        // Energy and refits exist only against a server: the account is its to keep.
        Action::GrantEnergy(_) | Action::FillStorage | Action::ApplyRefit | Action::CancelRefit
            if !session.remote =>
        {
            effects.push(Effect::Notify("no server, so nothing to refit or fill".into()));
        }
        Action::GrantEnergy(joules) => effects.push(Effect::Grant(joules)),
        Action::RunCommand(line) => effects.push(Effect::Command(line)),
        Action::FillStorage => {
            let now = session.coordinate_time_s();
            if let Some(fitting) = session.ship.fitting() {
                let room = fitting.capacity_j_at(now) - fitting.stored_j_at(&session.ship.motion, now);
                if room > 0.0 {
                    effects.push(Effect::Grant(room));
                }
            }
        }
        Action::DraftRefit(loadout) => ui.refit_draft = Some(loadout),
        Action::ResetRefitDraft => ui.refit_draft = None,
        Action::ApplyRefit => {
            // Kept, so a refused refit leaves the sliders where they were.
            if let Some(target) = ui.refit_draft {
                effects.push(Effect::Send(lc_proto::Order::Refit { target: target.into() }));
            }
        }
        Action::CancelRefit => effects.push(Effect::Send(lc_proto::Order::CancelRefit)),
    }
    effects
}

/// Run a nested action, keeping its effects. Only for actions composed of other actions.
fn apply_to(ui: &mut UiState, session: &mut Session, action: Action, effects: &mut Vec<Effect>) {
    effects.extend(apply(action, ui, session));
}

/// The nearest star worth pointing at.
///
/// Not simply the first: the catalogue carries the Sun at about an astronomical unit, and
/// "nearest star" has to mean one that is somewhere else.
/// Put the telescope on a duty.
///
/// With a shard the duty is the shard's to take up, and nothing is taken up here: the
/// acceptance, or the refusal, is what the panel shows, so a refused order leaves nothing
/// to undo. Without one it starts now.
fn set_duty(ui: &UiState, session: &mut Session, duty: Duty, effects: &mut Vec<Effect>) {
    if session.remote {
        effects.push(Effect::Send(lc_proto::Order::SetDuty { duty: (&duty).into(), integration_s: ui.integration_s }));
        return;
    }
    session.observatory.integration_s = ui.integration_s.max(1.0);
    match duty {
        Duty::Stare(id) => session.point_at(Some(id)),
        duty => session.take_up(duty),
    }
}

/// The nearest star this ship has a position for, other than the one it is sitting in.
///
/// Out of what is *known*: the key picks a target to watch, and a target nobody has detected
/// is not one the ship could name, let alone point at.
fn nearest_interstellar(session: &Session) -> Option<StarId> {
    let here = session.ship.motion.position_ly;
    session
        .knowledge
        .stars()
        .filter_map(|(id, b)| Some((id, b.distance.from(here)?)))
        .filter(|(_, ly)| *ly > INTERSTELLAR_LY)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn aim(ui: &UiState, session: &Session) -> Option<Look> {
    let star = session.star(ui.selected?)?;
    Look::aimed_at(session.offset_to(star))
}

fn fly(ui: &mut UiState, session: &mut Session, id: Option<StarId>, effects: &mut Vec<Effect>) {
    let Some(id) = id.or(ui.selected) else {
        effects.push(Effect::Notify("no destination selected".into()));
        return;
    };
    let Some(star) = session.star(id) else {
        effects.push(Effect::Notify("that star is not loaded".into()));
        return;
    };
    let name = session.name_of(id);

    // A crossing stops short of a star, so a ship already inside a system is nearer than one
    // would leave it. Said here rather than sent, because the answer would come back as a bare
    // refusal and "the server refused that order" explains nothing about being already there.
    let reach = star.position_ly - session.ship.motion.position_ly;
    if reach.length() <= crate::flight::STANDOFF_LY {
        effects.push(Effect::Notify(format!("already at {name}")));
        return;
    }

    if session.remote {
        // Sent, not flown. The crossing this client would plan and the one the server flies
        // must be the same, so only one of them plans it — and it is the one with authority.
        effects.push(Effect::Send(lc_proto::Order::Cross {
            star: id.get(),
            accel_g: session.ship.motion.drive.accel_g,
            max_beta: session.ship.motion.drive.max_beta,
        }));
        // Looking at the destination is what anybody wants by default, and it costs nothing
        // to do before the answer arrives.
        if let Some(look) = Look::aimed_at(session.offset_to(session.star(id).unwrap())) {
            ui.look = look;
        }
        effects.push(Effect::Notify(format!("{name}: course sent")));
        return;
    }

    let Some(cruise) = session.fly_to(id) else { return };
    let (years, aboard) = (
        cruise.duration_s() / crate::flight::JULIAN_YEAR_S,
        cruise.proper_duration_s() / crate::flight::JULIAN_YEAR_S,
    );
    // Looking somewhere else during a crossing is allowed; starting one pointed at the
    // destination is what anybody wants by default.
    if let Some(look) = Look::aimed_at(session.offset_to(session.star(id).unwrap())) {
        ui.look = look;
    }
    effects.push(Effect::Notify(format!("{name}: {years:.2} years out, {aboard:.2} aboard")));
}

fn set_preset(ui: &mut UiState, session: &mut Session, index: usize, effects: &mut Vec<Effect>) {
    let all = presets::all();
    let Some((name, mapping)) = all.get(index) else {
        effects.push(Effect::Notify(format!("no band preset {index}")));
        return;
    };
    ui.preset = index;
    session.mapping = *mapping;
    session.retune();
    // The window follows the mapping: a different band is a different brightness.
    session.auto_expose();
    apply_exposure_offset(ui, session);
    effects.push(Effect::Notify(format!("band mapping: {name}")));
}

fn adjust_exposure(ui: &mut UiState, session: &mut Session, stops: f32) {
    ui.exposure_offset = (ui.exposure_offset + stops).clamp(-12.0, 12.0);
    session.auto_expose();
    apply_exposure_offset(ui, session);
}

/// Re-place the exposure window and re-apply the user's offset on top of it.
///
/// Public because the window has to be replaced whenever the scene's brightness moves on its
/// own — flying toward a star changes it by tens of stops without anyone touching a control.
pub fn refresh_exposure(ui: &UiState, session: &mut Session) {
    session.auto_expose();
    apply_exposure_offset(ui, session);
}

fn apply_exposure_offset(ui: &UiState, session: &mut Session) {
    session.tone = session.tone.exposed(ui.exposure_offset);
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::ui::{RATE_LADDER, rate_label};

    fn fixture() -> (UiState, Session) {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        // Charted, because a ship leaves port with charts and most of these tests are about
        // something else. `session::tests` is where an unsurveyed sky is the subject.
        session.issue_charts(30.0);
        (UiState::default(), session)
    }

    /// The map is a mode of the main view, so the key that shows it puts it away again and
    /// asking for the mode already in force is not a toggle.
    #[test]
    fn the_map_is_the_other_mode_of_the_main_view() {
        let (mut ui, mut s) = fixture();
        assert_eq!(ui.view, ViewMode::World, "a session starts flying");
        apply(Action::ToggleView, &mut ui, &mut s);
        assert_eq!(ui.view, ViewMode::Map);
        apply(Action::ToggleView, &mut ui, &mut s);
        assert_eq!(ui.view, ViewMode::World);
        apply(Action::SetView(ViewMode::Map), &mut ui, &mut s);
        apply(Action::SetView(ViewMode::Map), &mut ui, &mut s);
        assert_eq!(ui.view, ViewMode::Map);
    }

    /// **Switching the view leaves the map where it was.** The corner square and the whole
    /// screen are one camera, so coming back has to find the picture that was left — including
    /// the free camera a pan drops into, which is the state with no button of its own.
    #[test]
    fn switching_the_view_leaves_the_map_where_it_was() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetMapPlane(em_map::Plane::Galactic), &mut ui, &mut s);
        apply(Action::TurnMap { azimuth: 0.4, elevation: 0.1 }, &mut ui, &mut s);
        apply(Action::ZoomMap { notches: 2.0, anchor_ly: None }, &mut ui, &mut s);
        apply(Action::PanMap { right: 0.3, ahead: -0.2 }, &mut ui, &mut s);
        assert_eq!(ui.map.focus, crate::ui::MapFocus::Free, "a pan is the way into it");

        // Both ways in, because either could be the one that forgets.
        let held = ui.map;
        apply(Action::ToggleView, &mut ui, &mut s);
        assert_eq!(ui.map, held, "showing the map moved its camera");
        apply(Action::SetView(ViewMode::World), &mut ui, &mut s);
        assert_eq!(ui.view, ViewMode::World, "back where it started");
        assert_eq!(ui.map, held, "the map's camera did not survive the round trip");
    }

    #[test]
    fn panels_open_close_and_toggle_independently() {
        let (mut ui, mut s) = fixture();
        apply(Action::OpenPanel(Panel::Telescope), &mut ui, &mut s);
        apply(Action::OpenPanel(Panel::Debug), &mut ui, &mut s);
        assert!(ui.is_open(Panel::Telescope) && ui.is_open(Panel::Debug));
        // Nothing is modal: opening one does not close another.
        apply(Action::TogglePanel(Panel::Telescope), &mut ui, &mut s);
        assert!(!ui.is_open(Panel::Telescope) && ui.is_open(Panel::Debug));
    }

    #[test]
    fn back_closes_the_newest_panel_and_then_opens_the_escape_menu() {
        let (mut ui, mut s) = fixture();
        apply(Action::OpenPanel(Panel::Telescope), &mut ui, &mut s);
        apply(Action::OpenPanel(Panel::System), &mut ui, &mut s);
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(!ui.is_open(Panel::System) && ui.is_open(Panel::Telescope));
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(!ui.is_open(Panel::Telescope));
        apply(Action::CloseTopPanel, &mut ui, &mut s);
        assert!(ui.is_open(Panel::Escape), "with nothing left, back means the menu");
    }

    #[test]
    fn a_band_preset_reaches_the_session_and_re_exposes() {
        let (mut ui, mut s) = fixture();
        let before = s.tone.reference;
        let effects = apply(Action::SetBandPreset(2), &mut ui, &mut s);
        assert_eq!(ui.preset, 2);
        assert_eq!(s.mapping, presets::all()[2].1);
        assert_ne!(s.tone.reference, before, "a different band is a different brightness");
        assert!(matches!(effects.as_slice(), [Effect::Notify(_)]));
    }

    #[test]
    fn preset_cycling_wraps_both_ways() {
        let (mut ui, mut s) = fixture();
        let count = presets::all().len();
        for _ in 0..count {
            apply(Action::NextBandPreset, &mut ui, &mut s);
        }
        assert_eq!(ui.preset, 0, "a full cycle returns to the start");
        apply(Action::PreviousBandPreset, &mut ui, &mut s);
        assert_eq!(ui.preset, count - 1);
    }

    #[test]
    fn an_unknown_preset_is_reported_rather_than_applied() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::SetBandPreset(99), &mut ui, &mut s);
        assert_eq!(ui.preset, 0);
        assert!(matches!(effects.as_slice(), [Effect::Notify(m)] if m.contains("99")));
    }

    #[test]
    fn exposure_steps_in_stops_and_returns_to_auto() {
        let (mut ui, mut s) = fixture();
        let auto = s.tone.reference;
        apply(Action::ExposureUp, &mut ui, &mut s);
        assert!((ui.exposure_offset - EXPOSURE_STEP).abs() < 1e-6);
        assert!(s.tone.reference < auto, "opening up lowers the reference");
        apply(Action::ExposureAuto, &mut ui, &mut s);
        assert_eq!(ui.exposure_offset, 0.0);
        assert!((s.tone.reference - auto).abs() < auto * 1e-6);
    }

    /// Zoom is multiplicative and unbounded here on purpose: how close the camera may come is
    /// an angle, and only the frame that knows how wide a pixel is can clamp it.
    #[test]
    fn zooming_scales_the_boom_rather_than_stepping_it() {
        let (mut ui, mut s) = fixture();
        let start = ui.boom_lengths;
        apply(Action::Zoom(1.0), &mut ui, &mut s);
        let closer = ui.boom_lengths;
        assert!(closer < start, "a notch in must come closer: {start} to {closer}");
        apply(Action::Zoom(-1.0), &mut ui, &mut s);
        assert!((ui.boom_lengths - start).abs() < start * 1e-9, "a notch back is where it began");

        // Ten notches out is the same as one notch out ten times, which is what a wheel with a
        // pixel-precision device actually sends.
        let mut once = UiState::default();
        apply(Action::Zoom(-10.0), &mut once, &mut s);
        let mut ten = UiState::default();
        for _ in 0..10 {
            apply(Action::Zoom(-1.0), &mut ten, &mut s);
        }
        assert!((once.boom_lengths - ten.boom_lengths).abs() < ten.boom_lengths * 1e-9);
    }

    #[test]
    fn exposure_is_bounded() {
        let (mut ui, mut s) = fixture();
        for _ in 0..200 {
            apply(Action::ExposureUp, &mut ui, &mut s);
        }
        assert!(ui.exposure_offset <= 12.0);
        assert!(s.tone.reference.is_finite() && s.tone.reference > 0.0);
    }

    /// Review item 4. Selecting a star changes what the panels describe and nothing else: a
    /// sweep under way stays under way. Staring is its own, deliberate action.
    #[test]
    fn selecting_describes_and_staring_is_deliberate() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::SurveySky, &mut ui, &mut s);
        let effects = apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        assert_eq!((ui.selected, s.described), (Some(id), Some(id)));
        assert!(matches!(s.observatory.duty, Duty::Sweep(_)), "the sweep goes on");
        assert!(effects.is_empty(), "nothing was ordered");

        let effects = apply(Action::StareSelected, &mut ui, &mut s);
        assert_eq!(s.pointing, Some(id));
        assert!(matches!(s.observatory.duty, Duty::Stare(on) if on == id));
        assert!(matches!(effects.as_slice(), [Effect::Notify(_)]));
    }

    #[test]
    fn god_view_is_absent_rather_than_hidden() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::ToggleGodView, &mut ui, &mut s);
        if cfg!(feature = "godview") {
            assert!(ui.god_view);
        } else {
            assert!(!ui.god_view);
            assert!(matches!(effects.as_slice(), [Effect::Notify(m)] if m.contains("not compiled")));
        }
    }

    #[test]
    fn effects_are_returned_rather_than_performed() {
        let (mut ui, mut s) = fixture();
        assert_eq!(apply(Action::Quit, &mut ui, &mut s), vec![Effect::Quit]);
        assert_eq!(apply(Action::WriteSnapshot, &mut ui, &mut s), vec![Effect::WriteSnapshot]);
        assert_eq!(apply(Action::StartGame, &mut ui, &mut s), vec![Effect::StartGame]);
    }

    /// A session driven entirely by actions, which is what makes the UI testable.
    #[test]
    fn a_whole_interaction_runs_with_no_window() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        for action in [
            Action::StartGame,
            Action::SelectTarget(Some(id)),
            Action::StareSelected,
            Action::OpenPanel(Panel::Telescope),
            Action::SetBandPreset(2),
            Action::ExposureDown,
            Action::SetIntegration(1e4),
        ] {
            apply(action, &mut ui, &mut s);
        }
        assert!(ui.is_open(Panel::Telescope));
        assert_eq!(ui.preset, 2);
        assert_eq!(ui.integration_s, 1e4);
        s.observe(ui.integration_s);
        assert!(
            !s.curve().is_empty(),
            "the telescope should have recorded something"
        );
    }

    #[test]
    fn looking_accumulates_and_the_pitch_stops_at_the_pole() {
        let (mut ui, mut s) = fixture();
        apply(Action::Look { yaw: 0.3, pitch: 0.2 }, &mut ui, &mut s);
        apply(Action::Look { yaw: 0.3, pitch: 0.2 }, &mut ui, &mut s);
        assert!((ui.look.yaw - 0.6).abs() < 1e-12);
        assert!((ui.look.pitch - 0.4).abs() < 1e-12);
        for _ in 0..200 {
            apply(Action::Look { yaw: 0.0, pitch: 0.5 }, &mut ui, &mut s);
        }
        assert!(ui.look.pitch <= Look::PITCH_LIMIT, "{}", ui.look.pitch);
        assert!(ui.look.forward().is_finite());
    }

    #[test]
    fn yaw_wraps_rather_than_growing_without_bound() {
        let (mut ui, mut s) = fixture();
        for _ in 0..1000 {
            apply(Action::Look { yaw: 1.0, pitch: 0.0 }, &mut ui, &mut s);
        }
        assert!(ui.look.yaw >= 0.0 && ui.look.yaw < std::f64::consts::TAU, "{}", ui.look.yaw);
    }

    #[test]
    fn looking_at_the_selection_points_the_camera_at_it() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert_eq!(apply(Action::LookAtSelected, &mut ui, &mut s).len(), 1, "nothing selected");
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        apply(Action::LookAtSelected, &mut ui, &mut s);
        let want = s.offset_to(s.star(id).unwrap()).normalize();
        assert!((ui.look.forward() - want).length() < 1e-12, "{:?} vs {want:?}", ui.look.forward());
    }

    #[test]
    fn flying_with_nothing_selected_says_so_and_starts_nothing() {
        let (mut ui, mut s) = fixture();
        let effects = apply(Action::FlyTo(None), &mut ui, &mut s);
        assert!(matches!(effects.as_slice(), [Effect::Notify(t)] if t.contains("no destination")));
        assert!(s.cruise().is_none());
    }

    #[test]
    fn flying_uses_the_selection_and_reports_both_clocks() {
        let (mut ui, mut s) = fixture();
        apply(Action::SelectTarget(Some(s.stars[0].id)), &mut ui, &mut s);
        let effects = apply(Action::FlyTo(None), &mut ui, &mut s);
        assert!(s.cruise().is_some());
        let text = effects
            .iter()
            .find_map(|e| match e {
                Effect::Notify(t) if t.contains("aboard") => Some(t.clone()),
                _ => None,
            })
            .expect("a crossing report");
        assert!(text.contains("years out"), "{text}");
    }

    /// Starting a crossing turns the camera to the destination; nothing else may.
    #[test]
    fn starting_a_crossing_aims_the_camera_at_it() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::Look { yaw: 2.0, pitch: -0.5 }, &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        let want = s.offset_to(s.star(id).unwrap()).normalize();
        assert!((ui.look.forward() - want).length() < 1e-12);
    }

    #[test]
    fn cutting_a_drive_that_is_not_running_says_nothing() {
        let (mut ui, mut s) = fixture();
        assert!(apply(Action::AbortFlight, &mut ui, &mut s).is_empty());
        apply(Action::FlyTo(Some(s.stars[0].id)), &mut ui, &mut s);
        assert_eq!(apply(Action::AbortFlight, &mut ui, &mut s).len(), 1);
        assert!(s.cruise().is_none());
    }

    #[test]
    fn the_drive_setting_is_clamped_to_something_flyable() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetDriveAccel(1e9), &mut ui, &mut s);
        assert_eq!(s.ship.motion.drive.accel_g, MAX_ACCEL_G);
        apply(Action::SetDriveAccel(-4.0), &mut ui, &mut s);
        assert_eq!(s.ship.motion.drive.accel_g, MIN_ACCEL_G);
    }

    /// The setting has to reach the crossing, not just the readout.
    #[test]
    fn a_harder_drive_plans_a_shorter_crossing() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::SetDriveAccel(1.0), &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        let slow = s.cruise().as_ref().unwrap().duration_s();
        apply(Action::SetDriveAccel(50.0), &mut ui, &mut s);
        apply(Action::FlyTo(Some(id)), &mut ui, &mut s);
        assert!(s.cruise().as_ref().unwrap().duration_s() < slow);
    }

    #[test]
    fn the_clock_steps_along_the_ladder_and_stops_at_the_ends() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(RATE_LADDER[0].0), &mut ui, &mut s);
        apply(Action::TimeRateDown, &mut ui, &mut s);
        assert_eq!(ui.time_rate, RATE_LADDER[0].0, "the bottom rung is the bottom");
        for _ in 0..20 {
            apply(Action::TimeRateUp, &mut ui, &mut s);
        }
        assert_eq!(ui.time_rate, RATE_LADDER[RATE_LADDER.len() - 1].0, "and the top is the top");
    }

    #[test]
    fn stepping_from_a_rate_off_the_ladder_still_moves_the_right_way() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(200.0), &mut ui, &mut s);
        apply(Action::TimeRateUp, &mut ui, &mut s);
        assert!(ui.time_rate > 200.0, "went to {}", ui.time_rate);
        apply(Action::SetTimeRate(200.0), &mut ui, &mut s);
        apply(Action::TimeRateDown, &mut ui, &mut s);
        assert!(ui.time_rate < 200.0, "went to {}", ui.time_rate);
    }

    /// Every rung has to be reachable by stepping, or a key cannot get to it.
    #[test]
    fn every_rung_is_reachable_by_stepping_up_from_the_bottom() {
        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(RATE_LADDER[0].0), &mut ui, &mut s);
        for (rate, _) in RATE_LADDER.iter().skip(1) {
            apply(Action::TimeRateUp, &mut ui, &mut s);
            assert_eq!(ui.time_rate, *rate, "the ladder skipped a rung");
        }
    }

    #[test]
    fn a_rate_is_named_by_what_it_feels_like() {
        assert_eq!(rate_label(60.0), "1 year / minute");
        assert_eq!(rate_label(360.0), "1 year / 10 s");
        // One off the ladder reads as a period too, not as a factor — the point of the label
        // is that a rate is something you can feel, and "123x" is not.
        assert_eq!(rate_label(123.0), "1 year / 29 seconds");
        assert_eq!(rate_label(0.05), "7 minutes / second");
    }

    /// The exit criterion, driven entirely through actions with no window.
    #[test]
    fn a_swarm_reads_as_a_deficit_in_the_visible_and_an_excess_in_the_thermal() {
        use lc_world::sky::{AuthoredStars, StarProvider};
        let provider = AuthoredStars::sample();
        let Some(star) = provider.stars().iter().find(|s| {
            lc_world::sky::generate::swarm_for(s).is_some()
        }) else {
            // The sample sky is three stars and may carry no swarm. The catalogue test covers
            // the populated case; this one has nothing to say.
            return;
        };
        let mut s = Session::new(&provider, 3);
        let mut ui = UiState::default();
        apply(Action::SelectTarget(Some(star.id)), &mut ui, &mut s);
        apply(Action::StareSelected, &mut ui, &mut s);

        let watch = |s: &mut Session, ui: &mut UiState, band: Band| {
            apply(Action::SetCurveBand(band), ui, s);
            for _ in 0..40 {
                s.advance(30.0);
                s.observe(1.0e4);
            }
            let samples = s.curve().samples().to_vec();
            samples.iter().map(|(_, d)| *d).sum::<f64>() / samples.len() as f64
        };
        assert!(watch(&mut s, &mut ui, Band::V) > 0.0, "the visible must be a shadow");
        assert!(watch(&mut s, &mut ui, Band::ThermalIr) < 0.0, "and the thermal a source");
    }

    /// One exposure measures every band the sensor has, so choosing which to read is a choice
    /// about the display and not about what was recorded.
    #[test]
    fn changing_the_curve_band_reads_the_other_series_rather_than_losing_one() {
        let (mut ui, mut s) = fixture();
        apply(Action::SelectTarget(Some(s.stars[0].id)), &mut ui, &mut s);
        apply(Action::StareSelected, &mut ui, &mut s);
        s.observe(1.0e4);
        assert!(!s.curve().is_empty());
        apply(Action::SetCurveBand(Band::K), &mut ui, &mut s);
        assert_eq!(s.curve().band(), Band::K);
        assert!(
            !s.curve().is_empty(),
            "the K exposure happened at the same time as the V one"
        );
        apply(Action::SetCurveBand(Band::V), &mut ui, &mut s);
        assert!(
            !s.curve().is_empty(),
            "and nothing was thrown away to look at it"
        );
    }

    #[test]
    fn a_band_the_sensor_cannot_reach_is_refused_rather_than_measured() {
        let (mut ui, mut s) = fixture();
        s.telescope = s.telescope.with_bands(em_spectra::BandMask::SILICON);
        let effects = apply(Action::SetCurveBand(Band::Radio), &mut ui, &mut s);
        assert!(matches!(effects.as_slice(), [Effect::Notify(t)] if t.contains("cannot reach")));
        assert_ne!(s.curve().band(), Band::Radio);
    }

    /// The telescope must not be limited to the stars the sky happens to model.
    #[test]
    fn any_star_in_the_list_can_be_watched() {
        let (mut ui, mut s) = fixture();
        let last = s.stars.last().unwrap().id;
        apply(Action::SelectTarget(Some(last)), &mut ui, &mut s);
        apply(Action::StareSelected, &mut ui, &mut s);
        assert!(s.target(last).is_some(), "pointing at a star should model it");
        assert!(s.observe(1.0e4).is_some());
    }

    /// A name is this ship's, not the star's — and it belongs to something detected. Naming a
    /// light nobody has picked up is naming nothing.
    #[test]
    fn naming_a_star_is_this_ship_saying_so() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        apply(Action::NameSelected("The Kettle".into()), &mut ui, &mut s);
        assert_eq!(s.name_of(id), "The Kettle");
        let naming = s.belief(id).unwrap().name.clone().unwrap();
        assert_eq!(
            naming.witness, s.knowledge.owner,
            "ours, not the catalogue's"
        );
        assert!(naming.lineage.is_empty(), "nobody told us this one");

        let unknown = lc_world::sky::StarId::synthesise("absent", 7);
        ui.selected = Some(unknown);
        let effects = apply(Action::NameSelected("Nowhere".into()), &mut ui, &mut s);
        assert!(
            matches!(effects.as_slice(), [Effect::Notify(t)] if t.contains("nothing detected"))
        );
    }

    /// With a shard, the telescope and the knowledge are the shard's: selecting a star, surveying,
    /// naming and reporting all become orders, and the client keeps no marks of its own.
    #[test]
    fn with_a_shard_the_instruments_are_ordered_not_run() {
        let (mut ui, mut s) = fixture();
        s.remote = true;
        let id = s.stars[0].id;
        let orders = |effects: &[Effect]| {
            effects
                .iter()
                .filter_map(|e| match e {
                    Effect::Send(order) => Some(order.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(orders(&apply(Action::SelectTarget(Some(id)), &mut ui, &mut s)).is_empty(), "selecting orders nothing");
        let sent = orders(&apply(Action::StareSelected, &mut ui, &mut s));
        assert!(
            matches!(sent.as_slice(), [lc_proto::Order::SetDuty { duty: lc_proto::Duty::Stare { star }, .. }] if *star == id.get()),
            "{sent:?}",
        );
        assert_eq!(s.observatory.duty, Duty::Idle, "and nothing is taken up until the shard says so");
        let sent = orders(&apply(Action::SurveySky, &mut ui, &mut s));
        assert!(matches!(sent.as_slice(), [lc_proto::Order::SetDuty { duty: lc_proto::Duty::Sweep { .. }, .. }]));
        let sent = orders(&apply(Action::NameSelected("Kettle".into()), &mut ui, &mut s));
        assert!(matches!(sent.as_slice(), [lc_proto::Order::NameIt { .. }]), "{sent:?}");
        assert_ne!(s.name_of(id), "Kettle", "named when the shard says so, not before");
        let to = Some(lc_proto::ShipId(7));
        let report = Action::SendReport { to, aim: lc_proto::Aim::Omni, secrecy: lc_proto::Secrecy::Open };
        let sent = orders(&apply(report, &mut ui, &mut s));
        assert!(matches!(sent.as_slice(), [lc_proto::Order::SendReport { to: Some(_), .. }]));
        let sent = orders(&apply(Action::RetainRaw(id, true), &mut ui, &mut s));
        assert!(matches!(sent.as_slice(), [lc_proto::Order::RetainRaw { keep: true, .. }]), "{sent:?}");
        assert!(!s.knowledge.retained(id), "kept when the shard says so, not before");
        let sent = orders(&apply(Action::Analyze, &mut ui, &mut s));
        assert!(matches!(sent.as_slice(), [lc_proto::Order::Analyze]), "{sent:?}");
        assert_eq!(s.knowledge.analyzing(), 0, "analyzing when the shard says so, not before");
    }


    /// The charts a ship launches with carry the charting office's names, with the office's
    /// name on them. Nothing reads a name off the catalogue.
    #[test]
    fn a_charted_star_is_called_what_the_office_called_it() {
        let (_, s) = fixture();
        let id = s.stars[0].id;
        let naming = s.belief(id).unwrap().name.clone().unwrap();
        assert_ne!(naming.witness, s.knowledge.owner);
        assert_eq!(naming.lineage.len(), 1, "one hop: somebody handed it over");
        assert_eq!(s.name_of(id), naming.name);
    }

    #[test]
    fn a_tuning_change_reaches_the_pass_it_names_and_not_the_other() {
        let (mut ui, mut s) = fixture();
        let mut style = ui.local;
        style.corona_gain = 3.0;
        apply(Action::SetPointStyle { which: Which::Local, style }, &mut ui, &mut s);
        assert_eq!(ui.local.corona_gain, 3.0);
        assert_eq!(ui.distant.corona_gain, crate::starfield::DISTANT.corona_gain);
    }

    #[test]
    fn resetting_restores_the_shipped_values() {
        let (mut ui, mut s) = fixture();
        for which in [Which::Local, Which::Distant, Which::Bodies] {
            let mut style = crate::starfield::style_for(&ui, which);
            style.brightness = 99.0;
            apply(Action::SetPointStyle { which, style }, &mut ui, &mut s);
            apply(Action::ResetPointStyle { which }, &mut ui, &mut s);
        }
        assert_eq!(ui.local, crate::starfield::LOCAL);
        assert_eq!(ui.distant, crate::starfield::DISTANT);
        assert_eq!(ui.bodies, crate::starfield::BODIES);
    }

    /// A knob with no slider is a knob nobody finds. The panel is built from this table, so the
    /// check is that the table covers the style rather than that the panel does.
    #[test]
    fn every_tunable_field_is_reachable_and_its_default_is_inside_its_range() {
        let mut style = crate::starfield::LOCAL;
        let mut seen: Vec<*const f32> = Vec::new();
        for (name, field, lo, hi) in crate::starfield::KNOBS {
            assert!(lo < hi, "{name} has an empty range");
            let at = field(&mut style);
            assert!(*at >= lo && *at <= hi, "{name} ships at {at}, outside {lo}..{hi}");
            let ptr = at as *const f32;
            assert!(!seen.contains(&ptr), "{name} is bound to a field another knob already has");
            seen.push(ptr);
        }
        assert!(seen.len() >= 10, "only {} knobs reached", seen.len());
    }

    /// A course is a crossing and a standing order at once, and cutting the drive has to drop
    /// both. A ship that stopped flying but kept its station would be snapped to the
    /// destination the moment the crossing ended, which is a teleport with extra steps.
    #[test]
    fn a_course_sets_a_station_and_cutting_the_drive_gives_it_up() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        // The sample sky has no system to navigate, so the course has nowhere to go.
        let refused = apply(Action::SetCourse(Course::LeaveSystem), &mut ui, &mut s);
        assert_eq!(refused.len(), 1, "it says so rather than doing nothing");
        assert!(s.station().is_none());

        // Holding when nothing is held is silent: it is already true.
        assert!(apply(Action::AbortFlight, &mut ui, &mut s).is_empty());
        s.fly_to(s.stars[0].id);
        assert_eq!(apply(Action::AbortFlight, &mut ui, &mut s).len(), 1);
        assert!(s.cruise().is_none() && s.station().is_none());
    }

    /// Crossing to another star abandons the station: it was defined against bodies that will
    /// be four light-years away.
    #[test]
    fn leaving_for_another_star_gives_up_the_station() {
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        s.ship.motion.begin_holding(crate::navigation::Waypoint::Fixed(glam::DVec3::X));
        s.fly_to(s.stars[0].id);
        assert!(s.station().is_none());
    }

    /// The System window's whole flow: pick something, arm one of its courses, press Go.
    /// Nothing flies until Go, which is the point of arming it separately.
    #[test]
    fn focusing_arming_and_going_are_three_separate_steps() {
        let provider =
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv");
        let Ok(provider) = provider else { return };
        let mut ui = UiState::default();
        let mut s = Session::new(&provider, 64);
        s.sync_system();
        let system = s.system.as_ref().expect("the solar system");

        let target = crate::navigation::Target::Body("Earth".into());
        let (_, course) = crate::navigation::options_for(system, &target)
            .into_iter()
            .find(|(label, _)| label == "polar orbit, high")
            .expect("Earth offers a high polar orbit");

        apply(Action::FocusTarget(Some(target.clone())), &mut ui, &mut s);
        assert_eq!(ui.focus, Some(target.clone()));
        assert!(ui.course.is_none(), "focusing arms nothing");

        apply(Action::ChooseCourse(Some(course.clone())), &mut ui, &mut s);
        assert_eq!(ui.course, Some(course.clone()));
        assert!(s.cruise().is_none() && s.station().is_none(), "arming flies nothing");

        let effects = apply(Action::SetCourse(course), &mut ui, &mut s);
        assert_eq!(effects.len(), 1, "Go says where it is going");
        // Crossing and holding are exclusive: the ship is flying *to* the station, and
        // arriving is what turns one into the other.
        assert!(s.cruise().is_some(), "Go did not begin a crossing");
        assert!(s.ship.motion.bound_for().is_some(), "and the crossing is not for anywhere");
        assert!(s.station().is_none(), "it cannot be holding a place it has not reached");

        // Focusing something else drops the armed course: it belonged to the last one.
        apply(Action::FocusTarget(Some(crate::navigation::Target::Band(0))), &mut ui, &mut s);
        assert!(ui.course.is_none());
    }
    /// With a server answering, a flight order is **sent**, not applied. Applying it locally
    /// would use the acceleration that was asked for rather than the one the server flew.
    #[test]
    fn a_remote_session_sends_its_flight_orders_instead_of_flying_them() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        s.remote = true;
        s.ship.motion.drive.accel_g = 7.0;
        let before = s.ship.motion.position_ly;

        let effects = apply(
            Action::SetCourse(crate::navigation::Course::LeaveSystem),
            &mut ui,
            &mut s,
        );
        let sent: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Send(order) => Some(order),
                _ => None,
            })
            .collect();
        assert_eq!(
            sent.as_slice(),
            [&lc_proto::Order::SetCourse {
                course: lc_proto::Course::LeaveSystem,
                accel_g: 7.0,
                max_beta: 0.999,
            }],
            "{effects:?}",
        );
        assert_eq!(s.ship.motion.position_ly, before, "it flew as well as sent");
    }

    /// And with no server it still flies, which is every build before there was one.
    #[test]
    fn a_local_session_still_flies_its_own_orders() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        assert!(!s.remote);
        let effects = apply(
            Action::SetCourse(crate::navigation::Course::LeaveSystem),
            &mut ui,
            &mut s,
        );
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Send(_))),
            "a single-process session sent an order to nobody: {effects:?}",
        );
    }

    /// A crossing goes over the wire by **star id**, and the client does not fly it. Both ends
    /// must plan the same crossing, so only the one with authority plans it.
    #[test]
    fn a_remote_session_sends_a_crossing_and_does_not_fly_it() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        s.remote = true;
        s.ship.motion.drive.accel_g = 4.0;
        let destination = s.stars[0].id;
        let before = s.ship.motion.position_ly;

        let effects = apply(Action::FlyTo(Some(destination)), &mut ui, &mut s);
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::Send(lc_proto::Order::Cross { star, accel_g, .. })
                    if *star == destination.get() && *accel_g == 4.0
            )),
            "{effects:?}",
        );
        assert!(s.cruise().is_none(), "it flew a crossing the server has not agreed to");
        assert_eq!(s.ship.motion.position_ly, before);
    }

    /// And with no server it still flies it itself, which is the single-process game.
    #[test]
    fn a_local_session_flies_its_own_crossing() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        let effects = apply(Action::FlyTo(Some(s.stars[0].id)), &mut ui, &mut s);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Send(_))), "{effects:?}");
        assert!(s.cruise().is_some(), "it sent an order to nobody instead of flying");
    }
    /// The server owns the rate — doc 13 says so, and the client never enforced it. A client
    /// that warps runs its clock away from the server's, and then every order it sends comes
    /// back stamped in its own past.
    #[test]
    fn a_remote_session_cannot_take_the_clock() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        s.remote = true;
        let before = ui.time_rate;

        for action in [Action::TimeRateUp, Action::TimeRateDown, Action::SetTimeRate(3600.0)] {
            let effects = apply(action, &mut ui, &mut s);
            assert_eq!(ui.time_rate, before, "the client took the clock");
            assert!(
                effects.iter().any(|e| matches!(e, Effect::Notify(t) if t.contains("server"))),
                "it changed nothing and said nothing: {effects:?}",
            );
        }
    }

    /// And offline it is still the player's, which is the single-process game and the whole
    /// reason the ladder exists.
    #[test]
    fn a_local_session_still_owns_its_own_clock() {
        let mut ui = UiState::default();
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        apply(Action::SetTimeRate(60.0), &mut ui, &mut s);
        assert_eq!(ui.time_rate, 60.0);
    }
    /// Only an authority can put a craft somewhere, so offline the button says so rather than
    /// appearing to work.
    #[test]
    fn a_scene_is_asked_for_of_the_server_and_nobody_else() {
        let (mut ui, mut s) = fixture();
        let said = apply(Action::StageDemo("chase".into()), &mut ui, &mut s);
        assert!(
            matches!(said.as_slice(), [Effect::Notify(m)] if m.contains("no server")),
            "{said:?}",
        );

        s.remote = true;
        let said = apply(Action::StageDemo("chase".into()), &mut ui, &mut s);
        assert_eq!(said, vec![Effect::Stage("chase".into())]);
    }

    /// A scene carries where to stand, so staging one moves the camera there and nothing has to
    /// be passed in. `closing` is the one that is watched from its cast; the rest are watched
    /// from the player, which is what the camera has always done.
    #[test]
    fn staging_a_scene_stands_where_the_scene_says() {
        let (mut ui, mut s) = fixture();
        s.remote = true;

        apply(Action::StageDemo("closing".into()), &mut ui, &mut s);
        let want = lc_world::scenario::Scenario::craft_for(lc_world::scenario::CLOSING.watch);
        assert_eq!(
            ui.perspective,
            want.map(|id| crate::ui::CameraPerspective::Pov(lc_proto::ShipId(id))),
        );
        assert!(ui.perspective.is_some(), "premise: closing is watched from its cast");

        // And one watched from the player puts the camera back, rather than leaving it on
        // whoever the last scene was about.
        apply(Action::StageDemo("approach".into()), &mut ui, &mut s);
        assert_eq!(ui.perspective, None);
    }

}