//! Who may do what to whom.
//!
//! Every rule as a pure function of two levels — no pool, no request, no session — which is
//! what lets the whole table be asserted exhaustively.
//!
//! - promote **up to your own level**, so a 2 hands out 2 and 3 and never 1;
//! - demote anyone **below** your level, so a 2 may not touch another 2 or a 1;
//! - an administrator cannot be banned. De-admin first.
//!
//! Two consequences, both intended. **Nobody changes their own level**, demotion needing an
//! actor who outranks the subject. And **a superadmin cannot be demoted by anyone**, there
//! being nobody above 1 — so the one irreversible act is a row changed by hand, not a web
//! page.
//!
//! `lc_server::ability` keeps the same idea over its own vocabulary, agreeing by contract
//! rather than by a shared crate; see `lightcone/docs/16-identity.md`.

use uuid::Uuid;

use crate::level::Level;

/// Each is an administrator being told the shape of the rules, so none is a secret.
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

pub fn may_administer(actor: Level) -> bool {
    actor.is_admin()
}

/// The subject is named by id as well as by level so acting on your own account is refused
/// as itself rather than falling out of the demotion rule with a misleading message.
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
        // The ceiling is the actor's own level; `at_least` states it in the rule's direction.
        return actor
            .at_least(proposed)
            .then_some(())
            .ok_or(Denied::AboveYou);
    }
    // Strictly below, so an equal is refused.
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
    // Catches the self case on the way past, and says the useful thing about it.
    if subject.is_admin() {
        return Err(Denied::SubjectIsAnAdmin);
    }
    Ok(())
}

/// Any administrator, including one who did not issue it. Mercy is not ranked; the log
/// records who granted it.
pub fn may_lift(actor: Level) -> Result<(), Denied> {
    actor.is_admin().then_some(()).ok_or(Denied::NotAnAdmin)
}

/// Derived from [`may_set_level`] rather than written beside it, so a rule change cannot
/// leave the interface offering something the act refuses.
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
        assert_eq!(set(Level::ADMIN, Level::PLAYER, Level::DEBUG), Ok(()));
        assert_eq!(set(Level::ADMIN, Level::DEBUG, Level::ADMIN), Ok(()));

        assert_eq!(
            set(Level::ADMIN, Level::PLAYER, Level::SUPERADMIN),
            Err(Denied::AboveYou)
        );
        assert_eq!(
            set(Level::DEBUG, Level::PLAYER, Level::ADMIN),
            Err(Denied::AboveYou)
        );
        // An owner may hand out their own level. There is no higher one to be refused.
        assert_eq!(
            set(Level::SUPERADMIN, Level::PLAYER, Level::SUPERADMIN),
            Ok(())
        );
    }

    #[test]
    fn an_administrator_demotes_only_those_below_them() {
        assert_eq!(set(Level::SUPERADMIN, Level::ADMIN, Level::PLAYER), Ok(()));
        assert_eq!(set(Level::ADMIN, Level::DEBUG, Level::PLAYER), Ok(()));

        assert_eq!(
            set(Level::ADMIN, Level::ADMIN, Level::DEBUG),
            Err(Denied::NotBelowYou),
            "an equal was demoted"
        );
        assert_eq!(
            set(Level::DEBUG, Level::ADMIN, Level::PLAYER),
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
            for proposed in [Level::PLAYER, Level::DEBUG, Level::ADMIN] {
                assert_eq!(
                    set(actor, Level::SUPERADMIN, proposed),
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
            vec![Level::ADMIN, Level::DEBUG],
            "an administrator was offered a level they cannot grant",
        );
        assert_eq!(
            levels_offerable(Level::DEBUG, a, Level::PLAYER, s),
            vec![Level::DEBUG],
        );
        assert_eq!(
            levels_offerable(Level::ADMIN, a, Level::DEBUG, s),
            vec![Level::PLAYER, Level::ADMIN],
        );
        // Their own account offers nothing at all, rather than offering and then refusing.
        assert!(levels_offerable(Level::SUPERADMIN, a, Level::SUPERADMIN, a).is_empty());
        assert!(levels_offerable(Level::PLAYER, a, Level::PLAYER, s).is_empty());
    }
}
