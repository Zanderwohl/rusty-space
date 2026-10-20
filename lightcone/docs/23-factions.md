# Factions, relays and what things are called

[22-provenance.md](22-provenance.md) gave every record a witness and a route. This document is
what happens when craft start passing records on for each other: who counts as "us", how that
is enforced when nothing can be checked faster than light, how a report finds its way across a
network that is rearranging itself as it flies, and what anyone calls anything.

**Status: partly built.** What things are called is built: subjects, assigned names as rules,
planet letters and the descriptor ladder in `lc_world::knowledge::{subject, names, called}`, and a
hop added to every record a report carries (`lc_world::knowledge::record::Hop`). Factions are built
(11e): keys and generations in `lc_server::faction`, `Order::{Found, GiveKey}` and
`Secrecy::Faction` in `lc_server::radio`, rows in `lc_store::chat` (`0014_factions`), and the chats
in `lc_client::{chat, radio_panel}`. Notes and chosen names are built as threads (11g, in part):
`lc_world::knowledge::thread`, `Order::Write`, and `lc_client::thread_view` in the telescope and
system panels. Not built: assets and their minimum generation, which wait for assets; out-of-game
invitations; relays, bundles and transmission cost, with no `transmit_gain` (11f); the subject card
opened from the map and the sky, and report transcript lines (11g). Craft names are claims every
transmission carries (`lc_proto::Claim`, `lc_client::craft_names`).

The rule that shapes all of it:

**A faction is not a list. It is a key, and its chat.** There is no table the server consults to
decide who is in, and no roster anywhere. There is who has been sent a faction's key, as of when
that light landed. Everything else — who a player thinks is in, who a relay forwards to — is a
belief built from what has been said under the key, and it can be wrong.

## Founding

Any craft can found a faction, and founding is all it takes: the craft mints a new key, names it,
and holds it from that instant. Nothing goes on the air. The key comes with its own chat, the way
a new group chat does — but there is nobody to add. A faction of one is a key nobody else has
been sent.

The name travels with the key. Whoever is sent the key is sent what its founder called it, and an
outsider learns the name only by overhearing it said in the open.

## Joining is being sent the key

To bring another craft in, **beam it the key**. That is a transmission like any other: aimed,
sealed or open, at `c`. Whoever can read it when it lands holds the key from then on, and nothing
else is involved — no acceptance, no certificate, no invitation the server keeps.

| sent | who holds the key afterwards |
|---|---|
| sealed to the recipient's ship key | the recipient alone |
| open | **everybody it reaches**, which across a crowded system is the whole system |

Sealing needs the recipient's ship key, the same as any private message
([05-observation.md](05-observation.md#keys-and-why-first-contact-is-loud)). The interface lets a
player send a faction key in the open and says plainly what that does. Anybody who holds the key
can pass it on; holding it is the only permission there is.

**A new craft belongs to nothing.** It starts holding no faction keys, and the only way in is
being sent one.

The one exception is an invitation from outside the game: a player invites a friend by email or
the like, and when the friend creates an account, their first craft is created **already holding
the key**, as if it had landed the instant the craft existed. It is the single place the game lets
anything skip light delay, and it is honest about why: the invitation was not sent through space.
It is not built yet; it needs the identity broker to carry an invite from one account to the next.

## Faction chatter

A faction's chat is every message sealed to its key, shouted or beamed like any other.

- **Readable** by every craft that held the key when the message landed. A craft sent the key
  later does not get to read the history it overheard before — the same as joining a group chat.
- **Heard** by everybody in range, as always. That a faction is talking — how much, how often,
  from where — is visible to anybody listening, and it is intelligence. A listener without the key
  can tell two messages were sealed to the same key, and nothing else.
- **Addressed to nobody**, so nobody acknowledges it automatically. Acknowledgment is between two
  craft; a chat of forty would acknowledge itself forever.

There is no members list to show, because there is no membership. What a player sees is who has
spoken under the key, and when their words landed.

## Membership is not exclusive

Nothing about holding one key stops a craft holding another. A craft can belong to as many
factions as will send it a key, and the model does not distinguish the reasons:

| arrangement | what it looks like in keys |
|---|---|
| overlapping goals | two keys with holders in common, each chat read by the overlap |
| a loose coalition — an EU, a UN | a key held by the leading craft of other factions; it says nothing about their own keys |
| a double agent | a craft that holds both keys and tells each side what suits it |

The consequence worth designing for is **what moves between factions**. Automatic relaying
stays inside the key a bundle is sealed to: a craft holding two forwards A's traffic to A and B's
to B, and never one to the other on its own. Carrying anything across is a manual relay — a
choice somebody makes, recorded on every record it carries as a hop through their ship. A leak is
therefore always somebody's act, and lineage says whose, to anybody who later holds the record.
That is not protection against a double agent. It is evidence against one.

The interface needs a chat per key held, and a message, a report or a note has to say which key
it goes out under. Getting that wrong is a real way to leak, and the interface should make it
possible and visible rather than impossible.

## Removal is a new key

A member who turns out to be a traitor still holds the key. No message can take it back. The only
defense is to **found again**: mint a new key that **replaces** the old one, and send it to
everyone except the traitor. The new key is a new chat; the old one goes on existing, readable by
everyone who held it, traitor included.

### Generations

A key may name the key it replaces, and so the keys of a faction form a chain, each a
**generation** one later than the key it replaces. Only a holder of a key can mint its
replacement. The chain and the founder travel with the key, so whoever is sent generation 3
knows it replaces generation 2 whether or not they ever held 2, and knows who made it.

The chain is what the interface reads: a chat under a key this ship holds a later generation of
shows that there is a newer key, which, and whose. Nothing stops a player speaking under the old
one anyway — sometimes that is exactly who they mean to reach.

### Forks

**Any holder can replace a key, so two can.** Each makes a key of the same generation, and
the chain splits. Nothing reconciles it: a shard that picked a winner would be acting on what
nobody in the game knows yet.

Sometimes a fork is innocent. Two loyal members light-years apart notice the same traitor and
re-key within the same week, neither able to know of the other, and the faction is split in two
until somebody holding both keys says so. Sometimes it is the attack. A traitor whose defection
nobody has noticed re-keys first and sends the new key to part of the faction, which follows it
in good faith; that part is cut off from the rest, speaking under a key the traitor holds and
the others do not, until someone who saw both explains what happened. It is the same mechanic as
a loyal re-key, and nothing but who founded each branch tells them apart.

So the interface shows **every branch, with its founder**: a chat superseded twice says so and
names both, and two held keys sharing a name and a generation are labeled by who made them.
Which to follow is the player's decision, made on what they know about the founders.

The detail that makes this a mechanic rather than a button: **the new key cannot be sent sealed
to the old one**, because the traitor holds the old one. It goes out one transmission per member
kept, each sealed to that member's own ship key. So:

- moving to a new key requires holding the ship key of everybody you mean to keep, which is a
  reason for members to trade ship keys early and a cost for anyone who did not;
- it takes as long to complete as light takes to reach the furthest member kept;
- until each member hears it, they are still speaking under the old key — and the traitor can
  still read every word.

### "It's an older key, sir, but it checks out"

A message under the old key is still readable by everyone who holds it. It may be a loyal member
four light-years out who has not been sent the new one yet. It may be the traitor. **Nothing about
the message says which**, and the interface does not pretend to. Deciding whether to trust it is
a player's decision made under uncertainty — the game working, the same way as acting on a
two-sigma bump in a light curve.

Identity is not forgeable yet: the server knows which ship transmitted, and a receiver sees the
true sender. A traitor lies with their own name. Stealing a ship key, which would make forgery
possible, is a later mechanic and should be designed as one — it breaks the one guarantee this
section otherwise keeps.

### Later: assets accept keys

When there are probes, swarms and structures to command, each will hold the faction keys it has
been **sent**, and a **minimum generation**, and obey an order sealed to a key it holds at or
above the minimum. Descending from a key it holds grants nothing: any holder can replace a key, the
traitor included, so a rule that honored the chain would hand every asset to whoever forked it.
Raising the minimum is itself an order, and it is light-delayed like everything else. A
swarm that has not heard it takes orders from the traitor for as long as the order takes to reach
it. An asset cannot be raised past a generation it has never been sent, so re-keying it is two
transmissions, the key and the order. Two contradictory orders sent from opposite sides of a
sector are obeyed in whichever order the geometry delivers them, and the loyal side wins a race it
may not know it is in by being **closer**. Nothing about this needs
certificates or capabilities; it waits for there to be assets.

## Relays

A relay is a craft that passes on what it received. Radio and the internet solved this problem
already, and the useful part of their answers carries over — with one difference that dominates:
**the network moves**. Every link's length is a light-time between two craft that are both in
motion, and every route a craft knows was computed from where its neighbors *were*.

The closest real analog is not the internet. It is **delay-tolerant networking** — the
Bundle Protocol (RFC 9171) and the contact-graph routing NASA flies between spacecraft — which
was built for exactly this: links that exist only at predictable times, and delays long enough
that end-to-end conversation is impossible.

### What travels

A relayable unit is a **bundle**:

| field | why |
|---|---|
| origin, and the origin's key for it | duplicate suppression: a bundle is the same bundle however it arrived |
| hop limit (TTL) | a bound on how far it spreads, set by the origin |
| path so far | split horizon, and the lineage every record in it already carries |
| seal | who can read it; a relay forwards what it cannot read exactly as it forwards what it can |
| payload | a report, a message, a key |

### Two kinds of forwarding

**To the faction: controlled flooding.** A bundle meant for the whole faction is re-transmitted
by every member that receives it, subject to three rules, all standard:

1. **Echo only new.** A relay forwards a bundle the first time it sees it, and forwards a report
   only for the records that were new to it. The merge in `Knowledge::receive` already knows
   which those are.
2. **Split horizon.** Never back toward a craft on the bundle's path.
3. **TTL.** Each hop decrements it; at zero, keep it and stop.

Flooding needs no routes at all, which is why it is the default: it works on a network nobody
has a current map of.

**To one craft: routed.** An addressed bundle goes toward its destination by whatever route
looks cheapest. Each craft keeps a table of `destination -> (next hop, cost, as of)`:

- **cost is light-time**, from where the next hop is predicted to be when the light arrives,
  plus the next hop's own advertised cost onward;
- routes are **advertised in reports** — distance-vector, Bellman-Ford over light delay — so a
  route to a craft forty light-years out is at least forty years old when it arrives;
- every entry **ages**, and an old route is not a wrong route but a guess, weighted as one;
- where trajectories are known — an orbit, a course with a burn plan — the route can be
  computed from **predicted contacts** rather than advertised ones, which is contact-graph
  routing and is strictly better whenever it applies.

**Custody.** A relay that accepts a bundle for forwarding holds it until it can send it. That
is store-and-forward, and it is what lets a message cross a region where no two craft are ever
in range at the same time.

### Cost

**Not built yet: this is phase 11f.** Today a transmission costs nothing.

**Every transmission is to cost stored energy**, relayed or not, by real physics scaled by one
constant — the same arrangement solar income uses
([20-solar-power.md](20-solar-power.md#balance)).

The physics is a link budget. Delivering `bits` of data to a receiver of effective area `A` at
distance `d`, with enough energy per bit `E_b` to clear its noise floor, takes

```
E = bits * E_b * 4 pi d^2 / (A * gain)
```

where `gain` is 1 for a shout and `4 pi / Omega` for a beam of solid angle `Omega`. Everything in
that line is a mechanic already: range costs as its square, a beam is millions of times cheaper
than a shout on axis, and a report costs in proportion to its size. The **range is the sender's
choice** — enough to reach the addressee where it is believed to be, or a radius for a broadcast —
and the same choice sets how far the signal is heard, which is the fan-out rule of
[02-event-store.md](02-event-store.md#delivery-scheduling): a louder transmitter is heard by more
craft and costs more.

Real numbers are absurdly small against a module-energy of 1.4e25 J, so the energy drawn is that
physical energy times `transmit_gain`, a `Balance` setting derived from a named anchor the way
`solar_gain` is — for example, "an omnidirectional report of sixty-four stars to one light-year
costs a hundredth of a storage module". Retuning is a change to the anchor.

Until data rates exist, `bits` is the size of what is sent and nothing limits how fast it goes;
rate limits are deferred by decision.

A relay policy is therefore a budget as well as a rule: how much of its storage a craft will
spend carrying other people's news. A faction that floods everything is loud, expensive and
visible across the sky.

### Swarms relay as populations

A craft relays if it has a transmitter and a receiver fitted, and that is as true of a craft
nobody flies as of one somebody does. Non-player craft will have modules the way a ship does
([19-ship-fitting.md](19-ship-fitting.md)), so a swarm built with relay modules relays.

A swarm is not a roster, though. It is a population — a distribution over orbits, a count and a
cross-section — and [03-world-model.md](03-world-model.md#swarms-are-populations-not-entities)
makes it a hard rule that nothing enumerates its members. Relaying follows the same rule: it is
a property of the distribution, not of elements.

| property of the population | what it gives the network |
|---|---|
| fraction of elements fitted with relays | how much of its count carries traffic |
| the orbital distribution | a footprint: the region inside which a bundle can enter the swarm and leave it anywhere else |
| element spacing, from count and footprint | hop length inside the swarm, and so its internal delay |
| total transmitter power | how far outside its footprint the swarm can be heard, and its capacity |

To the rest of the network a relay swarm is **one node with a size**: a bundle reaching any part
of its footprint is inside it, and crosses it in the time light takes to cross the footprint —
minutes for a swarm at an AU, not zero. No element is ever addressed.

Its keys follow the same granularity. A swarm's accepted keys belong to the population record,
or — since sub-populations exist and nest — to each sub-population. A re-keying reaching a swarm
built in waves arrives at one wave before the next, and for the minutes between, the swarm is two
things obeying two sets of keys. That is the smallest version of the race above, and it is free.

A swarm is also one witness, not a million: what a telescope swarm sees is recorded as the
population's observation through `Optics::joined`, with the collecting area and baseline its
distribution implies.

### Manual relay

Any received report can be passed on by hand: to one craft, to the channel, or shouted. The
record keeps its witness and gains a hop, so "Osprey saw this; Kestrel told us" survives the
retelling.

## What things are called

Everything a craft can know about gets a name the same way a star does: a
[`Naming`](22-provenance.md#nothing-has-a-name) with a witness, chosen or assigned, overridable.
The difference is the rule that assigns one before anybody chooses.

### Subjects

Knowledge today is keyed by star. It has to be keyed by **subject**:

| subject | identity | assigned name, until somebody chooses one |
|---|---|---|
| star | synthetic star id | the bearing it was found along |
| planet found by transit | star + its place in the generated system | `{what we call the star} {letter}` |
| any other body | star + its place in the generated system | discovery time, `{year}-{order}` |
| population | star + population index | `{what we call the star} {belt / cloud} {n}` |
| craft not ours | ship id | whatever it calls itself, from the first transmission heard; discovery time before that |

**A body is shown with what it is believed to be in front of its designation**, until somebody
names it: `Object 2-14`, then `Planetoid 2-14` once an orbit is fitted, then `Cold Jupiter 2-14`
once a radius is measured, then whatever type the evidence settles. That descriptor is derived at
every read from the body's belief and never filed, so it promotes itself and two craft can
disagree on it; only the designation is stored. The ladder is `lc_world::knowledge::called`.

**An assigned name is a rule, not a string.** "Kettle b" is stored as "the planet in letter slot
`b` of the star this ship calls the Kettle", so renaming the star renames its planets with it. A
name somebody *chose* is a string and stays put.

### Planet letters

Letters go outward from the star, starting at `b` — `a` is the star — and **a letter is frozen
once assigned**. Two craft that assigned letters at different times from different data can
hold different letters for the same planet, and that is a disagreement between witnesses like
any other.

A letter is a guess at a planet's **place in the whole system**, not its rank among the planets
found so far. The rule places a new planet by its orbit against the spacing the system is
expected to have, and leaves gaps for the members it has not found:

1. **Expected slots.** From the star's mass and luminosity, the generator's own spacing law gives
   where the innermost planet is expected and the typical ratio between neighbors' orbits — the
   same honest prior conclusions use ([24-standing-instruments.md](24-standing-instruments.md)).
   Slot `n` is the `n`th expected orbit outward.
2. **A planet takes the slot nearest its orbit**, in log-distance. A hot Jupiter at 0.05 AU found
   first may well be `b`; a Neptune at 30 AU found first may be `h`, with five letters left open
   inside it on the expectation that something is there.
3. **Too many gaps is fine.** Letters are skipped where the system turned out emptier than
   expected, and nobody has to fill them.
4. **Too few is resolved by a second letter.** A planet whose nearest slot is taken is placed
   between that slot's holder and its neighbor on the side it falls, with a letter appended:
   between `b` and `c` go `bb`, `bc`, …, in order outward, and the rule recurses if it has to
   — between `b` and `bb` is `bab`. Alphabetical order is orbital order at every depth.
5. **Inside `b`** is `ab`, `ac`, …, the same rule with `a` as the letter before `b`.

A second letter starts at `b`, never `a`, for the same reason the first one does: `a` is kept
free at every depth to mean "inside the first", the way it means the star at the top. Without
it, nothing could ever be inserted between `b` and `ba`.

The spacing lives in `lc_world::knowledge::names::SPACING`, a table by luminosity. Its one row
today is measured from the generator — innermost orbit `0.70 sqrt(L)` AU, neighbors `1.83`
apart — and lettering eight thousand generated planets in random discovery order needs a second
letter for about one in nine.

A craft holds a planet's assigned letter beside its thread. Naming a planet "Spout" is shown in
place of its letter and never unassigns it: the letter stays frozen, holds its
place among the letters, and a refined orbit cannot bring it back over the name.

A little silly, and real catalogs have been sillier. What matters is that the rule is
deterministic given what the assigning craft knew, so two craft with the same data agree, and
that no later discovery ever renames anything.

**Everything about a system rides with its star.** A report entry for a star carries its bodies,
populations, namings and notes. A report about a system is one entry, not thirty.

The home system is the one place the rule is overridden from the start: its bodies are named by
the charts a ship launches with, which is the charting office's naming like any other, and
overridable like any other.

### Craft that are not ours

A craft's name is information too, and nothing reaches a player as a name unless somebody said it.
`Presence` carries none.

**Every transmission carries the name its sender calls itself** — its own newest name entry about
itself, with what a receiver needs to file the identical entry. A receiver files it as the light
lands, sealed message or not: who sent a signal is never hidden, only what it says. A craft starts
by claiming the name it was made with, the account's for a player's, and renames itself with a
name entry about itself, which is public or nothing.

**Who sent something is not a claim.** The shard attributes every transmission to the craft that
made it, so a craft's words, chats and thread entries hang off that craft and not off its name. A
name is the one thing that can be borrowed, and the screen guards it:

| a craft's name stands on | |
|---|---|
| this ship's word | anything this ship named it |
| a faction's | a name under a faction key this ship holds, which is vouching for it |
| anybody else's | a name some other craft gave it, in the open |
| its own | what it claims |

Stronger beats weaker, newer beats older within one. A craft whose name stands only on its own
word, when another craft already goes by that name on a better one or claimed it first, is shown
as **Wren (claimant 3fa1)**, and the parenthesis leads to the conversation with the Wren this ship
knows. When a craft claims a new name the log says so where its message lands: "Wren changed name
to Vela."

**A craft is never shown by its shard id.** Unnamed, it is `ship 3fa1`: a tag hashed from the id,
the same for every observer so it can be quoted, and saying nothing about how many ships there are
or which came first.

One language today, so a claimed name is always legible. When dictionaries arrive, a claim in a
language the receiver cannot read is exactly the case they filter.

### Later: dictionaries

Namings carry their witness, so a faction's names are already distinguishable from anybody
else's. The mechanic where factions cannot read each other's names without a deliberate exchange
is a filter on which namings a craft may *display*, applied on top of this. Nothing here needs to
change for it.

## Notes, and the names somebody chose

Everything a craft writes about a subject goes in that subject's **thread**: notes, and the names
somebody chose for it. A thread has no owner and no agreed order. Any craft appends to its own copy,
copies merge by union, and nothing is ever edited or removed — two authors light-years apart
cannot take turns. A correction is another entry.

| an entry has | why |
|---|---|
| author | the craft that wrote it; identity is not forgeable yet |
| when written | coordinate time at the author |
| parents | the thread's newest entries as the author held them when writing |
| scope | where it may travel: below |
| body | a note's text, or a name |

An entry is known by a hash of all of that, so it is the same entry however many routes it arrives
by, and two holders of the same entries hold the same thread.

**The parents are the order.** An entry comes after everything it names, directly or through other
entries; otherwise threads are shown by when each was written. Two entries neither of which names
the other were each **written without knowing of the other** — often because the light of one had
not reached the author of the other. The interface says so. It is the one place in the game where
a player sees two people talking past each other because of physics, and it needs no clock to tell:
an entry that had not heard of yours does not name it. An entry whose parent never reached this
craft — private, or under a key it lacks — is kept, and shown as a reply to something not held here.

**A name is an entry.** The name that stands is the newest no later name supersedes. Two names
neither of which knew the other both stand, and that is a disagreement nobody has settled: a craft
goes by its own, otherwise by the more recently written, and the thread shows both. What a *rule*
assigned — a bearing designation, a planet letter — is not an entry: nobody wrote it, so it stays
on file beside the thread and a chosen name is shown in its place.

### Scope

Chosen at write time, from a list: **for me**, **public**, or any faction key held. For me is the
default.

| scope | travels in |
|---|---|
| for me | nothing; it never leaves the craft |
| public | any report |
| a faction | only reports sealed under that key |

**A thread rides whole with its system.** A report that carries a system carries every entry about
every subject in it that its seal admits — new or not, and at no cost against the report's size.
So an entry a report could not carry is never left behind: the next report it may go in takes it.
A craft's reporting is tracked apart for each faction key, so an open broadcast does not move past
what only a faction report may carry. A member who receives a faction entry carries it on in its
own reports under that key, which is how a thread grows across a faction by relay.

## Transcripts

A report is not a conversation, but it is an event in one. **As built**, the receiving client
says so in a notification, not a transcript row:

> **Kestrel:** told you about 64 stars

Nothing is written to `lc_messages`, and a test holds it there, for the reason
[22-provenance.md](22-provenance.md#reports-on-the-air) gives: its content lives in the
receiver's knowledge from the moment it lands. A transcript line — a row with a summary body and
a flag, "relayed Osprey's survey of 12 stars" among them — is the plan once relays make it worth
reading back, and is 11g's to build.

## Open

- **The spacing law** behind planet letters: which of the generator's quantities define the
  expected innermost orbit and the neighbor ratio, and whether small bodies and moons get a
  scheme of their own beyond discovery time.
- **Whether flooding should be the default at all**, or opt-in, given what it costs in stored
  energy and in visibility. Measure it with twenty craft, with transmission costing, before
  deciding.
- **The anchor** for `transmit_gain`: one sentence, chosen when 11f builds it, and written down
  beside the solar anchor. `data_per_module`'s is in [19-ship-fitting.md](19-ship-fitting.md).
- **Key theft**, dictionaries, and commanding assets: deferred by decision, and each sits on top
  of faction keys without changing them.
- **Reading history on joining.** A key opens only what landed after it did. Real ciphertext
  recorded earlier would open too; whether the game should let a newcomer read what they overheard
  is worth trying once there are factions to try it in.
