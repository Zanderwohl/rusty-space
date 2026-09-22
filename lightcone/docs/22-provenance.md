# Provenance

The premise of the game is that information travels at `c`, and the premise has a corollary
the rest of these documents assume without ever stating: **nobody has a view of the world.**
A player has a pile of measurements. Each was taken somewhere, at some time, by some
instrument, and reached them by some route. What they call the universe is a fold over that
pile, and another player folding a different pile is not wrong.

[05-observation.md](05-observation.md) is what an instrument measures.
[03-world-model.md](03-world-model.md) names the "known-world snapshot" and leaves it at that.
This document is the shape of the pile: what a record is, what a belief made of records is,
how records move between craft, and what a craft that has not looked at something knows about
it, which is nothing.

## The rule

**A star nobody aboard has detected is absent.** Not dimmed, not grayed out, not listed
without a distance: absent. The catalogue is the world; it is not what anyone knows of the
world, and the client must never read it where a player can see the answer.

This was the thing most obviously wrong before this document existed. The client held 6000
catalogue stars, drew all of them on the map at their true positions, listed the nearest forty
in the telescope panel, and kept exactly one light curve — which it threw away the moment the
telescope moved. Knowledge was free and memory was not, which is precisely backwards.

## What a record is

Four kinds, and the distinctions between them are load-bearing.

| record | what it says | carries |
|---|---|---|
| sighting | something was detected in this direction, this bright | witness, arrival time, observer position, bearing and its sigma, band, flux and its sigma |
| sample | the flux of a known source changed by this much | witness, arrival time, signed deficit, sigma |
| claim | somebody states a distance | witness, the distance, when they said it |
| naming | somebody calls it something | witness, the name, whether it was chosen or assigned, when |
| lineage | how any of the above got here | a hop per handover: from, to, sent, received |

A **witness** is whoever took the measurement — a ship, a probe, a telescope, a charting
office nobody has met. It is not whoever holds the record. `Knowledge::owner` is the holder;
every record says who made it, and the two differ for everything that arrived by radio.

A **hop** is one handover. A sighting a probe made and relayed twice has two hops, and the
craft holding it knows it is third-hand without having to be told separately.

**Arrival time is recorded; emission time is derived.** This is the part that surprises. A
telescope knows exactly when the light landed on it. When the light *left* is the arrival time
minus the distance, and the distance is a measurement that may not exist yet — so a curve of a
star whose parallax nobody has taken is plotted against arrival, and the interface says so. An
epoch is not a property of a measurement. It is a conclusion drawn from two of them.

## Distance is the whole mechanic

One bearing is a direction. It stays a direction forever, however long you stare, from however
big a mirror.

Two bearings taken a baseline `B` apart differ by `B / d` radians, and that difference is the
only way anything here learns how far away anything else is. So:

- A ship parked at a station measures no distances at all.
- A ship in a wide orbit measures them over a fraction of a year, as its own orbit swings it
  across the baseline.
- A ship under way measures them in hours, because a crossing at a fifth of `c` opens a
  baseline of light-days in a day.
- Two craft a light-year apart measure in one exchange what one craft would need a century of
  orbiting to reach — which is what makes a second ship worth more than a second telescope.

Precision follows from the same formula. With angular error `sigma`, a distance `d` comes out
to `sigma_d = d^2 sigma / B`: quadratic in range, linear in the error, and improved only by
moving further. Three consequences worth keeping:

| | |
|---|---|
| a chart's error grows with range | so the far edge of a charted volume is the vague part, and it is vague in *depth* rather than in direction |
| a lower bound is a real result | no parallax found over baseline `B` means the star is further than about `B / 2 sigma`, which is information and is shown as such |
| luminosity is downstream of distance | flux is measured; `L = 4 pi d^2 F` is inferred, so a star with no parallax has no known output, and therefore no known kind |

### How it is solved

Least squares over every bearing held, in a frame whose `z` is the mean bearing, as a
**centered regression of transverse position on slope**. The obvious formulation — the normal
system `sum (I - u u^T) x = sum (I - u u^T) p` — is numerically hopeless here: its smallest
eigenvalue *is* the parallax squared, which at an AU of baseline over 25 light-years is 1e-12
of the others, and inverting that in f64 returns noise. Centering the slopes puts the same
information in small numbers that subtract cleanly.

Bearings are kept per star per witness, capped, and the cap drops **the closest pair** rather
than the oldest. A reservoir that kept the most recent sixteen would throw away the baseline
and with it the distance; sixteen well-spread bearings measure a parallax as well as a
thousand.

## A sky has blind spots

Two things stop a survey seeing something, and only one of them is depth.

**Coverage.** A telescope looks at one field at a time. A sweep visits fields in a fixed order
and each gets its dwell, so what has been covered is a function of how long the instrument has
been at it. A star is found when the sweep reaches the patch of sky it happens to be in — which
is why the sky fills in gradually and in a pattern, rather than appearing when a panel opens.
An all-sky pass at two degrees a field and a minute a dwell is about a week of in-game time.

**Glare.** A fainter source inside a brighter one's scattered-light halo is not detected at all.
With a scatter fraction `k` and resolution `theta`, the faint source is lost within

```
theta_block = theta * sqrt(k * F_bright / F_faint)
```

and the same formula covers both cases that matter:

| geometry | ratio | blind radius, 4 m^2 mirror |
|---|---|---|
| a neighboring star, 4 ly off, against one at 100 ly | 6e2 | under a milliarcsecond — a close pair, not a blind spot |
| the sun the telescope is orbiting, at 1 AU, against a star at 100 ly | 4e13 | a few degrees |
| the same sun against something genuinely faint | 1e16+ | tens of degrees |

So a ship inside a system has a hole in its sky around its own star, and the hole is bigger for
fainter targets. It is not a rule anybody wrote down; it falls out of one constant. And it
moves: the star's direction from a ship in orbit sweeps right round over a year, so the way to
fill the hole is to survey the same sky at a different time from a different place — which is
what observatories actually do.

Both are beaten by the same thing: **resolution is a baseline, not an aperture.** Instruments
combining into one image resolve `lambda / B` for the widest separation `B` between elements,
so a swarm spread over ten kilometers sees into a glare a single four-square-meter mirror
cannot, and a swarm spread over an AU is another four orders of magnitude past that. The
collecting area adds at the same time, which is depth. This is why telescope swarms are a
build target rather than a bigger number.

What depth is *not* is the binding constraint nearby. A four square meter mirror clears the
detection threshold on a sun-like star most of the way across the galaxy in a minute of
exposure. Inside a few hundred light-years a survey is limited by where it has pointed and
what is in the way, exactly as [05-observation.md](05-observation.md) says.

## Duties

An instrument does one thing at a time, and which thing decides what its owner can learn.

| duty | what it is for | what it costs |
|---|---|---|
| stare | one star, the whole exposure: the deepest curve and, from a moving ship, a parallax | everything else in the sky |
| sweep | fields in order across a region or the whole sky: finds what nobody has detected | depth per target, as `sqrt(N)` |
| watch | a rotation of targets, one dwell each: several systems under observation for years | depth again, and a curve full of gaps |

A watch is the swarm-monitoring regime. Every target is sampled every `dwell * targets` of
coordinate time, which is an unevenly sampled series with holes in it — which is why the period
finders in [05-observation.md](05-observation.md) are Lomb-Scargle and BLS rather than an FFT.

Exposure is **elapsed coordinate time**, never a number typed into a panel and never one sample
per rendered frame. A measurement labeled with an integration it did not get is a lie about its
own error bars, and at the design rate a frame is two minutes of in-game time, so the difference
is not subtle.

## Nothing has a name

A star does not come with a name any more than it comes with a distance. **A name is
information about a star, with a witness on it**: somebody called it something, and may have
told somebody else.

| where a name comes from | witness | shown as |
|---|---|---|
| the charts a ship launched with | the charting office | what the office called it |
| the crew | this ship | what the crew call it |
| a report from another craft | whoever coined it, with the hops it came through | their name, until this ship picks its own |
| nobody yet | the instrument that found it | a designation: the ecliptic bearing it was discovered along |

Which name a craft goes by is a fold like any other: a name somebody *chose* beats a
designation, this craft's own beats somebody else's, and the later statement beats the earlier.
The others stay on file, because "they call it Hearthlight and we call it the Kettle" is a fact
about a conversation, and losing it would lose the conversation.

**A designation is written at discovery and then fixed.** It is the bearing the source was found
along, in ecliptic degrees — recomputing it as the ship moved would give a catalogue number that
drifted, which is no use for talking about.

The catalogue's names survive exactly as its row numbers do: in
`sky::Provenance`, next to the key, never read by anything a player sees. `CatalogueStar` has no
`name` field to reach for by accident. When the shipped game moves to an authored galaxy, the
catalogue's names go with the catalogue and nothing else changes — which is the same argument
[03-world-model.md](03-world-model.md) makes about identity.

Two things this leaves open, and both belong with factions rather than here:

- **A faction name is a shared name.** The mechanism is already the one above: a faction
  relaying its catalogue is a witness whose namings everyone holds. What is missing is the rule
  about whose name wins on a shared screen, which is a question about the faction, not the star.
- **Cross-identification.** Two craft agreeing that their records are of the same star is done
  today by the synthetic star id, which is an engine convenience: real observers match positions
  and brightnesses, and two crews with poor parallaxes could reasonably disagree about whether
  they are looking at the same thing. Worth revisiting when a faction's catalogue is merged
  rather than copied.

## Beliefs, and where they are drawn

A belief is a fold over one star's records: the latest bearing, the best distance, how bright,
how many sightings by how many witnesses, the shortest route any of it took, and when this craft
learned of it.

Two rules about which distance wins.

**A measurement of your own beats a claim, whatever the error bars.** One is a thing this craft
can check and the other is a thing it was told. When a craft triangulates a star it had only
been told about, the chart stops being what it believes — and the difference between them is
visible on the map as a mark that moves.

**A claim is held on its witness's name.** That is what makes a charting office, a faction
catalogue, and a probe reporting a conclusion rather than its raw data all the same mechanism,
and what will make a lying faction possible without any new machinery.

The map draws **believed positions**, which are not true positions. A star charted at a percent
of its range sits a percent off, and a star with no distance is not on the map at all: it has a
direction and no place to be. The sky view is where it is visible — light arrives whether or not
anyone has identified its source — and the telescope is what turns it from a light into a star.

## Starting from nothing

A new ship is not issued the sky. It is issued the **charts of the volume it launched from**:
claims from a charting office it will never meet, one hop of lineage, error growing with range,
and nothing at all past the edge. Everything beyond that is sky the player surveys or is told
about.

This is a game decision as much as a physical one. A player who starts with 6000 stars has
nothing to do with a telescope; a player who starts with nothing has no reason to fly anywhere.
Twenty light-years of somebody else's parallax program is a map with an edge on it, and an
edge is the thing that makes a frontier.

## Moving records between craft

A report is what one craft sends another: everything it has **learned** since some time, by its
own clock, rather than everything measured since then — a decade-old sighting relayed yesterday
is news to whoever is hearing it now.

Receiving one stamps every item with a hop and folds it in. Two properties make this safe to do
automatically, which is what a faction needs:

- **Idempotent.** The same sighting arriving twice by two routes is one sighting; a series only
  grows at its end, so replaying it adds nothing.
- **Attributed.** Nothing loses its witness by being passed on. A craft can always say who
  measured something and how many hands it went through, which is what lets a player distrust a
  source without distrusting everything.

Automatic forwarding — a faction's relays passing on whatever they receive — needs no further
mechanism than a rule about when to call `report` and who to aim it at. The bandwidth question
is real and is not answered here: a full report of a surveyed sky is megabytes, and a beam has a
data rate. Sending conclusions instead of measurements is what a claim is for.

## Reports on the air

A report is a transmission, not a conversation, and the difference is the whole of how it
behaves.

| | a message | a report |
|---|---|---|
| event kind | `MESSAGE` | `REPORT` |
| what it is | something somebody said | what somebody has learned |
| filed as | a line in a transcript, kept for ever | records folded into the receiver's knowledge |
| acknowledged | yes, and automatically if asked | never — two craft reporting to each other would trade surveys for ever |
| sealed | to one addressee | the same rule, the same keyring |
| overheard | an eavesdropper sees that something was sent | an eavesdropper **reads an open report**, which is why sealing one is worth the key |

Everything else it inherits: it is aimed or shouted, it crosses at `c`, the shard schedules it
to whoever the beam covers, and it arrives when its light does.

**A report carries conclusions, not logs.** Its parts hold sightings, claims, names, orbits and
conclusions — never a craft's photometric samples, which are enormous next to what was learned
from them and cost energy to send in proportion. A conclusion keeps the name of the observer
whose log it came from, and a receiver holds each observer's separately, with where it looked
from: a transit seen from one direction may be invisible from another, so two can disagree and
both be right. What a craft believes is a fold over them.

**Reports drain a backlog.** The shard keeps a mark per sender per recipient — how far through
its own learning the sender has told them — and each transmission carries the oldest
[`ENTRIES_PER_REPORT`](../../crates/lc-world/src/knowledge/report.rs) systems past that mark.
The mark is a time and a system, so a volume of charts issued at one instant still pages, and
nothing learned after a report was sent goes in it. The mark moves only once the transmission
exists.

**A report is sent, not delivered.** It is not acknowledged, and the mark says what was sent: a
beamed report that lands under its recipient's noise floor is light that went past, and nothing
resends it. That is the game working — radio is lossy and a sender cannot know — and resending
is a player's decision, made by sending again after marking the recipient as needing it.
**The shard writes the report**, from the knowledge it holds for the sender; a client that wrote
its own could report anything it liked. A craft with nothing new to say is refused with
`NothingNew`, which is the correct amount of radio traffic for having learned nothing.

**The report itself is not filed as a conversation**, and no transcript row is written for it:
the receiving client says in a notification who told it about how many stars. A transcript is
read long after everything in it has arrived; a report is folded the moment it lands and its
content lives in the receiver's knowledge from then on. Putting surveys in `lc_messages` would
hand every sign-in a backlog of somebody else's astrometry.

A report lands on the shard, whether or not anybody is signed in to the craft it lands on, and
is written down with the rest of that craft's knowledge. **A report still in flight when a shard
stops is replayed on boot** from the journal's deliveries, which are kept until their light has
passed everyone, so it lands on time on the shard that comes back.

## What is built

In `lc-world::knowledge`, engine-free and tested:

| module | holds |
|---|---|
| `knowledge` | `File`, `Belief`, `Knowledge`, and `Report` with its `Entry` per system and `Part` per subject |
| `knowledge::record` | `Witness`, `Hop`, `Sighting`, `Sample`, `Series`, `Claim`, `Naming`, `Orbit` |
| `knowledge::subject` | `Subject` — star, body, population, craft — and `BodyId` |
| `knowledge::names` | designations, discovery designations, and planet letters against the `SPACING` table |
| `knowledge::astrometry` | bearings, centroid precision, the triangulation, `Distance` |
| `knowledge::survey` | `Optics`, detection and glare, the `Sweep` and its field order, `Duty` |

On the shard, since phase 11b: every craft's `Knowledge` and telescope live in
`lc-server::instruments` and run whether or not anybody is flying them, and a report lands there.
A client holds a copy, paged to it as `Outbound::Learned` and, for its own logs, `Outbound::Logged`;
it names, points and reports by order. Noise is seeded from `(witness, star, arrival time)` on both
sides, so a shard recomputes exactly what an instrument saw, which is what
[05-observation.md](05-observation.md) asks for. The client reads a distance only from belief —
measured with its error and the angle its baseline subtended, stated on somebody's word, a floor,
or a bearing only — and names the home system from the crew's namings; nothing a player sees
reads a catalogue name.

## What is not, and in what order

Most of what was on this list is now planned in detail as phase 11 of
[12-buildout.md](12-buildout.md): subjects beyond stars, instruments and knowledge on the server,
persistence, conclusions from logs ([24-standing-instruments.md](24-standing-instruments.md)),
factions and relays ([23-factions.md](23-factions.md)). Two items are not part of it:

1. **Navigation on beliefs.** Deferred by decision. A crossing still aims at the catalogue
   position. It ought to aim at the believed one and arrive off by the error on it — which for a
   charted star is far wider than the shell it is aiming into, so the crossing has to refine the
   fix as its own baseline opens. That is a mechanic of its own: the approach where you find out
   the star is not quite where you thought.
2. **Instruments that are not the ship.** `Optics::joined` models a swarm acting as one and
   nothing builds one. Telescopes as structures, with their own worldlines and their own witness
   ids, are what turn every number here into something to spend resources on.
