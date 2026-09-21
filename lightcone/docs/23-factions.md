# Factions, relays and what things are called

[22-provenance.md](22-provenance.md) gave every record a witness and a route. This document is
what happens when craft start passing records on for each other: who counts as "us", how that
is enforced when nothing can be checked faster than light, how a report finds its way across a
network that is rearranging itself as it flies, and what anyone calls anything.

The rule that shapes all of it:

**A faction is not a list. It is a key.** There is no table the server consults to decide who
is in. There is who has been told a faction's key, as of when that light landed, and what they
were given permission to do by somebody who had permission to give it. Everything else — the
roster a player sees, who a relay forwards to, who can read the channel — is a belief built from
those facts, and it can be wrong.

## Founding, and what a faction is called

Any craft can found a faction. Founding mints generation 1 of the faction's key, gives the
founder every capability, and names it.

The name is a [`Naming`](22-provenance.md#nothing-has-a-name) like any other, stated by the
founder. A faction's name is information about the faction: members learn it from their
invitation, and outsiders learn it only by overhearing open traffic that mentions it. Two
factions may call a third by different names, and a faction may not know what anybody else
calls it.

## Membership is not exclusive

Nothing about holding one key stops a craft holding another. A craft can belong to as many
factions as will have it, and the model does not distinguish the reasons:

| arrangement | what it looks like in keys |
|---|---|
| overlapping goals | two factions with members in common, each channel read by the overlap |
| a loose coalition — an EU, a UN | a faction whose members are the leading craft of other factions; the coalition's key says nothing about the members' own |
| a double agent | a craft that holds both keys and tells each side what suits it |

The consequence worth designing for is **what moves between factions**. Automatic relaying
stays inside the faction a bundle is sealed to: a craft in two factions forwards A's traffic to A
and B's to B, and never one to the other on its own. Carrying anything across is a manual relay —
a choice somebody makes, recorded on every record it carries as a hop through their ship. A leak
is therefore always somebody's act, and lineage says whose, to anybody who later holds the
record. That is not protection against a double agent. It is evidence against one.

The interface needs a channel per faction held, and a report or a note has to say which faction's
key it goes out under. Getting that wrong is a real way to leak, and the interface should make it
possible and visible rather than impossible.

## Keys and generations

A faction key has a **generation**. Holding generation `g` of faction `F` means exactly one
thing: a message sealed to `(F, g)` is readable when it lands. It is the same mechanic as the
ship keyring in [05-observation.md](05-observation.md#keys-and-why-first-contact-is-loud) —
there are no keys, only the fact of having been told one — with a generation number on it.

| fact the server keeps | written when | readable when |
|---|---|---|
| `holds(ship, F, g)` | the key is transmitted | its light lands: `learnt_t <= now` |
| `granted(issuer, subject, F, g, capabilities)` | the grant is transmitted | the same |

Both are rows written at transmission and gated on arrival, like `lc_keyring` and
`lc_message_receipts`. That is what lets a faction work for a craft nobody is flying.

### Rotation, and why removal is hard

A member who turns out to be a traitor still holds every generation they were ever sent. No
message can take it back. The only defence is to **rotate**: mint generation `g + 1` and give it
to everyone except the traitor.

The detail that makes this a mechanic rather than a button: **the new key cannot be sent sealed
to the old one**, because the traitor holds the old one. Rotation is one transmission per member
kept, each sealed to that member's own ship key. So:

- rotating requires holding the ship key of everybody you mean to keep, which is a reason for
  members to trade ship keys early and a cost for anyone who did not;
- it takes as long to complete as light takes to reach the furthest member kept;
- until each member hears it, they are still speaking in the old generation — and the traitor
  can still read every word.

### "It's an older key, sir, but it checks out"

A message sealed to an old generation is still readable by anyone who holds that generation.
It may be a loyal member four light-years out who has not heard of the rotation yet. It may be
the traitor. **Nothing about the message says which**, and the interface does not pretend to:

| shown | meaning |
|---|---|
| `sealed: gen 5` | current as far as this ship knows |
| `sealed: gen 3, current is 5` | readable, and old; this ship knows who was sent 3 and who has been sent 5 |
| `sealed: gen 6` | newer than anything held — heard, not readable, and news in itself: somebody rotated |

Deciding whether to trust an old-generation message is a player's decision made under
uncertainty. That is the game working, the same way as acting on a two-sigma bump in a light
curve.

Identity is not forgeable yet: the server knows which ship transmitted, and a receiver sees the
true sender. A traitor lies with their own name. Stealing a ship key, which would make forgery
possible, is a later mechanic and should be designed as one — it breaks the one guarantee this
section otherwise keeps.

## Permissions are certificates

The user-facing shape is an invitation in a chat log. Underneath, a permission is a
**certificate**: issuer, subject, faction, generation, and a set of capabilities, transmitted
like a message and valid from the moment it lands.

| capability | lets the holder |
|---|---|
| `invite` | send the current key and a membership certificate to somebody new |
| `grant` | issue certificates carrying capabilities the holder has, including `grant` |
| `rotate` | mint a new generation and decide who is sent it |
| `generation` | tell an asset which generations it accepts |
| *later*: `command` | order the faction's probes, swarms and structures |

A certificate is honoured if its issuer held the capability it hands on, all the way back to
the founder, and if it is **bound to a generation the checker holds**. Rotation re-issues the
certificates of the members kept, bound to the new generation; a certificate bound to an older
one is the permission equivalent of an old key, and is shown the same way.

### Assets accept a generation

A faction's telescopes, relays, swarms and structures each carry a **minimum generation**: the
oldest key they will take an order under. An order is obeyed if it carries a valid certificate
and is sealed to a generation at or above that minimum.

Moving the minimum is itself an order, and needs the `generation` capability:

| move | effect |
|---|---|
| **raise** it | security upgraded: everybody still on an old key, traitor included, is locked out of that asset — and so is every loyal member who has not heard of the rotation yet |
| **lower** it | an older key lets somebody back in. That is what a rat does: one order, and a craft excluded two rotations ago commands a swarm again |

Raising the minimum past a generation the asset has never been sent is refused: an asset cannot
require a key it does not hold. So a rotation is complete for an asset only when the new key has
reached it *and* an order raising its minimum has — two transmissions, both at `c`.

Every piece of this is light-delayed, and that is the Excession mechanic without a line of code
of its own. A swarm that has not heard of the latest rotation takes orders from the traitor for
as long as the new key takes to reach it. An order lowering the minimum and an order raising it,
sent from opposite sides of a sector, arrive in whichever order the geometry decides, and the
asset obeys whichever landed last. The loyal side wins a race it may not know it is in by being
**closer**.

### Invitations

An invite is a message in the conversation with the invitee, carrying the faction's name, the
current key and a certificate. It should be **sealed to the invitee's ship key**. It does not
have to be, and the interface allows the mistake while naming it: an open invitation hands the
faction key to every craft in earshot.

There is no acceptance step the server enforces. Holding the key makes a craft able to read the
channel; whether it considers itself a member, and whether anybody else does, is a social fact
recorded in what it says next.

## Faction chatter

A channel is a conversation sealed to `(F, g)`, shouted or beamed like any message.

- **Readable** by every craft that holds that generation when the light lands.
- **Heard** by everybody in range, as always. That a faction is talking — how much, how often,
  from where — is visible to anybody listening, and it is intelligence.
- **Not addressed to anyone**, so nobody acknowledges it automatically. Acknowledgement is
  between two craft; a channel of forty would acknowledge itself forever.

## Relays

A relay is a craft that passes on what it received. Radio and the internet solved this problem
already, and the useful part of their answers carries over — with one difference that dominates:
**the network moves**. Every link's length is a light-time between two craft that are both in
motion, and every route a craft knows was computed from where its neighbours *were*.

The closest real analogue is not the internet. It is **delay-tolerant networking** — the
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
| payload | a report, a message, a key, a certificate |

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

Every forwarded bundle is a transmission and spends energy from the same budget as everything
else. A relay policy is therefore a budget as well as a rule: how much of its power a craft is
willing to spend carrying other people's news. A faction that floods everything is loud,
expensive and visible across the sky.

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

Its keys follow the same granularity. A swarm's accepted generation belongs to the population
record, or — since sub-populations exist and nest — to each sub-population. A rotation reaching a
swarm built in waves arrives at one wave before the next, and for the minutes between, the swarm
is two things with two minimums. That is the smallest version of the race above, and it is free.

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
| planet | star + its place in the generated system | `{what we call the star} {letter}` |
| small body, comet | star + its place in the generated system | discovery time, `{year}-{order}` |
| population | star + population index | `{what we call the star} {belt / cloud} {n}` |
| craft not ours | ship id | discovery time, until it names itself |

**An assigned name is a rule, not a string.** "Kettle b" is stored as "the second planet of the
star this ship calls the Kettle", so renaming the star renames its planets with it. A name
somebody *chose* is a string and stays put.

**Everything about a system rides with its star.** A report entry for a star carries its bodies,
populations, namings and notes. A report about a system is one entry, not thirty.

The home system is the one place the rule is overridden from the start: its bodies are named by
the charts a ship launches with, which is the charting office's naming like any other, and
overridable like any other.

### Craft that are not ours

A craft's name is information too. Today a contact arrives carrying its account's display name,
straight from the server — which is the one place a name reaches a player without anybody having
said it. It should become a claim the craft makes about itself, carried on its own transmissions,
and heard like anything else. Until a craft has transmitted in range, it is a designation.

### Later: dictionaries

Namings carry their witness, so a faction's names are already distinguishable from anybody
else's. The mechanic where factions cannot read each other's names without a deliberate exchange
is a filter on which namings a craft may *display*, applied on top of this. Nothing here needs to
change for it.

## Notes

A note is a comment on a subject: author, text, when written, and a scope.

- **Append-only.** Nobody edits a note, because two authors light-years apart cannot take turns.
  A correction is another note that refers to the first.
- **Private by default.** A note shared with the faction is sealed to the faction key and rides
  in the next report about that subject, so it arrives with everything else and at the same speed.
- **Ordered by when it was written, marked with when it arrived.** Two notes written by members
  on opposite sides of a sector are often spacelike-separated: neither author could have known of
  the other's. The interface should show that rather than hide it — it is the one place in the
  game where a player sees two people talking past each other *because of physics*.

## Transcripts

A report is not a conversation, but it is an event in one. The transcript gets a line:

> **Kestrel:** told you about 64 stars
> **Kestrel:** relayed Osprey's survey of 12 stars

Stored as a row in `lc_messages` with a summary body and a flag. The report itself is never
stored there, for the reason [22-provenance.md](22-provenance.md#reports-on-the-air) gives: its
content lives in the receiver's knowledge from the moment it lands.

## Open

- **Letters for planets.** "Presumed order" read literally means letters by orbital order as the
  observer understands it, so finding an inner planet later would re-letter every planet outside
  it. Proposed: letters are assigned in presumed orbital order among what is known **at the time
  of assignment**, and then frozen, so a late discovery takes the next free letter. That is also
  what real astronomy does. Needs confirming.
- **What an asset does with a certificate issued under a generation older than its minimum** but
  presented with an order sealed to a newer one. Proposed: the certificate must itself be at or
  above the minimum, so rotation genuinely re-issues authority rather than only confidentiality.
- **Whether flooding should be the default at all**, or opt-in, given what it costs in power and
  in visibility. Measure it with twenty craft before deciding.
- **Foreign craft names** change what `Presence` carries and what the chat log shows today. It is
  a protocol change and wants its own pass.
- **Key theft**, dictionaries, and commanding assets: deferred by decision, and each sits on top
  of the certificates above without changing them.
