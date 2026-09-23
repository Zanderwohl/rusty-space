//! The console: lines of text a player types, read, checked and run by the shard.
//!
//! A client sends the line as typed and nothing else. Parsing, the level check and every
//! range are here, so no client can send a command no parser would have produced. A line is
//! queued when it arrives and run at one point in the tick, after the intents, and its answer
//! goes to the connection that sent it. See `lightcone/docs/27-console.md`.

mod fitting;
mod parse;
mod spec;
mod teleport;

use std::collections::VecDeque;
use std::fmt::Write as _;

use lc_proto::{COMMAND_LIMIT, ClientId, Outbound};
use lc_world::craft::CraftId;

pub use parse::{Arg, ParseError, Parsed, parse};
pub use spec::{ArgSpec, BindError, Bound, Kind, Limit, Need, Spec, Value, Verb, bind};

use crate::ability::Level;
use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Every command there is, and who may run it.
pub const COMMANDS: &[Spec] = &[
    Spec {
        name: "help",
        verb: Verb::Help,
        level: Level::PLAYER,
        summary: "what the commands are, or what one of them takes",
        args: &[ArgSpec {
            name: "command",
            // Text and not a word from the table: the list of choices in a refusal would name
            // commands the asker may not see.
            kind: Kind::Text,
            need: Need::Optional,
            level: Level::PLAYER,
            help: "a command name",
        }],
    },
    Spec {
        name: "teleport",
        verb: Verb::Teleport,
        level: Level::DEBUG,
        summary: "put a ship on station about a star or a body, without flying there",
        args: &[
            ArgSpec {
                name: "target",
                kind: Kind::Id,
                need: Need::Required,
                level: Level::DEBUG,
                help: "a star's catalog id or a body's id",
            },
            ArgSpec {
                name: "altitude",
                kind: Kind::Number(&[
                    Limit { level: Level::DEBUG, min: 0.1, max: 100.0 },
                    Limit { level: Level::SUPERADMIN, min: 0.01, max: 10_000.0 },
                ]),
                need: Need::Default("2"),
                level: Level::DEBUG,
                help: "radii above the surface",
            },
            ArgSpec {
                name: "star",
                kind: Kind::Id,
                need: Need::Optional,
                level: Level::DEBUG,
                help: "the star the body belongs to",
            },
            ArgSpec {
                name: "ship",
                kind: Kind::Id,
                need: Need::Optional,
                level: Level::ADMIN,
                help: "the ship to move; default your own",
            },
        ],
    },
    Spec {
        name: "where",
        verb: Verb::Where,
        level: Level::DEBUG,
        summary: "the ids of the star your ship is at and of the bodies around it",
        args: &[ArgSpec {
            name: "show",
            kind: Kind::Word(&["major", "all"]),
            need: Need::Default("major"),
            level: Level::DEBUG,
            help: "which bodies to list",
        }],
    },
    Spec {
        name: "energize",
        verb: Verb::Energize,
        level: Level::DEBUG,
        summary: "put energy in a ship's storage, up to what it holds",
        args: &[
            ArgSpec {
                name: "amount",
                kind: Kind::Number(&[Limit { level: Level::DEBUG, min: 0.0, max: 1.0e9 }]),
                need: Need::Optional,
                level: Level::DEBUG,
                help: "energy in ME; default enough to fill it",
            },
            ArgSpec {
                name: "ship",
                kind: Kind::Id,
                need: Need::Optional,
                level: Level::ADMIN,
                help: "the ship to fill; default your own",
            },
        ],
    },
    Spec {
        name: "finish-refit",
        verb: Verb::FinishRefit,
        level: Level::DEBUG,
        summary: "complete a refit under way, at once",
        args: &[ArgSpec {
            name: "ship",
            kind: Kind::Id,
            need: Need::Optional,
            level: Level::ADMIN,
            help: "the ship to finish; default your own",
        }],
    },
    Spec {
        name: "stage",
        verb: Verb::Stage,
        level: Level::DEBUG,
        summary: "put a named scene in the world",
        args: &[ArgSpec {
            name: "scene",
            kind: Kind::Word(SCENES),
            need: Need::Required,
            level: Level::DEBUG,
            help: "the scene",
        }],
    },
];

/// `lc_world::scenario::Scenario::ALL` by name, which a `const` cannot collect for itself.
const SCENES: &[&str] = &["traffic", "meeting", "approach", "closing", "chase", "corona"];

/// A command `level` may run, by name.
pub fn find(name: &str, level: Level) -> Option<&'static Spec> {
    COMMANDS.iter().find(|spec| spec.name == name && level.at_least(spec.level))
}

/// A line waiting for the tick to reach it.
#[derive(Clone, Debug)]
pub struct Queued {
    from: ClientId,
    seq: u32,
    line: String,
}

impl<J: Journal> Server<J> {
    /// Hold a line for [`Server::run_commands`].
    ///
    /// Charged against the sender's rate budget before this, like any other message. One too
    /// long to parse is answered here rather than held.
    pub(crate) fn enqueue(&mut self, from: ClientId, seq: u32, line: String, wire: &mut impl Transport) {
        if line.len() > COMMAND_LIMIT {
            let text = ParseError::TooLong.to_string();
            wire.send(from, Outbound::Answered { seq, ok: false, text });
            return;
        }
        self.commands.push_back(Queued { from, seq, line });
    }

    /// Run every queued line, in the order it arrived, and answer each.
    pub(crate) fn run_commands(
        &mut self,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let queued: VecDeque<Queued> = std::mem::take(&mut self.commands);
        for command in queued {
            let (ok, text) = match self.run(&command, wire, events, deliveries) {
                Ok(text) => (true, text),
                Err(text) => (false, text),
            };
            wire.send(command.from, Outbound::Answered { seq: command.seq, ok, text });
        }
    }

    fn run(
        &mut self,
        command: &Queued,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<String, String> {
        let level = self.commanding(command.from);
        let parsed = parse(&command.line).map_err(|e| e.to_string())?;
        let spec = find(&parsed.name, level).ok_or_else(|| unknown(&parsed.name))?;
        let args = bind(spec, &parsed, level).map_err(|e| e.to_string())?;
        match spec.verb {
            Verb::Help => help(args.text("command"), level),
            Verb::Teleport => self.teleport_command(command.from, &args, wire, events, deliveries),
            Verb::Where => self.where_command(command.from, args.word("show") == Some("all")),
            Verb::Energize => {
                let ship = self.ship_named(command.from, &args)?;
                self.energize(ship, args.number("amount"), wire)
            }
            Verb::FinishRefit => {
                let ship = self.ship_named(command.from, &args)?;
                self.finish_refit(ship, wire)
            }
            Verb::Stage => {
                let name = args.word("scene").unwrap_or_default();
                let scene = lc_world::scenario::Scenario::named(name).ok_or_else(|| format!("no scene {name}"))?;
                self.stage(scene).map_err(|_| format!("this shard does not hold the star {name} is set at"))?;
                Ok(format!("staged {name}"))
            }
        }
    }

    /// The ship a command acts on: `ship:` when given, and otherwise the asker's own.
    fn ship_named(&self, from: ClientId, args: &Bound) -> Result<CraftId, String> {
        match args.id("ship") {
            Some(raw) => i64::try_from(raw)
                .ok()
                .map(CraftId)
                .filter(|id| self.fleet.get(*id).is_some())
                .ok_or_else(|| format!("no ship {raw}")),
            None => self.owned_by(from).ok_or_else(|| "you have no ship".to_string()),
        }
    }

    /// The level this connection's commands are checked at.
    fn commanding(&self, from: ClientId) -> Level {
        let level = self.clients.get(&from).map_or(Level::PLAYER, |c| c.permission);
        crate::ability::commanding(level, crate::ability::Directing(self.directs))
    }
}

/// The same answer for a command that does not exist and one the asker may not run.
fn unknown(name: &str) -> String {
    format!("no command '{name}'; try help")
}

fn help(command: Option<&str>, level: Level) -> Result<String, String> {
    if let Some(name) = command {
        let name = name.trim_start_matches('/').to_lowercase();
        return find(&name, level).map(|spec| spec.help_for(level)).ok_or_else(|| unknown(&name));
    }
    let mut out = String::from("commands:");
    for spec in COMMANDS.iter().filter(|spec| level.at_least(spec.level)) {
        let _ = write!(out, "\n  {}: {}", spec.name, spec.summary);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
