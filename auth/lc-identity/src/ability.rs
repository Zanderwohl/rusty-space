//! Who may do what to whom.
//!
//! Every rule about one account acting on another is here, as a pure function of two levels
//! and nothing else — no pool, no request, no session. That is what lets the whole table below
//! be asserted exhaustively in a unit test, which is the only way anyone can be sure that
//! `2` may not quietly promote `2` to `1`.
//!
//! The rules themselves, in the words they were asked for:
//!
//! - an administrator may promote anyone **up to their own level**, so a level 2 may hand out
//!   2 and 3 and never 1;
//! - an administrator may demote anyone **below** their level, so a level 2 may demote a 3 and
//!   may not touch another 2 or a 1;
//! - an administrator may not be banned. De-admin first, which is a separate act by somebody
//!   who outranks them and leaves a line in the log.
//!
//! Two consequences fall out of those, and both are intended:
//!
//! - **Nobody may change their own level**, because demotion needs an actor who outranks the
//!   subject and nobody outranks themselves. An administrator who wants to step down asks
//!   somebody senior, and a stray click cannot strand a shard with no owner.
//! - **An owner may not be demoted by anyone**, for the same reason: there is nobody above 1.
//!   Removing an owner is a row change made by hand against the database, deliberately, so
//!   that the one irreversible administrative act is not reachable from a web page.
//!
//! The game server keeps the same idea over its own vocabulary — see `lc_server::ability` —
//! and the two agree by contract rather than by sharing a crate, on exactly the reasoning
//! `lightcone/docs/16-identity.md` gives for the ticket claims.

use uuid::Uuid;

use crate::level::Level;

/// Why an act was not allowed.
///
/// A refusal a person reads, not a status code. Every one of these is an administrator being
/// told the shape of the rules, so none of them is a secret.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Denied {
    /// The actor administers nothing.
    NotAnAdmin,
    /// A promotion to a level the actor does not hold.
    AboveYou,
    /// A demotion of somebody the actor does not outrank — an equal, or a senior.
    NotBelowYou,
    /// An act on the actor's own account.
    Yourself,
    /// A ban on an administrator.
    SubjectIsAnAdmin,
    /// The level asked for is the one already held.
    NoChange,
}

impl Denied {
    pub const ALL: [Denied; 6] = [
        Denied::NotAnAdmin,
        Denied::AboveYou,
        Denied::NotBelowYou,
        Denied::Yourself,
        Denied::SubjectIsAnAdmin,
        Denied::NoChange,
    ];

    /// A name a URL can carry, so a refusal survives a redirect.
    pub fn slug(self) -> &'static str {
        match self {
            Denied::NotAnAdmin => "not-an-admin",
            Denied::AboveYou => "above-you",
            Denied::NotBelowYou => "not-below-you",
            Denied::Yourself => "yourself",
            Denied::SubjectIsAnAdmin => "subject-is-an-admin",
            Denied::NoChange => "no-change",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Denied> {
        Denied::ALL.into_iter().find(|d| d.slug() == slug)
    }

    /// The sentence shown in the interface.
    pub fn said(self) -> &'static str {
        match self {
            Denied::NotAnAdmin => "You do not administer anything.",
            Denied::AboveYou => "You cannot grant a level above your own.",
            Denied::NotBelowYou => "You can only demote administrators below your own level.",
            Denied::Yourself => "You cannot change your own level.",
            Denied::SubjectIsAnAdmin => {
                "An administrator cannot be banned. Remove their level first."
            }
            Denied::NoChange => "That is the level they already hold.",
        }
    }
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.said())
    }
}

impl std::error::Error for Denied {}

/// Whether the administration pages are open to this level at all.
pub fn may_administer(actor: Level) -> bool {
    actor.is_admin()
}

/// Whether `actor` may move `subject` from the level they hold to `proposed`.
///
/// The subject is named by id as well as by level so that acting on your own account is
/// refused as itself rather than falling out of the demotion rule with a misleading message.
pub fn may_set_level(
    actor: Level,
    actor_id: Uuid,
    subject: Level,
    subject_id: Uuid,
    proposed: Level,
) -> Result<(), Denied> {
    if !actor.is_admin() {
        return Err(Denied::NotAnAdmin);
    }
    if actor_id == subject_id {
        return Err(Denied::Yourself);
    }
    if proposed == subject {
        return Err(Denied::NoChange);
    }
    if proposed.outranks(subject) {
        // A promotion. The ceiling is the actor's own level, which `at_least` states in the
        // direction the rule is written in: "up to their own level" includes their own level.
        return actor
            .at_least(proposed)
            .then_some(())
            .ok_or(Denied::AboveYou);
    }
    // A demotion, including down to player. Strictly below, so an equal is refused.
    actor
        .outranks(subject)
        .then_some(())
        .ok_or(Denied::NotBelowYou)
}

/// Whether `actor` may ban an account at `subject`.
pub fn may_ban(actor: Level, subject: Level) -> Result<(), Denied> {
    if !actor.is_admin() {
        return Err(Denied::NotAnAdmin);
    }
    // No id comparison: an administrator's own level is administrative, so this catches the
    // self case on the way past and says the useful thing about it.
    if subject.is_admin() {
        return Err(Denied::SubjectIsAnAdmin);
    }
    Ok(())
}

/// Whether `actor` may lift a ban.
///
/// Any administrator may lift any ban, including one they did not issue and one issued by
/// somebody senior. Mercy is not ranked; the log records who granted it.
pub fn may_lift(actor: Level) -> Result<(), Denied> {
    actor.is_admin().then_some(()).ok_or(Denied::NotAnAdmin)
}

/// Every level `actor` may hand to `subject`, most senior first.
///
/// What a level picker offers. Derived from [`may_set_level`] rather than written beside it,
/// so a rule change cannot leave the interface offering something the act will refuse.
pub fn levels_offerable(
    actor: Level,
    actor_id: Uuid,
    subject: Level,
    subject_id: Uuid,
) -> Vec<Level> {
    Level::ALL
        .into_iter()
        .filter(|proposed| may_set_level(actor, actor_id, subject, subject_id, *proposed).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn them() -> (Uuid, Uuid) {
        (
            Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
        )
    }

    fn set(actor: Level, subject: Level, proposed: Level) -> Result<(), Denied> {
        let (a, s) = them();
        may_set_level(actor, a, subject, s, proposed)
    }

    /// The whole table, written out. Anything that changes the rules changes this, which is
    /// the point: a rule this small is cheaper to assert exhaustively than to argue about.
    #[test]
    fn every_pair_of_levels_agrees_with_the_stated_rules() {
        use Level as L;
        for actor in L::ALL {
            for subject in L::ALL {
                for proposed in L::ALL {
                    let got = set(actor, subject, proposed);
                    let want = if !actor.is_admin() {
                        Err(Denied::NotAnAdmin)
                    } else if proposed == subject {
                        Err(Denied::NoChange)
                    } else if proposed.outranks(subject) {
                        // Promotion: capped at the actor's own level.
                        if actor.at_least(proposed) {
                            Ok(())
                        } else {
                            Err(Denied::AboveYou)
                        }
                    } else if actor.outranks(subject) {
                        Ok(())
                    } else {
                        Err(Denied::NotBelowYou)
                    };
                    assert_eq!(got, want, "{actor} acting on {subject} -> {proposed}");
                }
            }
        }
    }

    /// The headline rule, in the words it was asked for.
    #[test]
    fn an_administrator_promotes_up_to_their_own_level_and_no_further() {
        assert_eq!(set(Level::ADMIN, Level::PLAYER, Level::ADMIN), Ok(()));
        assert_eq!(set(Level::ADMIN, Level::PLAYER, Level::MODERATOR), Ok(()));
        assert_eq!(set(Level::ADMIN, Level::MODERATOR, Level::ADMIN), Ok(()));

        assert_eq!(
            set(Level::ADMIN, Level::PLAYER, Level::OWNER),
            Err(Denied::AboveYou)
        );
        assert_eq!(
            set(Level::MODERATOR, Level::PLAYER, Level::ADMIN),
            Err(Denied::AboveYou)
        );
        // An owner may hand out their own level. There is no higher one to be refused.
        assert_eq!(set(Level::OWNER, Level::PLAYER, Level::OWNER), Ok(()));
    }

    #[test]
    fn an_administrator_demotes_only_those_below_them() {
        assert_eq!(set(Level::OWNER, Level::ADMIN, Level::PLAYER), Ok(()));
        assert_eq!(set(Level::ADMIN, Level::MODERATOR, Level::PLAYER), Ok(()));

        assert_eq!(
            set(Level::ADMIN, Level::ADMIN, Level::MODERATOR),
            Err(Denied::NotBelowYou),
            "an equal was demoted"
        );
        assert_eq!(
            set(Level::MODERATOR, Level::ADMIN, Level::PLAYER),
            Err(Denied::NotBelowYou),
        );
    }

    /// Both consequences the module doc promises, asserted rather than described.
    #[test]
    fn nobody_demotes_themselves_and_nobody_demotes_an_owner() {
        let (a, _) = them();
        for level in Level::ADMINISTRATIVE {
            for proposed in Level::ALL {
                if proposed == level {
                    continue;
                }
                assert_eq!(
                    may_set_level(level, a, level, a, proposed),
                    Err(Denied::Yourself),
                    "{level} changed their own level to {proposed}",
                );
            }
        }
        for actor in Level::ADMINISTRATIVE {
            for proposed in [Level::PLAYER, Level::MODERATOR, Level::ADMIN] {
                assert_eq!(
                    set(actor, Level::OWNER, proposed),
                    Err(Denied::NotBelowYou),
                    "{actor} demoted an owner",
                );
            }
        }
    }

    #[test]
    fn every_refusal_has_its_own_name_and_its_own_sentence() {
        let mut slugs: Vec<&str> = Denied::ALL.iter().map(|d| d.slug()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), Denied::ALL.len(), "two refusals share a name");

        let mut said: Vec<&str> = Denied::ALL.iter().map(|d| d.said()).collect();
        said.sort_unstable();
        said.dedup();
        assert_eq!(said.len(), Denied::ALL.len(), "two refusals read alike");

        for denied in Denied::ALL {
            assert_eq!(Denied::from_slug(denied.slug()), Some(denied));
        }
        assert_eq!(Denied::from_slug("nope"), None);
    }

    #[test]
    fn an_administrator_cannot_be_banned_at_any_level() {
        for actor in Level::ADMINISTRATIVE {
            assert_eq!(may_ban(actor, Level::PLAYER), Ok(()));
            assert_eq!(may_lift(actor), Ok(()));
            for subject in Level::ADMINISTRATIVE {
                assert_eq!(
                    may_ban(actor, subject),
                    Err(Denied::SubjectIsAnAdmin),
                    "{actor} banned {subject}",
                );
            }
        }
        assert_eq!(
            may_ban(Level::PLAYER, Level::PLAYER),
            Err(Denied::NotAnAdmin)
        );
        assert_eq!(may_lift(Level::PLAYER), Err(Denied::NotAnAdmin));
        assert!(!may_administer(Level::PLAYER));
    }

    /// The picker offers what the act accepts, and nothing else.
    #[test]
    fn the_offered_levels_are_exactly_the_permitted_ones() {
        let (a, s) = them();
        assert_eq!(
            levels_offerable(Level::ADMIN, a, Level::PLAYER, s),
            vec![Level::ADMIN, Level::MODERATOR],
            "an administrator was offered a level they cannot grant",
        );
        assert_eq!(
            levels_offerable(Level::MODERATOR, a, Level::PLAYER, s),
            vec![Level::MODERATOR],
        );
        assert_eq!(
            levels_offerable(Level::ADMIN, a, Level::MODERATOR, s),
            vec![Level::PLAYER, Level::ADMIN],
        );
        // Their own account offers nothing at all, rather than offering and then refusing.
        assert!(levels_offerable(Level::OWNER, a, Level::OWNER, a).is_empty());
        assert!(levels_offerable(Level::PLAYER, a, Level::PLAYER, s).is_empty());
    }
}
