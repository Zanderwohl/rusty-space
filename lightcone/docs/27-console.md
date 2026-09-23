# The console

`/` opens a line at the top of the screen. What is typed there goes to the shard **as text**,
and the shard parses it, checks it against the asker's level, runs it at a fixed point in the
tick and answers the connection that sent it. The client knows nothing about any command. Code
is `lc_server::command` and `lc_client::console`.

Status: **built** — `help`, `teleport`, `where`, `energize`, `finish-refit`, `stage`.

## Why text on the wire

A client that sent a parsed command could send one no parser would have produced: an argument
out of range, a key the command does not take, a level it does not have. Sending the line leaves
exactly one parser, and it is on the side that decides. It also means a new command is a server
change and never a client release.

`Inbound::Command { seq, line }` carries it; `seq` is the client's own count, and
`Outbound::Answered { seq, ok, text }` finds its way back to the line by it. The text is for a
person and nothing parses it.

## The grammar

```text
teleport 0x6c1f... altitude:3 star:"0x9e2a..."
```

A command name, then space-separated arguments. Each is a bare word, a quoted string, or
`key:value` where the value is bare or quoted. A quoted value takes `\"` and `\\` and no other
escape. A bare value may not contain `"` or `:`. `a:b:c` is refused and the player is asked to
quote it, because no split of it is obviously the right one. A leading `/` on the name is
ignored. Names and keys are case-insensitive, and values are kept exactly as typed.

Errors name a column, counted in characters.

## Binding

Every argument has a name and may be given by it or by position. A positional value fills the
first argument not yet given, in declared order. The line is read in the order typed, so
`go 7 target:8` is a target given twice rather than a `7` that quietly moved to the next
argument. An argument is **required**, **optional** (absent unless given), or has a
**default** that is read exactly as if it had been typed, so it goes through the same range
check.

## Levels

Levels are `crate::ability::Level`: a player, then debug, admin and superadmin. `COMMANDS` is
the table, and it states a level three times:

| | below it |
|---|---|
| the command | the name answers as if no command had it, and `help` does not list it |
| an argument | the command does not take it, and positionals never reach it |
| a numeric range | `Limit`s widen with seniority; the widest the asker reaches applies |

**Below its level a thing does not exist**, and every refusal is worded so. A console that said
"not permitted" would tell a player which administrative commands a shard has and what they
take.

On a directing shard, which covers `--local` and `--demo`, everyone is a superadmin, for the
reason `ability::allows` opens development there: the population is whoever ran `cargo run`.
`energize` and `stage` sit at the levels the old `Inbound::Grant` and `Inbound::Stage` are allowed
at, and a test holds the two tables to each other.

`teleport`, `energize` and `finish-refit` act on the asker's own ship from debug (3). Their `ship:`
argument, which names any ship, starts at admin (2), so admins and superadmins act on anyone's.

## The queue

A line is charged against the sender's rate budget like any other message, held, and run once a
tick **after the intents**. A command therefore sees every order that arrived with it already
flown, and nothing it does is re-solved by a pursuit before the tick is out.

## Energize

`energize [amount:<ME>] [ship:<id>]` adds energy to a ship's storage. With no amount it fills the
ship, and an amount past what fits tops it off at capacity rather than being refused. The ship's
account is settled first, so "what fits" counts everything collected and spent up to now.

## Finish-refit

`finish-refit [ship:<id>]` completes a refit under way now: the target loadout, and the energy the
remaining steps would have taken. Only the time is skipped, so the crew's upkeep over it is not
charged, and a ship that is not refitting is an error.

## Teleport

`teleport <target> [altitude:2] [star:<id>] [ship:<id>]` puts a ship on an equatorial orbit of
a star or body at once. The target is a star's catalog id, or a body's `BodyId`. A body is
looked for only in systems already loaded, unless `star:` says which. Loading every star's
system to search would generate the whole catalog in one tick. `where` prints the ids. It
never prints names: the generator's keys must not reach a player.

Moving somebody else's ship takes `ship:`. The burn-attribution
concern in `ability.rs` does not arise here. A teleport's events are kinds no order produces, so
nothing in the record reads as the owner having done it.

### What it does to spacetime

Everything else a ship does is motion, and a worldline is continuous. A teleport is a
**spacelike jump**, and three things in the light-cone model had assumed it could not happen:

1. **The retarded solve.** `f(t) = t + |x_o - w(t)| - t_o` is strictly increasing for anything
   sub-luminal. Across a jump it steps, and a bisection over the step converges onto it: an
   image at the moment of the jump, which no light left from. So `Worldline::breaks` names the
   jump times and both solvers split there. Each piece has at most one root, and
   `retarded_times` returns one per piece, oldest first. Near where the ship landed there are
   **two images** for a while: the new one, and old light still arriving from where it left.
   Near where it left there are **none** between the light of the vanishing and the light of
   the landing. Contacts take the newest.
2. **The stretch it left.** A craft's remembered stretches used to be evaluated against the
   system it is in *now*. A station about a body the new system lacks froze where it was, and
   an observer still receiving the old light saw it stop the instant the ship left. Each
   `Past` now carries the system it was flown in.
3. **Who is worth solving for.** A contact was whoever shares your system now, so a ship that
   jumped out vanished for everyone there on the next tick. `chase::in_sight` now asks whether
   the other craft has *been* in your system within what it remembers. Whether its light is
   still arriving is then the solver's question, as it always should have been.

What anyone learns of the jump itself arrives as two events at the same coordinate time:
`kind::VANISH` where it was and `kind::APPEAR` where it landed. Each travels at `c` from its own
end, so a ship far from both hears of them years apart, and neither says where the other end is.

`command::tests::a_ship_that_jumps_is_seen_to_go_only_when_the_light_of_it_arrives` fails without
the first (an image at the landing, light-years off, the moment the old light runs out) and
without the third (the contact lost on the next tick). Both were seen. The second is
`craft::tests::a_teleport_leaves_the_old_stretch_where_it_was_and_breaks_there` in `lc-world`.

## The window

An egui window hung from the top center, opaque because it draws over whatever else is open,
and in `Order::Foreground`. The history scrolls above the field. Up and Down recall earlier
lines and Escape closes it. `/` only ever opens it: once the field has the keyboard, a slash is
a slash. `--console <line>` types one line once the shard has welcomed the client, which is
the only way to photograph an answer.
