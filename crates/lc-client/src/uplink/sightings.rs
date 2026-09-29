//! What other craft are seen to be and do, on the statement or event that shows it.

// No panics in the game loop, as in `super`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use lc_proto::{Body, Outbound, ShipId, Sighting};

use super::{Contact, Uplink};

/// Drive events remembered per craft. Only the latest before the instant drawn matters, and
/// the instant drawn is never more than a tick or two behind the newest.
const REMEMBERED_DRIVES: usize = 16;

/// Ships whose drives are remembered. One out of sight keeps its history for when it comes
/// back, until a busy shard pushes the count past this.
const REMEMBERED_DRIVERS: usize = 1024;

/// How many sightings are remembered. A bound rather than a policy: the fold that replaces
/// this will not keep a list at all.
const REMEMBERED: usize = 256;

pub(super) fn fold(
    uplink: &mut Uplink,
    game: &mut crate::app::Game,
    ui: &mut crate::app::Ui,
    message: Outbound,
) {
    match message {
        Outbound::Present(cleared) => {
            let system = game.0.system.as_deref();
            uplink.contacts =
                cleared.into_iter().map(|c| Contact::seen(c.into_inner(), system)).collect();
            // The server drops a pursuit when its quarry goes out of sight and does not say
            // so — saying so would be a message about somewhere this client can no longer see.
            // Losing the contact is the same fact arriving the only way it can.
            if uplink
                .chasing
                .is_some_and(|p| !uplink.contacts.iter().any(|c| c.ship_id == p.quarry))
            {
                uplink.chasing = None;
            }
        }
        Outbound::Sightings(cleared) => {
            let seen: Vec<Sighting> = cleared.into_iter().map(|c| c.into_inner()).collect();
            for sighting in seen
                .iter()
                .filter(|s| matches!(s.kind, lc_proto::kind::MESSAGE | lc_proto::kind::KEY))
            {
                let Ok(spoken) = serde_json::from_str::<lc_proto::Spoken>(&sighting.payload)
                else {
                    continue;
                };
                let from = ShipId(sighting.source_id);
                let name = uplink.contacts.iter().find(|c| c.ship_id == from).map(|c| c.name.clone());
                // Said before it is folded, because what the box shows is what this craft can
                // read — which for somebody else's sealed mail is the fact of it and no more.
                let who = name.clone().unwrap_or_else(|| uplink.name_of(from));
                // **Overheard traffic is announced and not quoted.** That two other craft are
                // talking is the news; what they said to each other is theirs, and repeating
                // it into this ship's own events box reads as if it had been said here.
                let notice = match (&spoken.body, uplink.chat.filing(spoken.to)) {
                    // News to the transcript, which marks a line delivered, and to nobody
                    // reading the box.
                    (Body::Ack, _) => None,
                    (_, crate::chat::Filing::Overheard) => {
                        let to = spoken.to.map(|to| uplink.name_of(ShipId(to)));
                        Some(format!("{who} -> {}", to.unwrap_or_else(|| "somebody".into())))
                    }
                    (Body::Key, _) => Some(format!("{who}: sent you its key")),
                    (Body::Text(body), _) => Some(format!("{who}: {body}")),
                    (Body::Unreadable, _) => Some(format!("{who}: (encrypted, and not for you)")),
                };
                if let Some(notice) = notice {
                    ui.0.heard(from, notice, sighting.arrive_t as f64 * 1e-6);
                }
                uplink.chat.received(
                    from,
                    name.as_deref(),
                    sighting.event_id,
                    spoken,
                    sighting.emitted_t as f64 * 1e-6,
                    sighting.arrive_t as f64 * 1e-6,
                    sighting.strength,
                );
            }
            // A report is not something anybody said, so it is not filed in a conversation and
            // never answered automatically. What it does is fold into what this craft knows,
            // stamped with the moment its light landed — which is where every record in it
            // gains the hop that says it came from over there. See
            // `lightcone/docs/22-provenance.md`.
            for sighting in seen.iter().filter(|s| s.kind == lc_proto::kind::REPORT) {
                let Ok(reported) = serde_json::from_str::<lc_proto::Reported>(&sighting.payload)
                else {
                    continue;
                };
                let from = ShipId(sighting.source_id);
                let who = uplink.name_of(from);
                let arrived_s = sighting.arrive_t as f64 * 1e-6;
                let Some(body) = reported.body.as_deref() else {
                    ui.0.heard(from, format!("{who} sent a sealed report"), arrived_s);
                    continue;
                };
                // Only counted here. The shard folds it into this craft's knowledge when its light
                // lands, signed in or not, and what it taught arrives in the next `Learned`.
                let stars = lc_proto::decode_report::<lc_world::knowledge::Report>(body).map(|r| r.stars());
                let notice = match stars {
                    Ok(n) => format!("{who}: told you about {n} stars"),
                    Err(_) => format!("{who} sent a report that made no sense"),
                };
                ui.0.heard(from, notice, arrived_s);
            }
            // Each end of a jump arrives at its own light delay. A collapse is `crate::field`'s.
            // Not this ship's own: its console already said where it went.
            let me = uplink.joined().map(|joined| joined.ship_id);
            for sighting in seen.iter().filter(|s| matches!(s.kind, lc_proto::kind::VANISH | lc_proto::kind::APPEAR)) {
                let from = ShipId(sighting.source_id);
                if Some(from) == me {
                    continue;
                }
                let who = uplink.name_of(from);
                let what = if sighting.kind == lc_proto::kind::VANISH { "vanished" } else { "appeared" };
                ui.0.heard(from, format!("{who} {what}"), sighting.arrive_t as f64 * 1e-6);
            }
            for sighting in seen.iter().filter(|s| s.kind == lc_proto::kind::DRIVE) {
                let Ok(change) = serde_json::from_str::<lc_proto::DriveChange>(&sighting.payload) else {
                    continue;
                };
                let drives = uplink.drives.entry(ShipId(sighting.source_id)).or_default();
                let at = crate::contact::DriveAt::of(sighting.emitted_t as f64 * 1e-6, &change);
                let place = drives.partition_point(|d| d.at_s <= at.at_s);
                drives.insert(place, at);
                let excess = drives.len().saturating_sub(REMEMBERED_DRIVES);
                drives.drain(..excess);
            }
            if uplink.drives.len() > REMEMBERED_DRIVERS {
                let contacts = &uplink.contacts;
                uplink.drives.retain(|ship, _| contacts.iter().any(|c| c.ship_id == *ship));
            }
            uplink.seen.extend(seen);
            let excess = uplink.seen.len().saturating_sub(REMEMBERED);
            uplink.seen.drain(..excess);
        }
        // Routed elsewhere by `super::fold`.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::tests::*;
    use crate::uplink::*;
    use crate::uplink::fold;
    use lc_proto::{Cleared, Order};
    use crate::uplink::welcome::lc_server_tick_us;

    /// A statement about other craft becomes the list the renderer and the reticle draw from,
    /// carrying the retarded position rather than a recipe for working out a present one.
    #[test]
    fn a_presence_becomes_a_contact_to_draw() {
        let (mut uplink, mut game, mut ui) = app();
        let presence = lc_proto::Presence {
            ship_id: ShipId(7),
            name: "Vela".into(),
            length_m: 1_200.0,
            at_ly: [1.0, 2.0, 3.0],
            beta: [0.0, 0.1, 0.0],
            facing: [0.0, 0.0, 2.0],
            drive_w: 4.2e17,
            emit_fore_w: 0.0,
            emit_aft_w: 0.0,
            emit_axis: [0.0; 3],
            emit_spread_rad: 0.0,
            emitted_t: 500_000,
            arrive_t: 1_000_000,
            form: lc_proto::Form::default(),
            building: None,
            glow: None,
            glare: None,
        };
        let cleared = lc_proto::Cleared::<lc_proto::Presence>::clear(presence, 1_000_000).unwrap();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Present(vec![cleared]));

        let [contact] = uplink.contacts.as_slice() else { panic!("{:?}", uplink.contacts) };
        assert_eq!(contact.name, "Vela");
        assert_eq!(contact.length_m, 1_200.0);
        assert_eq!(contact.position_ly, glam::DVec3::new(1.0, 2.0, 3.0));
        // Normalized on the way in, so nothing downstream has to wonder.
        assert_eq!(contact.facing, glam::DVec3::Z);
        assert_eq!(contact.emitted_s, 0.5);
        assert_eq!(contact.drive_w, 4.2e17, "it was seen burning");

        // Replaced wholesale, not merged: a contact missing from a statement is gone.
        fold(&mut uplink, &mut game, &mut ui, Outbound::Present(Vec::new()));
        assert!(uplink.contacts.is_empty(), "a dropped contact was kept");
    }

    /// **The flicker this exists for.** Two craft a kilometer and a half apart in low orbit of
    /// Jupiter, and a statement once a server tick — 438 coordinate seconds at the design rate,
    /// three frames at sixty. Held still, the contact fell up to twenty thousand kilometers
    /// behind the ship between statements and snapped back on each one. Reckoned, the range
    /// reads the formation.
    #[test]
    fn a_contact_in_formation_holds_its_range_between_statements() {
        use lc_world::navigation::{Course, Plane};
        use lc_world::system::M_PER_LY;

        let provider =
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalog");
        let sun = lc_world::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("the Sun");
        let mut system = lc_world::system::LocalSystem::for_star(sun).expect("the solar system");
        system.advance_to(0.0);
        let station =
            Course::Orbit { body: "Jupiter".into(), altitude_radii: 0.5, plane: Plane::Equatorial }
                .resolve(&system, DVec3::ZERO, 0.0)
                .expect("an orbit");

        let standoff = DVec3::new(900.0, -1_200.0, 0.0) / M_PER_LY;
        let observer_at = |t: f64| station.place_at(&system, t).unwrap() + standoff;
        let tick_s = lc_server_tick_us(1.0) as f64 * 1e-6;
        let frame_s = tick_s / 3.0;

        for statement in 0..3 {
            let emitted_s = 1_000.0 + statement as f64 * tick_s;
            let presence = lc_proto::Presence {
                ship_id: ShipId(2),
                name: "Vela".into(),
                length_m: 500.0,
                at_ly: station.place_at(&system, emitted_s).unwrap().to_array(),
                beta: lc_world::coast::beta_of(station.velocity_at(&system, emitted_s).unwrap())
                    .to_array(),
                facing: [1.0, 0.0, 0.0],
                drive_w: 0.0,
                emit_fore_w: 0.0,
                emit_aft_w: 0.0,
                emit_axis: [0.0; 3],
                emit_spread_rad: 0.0,
                emitted_t: (emitted_s * 1e6) as i64,
                arrive_t: (emitted_s * 1e6) as i64,
                form: lc_proto::Form::default(),
                building: None,
                glow: None,
                glare: None,
            };
            let mut contact = Contact::seen(presence, Some(&system));
            // Every frame drawn before the next statement lands, and frames from a client whose
            // clock is behind the shard's, which receives each sample from its own future.
            for frame in -3..4 {
                let now_s = emitted_s + frame as f64 * frame_s;
                let here = observer_at(now_s);
                contact.reckon(Some(&system), here, now_s, &[]);
                let range_m = here.distance(contact.position_ly) * M_PER_LY;
                assert!(
                    (range_m - 1_500.0).abs() < 5.0,
                    "{range_m:.0} m at {frame} frames after statement {statement}, not 1500"
                );
            }
        }
    }

    /// A report landing is a line in the events box, counted — and nothing more here. The shard
    /// folds it into the craft's knowledge when its light lands, signed in or not, and what it
    /// taught arrives in the next `Learned`.
    #[test]
    fn a_report_arriving_is_counted_and_its_content_comes_from_the_shard() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let report = lc_world::knowledge::Report { from: lc_world::knowledge::Witness(7), sent_s: 1.0, entries: Vec::new() };
        let reported = lc_proto::Reported {
            to: Some(0),
            beamed: false,
            idem: 3,
            sealed: false,
            body: Some(lc_proto::encode_report(&report)),
            format: lc_proto::REPORT_FORMAT,
        };
        let sighting = Sighting {
            event_id: 21,
            source_id: 7,
            arrive_t: 4_000_000,
            emitted_t: 1_000_000,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::REPORT,
            payload: serde_json::to_string(&reported).unwrap(),
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Sightings(vec![lc_proto::Cleared::<Sighting>::clear(sighting, 4_000_000, 0.0).unwrap()]),
        );
        assert!(
            ui.0.notifications.iter().any(|n| n.text.contains("told you about 0 stars")),
            "{:?}",
            ui.0.notifications.iter().map(|n| n.text.clone()).collect::<Vec<_>>(),
        );
        assert!(game.0.knowledge.is_empty(), "the shard folds it, not the client");
    }

    /// A sealed report for somebody else arrives as the fact of it. An eavesdropper learns
    /// that a survey went past and nothing about what is in it.
    #[test]
    fn a_sealed_report_teaches_an_eavesdropper_nothing() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let reported = lc_proto::Reported {
            to: Some(9),
            beamed: false,
            idem: 4,
            sealed: true,
            body: None,
            format: lc_proto::REPORT_FORMAT,
        };
        let sighting = Sighting {
            event_id: 22,
            source_id: 7,
            arrive_t: 4_000_000,
            emitted_t: 1_000_000,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::REPORT,
            payload: serde_json::to_string(&reported).unwrap(),
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Sightings(vec![
                lc_proto::Cleared::<Sighting>::clear(sighting, 4_000_000, 0.0).unwrap(),
            ]),
        );
        assert!(game.0.knowledge.is_empty(), "nothing was learned from it");
        assert!(
            ui.0.notifications
                .iter()
                .any(|n| n.text.contains("sealed report")),
            "but that it happened is not a secret",
        );
    }

    /// A message arriving is two things at once: a line in the transcript, and a green line in
    /// the events box that opens it. The second is what makes the first findable.
    #[test]
    fn a_message_arriving_is_both_a_line_and_a_notice() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 11,
            sealed: false,
            body: Body::Text("are you there".into()),
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(99, 2, spoken, lc_proto::kind::MESSAGE));

        let conversation = uplink.chat.get(ShipId(2)).expect("a conversation with the sender");
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.lines[0].body, Body::Text("are you there".into()));

        let note = ui.0.notifications.last().expect("nothing in the events box");
        assert_eq!(note.from, Some(ShipId(2)), "the notice does not open anything");
        assert!(note.text.contains("are you there"));
    }

    /// An acknowledgment marks a line delivered and puts nothing in the events box,
    /// whether it was meant for this ship or overheard on its way to somebody else.
    #[test]
    fn an_acknowledgment_is_not_a_notice() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        fold(&mut uplink, &mut game, &mut ui, Outbound::Accepted {
            ship_id: ShipId(7),
            event_id: 4242,
            at_t: 2_000_000,
            order: Order::Say {
                to: Some(ShipId(2)),
                aim: lc_proto::Aim::Omni,
                secrecy: lc_proto::Secrecy::Open,
                body: "hello".into(),
                idem: 4242,
            },
        });
        let before = ui.0.notifications.len();
        let ack = |to| lc_proto::Spoken {
            to: Some(to),
            beamed: false,
            idem: 14,
            sealed: false,
            body: Body::Ack,
            acks: vec![4242],
        };
        fold(&mut uplink, &mut game, &mut ui, heard(44, 2, ack(7), lc_proto::kind::MESSAGE));
        fold(&mut uplink, &mut game, &mut ui, heard(45, 3, ack(99), lc_proto::kind::MESSAGE));

        let conversation = uplink.chat.get(ShipId(2)).unwrap();
        assert!(conversation.delivered(&conversation.lines[0].clone()), "the ack was lost");
        assert_eq!(ui.0.notifications.len(), before, "{:?}", ui.0.notifications.last());
    }

    /// Somebody else's sealed mail is heard and not read, and the box says exactly that rather
    /// than staying silent — a signal falling on the antenna is a fact about the world.
    #[test]
    fn a_sealed_message_for_somebody_else_is_still_noticed() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken =
            lc_proto::Spoken {
                to: Some(99),
                beamed: false,
                idem: 12,
                sealed: true,
                body: Body::Unreadable,
                acks: Vec::new(),
            };
        fold(&mut uplink, &mut game, &mut ui, heard(98, 2, spoken, lc_proto::kind::MESSAGE));

        // Somebody else's mail, so it is overheard rather than a conversation with the sender.
        let overheard = uplink.chat.overheard();
        let heard = overheard.first().expect("nothing was overheard");
        assert!(heard.line.sealed);
        assert_eq!(heard.line.body, Body::Unreadable);
        assert_eq!(heard.to, Some(ShipId(99)), "it forgot who it was for");
        assert!(uplink.chat.get(ShipId(2)).is_none(), "it became a conversation with the sender");
        assert!(ui.0.notifications.last().is_some_and(|n| n.from == Some(ShipId(2))));
    }

    /// **Overheard traffic is announced, not quoted.** That two other craft are talking is the
    /// news; what they said to each other is theirs, and repeating it into this ship's own
    /// events box would read as if it had been said here.
    #[test]
    fn an_overheard_message_is_announced_without_its_contents() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let spoken = lc_proto::Spoken {
            to: Some(99),
            beamed: false,
            idem: 21,
            sealed: false,
            body: Body::Text("rendezvous at the third moon".into()),
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(97, 2, spoken, lc_proto::kind::MESSAGE));

        let note = ui.0.notifications.last().expect("nothing in the events box");
        assert!(!note.text.contains("rendezvous"), "it quoted somebody else's mail: {}", note.text);
        assert!(note.text.contains("->"), "it did not say who was talking to whom: {}", note.text);
        // And it still opens somewhere: the craft that transmitted it.
        assert_eq!(note.from, Some(ShipId(2)));
    }

    /// A key offer arriving is what puts a key in the ring, and the ring is what the interface
    /// reads to decide whether sealing can be offered at all.
    #[test]
    fn a_key_offer_arriving_is_what_lets_the_interface_offer_sealing() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        assert!(!uplink.chat.holds_key(ShipId(2)));
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 0,
            sealed: false,
            body: Body::Key,
            acks: Vec::new(),
        };
        fold(&mut uplink, &mut game, &mut ui, heard(50, 2, spoken, lc_proto::kind::KEY));
        assert!(uplink.chat.holds_key(ShipId(2)));
        assert!(uplink.chat.get(ShipId(2)).unwrap().lines[0].body == Body::Key, "it is in the transcript too");
    }

    /// **A flip between two statements still goes dark.** Sampled burning, then told by drive
    /// events that it went out at 100 s and lit again at 160 s: drawn in between, the plume is
    /// out, though no statement ever saw it so.
    #[test]
    fn a_contacts_plume_follows_its_drive_events_between_statements() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let burning = 4.0e17;
        let presence = lc_proto::Presence {
            ship_id: ShipId(2),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            drive_w: burning,
            emit_fore_w: 0.0,
            emit_aft_w: 0.0,
            emit_axis: [0.0; 3],
            emit_spread_rad: 0.0,
            emitted_t: 0,
            arrive_t: 0,
            form: lc_proto::Form::default(),
            building: None,
            glow: None,
            glare: None,
        };
        let present = |p: lc_proto::Presence| {
            let arrive_t = p.arrive_t;
            Outbound::Present(vec![Cleared::<lc_proto::Presence>::clear(p, arrive_t).unwrap()])
        };
        fold(&mut uplink, &mut game, &mut ui, present(presence.clone()));

        let drive = |event_id, at_s: f64, power_w| {
            let change = lc_proto::DriveChange { power_w, facing: [1.0, 0.0, 0.0], emit_fore_w: 0.0, emit_aft_w: 0.0 };
            let sighting = Sighting {
                event_id,
                source_id: 2,
                arrive_t: (at_s * 1e6) as i64,
                emitted_t: (at_s * 1e6) as i64,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: lc_proto::kind::DRIVE,
                payload: serde_json::to_string(&change).unwrap(),
            };
            Cleared::<Sighting>::clear(sighting, (at_s * 1e6) as i64, 0.0).unwrap()
        };
        let events = vec![drive(1, 100.0, 0.0), drive(2, 160.0, burning)];
        fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(events));

        let power_at = |uplink: &mut Uplink, now_s: f64| {
            uplink.reckon(None, DVec3::X * 1e3 / lc_world::system::M_PER_LY, now_s);
            uplink.contacts[0].drive_w
        };
        assert_eq!(power_at(&mut uplink, 50.0), burning, "before the flip");
        assert_eq!(power_at(&mut uplink, 130.0), 0.0, "in the flip");
        assert_eq!(power_at(&mut uplink, 170.0), burning, "after it");

        // A statement newer than every event is the latest word, and survives the next one.
        let later = lc_proto::Presence { drive_w: 0.0, emitted_t: 400_000_000, arrive_t: 400_000_000, ..presence };
        fold(&mut uplink, &mut game, &mut ui, present(later));
        assert_eq!(power_at(&mut uplink, 410.0), 0.0, "an older event outranked a newer statement");

        // An emit is stated as a burn is: lit out of its end until it goes out.
        let emit = |event_id, at_s: f64, aft_w| {
            let change = lc_proto::DriveChange { power_w: 0.0, facing: [1.0, 0.0, 0.0], emit_fore_w: 0.0, emit_aft_w: aft_w };
            let sighting = Sighting {
                event_id,
                source_id: 2,
                arrive_t: (at_s * 1e6) as i64,
                emitted_t: (at_s * 1e6) as i64,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: lc_proto::kind::DRIVE,
                payload: serde_json::to_string(&change).unwrap(),
            };
            Cleared::<Sighting>::clear(sighting, (at_s * 1e6) as i64, 0.0).unwrap()
        };
        fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(vec![emit(3, 420.0, burning), emit(4, 480.0, 0.0)]));
        let emit_at = |uplink: &mut Uplink, now_s: f64| {
            uplink.reckon(None, DVec3::X * 1e3 / lc_world::system::M_PER_LY, now_s);
            uplink.contacts[0].emit.aft_w
        };
        assert_eq!(emit_at(&mut uplink, 450.0), burning, "lit");
        assert_eq!(emit_at(&mut uplink, 490.0), 0.0, "out");
    }

    /// Past [`REMEMBERED_DRIVERS`], only the contacts in sight keep their drive histories.
    #[test]
    fn drive_histories_are_bounded_by_the_ships_in_sight() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let presence = lc_proto::Presence {
            ship_id: ShipId(2),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [0.0; 3],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            drive_w: 0.0,
            emit_fore_w: 0.0,
            emit_aft_w: 0.0,
            emit_axis: [0.0; 3],
            emit_spread_rad: 0.0,
            emitted_t: 0,
            arrive_t: 0,
            form: lc_proto::Form::default(),
            building: None,
            glow: None,
            glare: None,
        };
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Present(vec![Cleared::<lc_proto::Presence>::clear(presence, 0).unwrap()]),
        );
        let drive = |source_id: i64| {
            let change = lc_proto::DriveChange { power_w: 1.0, facing: [1.0, 0.0, 0.0], emit_fore_w: 0.0, emit_aft_w: 0.0 };
            let sighting = Sighting {
                event_id: source_id,
                source_id,
                arrive_t: 1_000_000,
                emitted_t: 1_000_000,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: lc_proto::kind::DRIVE,
                payload: serde_json::to_string(&change).unwrap(),
            };
            Cleared::<Sighting>::clear(sighting, 1_000_000, 0.0).unwrap()
        };
        let under: Vec<_> = (2..REMEMBERED_DRIVERS as i64 + 2).map(drive).collect();
        fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(under));
        assert_eq!(uplink.drives.len(), REMEMBERED_DRIVERS, "out of sight is kept while there is room");

        let over = vec![drive(REMEMBERED_DRIVERS as i64 + 2)];
        fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(over));
        assert_eq!(uplink.drives.keys().copied().collect::<Vec<_>>(), vec![ShipId(2)]);
    }

    /// A bound, so a long session does not grow without limit before the fold exists.
    #[test]
    fn remembered_sightings_are_bounded_and_keep_the_newest() {
        let (mut uplink, mut game, mut ui) = app();
        for n in 0..(REMEMBERED as i64 + 10) {
            let sighting = Sighting {
                event_id: n,
                source_id: 1,
                arrive_t: n,
                emitted_t: 0,
                direction: [1.0, 0.0, 0.0],
                strength: 1.0,
                kind: 0,
                payload: String::new(),
            };
            let cleared = Cleared::<Sighting>::clear(sighting, i64::MAX, 0.0).unwrap();
            fold(&mut uplink, &mut game, &mut ui, Outbound::Sightings(vec![cleared]));
        }
        assert_eq!(uplink.seen.len(), REMEMBERED);
        assert_eq!(
            uplink.seen.last().unwrap().event_id,
            REMEMBERED as i64 + 9,
            "it kept the oldest instead of the newest",
        );
    }
}
