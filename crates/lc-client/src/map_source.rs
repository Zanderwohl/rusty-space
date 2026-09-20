//! Building a map snapshot out of what this client holds.
//!
//! Every provider here returns a plain [`MapSnapshot`]. The map renders one and knows nothing
//! about where it came from, which is what makes switching perspective a choice of function
//! rather than a second renderer — and what leaves room for the sources that are not built
//! yet: a relay, a probe, a fleet's shared picture.
//!
//! **The catalogue is test data.** Stars come from [`Session::stars`], which is whatever the
//! session was handed — a CSV today, a shard's answer once the server is authoritative. Both
//! ends already agree by construction: `local::start` passes the client's own stars to the
//! server in the box precisely so the two place craft in the same systems.

use em_map::{ItemKey, ItemKind, MapItem, MapSnapshot};
use glam::DVec3;
use lc_world::navigation::Kind;

use crate::session::Session;
use crate::starfield::Bodies;
use crate::uplink::Uplink;

/// How far the map reaches, light-years.
///
/// A fixed sphere, deliberately: the interesting question is what is near, and a reach that
/// moved with the zoom would change what exists as well as what is framed. Twenty-five
/// light-years is the conventional solar neighborhood and is 166 stars out of the bundled
/// catalogue's 119 625 — small enough to draw whole, large enough to have somewhere to go.
/// One solar mass. The catalogue states a star's mass in them and [`em_map::MapItem::weight`]
/// wants kilograms, because a planet's does too and a common scale is the whole point.
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
/// Bodies come off the [`Bodies`] resource rather than from a second call to `drawables_at`:
/// it is refilled every frame from the same eye the sky is drawn from, so reading it is what
/// makes the map and the view agree by construction instead of by coincidence.
///
/// Ships come from the uplink's contacts, and those are genuinely retarded — a contact is
/// drawn where the light arriving now left from. Stars are older still. Bodies inside the
/// observer's own system are at coordinate time, because `update_bodies` places them there and
/// says why: across one system the delay moves a planet by less than a pixel.
pub fn observed(session: &Session, bodies: &Bodies, uplink: &Uplink, eye_ly: DVec3)
    -> MapSnapshot {
    let mut items = Vec::with_capacity(bodies.drawn.len() + uplink.contacts.len() + 64);
    items.push(observer(session, eye_ly));
    push_local_system(&mut items, session);
    push_bodies(&mut items, bodies);
    push_stars(&mut items, session, eye_ly);

    for contact in &uplink.contacts {
        items.push(MapItem::body(
            ItemKey::from_id("ship", contact.ship_id.0 as u64),
            contact.name.clone(),
            ItemKind::Ship,
            contact.position_ly,
            // A hull is a few hundred meters and nothing on this map is drawn at its true
            // size anyway; half the length is an honest radius to hang a marker on.
            contact.length_m * 0.5,
            contact.facing,
        ).weighing(f64::INFINITY));
    }

    MapSnapshot::observed(session.coordinate_time_s(), items)
}

/// Every worldline at the coordinate clock, with no light delay.
///
/// **What this can honestly show is bodies, and that is the gate working rather than a gap.**
/// The client is never sent an un-retarded contact: `lc_proto::Cleared` makes one
/// unconstructible, and `lightcone/docs/07-rendering.md` records that the server gate is what
/// enforces god view. So other ships are absent here and the panel says so, rather than
/// drawing a picture that is silently missing half of what it claims to show.
///
/// The difference from [`observed`] is the instant each worldline is sampled at and nothing
/// else, which is `07-rendering.md`'s "a parameter of the extract step, not a separate
/// renderer" written out.
#[cfg(feature = "godview")]
pub fn coordinate(session: &Session, eye_ly: DVec3) -> MapSnapshot {
    let now = session.coordinate_time_s();
    let mut items = vec![observer(session, eye_ly)];
    push_local_system(&mut items, session);
    if let Some(system) = session.system.as_ref() {
        for body in system.drawables_at(eye_ly, now) {
            items.push(drawable_item(&body));
        }
    }
    push_stars(&mut items, session, eye_ly);
    MapSnapshot::coordinate(now, items)
}

/// Whether this client may ask for the god view at all.
///
/// **Advisory, and only about whether the control is offered.** A client asserts this from its
/// own ticket and a client can assert anything; what a shard will answer is the shard's, and
/// the compile gate is what keeps the mechanism out of a shipped build entirely. Three gates
/// doing three jobs, as `07-rendering.md` has it.
///
/// `perm` is the claim `lc_server::ability::Level` reads, and the rule is that one: **zero is
/// the only non-administrative level**, and anything outside 1..=3 is a player. The duplication
/// is the one `ability.rs` already argues for about `lc_identity` — this crate must not link
/// the server, because the browser build has none.
///
/// No ticket at all is the development and offline case, which is the same one `local::start`
/// hands `directing(true)` to: a server in the box, started by the player, in their own
/// process, with nobody to protect.
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

/// Decode base64url without padding.
///
/// Written out rather than taken from the `base64` crate, which this client links only off the
/// browser — and the browser is exactly where a gate must not quietly evaporate.
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

fn observer(session: &Session, eye_ly: DVec3) -> MapItem {
    MapItem::body(
        ItemKey::from_name("observer"),
        "this ship",
        ItemKind::Observer,
        eye_ly,
        session.ship.length_m * 0.5,
        session.ship.motion.attitude,
    )
}

fn drawable_item(body: &lc_world::system::Drawable) -> MapItem {
    MapItem::body(
        ItemKey::from_name(&body.name),
        body.name.clone(),
        kind_of(body.kind),
        body.position_ly,
        body.radius_m,
        body.pole,
    )
    .weighing(body.mass_kg)
}

fn push_bodies(items: &mut Vec<MapItem>, bodies: &Bodies) {
    items.extend(bodies.drawn.iter().map(drawable_item));
}

/// The local system's own star and its belts. `drawables_at` returns neither: the primary is
/// excluded by construction and a population is not a body.
fn push_local_system(items: &mut Vec<MapItem>, session: &Session) {
    let Some(system) = session.system.as_ref() else { return };
    items.push(MapItem::body(
        ItemKey::from_id("star", system.star.get()),
        system.star_name.clone(),
        ItemKind::Star,
        system.star_position_ly(),
        system.star_radius_m(),
        DVec3::Z,
    ).weighing(system.star_mass_kg()));
    let origin = system.star_position_ly();
    for population in &system.populations {
        let Some(extent) = population.extent() else { continue };
        let name = lc_world::navigation::band_designation(population);
        items.push(MapItem::annulus(
            ItemKey::from_name(&name),
            name,
            origin,
            population.pole,
            em_map::outline::Extent {
                inner: extent.inner_m,
                outer: extent.outer_m,
                // Without this a belt and a cloud are the same pair of radii, and one of them
                // is a shell. `lc_world::navigation::is_flat` reads the same number.
                half_angle_rad: extent.half_angle_rad,
            },
        ));
    }
}

/// Catalogue stars inside the reach, less the one this system is already drawing.
fn push_stars(items: &mut Vec<MapItem>, session: &Session, eye_ly: DVec3) {
    let here = session.system.as_ref().map(|s| s.star);
    for star in &session.stars {
        if Some(star.id) == here {
            continue;
        }
        if star.position_ly.distance(eye_ly) > REACH_LY {
            continue;
        }
        items.push(MapItem::body(
            ItemKey::from_id("star", star.id.get()),
            star.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get())),
            ItemKind::Star,
            star.position_ly,
            star.star.radius_m,
            DVec3::Z,
        ).weighing(star.mass_solar * SOLAR_MASS_KG));
    }
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;

    /// The three authored stars sit at 4.2, 11 and 25 light-years along `+X`.
    fn session() -> Session {
        Session::new(&AuthoredStars::sample(), 3)
    }

    fn keys(snapshot: &MapSnapshot) -> Vec<ItemKey> {
        snapshot.items.iter().map(|i| i.key).collect()
    }

    /// A ship between systems, with nothing in sight, is still somewhere.
    ///
    /// Break it by returning early when there is no local system and the player vanishes from
    /// their own map — which reads as the map being broken rather than as being empty.
    #[test]
    fn the_observer_is_in_every_snapshot() {
        let session = session();
        let snapshot = observed(&session, &Bodies::default(), &Uplink::default(), DVec3::ZERO);
        let observer = snapshot.observer().expect("the observer is not on their own map");
        assert_eq!(observer.position_ly, DVec3::ZERO);
        assert_eq!(snapshot.provenance, em_map::Provenance::Observed);
    }

    /// The reach is a sphere about the observer, not about the world origin: a ship that flies
    /// somewhere should find different stars there.
    #[test]
    fn the_reach_is_a_sphere_about_the_observer() {
        let session = session();
        let count = |eye: DVec3| {
            observed(&session, &Bodies::default(), &Uplink::default(), eye)
                .items
                .iter()
                .filter(|i| i.kind == ItemKind::Star)
                .count()
        };
        // From the origin all three are inside 25 ly; a light-year the other way puts the
        // furthest one out.
        assert_eq!(count(DVec3::ZERO), 3);
        assert_eq!(count(DVec3::new(-1.0, 0.0, 0.0)), 2);
        assert_eq!(count(DVec3::new(-100.0, 0.0, 0.0)), 0);
    }

    /// Two things sharing a key share an entity and a selection, which looks like a rendering
    /// fault a long way from its cause.
    #[test]
    fn nothing_shares_a_key() {
        let snapshot = observed(&session(), &Bodies::default(), &Uplink::default(), DVec3::ZERO);
        let mut seen = keys(&snapshot);
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "a key is used twice");
    }

    /// A snapshot is stated at one clock, and asking twice at the same clock gives the same
    /// answer. Break it by reading the star out of the propagated arena and the bodies from
    /// `drawables_at` — the one-instant trap `AGENTS.md` names.
    #[test]
    fn a_snapshot_is_stated_at_one_epoch() {
        let session = session();
        let once = observed(&session, &Bodies::default(), &Uplink::default(), DVec3::ZERO);
        let twice = observed(&session, &Bodies::default(), &Uplink::default(), DVec3::ZERO);
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
        /// 1..=3 is a player — a ticket from a broker that has grown a fourth level grants
        /// nothing here rather than whatever `>=` says about an integer this build has never
        /// heard of. The same rule `lc_server::ability::Level::from_claim` applies.
        #[test]
        fn only_an_administrative_level_may_see_everything() {
            for (perm, want) in [(0, false), (1, true), (2, true), (3, true), (4, false),
                (-1, false), (99, false)] {
                let t = ticket(&format!(r#"{{"sub":"acct-1","perm":{perm}}}"#));
                assert_eq!(may_see_everything(Some(&t)), want, "perm {perm}");
            }
        }

        /// A ticket from before permissions existed carries no claim, and an absent claim is a
        /// player. Anything that is not a ticket is a player too, rather than a panic.
        #[test]
        fn a_ticket_without_the_claim_is_a_player() {
            assert!(!may_see_everything(Some(&ticket(r#"{"sub":"acct-1"}"#))));
            for junk in ["", "a", "a.b", "a.b.c", "....", "a.!!!!.c", "head..sig"] {
                assert!(!may_see_everything(Some(junk)), "{junk:?} was let through");
            }
        }

        /// No ticket at all is the development and offline case — a server in the box, started
        /// by the player, in their own process. The same one `local::start` hands
        /// `directing(true)` to.
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

        /// God view says so about itself, and draws no ships: the client is never sent an
        /// un-retarded contact, and a picture silently missing half its subject would be
        /// worse than one that says what it is.
        #[test]
        fn the_god_view_is_marked_and_carries_no_ships() {
            let snapshot = coordinate(&session(), DVec3::ZERO);
            assert_eq!(snapshot.provenance, em_map::Provenance::Coordinate);
            assert!(snapshot.items.iter().all(|i| i.kind != ItemKind::Ship));
            assert!(snapshot.observer().is_some());
        }
    }
}
