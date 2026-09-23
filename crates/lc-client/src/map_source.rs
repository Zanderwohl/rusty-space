//! Building a map snapshot out of what this client holds.
//!
//! Every provider returns a [`Picture`]: a plain [`MapSnapshot`], and what each of its items
//! is in the terms the rest of the interface selects things in. The map renders the snapshot
//! and knows nothing about where it came from, so switching perspective is a choice of
//! function rather than a second renderer, and a relay or a fleet's shared picture is another
//! provider.
//!
//! The catalogue is test data. Stars come from [`Session::stars`], whatever the session was
//! handed: a CSV today, a shard's answer once the server is authoritative. `local::start`
//! passes the client's own stars to the server in the box so both place craft alike.

use std::collections::HashMap;

use em_map::{ItemKey, ItemKind, MapItem, MapSnapshot};
use glam::DVec3;
use lc_world::knowledge::Placed;
#[cfg(feature = "godview")]
use lc_world::navigation::Kind;

use crate::pick::Subject;
use crate::session::Session;
use crate::uplink::Uplink;

/// How far the map reaches, light-years.
///
/// A fixed sphere: a reach that moved with the zoom would change what exists as well as what
/// is framed. Twenty-five light-years is 166 stars out of the bundled catalogue's 119 625.
/// One solar mass. The catalogue states a star's mass in them and
/// [`em_map::MapItem::weight`] wants the kilograms a planet's is in.
const SOLAR_MASS_KG: f64 = 1.988_41e30;

pub const REACH_LY: f64 = 25.0;

/// Which perspective the map is drawn from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// This ship's own instruments.
    #[default]
    Observed,
    /// Every worldline at the coordinate clock, with no light delay.
    #[cfg(feature = "godview")]
    God,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Observed => "observed",
            #[cfg(feature = "godview")]
            Source::God => "god view",
        }
    }
}

/// A snapshot, and what each of its items is in the terms the rest of the interface selects
/// things in.
///
/// Both at once because only the provider can say: an [`ItemKey`] is a digest and nothing
/// reads back out of it. Recovering a star's id afterwards would mean hashing the whole
/// catalogue every frame.
///
/// Not every item has a subject. The reader's own craft has none and is picked through.
///
/// Keyed rather than a list of pairs: picking asks this once per placement, every frame the map
/// is up, and a scan made that quadratic in the size of the map.
pub struct Picture {
    pub snapshot: MapSnapshot,
    pub subjects: HashMap<ItemKey, Subject>,
}


/// A picture under construction.
#[derive(Default)]
struct Build {
    items: Vec<MapItem>,
    subjects: HashMap<ItemKey, Subject>,
}

impl Build {
    fn with_capacity(n: usize) -> Self {
        Self { items: Vec::with_capacity(n), subjects: HashMap::with_capacity(n) }
    }

    fn push(&mut self, item: MapItem, subject: Option<Subject>) {
        if let Some(subject) = subject {
            self.subjects.insert(item.key, subject);
        }
        self.items.push(item);
    }

    fn into_picture(self, snapshot: impl FnOnce(Vec<MapItem>) -> MapSnapshot) -> Picture {
        Picture { snapshot: snapshot(self.items), subjects: self.subjects }
    }
}

#[cfg(feature = "godview")]
fn kind_of(kind: Kind) -> ItemKind {
    match kind {
        Kind::Star => ItemKind::Star,
        Kind::Planet => ItemKind::Planet,
        Kind::Moon => ItemKind::Moon,
        Kind::Minor => ItemKind::Minor,
        Kind::Band => ItemKind::Population,
    }
}

/// What this ship can see, as its own instruments have it.
///
/// **The map is a chart, not a camera.** Bodies here are what this craft *believes* is in the
/// system, not what the generator put there — a planet appears once something has found it. The
/// sky is the other half of that split and stays truth: it is what a craft discovers planets
/// with, and one filtered by knowledge would be one in which nothing could be found. See
/// `lightcone/docs/25-system-knowledge.md`.
///
/// Contacts are retarded — a ship is drawn where the light arriving now left from — and stars
/// are older still. Bodies in the observer's own system are at coordinate time: across one
/// system the delay is under a pixel.
pub fn observed(session: &Session, uplink: &Uplink, eye_ly: DVec3) -> Picture {
    let mut build = Build::with_capacity(uplink.contacts.len() + 64);
    build.push(observer(session, uplink, eye_ly), None);
    push_local_system(&mut build, session);
    push_believed(&mut build, session);
    push_stars(&mut build, session, eye_ly);

    for contact in &uplink.contacts {
        build.push(
            MapItem::body(
                ItemKey::from_id("ship", contact.ship_id.0 as u64),
                contact.name.clone(),
                ItemKind::Ship,
                contact.position_ly,
                // Nothing on this map is drawn at its true size; half the length is a radius to
                // hang a marker on.
                contact.length_m * 0.5,
                contact.facing,
            ).weighing(f64::INFINITY),
            Some(Subject::Craft(contact.ship_id, contact.name.clone())),
        );
    }

    let now = session.coordinate_time_s();
    build.into_picture(|items| MapSnapshot::observed(now, items))
}

/// Every worldline at the coordinate clock, with no light delay.
///
/// This can show bodies and not ships, which is the gate working rather than a gap: the client
/// is never sent an un-retarded contact, because `lc_proto::Cleared` makes one
/// unconstructible. Other ships are absent and the panel says so.
///
/// The only difference from [`observed`] is the instant each worldline is sampled at. See
/// `lightcone/docs/07-rendering.md`.
#[cfg(feature = "godview")]
pub fn coordinate(session: &Session, uplink: &Uplink, eye_ly: DVec3) -> Picture {
    let now = session.coordinate_time_s();
    let mut build = Build::default();
    build.push(observer(session, uplink, eye_ly), None);
    push_local_system(&mut build, session);
    if let Some(system) = session.system.as_ref() {
        let labels = session.home_labels();
        for body in system.drawables_at(eye_ly, now) {
            push_drawable(&mut build, &body, &labels);
        }
    }
    push_stars(&mut build, session, eye_ly);
    build.into_picture(|items| MapSnapshot::coordinate(now, items))
}

/// Whether this client may ask for the god view.
///
/// Advisory: it decides whether the control is offered, nothing more. A client asserts this
/// from its own ticket, the shard decides what it answers, and the compile gate keeps the
/// mechanism out of a shipped build. See `07-rendering.md`.
///
/// `perm` is the claim `lc_server::ability::Level` reads, duplicated because this crate must
/// not link the server. No ticket is the offline case, which `local::start` grants.
#[cfg(feature = "godview")]
pub fn may_see_everything(ticket: Option<&str>) -> bool {
    let Some(ticket) = ticket else { return true };
    matches!(permission_claim(ticket), Some(1..=3))
}

/// The `perm` claim out of a ticket's payload, if it has one.
#[cfg(feature = "godview")]
fn permission_claim(ticket: &str) -> Option<i64> {
    let payload = ticket.split('.').nth(1)?;
    let claims: serde_json::Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    claims.get("perm")?.as_i64()
}

/// Decode base64url without padding. Written out because this client links the `base64` crate
/// only off the browser, and the browser is where the gate must still hold.
#[cfg(feature = "godview")]
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// This ship, by the name every other client has for it.
///
/// Named like any other craft and weighed like one: a map that draws five ships and names four
/// of them is a map with a hole where the reader is. The weight is what keeps it out of the
/// mass comparison the names are ranked by — see [`em_map::weight`].
fn observer(session: &Session, uplink: &Uplink, eye_ly: DVec3) -> MapItem {
    MapItem::body(
        ItemKey::from_name("observer"),
        uplink.own_name(),
        ItemKind::Observer,
        eye_ly,
        session.ship.length_m * 0.5,
        session.ship.motion.attitude,
    )
    .weighing(f64::INFINITY)
}


/// A body, and the target the rest of the interface names it by.
///
/// The same name `pick.rs` builds a `Target::Body` from over the sky, so a click means the
/// same thing in either mode.
#[cfg(feature = "godview")]
fn push_drawable(build: &mut Build, body: &lc_world::system::Drawable, labels: &lc_world::labels::Labels) {
    let label = labels.of(&body.name);
    build.push(
        MapItem::body(
            ItemKey::from_name(&body.name),
            label.clone(),
            kind_of(body.kind),
            body.position_ly,
            body.radius_m,
            body.pole,
        )
        .weighing(body.mass_kg),
        Some(Subject::Body(body.name.clone(), label)),
    );
}

/// What holds the ship, as a key into the snapshot.
///
/// The same body the readout names while coasting — `lc_world::coast::Coast::primary` asks the
/// system the same question — so "about Earth" and the center of the map agree by construction.
/// `None` between the stars, where there is no system to be held by.
///
/// The star is keyed by its catalogue id and every other body by name, because `drawables_at`
/// leaves the star out and [`push_local_system`] puts it back under a key of its own. See
/// [`key_of`].
pub fn primary(session: &Session) -> Option<ItemKey> {
    let system = session.system.as_ref()?;
    Some(key_of(system, system.holding(session.ship.motion.position_ly,
        session.coordinate_time_s())))
}

/// How a body of the local system is keyed.
fn key_of(system: &lc_world::system::LocalSystem, index: em_sim::id::BodyIndex) -> ItemKey {
    match index == system.primary() {
        true => ItemKey::from_id("star", system.star.get()),
        false => ItemKey::from_name(system.sim().name(index)),
    }
}

/// What this craft believes is in the local system.
///
/// Keyed the way truth's bodies were, because [`primary`] and the focus are keys and a body
/// keyed differently is a button that does nothing at all. A body no generator made — a
/// transit's false positive — falls back to its own id, which nothing else will ask for.
///
/// Nothing here carries a radius or a mass: a transit says a body exists and roughly where, not
/// how big it is. Both arrive with imaging, in phase 6.
fn push_believed(build: &mut Build, session: &Session) {
    let Some(system) = session.system.as_ref() else { return };
    let now = session.coordinate_time_s();
    let star_ly = system.star_position_ly();
    let pole = match session.knowledge.system_plane(system.star) {
        lc_world::knowledge::SystemPlane::Known { pole, .. } => pole,
        _ => DVec3::Z,
    };
    for belief in session.knowledge.bodies_of(system.star, now) {
        let key = believed_key(system, system.star, belief.body);
        let label = belief.name.clone().unwrap_or_else(|| "unnamed body".to_string());
        let subject = Some(Subject::Body(label.clone(), label.clone()));
        match belief.position_now {
            // Where on the ring it is, with the error drawn along the ring rather than across
            // it: what is uncertain is how far round it has got.
            Placed::Known { offset_au, sigma_au } => {
                let at = star_ly + offset_au * AU_LY;
                let along = offset_au.normalize_or(DVec3::X).cross(pole).normalize_or(DVec3::X);
                build.push(
                    MapItem::body(key, label, ItemKind::Planet, at, 0.0, pole)
                        .spread(at - along * (sigma_au * AU_LY), at + along * (sigma_au * AU_LY)),
                    subject,
                );
            }
            // A sphere of that radius, dashed, and its thickness *is* the error: an orbit of
            // known size and unknown orientation is not a ring in a guessed plane. The shell
            // falls out of `outline::torus` at a right half-angle, which the Oort cloud
            // already draws.
            Placed::Shell { radius_au, sigma_au } => {
                let (r, s) = (radius_au * AU_LY, sigma_au.abs() * AU_LY);
                build.push(
                    MapItem::annulus(
                        key,
                        label,
                        star_ly,
                        pole,
                        em_map::outline::Extent {
                            inner: (r - s).max(0.0),
                            outer: r + s,
                            half_angle_rad: std::f64::consts::FRAC_PI_2,
                        },
                    ),
                    subject,
                );
            }
            // Believed to exist, with nowhere to put it. Drawing it anywhere would be a claim.
            Placed::Unknown => {}
        }
    }
}

/// Light-years in an astronomical unit.
const AU_LY: f64 = lc_world::navigation::AU / lc_world::system::M_PER_LY;

/// A believed body's map key: the generator's, where the body is one the generator made.
fn believed_key(
    system: &lc_world::system::LocalSystem,
    star: lc_world::sky::StarId,
    body: lc_world::knowledge::BodyId,
) -> ItemKey {
    system
        .inventory()
        .iter()
        .find_map(|entry| match &entry.target {
            lc_world::navigation::Target::Body(name)
                if lc_world::knowledge::BodyId::of(star, name) == body =>
            {
                Some(ItemKey::from_name(name))
            }
            _ => None,
        })
        .unwrap_or_else(|| ItemKey::from_id("phantom", body.get()))
}

/// The local system's own star and its belts. `drawables_at` returns neither: the primary is
/// excluded by construction and a population is not a body.
fn push_local_system(build: &mut Build, session: &Session) {
    let Some(system) = session.system.as_ref() else { return };
    let name = session.name_of(system.star);
    build.push(
        MapItem::body(
            ItemKey::from_id("star", system.star.get()),
            name.clone(),
            ItemKind::Star,
            system.star_position_ly(),
            system.star_radius_m(),
            // The star's own spin, not its planets' plane: a few degrees apart, and this is
            // where its surface features will sit.
            system.star_spin,
        ).weighing(system.star_mass_kg()),
        Some(Subject::Star(system.star, name)),
    );
    let origin = system.star_position_ly();
    for (index, population) in system.populations.iter().enumerate() {
        let Some(extent) = population.extent() else { continue };
        let name = lc_world::navigation::band_designation(population);
        build.push(
            MapItem::annulus(
                ItemKey::from_name(&name),
                name.clone(),
                origin,
                population.pole,
                em_map::outline::Extent {
                    inner: extent.inner_m,
                    outer: extent.outer_m,
                    // Without it a belt and a cloud are the same pair of radii.
                    // `lc_world::navigation::is_flat` reads the same number.
                    half_angle_rad: extent.half_angle_rad,
                },
            ),
            // The index into the system's own list, which is what `Target::Band` means.
            Some(Subject::Swarm(index, name)),
        );
    }
}

/// Stars this ship has a position for, less the one this system is already drawing.
///
/// **Believed positions, not catalogue positions.** A star is on the map because somebody
/// measured a parallax to it, and it is drawn where that measurement puts it — off by the
/// error on the measurement, which for a charted distance is a percent of the range. A star
/// detected but never triangulated has a direction and no place to be, so it is not here; the
/// sky view is where it is visible, and the telescope is what fixes it.
fn push_stars(build: &mut Build, session: &Session, eye_ly: DVec3) {
    let here = session.system.as_ref().map(|s| s.star);
    for (id, belief) in session.knowledge.stars() {
        if Some(id) == here {
            continue;
        }
        let lc_world::knowledge::Distance::Measured { position_ly, sigma_ly } = belief.distance else {
            continue;
        };
        if position_ly.distance(eye_ly) > REACH_LY {
            continue;
        }
        let line = (position_ly - eye_ly).normalize_or_zero();
        let name = session.name_of(id);
        // Sized from what is believed, not looked up: brighter for its distance is ranked above
        // fainter, and nothing on the map is drawn at a size this ship has not measured.
        let radius_m = lc_world::star::Star::SOL.radius_m;
        let mass_solar = belief
            .luminosity_w()
            .map_or(1.0, |watts| (watts / sun_band_w(belief.band)).max(0.0).powf(0.25));
        build.push(
            MapItem::body(
                ItemKey::from_id("star", id.get()),
                name.clone(),
                ItemKind::Star,
                position_ly,
                radius_m,
                DVec3::Z,
            )
            .weighing(mass_solar * SOLAR_MASS_KG)
            // A parallax is vague in depth and sharp across it, so the error is drawn along the
            // line of sight: the far edge of a charted volume is visibly the uncertain part.
            .spread(position_ly - line * sigma_ly, position_ly + line * sigma_ly),
            Some(Subject::Star(id, name)),
        );
    }
}

/// The Sun's luminosity in one band, watts: what a believed luminosity is ranked against.
fn sun_band_w(band: em_spectra::Band) -> f64 {
    let m = lc_world::system::M_PER_LY;
    4.0 * std::f64::consts::PI * m * m * lc_world::knowledge::survey::flux_from(&lc_world::star::Star::SOL, band, m)
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;

    /// The three authored stars sit at 4.2, 11 and 25 light-years along `+X`.
    ///
    /// Charted out to thirty, because the map draws what is *known* and a session that has
    /// looked at nothing has an empty map — which is the rule this file now enforces and is
    /// tested for on its own below.
    fn session() -> Session {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        session.issue_charts(30.0);
        session
    }

    fn keys(snapshot: &MapSnapshot) -> Vec<ItemKey> {
        snapshot.items.iter().map(|i| i.key).collect()
    }

    /// A ship between systems is still somewhere. Returning early when there is no local
    /// system loses the player off their own map.
    #[test]
    fn the_observer_is_in_every_snapshot() {
        let session = session();
        let snapshot = observed(&session, &Uplink::default(), DVec3::ZERO)
            .snapshot;
        let observer = snapshot.observer().expect("the observer is not on their own map");
        assert_eq!(observer.position_ly, DVec3::ZERO);
        assert_eq!(snapshot.provenance, em_map::Provenance::Observed);
    }

    /// The reach is a sphere about the observer, not the world origin, so flying somewhere
    /// finds different stars.
    #[test]
    fn the_reach_is_a_sphere_about_the_observer() {
        let session = session();
        let count = |eye: DVec3| {
            observed(&session, &Uplink::default(), eye)
                .snapshot
                .items
                .iter()
                .filter(|i| i.kind == ItemKind::Star)
                .count()
        };
        // From the origin the near two are well inside 25 ly and the third is on the line —
        // where it falls depends on the error on its charted distance, which is the point.
        assert!(count(DVec3::ZERO) >= 2);
        assert_eq!(count(DVec3::new(-15.0, 0.0, 0.0)), 1);
        assert_eq!(count(DVec3::new(-100.0, 0.0, 0.0)), 0);
    }

    /// A ship that has surveyed nothing and been given no charts has nothing to draw. The map
    /// is a record of what has been measured, not a view of the world.
    #[test]
    fn an_unsurveyed_sky_puts_no_stars_on_the_map() {
        let blank = Session::new(&AuthoredStars::sample(), 3);
        let snapshot =
            observed(&blank, &Uplink::default(), DVec3::ZERO).snapshot;
        assert!(!snapshot.items.iter().any(|i| i.kind == ItemKind::Star));
        assert!(
            snapshot.observer().is_some(),
            "the ship is still on its own map"
        );
    }

    /// A charted position is somebody else's parallax, so the mark sits where they measured
    /// it rather than where the star is.
    #[test]
    fn a_star_is_drawn_where_it_is_believed_to_be() {
        let session = session();
        let star = &session.stars[1];
        let snapshot = observed(&session,
            &Uplink::default(),
            DVec3::ZERO,
        )
        .snapshot;
        let drawn = snapshot
            .items
            .iter()
            .find(|i| i.key == ItemKey::from_id("star", star.id.get()))
            .expect("a charted star is on the map");
        let error = drawn.position_ly.distance(star.position_ly);
        assert!(error > 0.0, "a measured position is not the truth");
        assert!(
            error < 0.05 * star.position_ly.length(),
            "but it is close: {error} ly"
        );
    }

    /// **The primary has to be a key the snapshot holds.** The star is keyed by its catalogue
    /// id and every other body by name, and a primary keyed the other way is a button that
    /// does nothing at all.
    #[test]
    fn the_primary_is_a_key_the_snapshot_holds() {
        let stars = AuthoredStars::sample();
        // The third authored star is the one whose generated system has planets.
        let star = &lc_world::sky::StarProvider::stars(&stars)[2];
        let system = lc_world::system::LocalSystem::for_star(star).expect("a generated system");
        let drawn = system.drawables_at(star.position_ly, 0.0);
        let body = drawn.first().expect("a generated system has bodies");

        // At a body's own center, that body holds the ship.
        assert_eq!(
            key_of(&system, system.holding(body.position_ly, 0.0)),
            ItemKey::from_name(&body.name),
            "the map keys a body by name",
        );
        // At the star's, nothing closer does.
        let star_key = key_of(&system, system.holding(star.position_ly, 0.0));
        assert_eq!(star_key, ItemKey::from_id("star", system.star.get()));
        assert_ne!(star_key, ItemKey::from_name(&system.star_name), "the star is not by name");
    }

    /// **The map draws no planet nobody has found.** This is the whole of the chart-not-camera
    /// split: the same system, before and after one body is known, and the sky is untouched
    /// either way. See `lightcone/docs/25-system-knowledge.md`.
    #[test]
    fn the_map_draws_only_bodies_this_craft_believes_in() {
        let mut session = session();
        let star = lc_world::sky::StarProvider::stars(&AuthoredStars::sample())[2].clone();
        session.ship.motion.position_ly = star.position_ly;
        session.sync_system();
        let system = session.system.clone().expect("the ship is at a star");
        assert!(!system.inventory().is_empty(), "the generator made bodies to not draw");

        let planets = |session: &Session| {
            observed(session, &Uplink::default(), star.position_ly)
                .snapshot
                .items
                .iter()
                .filter(|i| i.kind == ItemKind::Planet || i.annulus_m.is_some())
                .count()
        };
        // Populations are the generator's until phase 8, so count from where we start rather
        // than from zero: what matters is that a body appears when it is found and not before.
        let before = planets(&session);

        // One settled transit's worth of knowledge: an orbit of known size and unknown
        // orientation.
        let key = system
            .inventory()
            .iter()
            .find_map(|e| match &e.target {
                lc_world::navigation::Target::Body(key) => Some(key.clone()),
                _ => None,
            })
            .expect("a body");
        let body = lc_world::knowledge::BodyId::of(star.id, &key);
        let period = 3.0e7;
        session.knowledge.found_planet(
            star.id,
            body,
            lc_world::knowledge::Orbit {
                about: None,
                witness: session.knowledge.owner,
                period_s: (period, period * 1.0e-3),
                semi_major_au: (1.5, 0.15),
                eccentricity: None,
                orientation: lc_world::knowledge::Orientation::EdgeOnTo { toward: DVec3::X },
                epoch_s: Some(0.0),
                method: lc_world::knowledge::Method::Transit,
                stated_s: 0.0,
                lineage: Vec::new(),
            },
            1.0,
            0.0,
        );
        assert_eq!(planets(&session), before + 1, "the found body is not drawn");

        // And it is drawn as a shell, not as a ring in a plane nobody solved: an orbit of known
        // size and unknown orientation is a sphere of that radius.
        let snapshot = observed(&session, &Uplink::default(), star.position_ly).snapshot;
        let shell = snapshot
            .items
            .iter()
            .find(|i| i.key == ItemKey::from_name(&key))
            .expect("the believed body is on the map");
        let extent = shell.annulus_m.expect("a shell has an extent");
        assert!(
            (extent.half_angle_rad - std::f64::consts::FRAC_PI_2).abs() < 1.0e-12,
            "a right half-angle is what makes it a shell rather than a belt",
        );
        assert!(extent.inner < extent.outer, "the thickness is the distance error");
    }

    /// The star is drawn about its own spin axis, not about `+Z`.
    ///
    /// It was `DVec3::Z` for every system, which drew a generated star lying in the ecliptic of
    /// J2000 while its planets orbited somewhere else entirely. See
    /// `lightcone/docs/25-system-knowledge.md`.
    #[test]
    fn the_star_is_drawn_about_its_own_axis() {
        // The third authored star is the one whose generated system has planets, and a local
        // system is loaded from where the ship is rather than set.
        let mut session = session();
        let star = lc_world::sky::StarProvider::stars(&AuthoredStars::sample())[2].clone();
        session.ship.motion.position_ly = star.position_ly;
        session.sync_system();
        let system = session.system.clone().expect("the ship is at a star");
        let snapshot =
            observed(&session, &Uplink::default(), star.position_ly).snapshot;
        let star = snapshot
            .items
            .iter()
            .find(|i| i.key == ItemKey::from_id("star", system.star.get()))
            .expect("the local star is on the map");

        assert_eq!(star.pole, system.star_spin);
        assert!(star.pole.dot(DVec3::Z).abs() < 0.999, "the star is back on +Z");
        // Near its planets' plane but not in it: the spin is the star's own.
        let tilt = star.pole.dot(system.pole).clamp(-1.0, 1.0).acos();
        assert!(tilt <= lc_world::sky::generate::SPIN_TILT_MAX_RAD + 1.0e-12, "{tilt} rad off");
    }

    /// **This ship is named like any other.** A map that draws five ships and names four of
    /// them has a hole in it where the reader is, and the name is the one the account carries
    /// rather than a word for "you".
    #[test]
    fn this_ship_is_named_and_weighed_like_a_ship() {
        let snapshot = observed(&session(), &Uplink::default(), DVec3::ZERO)
            .snapshot;
        let observer = snapshot.observer().expect("the observer is not on their own map");
        assert!(!observer.label.is_empty(), "nothing to draw");
        assert!(observer.weight.is_infinite(), "a ship sets no bar for the names");
        // The literal, not the constant: comparing a constant to itself would pass whatever
        // the word was, and the point is that a nameless ship is named rather than described.
        assert_eq!(observer.label, "Anonymous Ship");
        assert_eq!(observer.label, crate::uplink::ANONYMOUS, "two answers to one question");
    }

    /// Two things sharing a key share an entity and a selection.
    #[test]
    fn nothing_shares_a_key() {
        let snapshot = observed(&session(), &Uplink::default(), DVec3::ZERO)
            .snapshot;
        let mut seen = keys(&snapshot);
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "a key is used twice");
    }

    /// A snapshot is stated at one clock, so asking twice at that clock gives one answer.
    /// Reading the star from the propagated arena and the bodies from `drawables_at` breaks
    /// it: the one-instant trap `AGENTS.md` names.
    #[test]
    fn a_snapshot_is_stated_at_one_epoch() {
        let session = session();
        let once = observed(&session, &Uplink::default(), DVec3::ZERO)
            .snapshot;
        let twice = observed(&session, &Uplink::default(), DVec3::ZERO)
            .snapshot;
        assert_eq!(once.epoch_s, session.coordinate_time_s());
        assert_eq!(once, twice);
    }

    #[cfg(feature = "godview")]
    mod gate {
        use super::*;

        /// A ticket is `header.payload.signature` and only the payload is read.
        fn ticket(claims: &str) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let bytes = claims.as_bytes();
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
                let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
                for i in 0..chunk.len() + 1 {
                    out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
                }
            }
            format!("head.{out}.signature")
        }

        /// The whole table. Zero is the only non-administrative level, and anything outside
        /// 1..=3 is a player, so a level this build has not heard of grants nothing. The rule
        /// `lc_server::ability::Level::from_claim` applies.
        #[test]
        fn only_an_administrative_level_may_see_everything() {
            for (perm, want) in [(0, false), (1, true), (2, true), (3, true), (4, false),
                (-1, false), (99, false)] {
                let t = ticket(&format!(r#"{{"sub":"acct-1","perm":{perm}}}"#));
                assert_eq!(may_see_everything(Some(&t)), want, "perm {perm}");
            }
        }

        /// An absent claim is a player, and so is anything that is not a ticket.
        #[test]
        fn a_ticket_without_the_claim_is_a_player() {
            assert!(!may_see_everything(Some(&ticket(r#"{"sub":"acct-1"}"#))));
            for junk in ["", "a", "a.b", "a.b.c", "....", "a.!!!!.c", "head..sig"] {
                assert!(!may_see_everything(Some(junk)), "{junk:?} was let through");
            }
        }

        /// No ticket is the development and offline case, which `local::start` hands
        /// `directing(true)`.
        #[test]
        fn no_sign_in_at_all_is_the_development_case() {
            assert!(may_see_everything(None));
        }

        #[test]
        fn base64url_decodes_what_a_ticket_carries() {
            assert_eq!(base64url("aGVsbG8").unwrap(), b"hello");
            assert_eq!(base64url("aGVsbG8=").unwrap(), b"hello", "padding is tolerated");
            assert_eq!(base64url("-_8").unwrap(), vec![0xfb, 0xff]);
            assert!(base64url("not base64!").is_none());
        }

        /// God view draws no ships, because the client is never sent an un-retarded contact,
        /// and says so rather than quietly omitting them.
        #[test]
        fn the_god_view_is_marked_and_carries_no_ships() {
            let snapshot = coordinate(&session(), &Uplink::default(), DVec3::ZERO).snapshot;
            assert_eq!(snapshot.provenance, em_map::Provenance::Coordinate);
            assert!(snapshot.items.iter().all(|i| i.kind != ItemKind::Ship));
            assert!(snapshot.observer().is_some());
        }
    }
}
