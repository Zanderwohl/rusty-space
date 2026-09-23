//! The System window: what a craft believes is here, and where it can be sent.
//!
//! Split out of [`crate::panels`] when the body list stopped reading the generator: a list of
//! beliefs needs a detail section with provenance behind it, which is a panel's worth of its
//! own. See `lightcone/docs/25-system-knowledge.md`.

use bevy::prelude::*;
use bevy_egui::egui;

use lc_world::knowledge::conclusion::{Kind, SETTLED};
use lc_world::knowledge::record::{Method, Orientation};
use lc_world::knowledge::{BodyBelief, BodyId, Placed, SystemPlane};
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
    uplink: &crate::uplink::Uplink,
    tab: &mut SystemTab,
    show_all: &mut bool,
    revealed: &mut Option<Target>,
    out: &mut MessageWriter<Requested>,
) {
    let Some(system) = game.system.as_ref() else {
        ui.label("Between systems. There is nothing local to go to.");
        return;
    };
    let now = game.coordinate_time_s();
    let known = game.knowledge.bodies_of(system.star, now);
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
    station(ui, state, game, out);
    ui.separator();
    if *tab == SystemTab::Ships {
        ships(ui, game, uplink, out);
        return;
    }

    // The star, and the plane its bodies are believed to share.
    ui.horizontal(|ui| {
        ui.label(game.name_of(system.star));
        ui.checkbox(show_all, "all");
    });
    ui.weak(plane_text(game.knowledge.system_plane(system.star)));
    ui.separator();

    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        if known.is_empty() {
            ui.weak("Nothing known here yet. What the telescope finds appears in this list.");
        }
        for belief in &known {
            let target = truth_target(system, system.star, belief.body);
            let picked = target.is_some() && state.focus.as_ref() == target.as_ref();
            ui.horizontal(|ui| {
                let row = ui.selectable_label(picked, name_of(belief));
                if row.clicked() {
                    if let Some(target) = target.clone() {
                        ask(out, Action::FocusTarget((!picked).then_some(target)));
                    }
                }
                // Once, when the focus changes. Every frame would fight the player's own
                // scrolling, and never would hide a body picked from anywhere but this list.
                if picked && revealed.as_ref() != target.as_ref() {
                    *revealed = target.clone();
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

        // The generator's, and labelled as the odd ones out until phase 8 makes them knowledge.
        let bands: Vec<_> = system
            .inventory()
            .iter()
            .filter(|e| matches!(e.target, Target::Band(_)))
            .filter(|e| *show_all || e.major)
            .collect();
        if !bands.is_empty() {
            ui.separator();
            for entry in bands {
                let picked = state.focus.as_ref() == Some(&entry.target);
                ui.horizontal(|ui| {
                    if ui.selectable_label(picked, &entry.designation).clicked() {
                        let next = (!picked).then(|| entry.target.clone());
                        ask(out, Action::FocusTarget(next));
                    }
                    ui.weak(span_m(entry.orbit_radius_m));
                });
            }
        }
    });
    ui.separator();

    let Some(target) = state.focus.as_ref() else {
        ui.label("Pick something to go to.");
        return;
    };
    let Some(entry) = system.inventory().iter().find(|e| &e.target == target) else { return };

    // A body's detail is its belief; a band has none yet, so it reads as it always did.
    match known.iter().find(|b| truth_target(system, system.star, b.body).as_ref() == Some(target))
    {
        Some(belief) => details(ui, belief, game, system, target),
        None => {
            ui.heading(&entry.designation);
            ui.weak(format!(
                "{} — {} out, {} away",
                entry.kind.label(),
                span_m(entry.orbit_radius_m),
                span(range_to(game, system, target)),
            ));
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
    target: &Target,
) {
    ui.heading(name_of(belief));
    ui.weak(format!("{} away", span(range_to(game, system, target))));

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
    ui.label(format!("Type: {}", type_text(belief)));
    ui.label(format!("Orientation: {}", orientation_text(belief.orientation)));

    egui::CollapsingHeader::new("Sources").default_open(true).show(ui, |ui| {
        for note in sources(belief, game.knowledge.owner) {
            ui.weak(note);
        }
    });
}

/// Which truth target a believed body is, if any.
///
/// A join over the inventory, because a [`BodyId`] is a hash and cannot be turned back into the
/// generator's key. `None` for a body no generator made — a transit's false positive — which
/// therefore has nowhere to be flown to. Both halves of that go away in phase 7, when a course
/// carries a subject and the shard plans it from knowledge.
fn truth_target(system: &lc_world::system::LocalSystem, star: StarId, body: BodyId) -> Option<Target> {
    system.inventory().iter().find_map(|entry| match &entry.target {
        Target::Body(key) if BodyId::of(star, key) == body => Some(entry.target.clone()),
        _ => None,
    })
}

fn name_of(belief: &BodyBelief) -> String {
    belief.name.clone().unwrap_or_else(|| "unnamed body".to_string())
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
fn type_text(belief: &BodyBelief) -> String {
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

/// Where a body's orbit came from and how far it travelled to get here.
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
    use lc_world::knowledge::Witness;

    use super::*;

    fn belief(method: Option<Method>, stated_by: Option<Witness>, hops: usize) -> BodyBelief {
        BodyBelief {
            subject: lc_world::knowledge::Subject::Star(StarId::synthesise("t", 1)),
            body: BodyId::from_raw(1),
            name: Some("b".into()),
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

    /// A body no generator made has nowhere to be flown to, which is what a false positive
    /// should mean rather than a crash or a course into empty space.
    #[test]
    fn a_phantom_body_joins_to_no_target() {
        let stars = lc_world::sky::AuthoredStars::sample();
        let star = lc_world::sky::StarProvider::stars(&stars)[2].clone();
        let system = lc_world::system::LocalSystem::for_star(&star).expect("a generated system");

        let real = system
            .inventory()
            .iter()
            .find_map(|e| match &e.target {
                Target::Body(key) => Some(key.clone()),
                _ => None,
            })
            .expect("a body");
        assert!(truth_target(&system, star.id, BodyId::of(star.id, &real)).is_some());
        assert_eq!(truth_target(&system, star.id, BodyId::from_raw(7)), None, "a phantom");
    }
}
