//! Who may do what, on this shard.
//!
//! Every gate on a client's action is one table, read from one function, rather than a
//! condition written at each call site. The reason is the one the identity broker's
//! `lc_identity::ability` gives for its half: a rule spread over six call sites is a rule
//! nobody can state, and an authorisation rule nobody can state is one nobody can check.
//!
//! An answer is a function of three things and nothing else — what is being asked, what the
//! asker stands in relation to the subject, and what level the ticket says they hold — so the
//! whole of it is asserted exhaustively below.
//!
//! ## Levels are a contract, not a shared type
//!
//! [`Level`] mirrors `lc_identity::level::Level`: the same integers, the same inversion, the
//! same names. It is duplicated deliberately. This crate must not depend on the broker — the
//! two are separate cargo workspaces and separate deployments, and `lightcone/docs/16-identity.md`
//! says why the ticket's claim set is a document rather than a crate. This is the same
//! agreement about the same column, and it is written down in the same place.
//!
//! **Lower is higher.** 0 is a player; 1, 2 and 3 administer, with 1 the most senior.
//! [`Level::outranks`] is the only ordering, for the reason the broker's copy gives: `a > b`
//! reads as "outranks" and is exactly backwards.

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

/// What a connection is trying to do.
///
/// Coarser than [`lc_proto::Order`] on purpose. What the gate cares about is the *kind* of
/// authority an act needs, and a burn, a course and a refit all need the same one: the right
/// to fly a particular craft. Splitting them would be four identical rows that can drift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Fly a craft: burn, set a course, cross, intercept, cut the drive, refit.
    Command,
    /// Transmit as a craft: speak, offer a key.
    Speak,
    /// Put energy into a craft out of nothing.
    GrantEnergy,
    /// Replace what is in the sky with a staged scene.
    Stage,
}

impl Act {
    pub const ALL: [Act; 4] = [Act::Command, Act::Speak, Act::GrantEnergy, Act::Stage];

    /// What authority an order needs.
    ///
    /// **Exhaustive on purpose, with no `_` arm.** A new [`Order`] variant is then a compile
    /// error here rather than an order that quietly falls through to whatever the catch-all
    /// said — which is how an ungated action ships.
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
            | Order::CancelRefit => Act::Command,
            // Speaking is separated from flying because the two will not always want the same
            // answer: a shard that silences somebody without grounding them is a thing an
            // administration wants, and a table with one row for both cannot express it.
            Order::Say { .. } | Order::OfferKey { .. } => Act::Speak,
        }
    }
}

/// What the asker is to the craft they are asking about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// This connection flies that craft.
    Flies,
    /// It does not, or there is no craft in question.
    Otherwise,
}

/// Who is asking.
#[derive(Clone, Copy, Debug)]
pub struct Asking {
    pub level: Level,
    pub standing: Standing,
}

/// Whether this shard is one that stages scenes.
///
/// A development and demonstration shard, set by the process that starts it rather than by
/// anything a client sends. On one of those, development actions are open to everybody,
/// because the whole population is whoever ran `cargo run`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Directing(pub bool);

/// **The table.** Whether `who` may `act`, on a shard that is or is not directing.
///
/// ## Commanding a craft is ownership, at every level
///
/// An administrator may not fly somebody else's ship, and that is a rule rather than a gap.
/// Every act a craft performs becomes an **event** — a point in spacetime attributed to that
/// craft, released to whoever the light reaches — and nothing downstream carries who at a
/// keyboard caused it. An administrator flying a player's ship would put a burn in the record
/// that the record says the player made, and there is no way for anyone, including the
/// administration, to tell afterwards that it was not. A moderation tool that falsifies the
/// evidence is not a moderation tool.
///
/// The remedy for a player who should not be flying is at the broker: they are banned, and the
/// ticket that would have let them connect is refused. That path leaves a row, a reason and a
/// name.
///
/// ## Development actions need a level, and every administrative level has it
///
/// Granting energy and staging a scene are open to [`Level::SUPERADMIN`], [`Level::ADMIN`] and
/// [`Level::DEBUG`] alike — which is to say to anyone who is not a player. `DEBUG` is the
/// junior tier and is named for exactly these two acts, so gating them above it would be a
/// tier that cannot do the thing it is called after.
///
/// What that costs is worth saying plainly: **the most junior administrative level can conjure
/// energy on a live shard.** The containment is that it cannot do so to somebody else's ship —
/// `GrantEnergy` reaches only the asker's own craft — and that promoting anyone to `DEBUG` is
/// an act with a name against it in `admin_actions`.
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
