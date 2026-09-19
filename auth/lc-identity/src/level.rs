//! Where an account stands.
//!
//! **Lower is higher**: 0 is a player, 1 the most senior of 1..3. The database stores the
//! integer; nothing above it compares two with `<`, because the comparison that reads
//! correctly is the one that is wrong. [`Level::outranks`] is the only ordering offered.
//!
//! Deliberately no `Ord`. A derived one would sort administrators most-junior-first while
//! reading as the opposite, and make `a > b` a compiling, plausible, wrong check.

use std::fmt;

/// An account's permission level.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Level(i32);

impl Level {
    /// Everybody. Plays the game and administers nothing.
    pub const PLAYER: Level = Level(0);
    /// May not be demoted by anyone, including another superadmin — see [`crate::ability`].
    pub const SUPERADMIN: Level = Level(1);
    pub const ADMIN: Level = Level(2);
    /// Administers accounts, and is the tier `lc_server::ability`'s development actions are
    /// named for.
    pub const DEBUG: Level = Level(3);

    /// Most senior first: the order a picker offers.
    pub const ADMINISTRATIVE: [Level; 3] = [Level::SUPERADMIN, Level::ADMIN, Level::DEBUG];
    pub const ALL: [Level; 4] = [Level::PLAYER, Level::SUPERADMIN, Level::ADMIN, Level::DEBUG];

    /// Anything outside the range reads as a player. A check constraint makes that unreachable
    /// through this application, so this is about the row edited by hand at three in the morning.
    pub fn from_stored(raw: i32) -> Level {
        match raw {
            1..=3 => Level(raw),
            _ => Level::PLAYER,
        }
    }

    pub fn as_i32(self) -> i32 {
        self.0
    }

    pub fn is_admin(self) -> bool {
        self.0 != 0
    }

    /// Strictly. A player outranks nobody, including another player.
    pub fn outranks(self, other: Level) -> bool {
        self.is_admin() && (!other.is_admin() || self.0 < other.0)
    }

    /// For two administrators, the test "may I hand out this level".
    pub fn at_least(self, other: Level) -> bool {
        self == other || self.outranks(other)
    }

    pub fn name(self) -> &'static str {
        match self.0 {
            1 => "Superadmin",
            2 => "Admin",
            3 => "Debug",
            _ => "Player",
        }
    }

    /// Stable across a rename of [`Level::name`], which is why they are two functions.
    pub fn slug(self) -> &'static str {
        match self.0 {
            1 => "superadmin",
            2 => "admin",
            3 => "debug",
            _ => "player",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.slug() == slug)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seniority_runs_opposite_to_the_integer() {
        assert!(Level::SUPERADMIN.outranks(Level::ADMIN));
        assert!(Level::ADMIN.outranks(Level::DEBUG));
        assert!(Level::DEBUG.outranks(Level::PLAYER));
        assert!(Level::SUPERADMIN.outranks(Level::PLAYER));

        assert!(!Level::ADMIN.outranks(Level::SUPERADMIN));
        assert!(!Level::PLAYER.outranks(Level::DEBUG));
        // And the integers really do run the other way, which is the whole reason this type
        // exists. If this ever fails the constants were renumbered and every call site that
        // reads `outranks` now means something else.
        assert!(Level::SUPERADMIN.as_i32() < Level::ADMIN.as_i32());
    }

    /// Nobody outranks themselves, at any level. The promote and demote rules both lean on
    /// this: it is what stops an administrator acting on their own account by a path that
    /// looks like acting on somebody else's.
    #[test]
    fn nobody_outranks_an_equal() {
        for level in Level::ALL {
            assert!(!level.outranks(level), "{level} outranked itself");
            assert!(level.at_least(level));
        }
    }

    /// A player is not a junior administrator. `0` is outside the ladder rather than at the
    /// bottom of it, and `outranks` has to say so in both directions.
    #[test]
    fn a_player_is_not_on_the_ladder() {
        assert!(!Level::PLAYER.is_admin());
        assert!(!Level::PLAYER.outranks(Level::PLAYER));
        assert!(!Level::PLAYER.at_least(Level::DEBUG));
        for admin in Level::ADMINISTRATIVE {
            assert!(admin.is_admin());
            assert!(admin.outranks(Level::PLAYER));
            assert!(!Level::PLAYER.outranks(admin));
        }
    }

    #[test]
    fn an_unrecognised_level_grants_nothing() {
        for raw in [-1, 4, 99, i32::MIN, i32::MAX] {
            assert_eq!(Level::from_stored(raw), Level::PLAYER, "{raw} was admitted");
        }
        for level in Level::ALL {
            assert_eq!(Level::from_stored(level.as_i32()), level);
        }
    }

    #[test]
    fn slugs_round_trip_and_are_distinct() {
        for level in Level::ALL {
            assert_eq!(Level::from_slug(level.slug()), Some(level));
        }
        assert_eq!(Level::from_slug("root"), None);
        assert_eq!(Level::from_slug(""), None);
        let mut slugs: Vec<&str> = Level::ALL.iter().map(|l| l.slug()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), Level::ALL.len(), "two levels share a slug");
    }
}
