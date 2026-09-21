//! Who may do what, on this shard.
//!
//! One table in one function rather than a condition at each call site: a rule spread over six
//! of them is one nobody can state, and so one nobody can check.
//!
//! [`Level`] mirrors `lc_identity::level::Level` and is duplicated deliberately — this crate
//! must not depend on the broker, so the two agree by `lightcone/docs/16-identity.md` as the
//! ticket claims do. **Lower is higher**: 0 is a player, 1 is the most senior. `a > b` reads
//! as "outranks" and is backwards, which is why [`Level::outranks`] is the only ordering.

use lc_proto::Order;

/// What a ticket's `perm` claim means.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Level(i32);

impl Level {
    pub const PLAYER: Level = Level(0);
    pub const SUPERADMIN: Level = Level(1);
    pub const ADMIN: Level = Level(2);
    pub const DEBUG: Level = Level(3);

    pub const ALL: [Level; 4] = [Level::PLAYER, Level::SUPERADMIN, Level::ADMIN, Level::DEBUG];

    /// What a ticket carries. Anything outside the range is read as a player.
    ///
    /// A ticket from a broker that has grown a fourth administrative level grants nothing here
    /// rather than granting whatever `>=` happens to say about an integer this build has never
    /// heard of.
    pub fn from_claim(raw: i32) -> Level {
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

    /// Whether `self` is strictly more senior than `other`. A player outranks nobody.
    pub fn outranks(self, other: Level) -> bool {
        self.is_admin() && (!other.is_admin() || self.0 < other.0)
    }

    /// Whether `self` is at least as senior as `other`.
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
}

/// Coarser than [`lc_proto::Order`]: a burn, a course and a refit all need the same authority,
/// and four identical rows would be four rows that can drift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Fly a craft: burn, set a course, cross, intercept, cut the drive, refit.
    Command,
    Speak,
    /// Out of nothing.
    GrantEnergy,
    Stage,
}

impl Act {
    pub const ALL: [Act; 4] = [Act::Command, Act::Speak, Act::GrantEnergy, Act::Stage];

    /// **No `_` arm, on purpose**: a new [`Order`] is then a compile error rather than an
    /// order falling through to whatever the catch-all said, which is how an ungated action
    /// ships.
    pub fn of(order: &Order) -> Act {
        match order {
            Order::Transmit { .. }
            | Order::Burn { .. }
            | Order::SetCourse { .. }
            | Order::Cross { .. }
            | Order::CutDrive
            | Order::Intercept { .. }
            | Order::BreakOff
            | Order::Refit { .. }
            | Order::CancelRefit
            // What the telescope does and what the crew call things are the ship's business.
            | Order::SetDuty { .. }
            | Order::NameIt { .. } => Act::Command,
            // Separate from flying so a shard can silence somebody without grounding them.
            Order::Say { .. } | Order::OfferKey { .. } | Order::SendReport { .. } => Act::Speak,
        }
    }
}

/// What the asker is to the craft in question, if there is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    Flies,
    Otherwise,
}

#[derive(Clone, Copy, Debug)]
pub struct Asking {
    pub level: Level,
    pub standing: Standing,
}

/// A development shard, set by the process that starts it and never by a client. On one, the
/// whole population is whoever ran `cargo run`, so development actions are open to all of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Directing(pub bool);

/// **The table.**
///
/// **Commanding is ownership at every level**, deliberately. A craft's acts become events
/// attributed to that craft and nothing downstream carries who caused them, so an
/// administrator flying a player's ship would put a burn in the record under the player's
/// name, unfalsifiably. The remedy for a player who should not be flying is a ban.
///
/// **Every administrative level may develop**, `DEBUG` included — it is named for these two
/// acts. So the most junior level can conjure energy on a live shard, contained only by
/// `GrantEnergy` reaching the asker's own craft and by the promotion leaving a row in
/// `admin_actions`.
pub fn allows(act: Act, who: Asking, directing: Directing) -> bool {
    match act {
        Act::Command | Act::Speak => who.standing == Standing::Flies,
        // A directing shard is a development one and open to whoever reached it; elsewhere
        // this is the level, from a ticket the broker signed.
        Act::GrantEnergy | Act::Stage => directing.0 || who.level.is_admin(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking(level: Level, standing: Standing) -> Asking {
        Asking { level, standing }
    }

    /// The whole table. A rule this small is cheaper to assert exhaustively than to argue
    /// about, and the assertion is what a reader can trust when the prose above is out of date.
    #[test]
    fn every_combination_answers_as_stated() {
        for act in Act::ALL {
            for level in Level::ALL {
                for standing in [Standing::Flies, Standing::Otherwise] {
                    for directing in [Directing(true), Directing(false)] {
                        let got = allows(act, asking(level, standing), directing);
                        let want = match act {
                            Act::Command | Act::Speak => standing == Standing::Flies,
                            Act::GrantEnergy | Act::Stage => directing.0 || level.is_admin(),
                        };
                        assert_eq!(
                            got, want,
                            "{act:?} by {} ({standing:?}, directing {})",
                            level.name(),
                            directing.0,
                        );
                    }
                }
            }
        }
    }

    /// The rule the doc comment spends a paragraph on. If this ever passes, an administrator
    /// can put a burn in the record under a player's name.
    #[test]
    fn no_level_flies_somebody_elses_ship() {
        for level in Level::ALL {
            for act in [Act::Command, Act::Speak] {
                for directing in [Directing(true), Directing(false)] {
                    assert!(
                        !allows(act, asking(level, Standing::Otherwise), directing),
                        "{} performed {act:?} on a craft they do not fly",
                        level.name(),
                    );
                }
                // And the owner of the craft may, whoever they are.
                assert!(allows(act, asking(level, Standing::Flies), Directing(false)));
            }
        }
    }

    /// Every administrative level may develop, and no player may. `DEBUG` is named for these
    /// two acts, so a gate above it would be a tier that cannot do what it is called after.
    #[test]
    fn development_actions_are_open_to_every_administrative_level() {
        for act in [Act::GrantEnergy, Act::Stage] {
            for level in [Level::SUPERADMIN, Level::ADMIN, Level::DEBUG] {
                assert!(
                    allows(act, asking(level, Standing::Flies), Directing(false)),
                    "{} could not {act:?}",
                    level.name(),
                );
            }
            assert!(
                !allows(act, asking(Level::PLAYER, Standing::Flies), Directing(false)),
                "a player granted themselves {act:?}",
            );
            // Except on a shard that stages, where the population is whoever ran it.
            assert!(allows(act, asking(Level::PLAYER, Standing::Flies), Directing(true)));
        }
    }

    /// The same inversion the broker's copy has, asserted here so the two cannot drift apart
    /// silently. `lc_identity::level::tests` is the other half of this.
    #[test]
    fn seniority_runs_opposite_to_the_integer() {
        assert_eq!(Level::SUPERADMIN.as_i32(), 1);
        assert_eq!(Level::ADMIN.as_i32(), 2);
        assert_eq!(Level::DEBUG.as_i32(), 3);
        assert!(Level::SUPERADMIN.outranks(Level::ADMIN));
        assert!(Level::ADMIN.outranks(Level::DEBUG));
        assert!(!Level::DEBUG.outranks(Level::ADMIN));
        for level in Level::ALL {
            assert!(!level.outranks(level));
            assert!(level.at_least(level));
        }
        assert!(!Level::PLAYER.is_admin());
        assert!(!Level::PLAYER.at_least(Level::DEBUG));
    }

    /// Every order needs an authority, and the two kinds are told apart. This is here so the
    /// exhaustive match in `Act::of` is exercised rather than merely written.
    #[test]
    fn every_order_is_gated_as_one_kind_or_the_other() {
        use lc_proto::{Aim, Closeness, Loadout, MessageKey, Secrecy, ShipId};

        let commands = [
            Order::Transmit { power_w: 1.0 },
            Order::Burn { beta: [0.1, 0.0, 0.0] },
            Order::CutDrive,
            Order::Intercept { ship_id: ShipId(1), closeness: Closeness::Company },
            Order::BreakOff,
            Order::Refit { target: Loadout::default() },
            Order::CancelRefit,
        ];
        for order in &commands {
            assert_eq!(Act::of(order), Act::Command, "{order:?}");
        }

        let speech = [
            Order::Say {
                to: None,
                aim: Aim::Omni,
                secrecy: Secrecy::Open,
                body: "hello".into(),
                idem: MessageKey::default(),
            },
            Order::OfferKey { to: None, aim: Aim::Omni },
        ];
        for order in &speech {
            assert_eq!(Act::of(order), Act::Speak, "{order:?}");
        }

        // And neither is a development action, whatever the level: an order is something a
        // craft does, and a development action is something done to the sky.
        for order in commands.iter().chain(speech.iter()) {
            let act = Act::of(order);
            assert!(act == Act::Command || act == Act::Speak);
            assert!(
                !allows(act, asking(Level::SUPERADMIN, Standing::Otherwise), Directing(true)),
                "{order:?} was allowed on somebody else's craft",
            );
        }
    }

    /// A claim from a broker this build does not understand grants nothing.
    #[test]
    fn an_unknown_claim_is_a_player() {
        for raw in [-1, 0, 4, 99, i32::MIN, i32::MAX] {
            assert_eq!(Level::from_claim(raw), Level::PLAYER, "{raw} was admitted");
        }
        for level in Level::ALL {
            assert_eq!(Level::from_claim(level.as_i32()), level);
        }
        // A ticket minted before the broker carried a level at all.
        assert_eq!(Level::from_claim(i32::default()), Level::PLAYER);
    }
}
