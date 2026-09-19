//! One account, as an administrator sees it.
//!
//! Visible to administrators and to nobody else — there is no "profile" here that the account
//! itself can reach, and several things on this page are the reason: the private notes on a
//! ban, every address the account has ever linked, and the labels of the machines it is signed
//! in from.
//!
//! The mutable half of the page is one region, `#user-detail`, and every act replaces the
//! whole of it. That is more bytes than swapping the one row that changed and it is worth it:
//! banning somebody changes their standing, their ban list, the action log and which controls
//! are offered, and a page that updates three of those four is a page that lies.

use chrono::{DateTime, Utc};
use lc_identity::actions::Entry;
use lc_identity::bans::{Ban, Reason, Sanction, State, Term};
use lc_identity::level::Level;
use lc_identity::{ability, store::Account};
use maud::{Markup, html};

use crate::auth::Admin;
use crate::detail::{Grant, Link};

/// Everything one user page reads.
pub struct Detail {
    pub account: Account,
    /// When the account was made. Off the index since it went to three columns, and here
    /// instead — a date that is worth having somewhere and worth a column nowhere.
    pub joined: Option<DateTime<Utc>>,
    pub links: Vec<Link>,
    pub grants: Vec<Grant>,
    pub bans: Vec<Ban>,
    pub log: Vec<Entry>,
    pub sanction: Option<Sanction>,
}

/// What just happened, if anything did.
pub struct Notice {
    pub kind: &'static str,
    pub said: String,
}

impl Notice {
    pub fn done(said: impl Into<String>) -> Notice {
        Notice {
            kind: "done",
            said: said.into(),
        }
    }

    pub fn refused(said: impl Into<String>) -> Notice {
        Notice {
            kind: "refused",
            said: said.into(),
        }
    }
}

pub fn page(admin: &Admin, detail: &Detail, notice: Option<&Notice>, now: DateTime<Utc>) -> Markup {
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::routes::USERS) { "← All users" } }
            (region(admin, detail, notice, now))
        }
    }
}

/// The part every act replaces.
pub fn region(
    admin: &Admin,
    detail: &Detail,
    notice: Option<&Notice>,
    now: DateTime<Utc>,
) -> Markup {
    let account = &detail.account;
    let level = account.level();
    html! {
        section id="user-detail" hx-target:inherited="#user-detail" hx-swap:inherited="outerHTML" {
            @if let Some(notice) = notice {
                (super::note(notice.kind, &notice.said))
            }

            header class="person" {
                h1 { (account.display_name) }
                p class="person-badges" {
                    (super::level_badge(level))
                    (standing(detail, now))
                }
                dl class="person-facts" {
                    dt { "Account" }
                    dd { code { (account.id) } }
                    @if let Some(joined) = detail.joined {
                        dt { "Joined" }
                        dd { (super::when(joined)) }
                    }
                }
            }

            (controls(admin, detail, now))
            (bans(admin, detail, now))
            (links(detail))
            (grants(detail, now))
            (log(detail))
        }
    }
}

fn standing(detail: &Detail, now: DateTime<Utc>) -> Markup {
    html! {
        @match &detail.sanction {
            None => span class="badge badge-clear" { "Clear" },
            Some(sanction) => {
                span class="badge badge-banned" {
                    "Banned"
                    @if sanction.count > 1 { " ×" (sanction.count) }
                }
                span class="standing-until" {
                    @match sanction.until {
                        None => "permanent",
                        Some(until) => { (super::how_long(until, now)) " left" },
                    }
                }
            }
        }
    }
}

/// The two acts: setting a level, and issuing a ban.
///
/// What is offered comes from `lc_identity::ability` rather than from a rule written again
/// here. An administrator is never shown a control whose only possible outcome is a refusal —
/// and, more to the point, never shown one the rules would in fact allow but this page forgot
/// about.
fn controls(admin: &Admin, detail: &Detail, now: DateTime<Utc>) -> Markup {
    let account = &detail.account;
    let level = account.level();
    let offerable = ability::levels_offerable(admin.level, admin.id, level, account.id);
    let bannable = ability::may_ban(admin.level, level);

    html! {
        div class="controls" {
            section class="control" {
                h2 { "Level" }
                @if offerable.is_empty() {
                    p class="nothing" {
                        // The reason, from the rules themselves, so it is right by
                        // construction rather than by a matching sentence written here.
                        (ability::may_set_level(
                            admin.level, admin.id, level, account.id,
                            if level == Level::PLAYER { Level::DEBUG } else { Level::PLAYER },
                        ).err().map_or("No change is available.", |denied| denied.said()))
                    }
                } @else {
                    form class="act" method="post" action=(crate::routes::user_level_url(account.id))
                        hx-post=(crate::routes::user_level_url(account.id)) {
                        label for="level" { "Set to" }
                        select id="level" name="level" {
                            @for offered in &offerable {
                                option value=(offered.slug()) { (offered.name()) }
                            }
                        }
                        button type="submit" { "Apply" }
                    }
                }
            }

            section class="control" {
                h2 { "Ban" }
                @match bannable {
                    Err(denied) => p class="nothing" { (denied.said()) },
                    Ok(()) => form class="act act-ban" method="post"
                        action=(crate::routes::user_ban_url(account.id))
                        hx-post=(crate::routes::user_ban_url(account.id))
                        // Irreversible enough to be worth asking about. `ts/confirm.ts`
                        // answers this with a dialog; with scripting off htmx falls back to
                        // the browser's own confirm, and with htmx off the form just submits.
                        hx-confirm={ "Ban " (account.display_name) "?" }
                    {
                        div class="field" {
                            label for="reason" { "Reason" }
                            select id="reason" name="reason" {
                                @for reason in Reason::ALL {
                                    option value=(reason.slug()) { (reason.said()) }
                                }
                            }
                        }
                        div class="field" {
                            label for="term" { "For" }
                            select id="term" name="term" {
                                @for term in Term::ALL {
                                    option value=(term.slug()) selected[term == Term::Week] {
                                        (term.label())
                                    }
                                }
                            }
                        }
                        div class="field field-wide" {
                            label for="notes" { "Notes" }
                            textarea id="notes" name="notes" rows="3"
                                placeholder="What was seen, and where. Never shown to the account." {}
                        }
                        button type="submit" class="danger" { "Ban" }
                        @if detail.sanction.is_some() {
                            // Bans are served concurrently, so this adds one rather than
                            // replacing what is there. Said out loud, because "ban" on a page
                            // that already says "banned" reads like an edit.
                            p class="fine-print" {
                                "This account already has "
                                (detail.sanction.as_ref().map_or(0, |s| s.count))
                                " ban(s) in force. A new one runs alongside them."
                            }
                        }
                    },
                }
            }
        }
        // Not a control: a note about the one rule that has no control at all, said where
        // somebody would otherwise go looking for the button.
        @if detail.sanction.is_some() && detail.bans.iter().all(|b| !b.in_force(now)) {
            (super::note("done", "Every ban on this account has run out."))
        }
    }
}

fn bans(admin: &Admin, detail: &Detail, now: DateTime<Utc>) -> Markup {
    html! {
        section class="panel" {
            h2 { "Bans" }
            @if detail.bans.is_empty() {
                p class="nothing" { "This account has never been banned." }
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Reason" }
                            th scope="col" { "Issued" }
                            th scope="col" { "Ends" }
                            th scope="col" { "State" }
                            th scope="col" { "Notes" }
                            th scope="col" { span class="visually-hidden" { "Actions" } }
                        }
                    }
                    tbody {
                        @for ban in &detail.bans { (ban_row(admin, ban, now)) }
                    }
                }
            }
        }
    }
}

fn ban_row(admin: &Admin, ban: &Ban, now: DateTime<Utc>) -> Markup {
    let state = ban.state(now);
    html! {
        tr class={ "ban ban-" (state.slug()) } {
            td { (ban.reason.said()) }
            td {
                (super::when(ban.issued_at))
                @if let Some(name) = &ban.issued_by_name {
                    span class="by" { "by " (name) }
                }
            }
            td {
                @match ban.expires_at {
                    None => span class="forever" { "Never" },
                    Some(until) => {
                        (super::when_exact(until))
                        @if state == State::InForce {
                            span class="left" { (super::how_long(until, now)) " left" }
                        }
                    },
                }
            }
            td {
                span class={ "badge badge-" (state.slug()) } { (state.label()) }
                @if let (State::Lifted, Some(at)) = (state, ban.lifted_at) {
                    span class="by" {
                        (super::when(at))
                        @if let Some(name) = &ban.lifted_by_name { " by " (name) }
                    }
                }
            }
            td class="cell-notes" {
                // Private. The page they are on is behind the `Admin` extractor, which is the
                // only thing keeping them so.
                @if !ban.notes.is_empty() { p { (ban.notes) } }
                @if !ban.lift_notes.is_empty() {
                    p class="lift-notes" { "Lifted: " (ban.lift_notes) }
                }
            }
            td class="cell-act" {
                @if state == State::InForce && ability::may_lift(admin.level).is_ok() {
                    form class="act-inline" method="post"
                        action=(crate::routes::user_lift_url(ban.account_id))
                        hx-post=(crate::routes::user_lift_url(ban.account_id))
                        hx-confirm="Lift this ban?" {
                        input type="hidden" name="ban" value=(ban.id);
                        input type="text" name="notes" placeholder="Why" aria-label="Why it was lifted";
                        button type="submit" { "Lift" }
                    }
                }
            }
        }
    }
}

fn links(detail: &Detail) -> Markup {
    html! {
        section class="panel" {
            h2 { "Ways in" }
            @if detail.links.is_empty() {
                p class="nothing" { "No sign-in is attached to this account." }
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Provider" }
                            th scope="col" { "Address" }
                            th scope="col" { "Subject" }
                            th scope="col" { "Added" }
                        }
                    }
                    tbody {
                        @for link in &detail.links {
                            tr {
                                td { span class="badge badge-provider" { (link.provider) } }
                                td {
                                    @match &link.email {
                                        None => span class="nothing" { "—" },
                                        Some(email) => {
                                            (email)
                                            // Whether the *provider* verified it. An
                                            // unverified address aligns no accounts, which is
                                            // the rule this column exists to make visible.
                                            @if link.email_verified {
                                                span class="badge badge-verified" { "verified" }
                                            } @else {
                                                span class="badge badge-unverified" { "unverified" }
                                            }
                                        },
                                    }
                                }
                                td { code class="subject" { (link.subject) } }
                                td { (super::when(link.created_at)) }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn grants(detail: &Detail, now: DateTime<Utc>) -> Markup {
    html! {
        section class="panel" {
            h2 { "Signed-in machines" }
            @if detail.grants.is_empty() {
                p class="nothing" { "No desktop client holds a grant for this account." }
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Machine" }
                            th scope="col" { "Since" }
                            th scope="col" { "Last used" }
                            th scope="col" { "Expires" }
                        }
                    }
                    tbody {
                        @for grant in &detail.grants {
                            tr class=[(!grant.is_live(now)).then_some("lapsed")] {
                                td { (grant.label) }
                                td { (super::when(grant.created_at)) }
                                td {
                                    @match grant.last_used {
                                        None => span class="nothing" { "never" },
                                        Some(at) => (super::when(at)),
                                    }
                                }
                                td {
                                    (super::when(grant.expires_at))
                                    @if !grant.is_live(now) {
                                        span class="badge badge-lapsed" { "lapsed" }
                                    }
                                }
                            }
                        }
                    }
                }
                p class="fine-print" {
                    // Worth saying on this page, because "revoke their device" is the thing
                    // an administrator will reach for next and it is not here.
                    "A grant is not a way past a ban: every ticket is checked against the \
                     account's bans when it is minted."
                }
            }
        }
    }
}

fn log(detail: &Detail) -> Markup {
    html! {
        section class="panel" {
            h2 { "History" }
            @if detail.log.is_empty() {
                p class="nothing" { "Nothing has been done to this account." }
            } @else {
                ol class="log" {
                    @for entry in &detail.log {
                        li {
                            span class="log-when" { (super::when_exact(entry.at)) }
                            span class="log-what" { (entry.label()) }
                            @if !entry.detail.is_empty() {
                                span class="log-detail" { (entry.detail) }
                            }
                            span class="log-who" {
                                @match &entry.actor_name {
                                    Some(name) => { "by " (name) },
                                    // The account was deleted. The line stays: somebody acted,
                                    // and that they are gone does not unmake it.
                                    None => "by a deleted account",
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn admin_at(level: Level) -> Admin {
        Admin {
            id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            name: "Ada".into(),
            level,
        }
    }

    fn detail_at(level: Level, bans: Vec<Ban>, sanction: Option<Sanction>) -> Detail {
        Detail {
            account: Account {
                id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
                display_name: "Grace".into(),
                permission: level.as_i32(),
            },
            joined: Some(Utc::now()),
            links: Vec::new(),
            grants: Vec::new(),
            bans,
            log: Vec::new(),
            sanction,
        }
    }

    fn a_ban(state: State, now: DateTime<Utc>) -> Ban {
        Ban {
            id: Uuid::new_v4(),
            account_id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            reason: Reason::Cheating,
            notes: "the private case notes".into(),
            issued_by: None,
            issued_by_name: Some("Ada".into()),
            issued_at: now - chrono::Duration::days(1),
            expires_at: match state {
                State::InForce => Some(now + chrono::Duration::days(3)),
                State::Expired => Some(now - chrono::Duration::hours(1)),
                State::Lifted => None,
            },
            lifted_at: (state == State::Lifted).then_some(now),
            lifted_by: None,
            lifted_by_name: Some("Ada".into()),
            lift_notes: if state == State::Lifted {
                "appealed".into()
            } else {
                String::new()
            },
        }
    }

    /// The controls are the rules. Offering a level the act will refuse is a console that
    /// lies; refusing one the rules allow is a console that is in the way.
    #[test]
    fn the_level_control_offers_exactly_what_the_rules_allow() {
        let now = Utc::now();
        let markup = controls(
            &admin_at(Level::ADMIN),
            &detail_at(Level::PLAYER, vec![], None),
            now,
        )
        .into_string();
        assert!(markup.contains(r#"value="admin""#), "{markup}");
        assert!(markup.contains(r#"value="debug""#), "{markup}");
        assert!(
            !markup.contains(r#"value="superadmin""#),
            "an administrator was offered the level above their own:\n{markup}",
        );

        // Their own account offers nothing, and says why rather than showing a dead menu.
        let mine = detail_at(Level::ADMIN, vec![], None);
        let mut me = admin_at(Level::ADMIN);
        me.id = mine.account.id;
        let markup = controls(&me, &mine, now).into_string();
        assert!(!markup.contains("<select id=\"level\""), "{markup}");
        assert!(markup.contains("your own level"), "{markup}");
    }

    /// An administrator cannot be banned, and the page has to say so where the button would
    /// be — otherwise the answer is a refusal after a confirmation dialog.
    #[test]
    fn an_administrator_shows_no_ban_form() {
        let now = Utc::now();
        let markup = controls(
            &admin_at(Level::SUPERADMIN),
            &detail_at(Level::ADMIN, vec![], None),
            now,
        )
        .into_string();
        assert!(!markup.contains(r#"name="reason""#), "{markup}");
        assert!(markup.contains("Remove their level first"), "{markup}");

        // And a player does show one.
        let markup = controls(
            &admin_at(Level::DEBUG),
            &detail_at(Level::PLAYER, vec![], None),
            now,
        )
        .into_string();
        assert!(markup.contains(r#"name="reason""#), "{markup}");
        assert!(markup.contains(r#"name="term""#), "{markup}");
        assert!(markup.contains(r#"name="notes""#), "{markup}");
    }

    /// Concurrent bans are the part of this design that surprises people. Adding a second one
    /// has to read as adding rather than as editing.
    #[test]
    fn banning_an_already_banned_account_says_it_runs_alongside() {
        let now = Utc::now();
        let detail = detail_at(
            Level::PLAYER,
            vec![a_ban(State::InForce, now)],
            Some(Sanction {
                count: 2,
                until: Some(now + chrono::Duration::days(3)),
            }),
        );
        let markup = controls(&admin_at(Level::ADMIN), &detail, now).into_string();
        assert!(markup.contains("runs alongside"), "{markup}");
        assert!(markup.contains("2 ban(s) in force"), "{markup}");
    }

    /// Only a ban in force can be lifted, and lifting is offered on exactly those.
    #[test]
    fn lifting_is_offered_only_where_there_is_something_to_lift() {
        let now = Utc::now();
        let admin = admin_at(Level::DEBUG);
        for (state, offered) in [
            (State::InForce, true),
            (State::Expired, false),
            (State::Lifted, false),
        ] {
            let markup = ban_row(&admin, &a_ban(state, now), now).into_string();
            assert_eq!(
                markup.contains("Lift</button>"),
                offered,
                "{state:?} offered the wrong thing:\n{markup}",
            );
            // The notes are on every row whatever its state: history is the point of keeping
            // a lifted ban.
            assert!(markup.contains("the private case notes"), "{markup}");
        }
    }

    /// Every act goes through one region, so a ban that changes four things changes all four.
    #[test]
    fn the_region_is_one_swap_target() {
        let now = Utc::now();
        let markup = region(
            &admin_at(Level::SUPERADMIN),
            &detail_at(Level::PLAYER, vec![a_ban(State::InForce, now)], None),
            Some(&Notice::refused("no")),
            now,
        )
        .into_string();
        assert_eq!(markup.matches(r#"id="user-detail""#).count(), 1, "{markup}");
        assert!(
            markup.contains(r##"hx-target:inherited="#user-detail""##),
            "{markup}"
        );
        assert!(markup.contains("note-refused"), "{markup}");
        // Every form posts, and every form is also a plain form.
        assert_eq!(
            markup.matches("hx-post=").count(),
            markup.matches(r#"method="post""#).count(),
            "a form is htmx-only:\n{markup}",
        );
    }
}
