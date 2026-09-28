//! The server's answer to an order: accepted and folded at its time, or refused in words.

// No panics in the game loop, as in `super`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic))]

use bevy::prelude::*;
use lc_proto::{Body, Order, Outbound, Refusal};

use super::Uplink;

pub(super) fn fold(
    uplink: &mut Uplink,
    game: &mut crate::app::Game,
    ui: &mut crate::app::Ui,
    message: Outbound,
) {
    match message {
        Outbound::Accepted { ship_id, event_id, at_t, order } => {
            // **Applied at the server's time, with the server's numbers.** Both are clamped
            // and neither is what was sent, so folding what was sent instead is how a client
            // ends up somewhere the server does not have it.
            let at_s = at_t as f64 * 1e-6;
            // The server drops a standing intercept on any flight order, so the pursuit the
            // interface shows is over too.
            if matches!(
                order,
                Order::SetCourse { .. } | Order::Cross { .. } | Order::CutDrive | Order::Burn { .. }
            ) {
                uplink.chasing = None;
            }
            let said = match &order {
                Order::SendReport { to, .. } => Some(match to.map(|t| uplink.name_of(t)) {
                    Some(who) => format!("report sent to {who}"),
                    None => "report broadcast".to_string(),
                }),
                Order::SetDuty { duty, integration_s } => {
                    game.0.adopt_duty(duty, *integration_s);
                    None
                }
                // What it did arrives in the next `Learned`, with everything else it knows.
                Order::NameIt { .. } => None,
                Order::RetainRaw { subject, keep } => {
                    game.0.knowledge.retain_raw(lc_world::knowledge::Subject::from(*subject), *keep);
                    None
                }
                // The count arrives as `Outbound::Analyzing`: only the shard knows which logs it holds.
                Order::Analyze => None,
                Order::SetCourse { course, accel_g, max_beta } => {
                    let course: lc_world::navigation::Course = course.clone().into();
                    match game.0.set_course_at(at_s, &course, *accel_g, *max_beta) {
                        Some(label) => Some(format!("course: {label} at {accel_g:.0} g")),
                        None => Some("that course could not be flown".into()),
                    }
                }
                Order::Cross { star, accel_g, max_beta } => {
                    // Resolved here too, against this client's own catalog — the same one
                    // the shard was given, which is what makes an id mean one thing on both
                    // ends. A star this build does not hold is a shard and a client that were
                    // handed different skies, and saying so is better than flying nowhere.
                    match game.0.star_by_raw(*star).map(|s| s.position_ly) {
                        Some(to_ly) => {
                            let name = game
                                .0
                                .star_by_raw(*star)
                                .map(|s| game.0.name_of(s.id))
                                .unwrap_or_else(|| "an undetected source".into());
                            match game.0.cross_to_at(at_s, to_ly, *accel_g, *max_beta) {
                                Some(cruise) => {
                                    let years =
                                        cruise.duration_s() / crate::flight::JULIAN_YEAR_S;
                                    let aboard = cruise.proper_duration_s()
                                        / crate::flight::JULIAN_YEAR_S;
                                    Some(format!(
                                        "{name}: {years:.2} years out, {aboard:.2} aboard"
                                    ))
                                }
                                None => Some("that crossing could not be flown".into()),
                            }
                        }
                        None => Some("this build does not have that star".into()),
                    }
                }
                Order::CutDrive => {
                    let note = match game.0.cut_drive_at(at_s) {
                        // What it says is where the ship ended up, because cutting does not
                        // stop it: it keeps its velocity and that velocity is now an orbit.
                        Some(coast) => format!("drive cut — {}", crate::hud::arc(&coast, &game.0.body_label(&coast.primary))),
                        None => "drive cut".to_string(),
                    };
                    Some(note)
                }
                // A standing intercept folds into nothing here. What it *does* arrives as a
                // motive, once per re-solve, through the same placement path a reconnect uses
                // — so the client is told the approach its ship is flying rather than working
                // one out from a quarry it can only see the past of.
                Order::Intercept { ship_id, closeness, approach } => {
                    uplink.chasing =
                        Some(lc_proto::Pursuit { quarry: *ship_id, closeness: *closeness, approach: *approach });
                    Some(format!("closing on {}", ship_id.0))
                }
                // The server cut the drive of a ship that was flying the pursuit, and this folds
                // the same cut at the same instant.
                Order::BreakOff => {
                    uplink.chasing = None;
                    if game.0.ship.motion.pursuing().is_some() {
                        game.0.cut_drive_at(at_s);
                    }
                    Some("broke off".into())
                }
                // Nothing to fold into the ship's motion. A transmission is an event, and the
                // client learns of it the same way anyone else does: when its light arrives.
                Order::Transmit { .. } | Order::Burn { .. } => None,
                // What a refit does to the account arrives straight after, as `Fitted`.
                Order::Refit { .. } => {
                    ui.0.form.accepted();
                    Some("refit begun".into())
                }
                // What a mode order does to the account arrives straight after, as `Fitted`.
                Order::FieldMode { .. } => None,
                // Folded by `Beams::fold`.
                Order::Emit { .. } => None,
                Order::CancelRefit => Some("refit stopped where it was".into()),
                // Recorded against the identifier the server minted, which is the only thing
                // an acknowledgment will ever name it by. Not shown in the events box: that
                // box is for what happened *to* this ship, and the chat window already has it.
                Order::Say { to, secrecy, body, idem, .. } => {
                    let name = to
                        .and_then(|t| uplink.contacts.iter().find(|c| c.ship_id == t))
                        .map(|c| c.name.clone());
                    uplink.chat.sent(
                        *to,
                        name.as_deref(),
                        event_id,
                        *idem,
                        Body::Text(body.clone()),
                        matches!(secrecy, lc_proto::Secrecy::Sealed),
                        at_s,
                    );
                    None
                }
                Order::OfferKey { to, .. } => {
                    let name = to
                        .and_then(|t| uplink.contacts.iter().find(|c| c.ship_id == t))
                        .map(|c| c.name.clone());
                    uplink.chat.sent(*to, name.as_deref(), event_id, 0, Body::Key, false, at_s);
                    None
                }
                // Answered by `AutoAcking`, never accepted.
                Order::AutoAck { .. } => None,
            };
            info!(?ship_id, at_t, ?order, "accepted");
            uplink.applied = said;
        }
        Outbound::Refused { ship_id, reason } => {
            warn!(?ship_id, ?reason, "an order was refused");
            if matches!(reason, Refusal::NotInSight | Refusal::TooFast) {
                uplink.chasing = None;
            }
            let said = refused(reason);
            // Orders are answered in the order sent, but another may have gone just before the
            // refit, so only a refusal a refit can earn is filed against the Apply that sent it.
            if let crate::ledger::Applying::Sent(target) = &ui.0.form.applying
                && refits_refuse(reason)
            {
                let target = target.clone();
                ui.0.form.applying = crate::ledger::Applying::Refused { target, why: said.clone() };
            }
            uplink.applied = Some(said);
        }
        // Routed elsewhere by `super::fold`.
        _ => {}
    }
}

fn refits_refuse(reason: Refusal) -> bool {
    matches!(
        reason,
        Refusal::Form(_) | Refusal::Short(_) | Refusal::UnderWay | Refusal::Refitting | Refusal::NoEnergy | Refusal::Impossible
    )
}

/// What a refusal reads as. Something to act on, per `lightcone/docs/18-ui-style.md`.
pub fn refused(reason: Refusal) -> String {
    match reason {
        Refusal::Impossible => "the server refused that order".into(),
        Refusal::NotYours | Refusal::NotYou => "that is not your ship".into(),
        Refusal::NotInSight => "there is nothing there to close on".into(),
        Refusal::TooFast => "too fast to match; kill the closing speed first".into(),
        Refusal::NoEnergy => "not enough energy stored for that".into(),
        Refusal::Refitting => "the drones are working: cancel the refit to fly".into(),
        Refusal::UnderWay => crate::ledger::Blocked::UnderWay.reason().into(),
        Refusal::Short(short) => crate::refit_panel::shortfall(short),
        // Their key has to arrive before it can be used, and asking for it is a
        // message like any other — which is to say, it takes as long as the light does.
        Refusal::NoKey => "no key for them yet; send yours and ask for theirs".into(),
        Refusal::NothingNew => "nothing new to report since the last one".into(),
        Refusal::NotBuilt => "this shard cannot do that yet".into(),
        Refusal::Switching => "the field is already switching".into(),
        Refusal::NoAperture => "engines at one end only: emit fore or aft".into(),
        Refusal::OverRating => "more power than those apertures are rated for".into(),
        Refusal::Form(fault) => crate::refit_panel::form_fault(fault),
        Refusal::TooManyPresets => "no room for another preset; delete one first".into(),
        Refusal::PresetName => {
            format!("a preset needs a name of 1 to {} bytes", lc_proto::form::PRESET_NAME_LIMIT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uplink::tests::*;
    use crate::uplink::*;
    use crate::uplink::fold;
    use lc_proto::Refusal;

    /// Sending is not receiving. A message this ship sent is in the transcript against the
    /// identifier the server minted — which is the only thing an acknowledgment can name — and
    /// is not in the events box, which is for what happened *to* this ship.
    #[test]
    fn an_accepted_message_is_recorded_against_the_identifier_it_will_be_acknowledged_by() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let before = ui.0.notifications.len();
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
        let conversation = uplink.chat.get(ShipId(2)).expect("a conversation");
        assert_eq!(conversation.lines[0].event_ids, vec![4242]);
        assert!(conversation.lines[0].mine);
        let sent = conversation.lines[0].clone();
        assert!(!conversation.delivered(&sent), "unanswered, so not acknowledged");
        assert_eq!(ui.0.notifications.len(), before, "a sent message reported itself as news");

        // And the acknowledgment, when it comes back, names it.
        let spoken = lc_proto::Spoken {
            to: Some(7),
            beamed: false,
            idem: 13,
            sealed: false,
            body: Body::Text("got it".into()),
            acks: vec![4242],
        };
        fold(&mut uplink, &mut game, &mut ui, heard(43, 2, spoken, lc_proto::kind::MESSAGE));
        assert!(uplink.chat.get(ShipId(2)).unwrap().delivered(&sent));
    }

    /// A flight order ends a standing intercept on the server, so the interface stops showing
    /// one. A transmission is not a flight order.
    #[test]
    fn an_accepted_flight_order_ends_the_pursuit_shown() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let accepted = |order| Outbound::Accepted { ship_id: ShipId(7), event_id: 1, at_t: 0, order };
        let pursuit = lc_proto::Pursuit {
            quarry: ShipId(2),
            closeness: lc_proto::Closeness::Company,
            approach: lc_proto::Approach::Direct,
        };

        uplink.chasing = Some(pursuit);
        fold(&mut uplink, &mut game, &mut ui, accepted(Order::Transmit { power_w: 1.0 }));
        assert_eq!(uplink.chasing, Some(pursuit), "a transmission ended it");

        for order in [Order::CutDrive, Order::Burn { beta: [0.0, 1e-3, 0.0] }] {
            uplink.chasing = Some(pursuit);
            fold(&mut uplink, &mut game, &mut ui, accepted(order.clone()));
            assert_eq!(uplink.chasing, None, "{order:?} left the pursuit showing");
        }
    }

    /// An order the server would not take is not a disconnection.
    #[test]
    fn a_refused_order_leaves_the_connection_alone() {
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        fold(
            &mut uplink,
            &mut game,
            &mut ui,
            Outbound::Refused {
                ship_id: ShipId(7),
                reason: Refusal::Impossible,
            },
        );
        assert!(
            uplink.joined().is_some(),
            "a refused order dropped the connection"
        );
    }

    /// A refusal while Apply awaits its answer is filed against the target sent, naming the part,
    /// and an acceptance clears the way for the next, and the editor's history.
    #[test]
    fn a_refit_refused_is_told_to_the_apply_that_sent_it() {
        use crate::ledger::Applying;
        let (mut uplink, mut game, mut ui) = app();
        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        let target = lc_world::form::Form::starting();
        ui.0.form.applying = Applying::Sent(target.clone());
        let fault = lc_proto::FormFault::EngineOffAxis(lc_proto::form::PartId(2));
        fold(&mut uplink, &mut game, &mut ui, Outbound::Refused { ship_id: ShipId(7), reason: Refusal::Form(fault) });
        let why = "part 2 must point fore or aft".to_string();
        assert_eq!(ui.0.form.applying, Applying::Refused { target: target.clone(), why });

        ui.0.form.applying = Applying::Sent(target.clone());
        let mut draft = crate::draft::Draft::new(target.clone());
        let twist = draft.twist(lc_world::form::PartId(4), 0.3).unwrap();
        draft.apply(&twist, &lc_world::fitting::Balance::DEFAULT).unwrap();
        ui.0.form.history.record(&twist, &draft.ship, None);
        let order = Order::Refit { target: (&target).into() };
        fold(&mut uplink, &mut game, &mut ui, Outbound::Accepted { ship_id: ShipId(7), event_id: 1, at_t: 2_000_000, order });
        assert_eq!(ui.0.form.applying, Applying::Idle);
        assert!(ui.0.form.history.entries().is_empty(), "an accepted round clears the history");

        fold(&mut uplink, &mut game, &mut ui, Outbound::Refused { ship_id: ShipId(7), reason: Refusal::NoKey });
        assert_eq!(ui.0.form.applying, Applying::Idle, "with nothing sent, a refusal is some other order's");

        ui.0.form.applying = Applying::Sent(target.clone());
        fold(&mut uplink, &mut game, &mut ui, Outbound::Refused { ship_id: ShipId(7), reason: Refusal::NoKey });
        assert_eq!(ui.0.form.applying, Applying::Sent(target.clone()), "a refit is never refused for want of a key");

        fold(&mut uplink, &mut game, &mut ui, welcome(0));
        assert_eq!(ui.0.form.applying, Applying::Idle, "a new session answers nothing sent on the old one");
    }
}
