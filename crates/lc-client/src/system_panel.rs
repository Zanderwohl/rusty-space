//! The System window: what a craft believes is here, and where it can be sent.
//!
//! Split out of [`crate::panels`] when the body list stopped reading the generator: a list of
//! beliefs needs a detail section with provenance behind it, which is a panel's worth of its
//! own. See `lightcone/docs/25-system-knowledge.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_world::knowledge::conclusion::{Kind, SETTLED};
use lc_world::knowledge::record::{Method, Orientation};
use lc_world::knowledge::sort::Measured;
use lc_world::knowledge::{BodyBelief, BodyId, Placed, Subject, SystemPlane};
use lc_world::navigation::Target;
use lc_world::sky::StarId;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::{SystemTab, ask, range_to, ships, span, span_m, station};

/// How far down the list a candidate's probability is worth reading out.
///
/// Below this the search has barely started on it and the number is noise dressed as a
/// measurement.
const WORTH_LISTING: f64 = 0.05;

/// The system window: what is believed to be here, and where the ship can be sent.
///
/// The body list is **knowledge**, not the generator's inventory: a body appears when this craft
/// holds evidence of it and not before, so a system nobody has looked at is empty and says so.
/// Populations are still the generator's, until belts become knowledge in phase 8.
#[allow(clippy::too_many_arguments)]
pub(crate) fn system(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    held: &crate::beliefs::Held,
    uplink: &crate::uplink::Uplink,
    tab: &mut SystemTab,
    show_all: &mut bool,
    picked: &mut Option<BodyId>,
    revealed: &mut Option<BodyId>,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
) {
    let Some(system) = game.system.as_ref() else {
        ui.label("Between systems. There is nothing local to go to.");
        return;
    };
    let known = &held.bodies;
    ui.horizontal(|ui| {
        ui.selectable_value(tab, SystemTab::Bodies, match known.len() {
            1 => "1 body".to_string(),
            n => format!("{n} bodies"),
        });
        // Counted in the tab, because whether anyone is here at all is the first thing worth
        // knowing and opening the other list to find out would be one click too many.
        ui.selectable_value(tab, SystemTab::Ships, match uplink.contacts.len() {
            0 => "no ships".to_string(),
            1 => "1 ship".to_string(),
            n => format!("{n} ships"),
        });
    });
    station(ui, game, out);
    ui.separator();
    if *tab == SystemTab::Ships {
        ships(ui, game, uplink, out);
        return;
    }

    // The plane its bodies are believed to share.
    ui.horizontal(|ui| {
        ui.weak(plane_text(held.plane));
        ui.checkbox(show_all, "all");
    });
    ui.separator();

    // The star is a row rather than a heading, because it is a thing in the system like the
    // rest and a player who can pick a moon expects to be able to pick its sun.
    //
    // Focused, not selected. A star is two things at once -- somewhere to cross to, and the
    // body at the middle of the system you are in -- and this list is the second. Selecting it
    // put it in the *interstellar* field, which offers no local course and left a star and a
    // planet picked at the same time in two fields that know nothing of each other.
    let here = star_target(system);
    let star_picked = here.as_ref().is_some_and(|t| state.focus.as_ref() == Some(t));
    if ui.selectable_label(star_picked, game.name_of(system.star)).clicked() {
        ask(out, Action::FocusTarget((!star_picked).then_some(here).flatten()));
    }

    *picked = settle_pick(state.focus.as_ref(), *picked, known, held);

    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        if known.is_empty() {
            ui.weak("Nothing known here yet. What the telescope finds appears in this list.");
        }
        for belief in known {
            let target = held.target(belief.body).cloned();
            let on = *picked == Some(belief.body);
            ui.horizontal(|ui| {
                let row = ui.selectable_label(on, game.called(belief));
                if row.clicked() {
                    // Off the focus either way: a phantom has no target to put there, and
                    // leaving the old one would light two rows at once.
                    ask(out, Action::FocusTarget((!on).then_some(target).flatten()));
                    *picked = (!on).then_some(belief.body);
                }
                // Once, when the pick changes. Every frame would fight the player's own
                // scrolling, and never would hide a body picked from anywhere but this list.
                if on && *revealed != Some(belief.body) {
                    *revealed = Some(belief.body);
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                ui.weak(distance_text(belief));
            });
        }

        // Below the bodies, dimmed: a transit the search has found and not settled. Worth
        // showing because it is what the telescope is working on, and worth dimming because it
        // is not a body yet.
        let candidates = unsettled(game, system.star);
        if !candidates.is_empty() {
            ui.separator();
            for (probability, period_s) in candidates {
                ui.weak(format!(
                    "candidate on a {} orbit — {:.0}%",
                    span_days(period_s),
                    probability * 100.0
                ));
            }
        }

        // The generator's, and labeled as the odd ones out until phase 8 makes them knowledge.
        let bands: Vec<_> = system
            .inventory()
            .iter()
            .filter(|e| matches!(e.target, Target::Band(_)))
            .filter(|e| *show_all || e.major)
            .collect();
        if !bands.is_empty() {
            ui.separator();
            for entry in bands {
                let Target::Band(index) = entry.target else { continue };
                let picked = state.focus.as_ref() == Some(&entry.target);
                ui.horizontal(|ui| {
                    if ui.selectable_label(picked, game.band_label(index)).clicked() {
                        let next = (!picked).then(|| entry.target.clone());
                        ask(out, Action::FocusTarget(next));
                    }
                    ui.weak(span_m(entry.orbit_radius_m));
                });
            }
        }
    });
    ui.separator();

    // A body's detail is its belief, whether or not anything in the arena answers to it.
    if let Some(belief) = picked.and_then(|body| known.iter().find(|b| b.body == body)) {
        details(ui, belief, game, system, draft, out);
    }
    let Some(target) = state.focus.as_ref() else {
        if picked.is_none() {
            ui.label("Pick something to go to.");
        }
        return;
    };
    let Some(entry) = system.inventory().iter().find(|e| &e.target == target) else { return };

    // A band has no belief yet, so it reads as it always did; a *body* with none is one this
    // craft has never detected, and its designation, its orbit and its range are all the
    // arena's rather than anything anybody measured. Reachable through a sky pick or
    // `--focus`, so it has to be refused here and not only not offered.
    if picked.is_none() {
        match entry.kind {
            // The star's file is under `Subject::Star`, so it is not in the body list and
            // `held.at` never finds it. Without this it read as "Nothing detected here" --
            // about the one body the craft is sitting inside.
            lc_world::navigation::Kind::Star => {
                ui.heading(game.name_of(system.star));
                ui.weak(match game.knowledge.belief(system.star) {
                    Some(belief) => format!(
                        "star — {}",
                        crate::range::short(Some(belief), game.ship.motion.position_ly)
                    ),
                    None => "star — nothing measured".to_string(),
                });
            }
            lc_world::navigation::Kind::Band => {
                let Target::Band(index) = entry.target else { return };
                ui.heading(game.band_label(index));
                let subject = Subject::Population { star: system.star, index: index as u32 };
                name_field(ui, game, subject, draft, out);
                ui.weak(format!(
                    "{} — {} out, {} away",
                    entry.kind.label(),
                    span_m(entry.orbit_radius_m),
                    span(range_to(game, system, target)),
                ));
            }
            _ => {
                ui.heading("Nothing detected here");
                ui.weak("This craft holds no evidence of a body at this target.");
            }
        }
    }

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

/// What is believed about one body, and on whose word.
fn details(
    ui: &mut egui::Ui,
    belief: &BodyBelief,
    game: &Game,
    system: &lc_world::system::LocalSystem,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
) {
    ui.heading(game.called(belief));
    if belief.given.is_some() {
        ui.weak(game.called(&BodyBelief { given: None, ..belief.clone() }));
    }
    name_field(ui, game, belief.subject, draft, out);
    // The range its own orbit implies, never the one the arena holds. A body the list calls
    // "distance unknown" has no range to give, and printing the true one here said what the
    // craft has not measured.
    ui.weak(match believed_range(belief, game, system) {
        Some(range) => format!("about {} away", span(range)),
        None => "range unknown".to_string(),
    });

    if let Some((period_s, sigma_s)) = belief.period_s {
        ui.label(format!("Year: {}", with_error(period_s / 86_400.0, sigma_s / 86_400.0, "d")));
    }
    if let Some((au, sigma)) = belief.semi_major_au {
        ui.label(format!("Distance from star: {}", with_error(au, sigma, "AU")));
    }
    // What a close pass buys, and nothing shows until one has been made: from across a system
    // a body is a bearing and a brightness. See `survey::CLOSE_ELEMENTS`.
    if let Some((radius_m, sigma_m)) = belief.radius_m {
        ui.label(format!("Radius: {}", with_error(radius_m / 1000.0, sigma_m / 1000.0, "km")))
            .on_hover_text("An angular diameter and a range from the one look, which needs a close pass");
    }
    if let Some((spin_s, sigma_s)) = belief.spin_s {
        ui.label(format!("Day: {}", with_error(spin_s / 3600.0, sigma_s / 3600.0, "h")))
            .on_hover_text("Timed by following features across the disc");
    }
    if let Some((velocity, sigma)) = belief.velocity_m_s {
        ui.label(format!("Speed: {}", with_error(velocity.length() / 1000.0, sigma / 1000.0, "km/s")))
            .on_hover_text("From the orbit, not from any measurement of its own");
    }
    ui.label(format!("Type: {}", type_text(belief, game)));
    ui.label(format!("Orientation: {}", orientation_text(belief.orientation)));

    egui::CollapsingHeader::new("Sources").default_open(true).show(ui, |ui| {
        for note in sources(belief, game.knowledge.owner) {
            ui.weak(note);
        }
    });
}

fn name_field(
    ui: &mut egui::Ui,
    game: &Game,
    subject: Subject,
    draft: &mut String,
    out: &mut MessageWriter<Requested>,
) {
    if !game.nameable(subject) {
        return;
    }
    let field = ui.add(egui::TextEdit::singleline(draft).hint_text("Add Name").desired_width(150.0));
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if entered && !draft.trim().is_empty() {
        ask(out, Action::Name(subject, std::mem::take(draft)));
    }
}

/// The star as something local to fly to, which is how the inventory lists it.
fn star_target(system: &lc_world::system::LocalSystem) -> Option<Target> {
    system.inventory().iter().find(|e| e.depth == 0 && e.kind == lc_world::navigation::Kind::Star)
        .map(|e| e.target.clone())
}

/// Which body the list has picked.
///
/// The focus is the authority whenever it names something, because a body picked on the map or
/// in the sky has to light up here too. The panel keeps a pick of its own as well, for a body
/// only this craft believes in: that has no target to be focused, and a row that cannot be
/// clicked puts the evidence for it -- the one thing that could ever disprove it -- behind the
/// click. A pick the list no longer holds is dropped.
fn settle_pick(
    focus: Option<&Target>,
    picked: Option<BodyId>,
    known: &[BodyBelief],
    held: &crate::beliefs::Held,
) -> Option<BodyId> {
    match focus {
        Some(target) => known.iter().find(|b| held.target(b.body) == Some(target)).map(|b| b.body),
        None => picked.filter(|body| known.iter().any(|b| b.body == *body)),
    }
}

/// How far off a believed body is, from its own orbit rather than from the arena.
fn believed_range(
    belief: &BodyBelief,
    game: &Game,
    system: &lc_world::system::LocalSystem,
) -> Option<f64> {
    let Placed::Known { offset_au, .. } = belief.position_now else { return None };
    let star = system.star_position_at(game.coordinate_time_s())?;
    let at = star + offset_au * (lc_world::navigation::AU / lc_world::system::M_PER_LY);
    Some(at.distance(game.ship.motion.position_ly))
}

/// A believed distance, or that there is not one.
fn distance_text(belief: &BodyBelief) -> String {
    match belief.position_now {
        Placed::Known { offset_au, .. } => format!("{:.3} AU", offset_au.length()),
        Placed::Shell { radius_au, sigma_au } => with_error(radius_au, sigma_au, "AU"),
        Placed::Unknown => "distance unknown".to_string(),
    }
}

/// A value and its error, with the error dropped when it is too small to read.
fn with_error(value: f64, sigma: f64, unit: &str) -> String {
    let places = if value.abs() < 10.0 { 3 } else { 1 };
    if !(sigma > 0.0) || sigma / value.abs().max(f64::MIN_POSITIVE) < 1.0e-4 {
        return format!("{value:.places$} {unit}");
    }
    format!("{value:.places$} ± {sigma:.places$} {unit}")
}

/// The hypotheses, most probable first, as percentages.
///
/// What a body has been measured to be beats what its transits implied: a radius, a density and
/// a color say what kind of world it is, and a transit only says rocky or giant. The transit
/// reading is what is left when nothing has been measured. See `lc_world::knowledge::sort`.
fn type_text(belief: &BodyBelief, game: &Game) -> String {
    let star = game.system.as_ref().and_then(|s| game.stars.iter().find(|c| c.id == s.star));
    if let Some(star) = star {
        let measured = Measured::from_belief(belief, &star.star);
        let named: Vec<String> = game
            .sorts()
            .given(&measured)
            .into_iter()
            .filter(|(_, p)| *p >= WORTH_LISTING)
            .map(|(sort, p)| format!("{} {:.0}%", sort.label(), p * 100.0))
            .collect();
        if !named.is_empty() {
            return named.join(", ");
        }
    }
    let named: Vec<String> = belief
        .kind
        .iter()
        .filter(|h| h.probability >= WORTH_LISTING)
        .filter_map(|h| match h.kind {
            Kind::Planet { class, .. } => {
                Some(format!("{} {:.0}%", class.name(), h.probability * 100.0))
            }
            _ => None,
        })
        .collect();
    if named.is_empty() {
        return "not classified".to_string();
    }
    named.join(", ")
}

/// How much of the orbit's orientation is held, in the terms doc 25 uses.
fn orientation_text(orientation: Orientation) -> String {
    match orientation {
        Orientation::Known { sigma_rad, .. } => {
            format!("known ± {:.1}°", sigma_rad.to_degrees())
        }
        // The honest reading of one transit: the pole is on a circle, and somebody watching from
        // elsewhere is what narrows it.
        Orientation::EdgeOnTo { .. } => "edge-on to one line of sight".to_string(),
        Orientation::Unknown => "unknown".to_string(),
    }
}

/// Where a body's orbit came from and how far it traveled to get here.
fn sources(belief: &BodyBelief, owner: lc_world::knowledge::Witness) -> Vec<String> {
    let mut notes = Vec::new();
    match belief.method {
        Some(Method::Transit) => notes.push("Orbit from a settled transit".to_string()),
        Some(Method::Astrometric) => notes.push("Orbit fitted to bearings".to_string()),
        Some(Method::Claim) => {
            notes.push("Orbit stated, with no measurements behind it".to_string())
        }
        None => notes.push("No orbit held".to_string()),
    }
    match belief.stated_by {
        Some(witness) if witness == owner => notes.push("Measured by this ship".to_string()),
        Some(witness) => notes.push(format!("On craft {}'s word", witness.0)),
        None => {}
    }
    match belief.hops {
        0 => {}
        1 => notes.push("Relayed once".to_string()),
        n => notes.push(format!("Relayed {n} times")),
    }
    // The distance a transit gives is never better than the mass it was divided by, and the
    // panel should not let that read as a measurement.
    if belief.method == Some(Method::Transit) {
        notes.push("Distance through the star's mass, which is a prior".to_string());
    }
    notes
}

/// What the system's plane is, as the map's option reads it.
fn plane_text(plane: SystemPlane) -> String {
    match plane {
        SystemPlane::Known { sigma_rad, .. } => {
            format!("System plane solved to ± {:.1}°", sigma_rad.to_degrees())
        }
        SystemPlane::Circle(_) => "System plane on a circle, from transits seen one way".to_string(),
        SystemPlane::Unknown => "System plane unknown".to_string(),
    }
}

/// Transits the search has found and not settled, most probable first.
fn unsettled(game: &Game, star: StarId) -> Vec<(f64, f64)> {
    let Some(file) = game.knowledge.file(star) else { return Vec::new() };
    let Some(conclusion) = file.conclusions().iter().max_by(|a, b| a.stated_s.total_cmp(&b.stated_s))
    else {
        return Vec::new();
    };
    let mut found: Vec<(f64, f64)> = conclusion
        .transits
        .iter()
        .filter(|h| h.probability < SETTLED && h.probability >= WORTH_LISTING)
        .filter_map(|h| match h.kind {
            Kind::Planet { transit, .. } => Some((h.probability, transit.period_s)),
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found
}

fn span_days(seconds: f64) -> String {
    match seconds / 86_400.0 {
        d if d < 1.0 => format!("{:.1}-hour", seconds / 3600.0),
        d if d < 1000.0 => format!("{d:.1}-day"),
        d => format!("{:.1}-year", d / 365.25),
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_world::knowledge::{BodyId, Witness};

    use super::*;

    fn belief(method: Option<Method>, stated_by: Option<Witness>, hops: usize) -> BodyBelief {
        BodyBelief {
            subject: lc_world::knowledge::Subject::Star(StarId::synthesize("t", 1)),
            body: BodyId::from_raw(1),
            given: None,
            designation: Some("b".into()),
            kind: Vec::new(),
            period_s: None,
            semi_major_au: None,
            orientation: Orientation::Unknown,
            method,
            position_now: Placed::Unknown,
            radius_m: None,
            spin_s: None,
            velocity_m_s: None,
            about: None,
            colors: None,
            mass_kg: None,
            stated_by,
            hops,
        }
    }

    /// An error too small to read is not printed, because "1.000 ± 0.000" reads as a precision
    /// nobody claimed.
    #[test]
    fn an_error_is_shown_only_when_it_is_worth_reading() {
        assert_eq!(with_error(1.5, 0.2, "AU"), "1.500 ± 0.200 AU");
        assert_eq!(with_error(1.5, 0.0, "AU"), "1.500 AU");
        assert_eq!(with_error(1.5, 1.0e-9, "AU"), "1.500 AU", "an unreadable error");
        // Big numbers get fewer places, or a year in days is a wall of digits.
        assert_eq!(with_error(4332.0, 11.0, "d"), "4332.0 ± 11.0 d");
    }

    /// **What a transit gives is a period; the distance is a division by a mass nobody
    /// measured.** The panel has to say so, or the number reads as an observation.
    #[test]
    fn a_transits_distance_is_marked_as_resting_on_a_prior() {
        let mine = sources(&belief(Some(Method::Transit), Some(Witness(1)), 0), Witness(1));
        assert!(mine.iter().any(|n| n.contains("settled transit")), "{mine:?}");
        assert!(mine.iter().any(|n| n.contains("Measured by this ship")), "{mine:?}");
        assert!(mine.iter().any(|n| n.contains("prior")), "{mine:?}");

        // A fitted orbit does not carry that caveat, because it measured the distance.
        let fitted = sources(&belief(Some(Method::Astrometric), Some(Witness(1)), 0), Witness(1));
        assert!(!fitted.iter().any(|n| n.contains("prior")), "{fitted:?}");
    }

    /// Somebody else's word, and how far it came, are the two things that let a player distrust
    /// one source without distrusting everything.
    #[test]
    fn a_relayed_orbit_names_its_witness_and_its_hops() {
        let notes = sources(&belief(Some(Method::Claim), Some(Witness(12)), 2), Witness(1));
        assert!(notes.iter().any(|n| n.contains("craft 12")), "{notes:?}");
        assert!(notes.iter().any(|n| n == "Relayed 2 times"), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("no measurements")), "{notes:?}");
    }

    /// The three states of a plane read differently, and the middle one is the interesting one:
    /// it says the observation was made and was not enough.
    #[test]
    fn the_plane_line_distinguishes_unknown_from_unresolved() {
        let known = plane_text(SystemPlane::Known {
            pole: DVec3::Z,
            sigma_rad: 0.02,
            zero: DVec3::X,
        });
        assert!(known.contains("1.1°"), "{known}");
        assert!(plane_text(SystemPlane::Circle(DVec3::X)).contains("circle"));
        assert!(plane_text(SystemPlane::Unknown).contains("unknown"));
    }

    /// A body no generator made can still be picked. Its row was unclickable while selection
    /// went through a truth target it has none of, which put the evidence for it -- the one
    /// thing that could ever disprove it -- behind a click that did nothing.
    #[test]
    fn a_phantom_row_can_be_picked_and_stays_picked() {
        let star = StarId::synthesize("t", 1);
        let real = BodyId::of(star, "Aa");
        let phantom = BodyId::phantom(star, Witness(1), 7);
        let target = Target::Body("Aa".into());
        let bodies: Vec<BodyBelief> = [real, phantom]
            .iter()
            .map(|id| BodyBelief { body: *id, ..belief(None, None, 0) })
            .collect();
        let held = crate::beliefs::Held::from_parts(
            bodies.clone(),
            [(real, target.clone())].into_iter().collect(),
        );

        assert_eq!(settle_pick(None, Some(phantom), &bodies, &held), Some(phantom));
        // And the focus wins whenever it names one, so a body picked on the map lights up here.
        assert_eq!(settle_pick(Some(&target), Some(phantom), &bodies, &held), Some(real));
        // A focus on something that is not a body in this list picks nothing.
        assert_eq!(settle_pick(Some(&Target::Band(0)), Some(real), &bodies, &held), None);
    }

    /// A pick the list no longer holds is dropped, or the detail pane would go on describing a
    /// body that has merged into a real one.
    #[test]
    fn a_pick_that_left_the_list_is_dropped() {
        let star = StarId::synthesize("t", 1);
        let gone = BodyId::phantom(star, Witness(1), 7);
        let held = crate::beliefs::Held::from_parts(Vec::new(), Default::default());
        assert_eq!(settle_pick(None, Some(gone), &[], &held), None);
    }

    /// A body the list calls "distance unknown" has no range to give. The detail pane used to
    /// read the arena for one, which told the player what the craft has not measured.
    #[test]
    fn a_body_with_no_believed_place_has_no_range() {
        let stars = lc_world::sky::AuthoredStars::sample();
        let star = lc_world::sky::StarProvider::stars(&stars)[2].clone();
        let system = lc_world::system::LocalSystem::for_star(&star).expect("a system");
        let mut game = Game(crate::session::Session::new(&stars, 3));
        game.0.ship.motion.position_ly = star.position_ly;
        game.0.sync_system();

        let nowhere = belief(None, None, 0);
        assert_eq!(nowhere.position_now, Placed::Unknown, "the fixture moved");
        assert_eq!(believed_range(&nowhere, &game, &system), None);
    }

    /// The star is something local to fly to, not only something to cross to. Without an entry
    /// for it the panel offered no course to the body the craft is sitting inside.
    #[test]
    fn the_star_is_a_local_target_with_courses() {
        let stars = lc_world::sky::AuthoredStars::sample();
        let star = lc_world::sky::StarProvider::stars(&stars)[2].clone();
        let system = lc_world::system::LocalSystem::for_star(&star).expect("a system");
        let target = star_target(&system).expect("the star is in the inventory");
        assert!(
            !crate::navigation::options_for(&system, &target).is_empty(),
            "the star has nowhere to be orbited from",
        );
    }
}
