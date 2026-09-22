# Standing instruments and the star log

The pull of the game is that **the universe runs when you are not there**. A telescope put on a
year-long watch keeps watching while its owner sleeps; a relay keeps relaying; a survey keeps
sweeping. None of that can live in a client, which exists only while a player is looking at it.

So instruments run on the server, knowledge lives on the server, and the client holds a copy.
This document is that move, what gets stored, and how a star's raw log is turned into
conclusions and then thrown away.

## Where things run

| | before 11b | now |
|---|---|---|
| telescope duty | client, per frame | server, per tick, for every craft with a duty — `lc_world::knowledge::observatory` |
| detection and noise | client, from its copy of the catalogue | server, the same functions |
| a craft's knowledge | client `Session::knowledge`, lost on restart | server, per craft (`lc-server`'s `instruments`); the client holds a replica. Persisted in 11c |
| naming, duties | client edits its own knowledge | `Order::NameIt`, `Order::SetDuty`: applied by the server, echoed |
| reports sent and received | client builds and folds them | server writes them from what it holds and folds them on landing, signed in or not |
| charts at launch | client, at game entry | server, when a craft is first seen |

A client with no shard — a test, the headless snapshot — runs the same observatory itself. With
one, it runs nothing: two telescopes recording one sky would be worse than one.

## Duties are orders

`Order::SetDuty { duty }` joins burns and courses. The server records it against the craft, and
each tick advances every craft's duty exactly as `Session::tick_instruments` does now: exposure
is elapsed coordinate time, a sweep collects the fields finished since last tick, a watch takes
its sample at the end of each turn.

A duty persists in the craft's checkpoint alongside its motion, so a shard restart resumes a
sweep where it was rather than starting a new one.

**Cost.** At the design rate a tick is 438 coordinate seconds, so an all-sky sweep at a minute a
field finishes about seven fields a tick. The expensive part is not detection but deciding which
stars are in those fields: today that walks every star. On the server it walks every star *for
every surveying craft*, so it wants a per-craft table from field to stars, rebuilt when the craft
has moved far enough to change which field a star falls in — rarely, since that takes a
substantial fraction of the star's distance. Measure it at a hundred surveying craft before
optimizing anything else.

## The client's copy

The client never writes its own knowledge. It receives it:

- on joining, the craft's whole knowledge, in pages;
- every tick after, what was learned that tick.

Both are the same shape as a report, because they are one: **a report from yourself**, with no
hop added. `Knowledge::receive` already folds it idempotently, so a replayed page or a
reconnect costs nothing but bandwidth.

Everything the player does to knowledge is an order — name this, note that, relay this report,
take up that duty — and comes back through the same stream. It is the pattern flight already
uses, for the same reason: one implementation, and nothing to reconcile.

## What is stored

A craft's knowledge is three kinds of thing with very different shapes, and they want different
storage.

| kind | size | lifetime | store |
|---|---|---|---|
| **files**: bearings, claims, namings, notes, conclusions — per subject | bounded: bearings are a reservoir of sixteen per witness | as long as the craft exists | one row per `(craft, subject)`, the file as a blob, rewritten when it changes |
| **logs**: photometric samples | bounded only by the room aboard: a craft's data modules | until consumed into a conclusion | append-only rows, partitioned by time |
| **routes and keys** | small | as long as they are true | beside the keyring |

A row per craft per subject is about 1e7 rows for a thousand craft that have each surveyed ten
thousand stars, which Postgres holds without noticing. The logs are the only thing that could
grow without bound, and the next section is why they do not.

A blob rather than columns because the file is what is read and written, whole, and because its
shape will keep changing for a while. A format version on the row says which shape wrote it, and
`knowledge::formats` reads every retired one up to the current shape, with a test each, exactly as
`lc_ships` does for motion; only a file from a newer shard is refused.

### As built

| what | where |
|---|---|
| files | `lc_knowledge`, one row per craft per subject, postcard with `archive::KNOWLEDGE_FORMAT` — `sql/0010_knowledge.sql` |
| samples | `lc_samples`, partitioned by **learned** time, kept ready by the journal beside events and deliveries. Observation time is stored as the exact f64 it was stamped with: a sweep finishes a field at an instant that is not a whole microsecond |
| duty and report marks | the ship checkpoint, `persist::Saved::instruments`, save format 9 |
| what is written | only what changed since the last checkpoint: `Knowledge::take_changes` hands over the files touched, without their samples, the samples taken, and the samples consumed to delete. All of it, the ships and the bookmarks go in one transaction, and a failed one hands everything back to be written next time |
| what a client is sent | its craft's knowledge in byte-bounded pages, as `Learned` reports and `Logged` pages of its own samples, each under `lc_proto::FRAME_LIMIT`, which both ends of the websocket state |

One thing is true for now and will not stay true: **the store is not partitioned by shard**:
a shard loads every craft's knowledge and keeps what belongs to the craft it adopted, the same way
it already loads every ship.

### Data modules, and fullness

What a craft knows takes room aboard. A **data module** ([19-ship-fitting.md](19-ship-fitting.md))
holds `data_per_module`, and a craft's knowledge has a fullness against its total exactly as
stored energy does against storage.

The capacity is anchored so that the *files* — bearings, names, notes, conclusions — of a
thoroughly surveyed neighborhood fit comfortably, and the *logs* are what fill it. That puts the
pressure where the design wants it:

- **full, a craft stops recording raw samples**: the telescope keeps measuring and nothing keeps
  the lines, and the panel says so. Files are never dropped for room — losing what you know
  because you learned something else would be a worse mechanic than refusing to learn more;
- **consuming a log into a conclusion frees its room**, so processing is how a craft keeps
  watching;
- **"retain raw" costs room for as long as it is set**, which is what makes keeping a log a
  decision.

A craft with no data modules still has a small fixed onboard store, enough for its files and very
little else, so a ship stripped for speed still knows where it is.

## Logs become conclusions

A star log is raw lines: a flux, an error bar, an arrival time. At first a player looks at them
directly, as the telescope panel does now. They are not what anyone wants in the end. What a
player wants is:

> **hot Jupiter**, P = 3.2 d — 70%
> **structured swarm**, knee at 0.4 mHz — 30%

So logs are **processed**: periodically, or when enough data has accumulated to change the
answer, each subject's log is run through the analyses of
[05-observation.md](05-observation.md) — box least squares for transits, a periodogram for
rotation and pulsation, the first two moments and the spectral knee for populations — and what
comes out is a set of **conclusions**.

### What a conclusion is

| field | |
|---|---|
| hypotheses | each with a probability and its parameters: period, depth, element size, element count |
| evidence | the statistics it was drawn from: significance, number of transits seen, band mask |
| covering | the span of **emission** time the data describe, which needs the distance and says so if it does not have one |
| witness and observer | who read the log and whose log it was, and where the observer looked from — a conclusion is a record, and it carries provenance like one. Which instrument is not recorded yet: every craft carries the one telescope |

Conclusions are knowledge records. They travel in reports, and they are far smaller than the
logs they came from, which matters more than anything else about them: **a faction shares
conclusions, not logs**, because a transmission costs stored energy in proportion to its size
([23-factions.md](23-factions.md#cost)), and a log is enormous next to what was learned from it.

### Probabilities, and where the priors come from

A probability needs a prior. The game has an honest one available: the distributions the
generator draws planets and populations from *are* the population statistics of this galaxy. A
conclusion is a likelihood from the data times that prior, normalized over the hypotheses.

The goal is conclusions that are well calibrated — a 70% hot Jupiter a hot Jupiter seven times
in ten — which is a design choice worth making on purpose. The alternative, deliberately
miscalibrated priors, is a way to make some instruments or some analyses better than others
later.

It is a goal and not yet a fact. The planet generator the priors are measured from is a
placeholder the shipped game will not run on, so planet readings are marked provisional in the
code and beside each one in the panel. What is true today is that the prior and the world agree
on orientation: every generated system has a pole of its own, uniform over the sky, shared by its
planets and belts, and the prior integrates over exactly that.

This keeps the rule [05-observation.md](05-observation.md#survey-regimes) sets: the client
reports a measurement and its uncertainty, and the player decides what to believe. A conclusion
is a probability *with its evidence*, never a verdict. "Planet detected" is still never shown.

### Throwing the log away

Once a log has been consumed, the samples behind the conclusion are deleted. What survives is
the conclusion and the **sufficient statistics** of the data it used — sums, sums of squares,
the periodogram's peaks — so that later data updates it rather than starting over.

This is lossy on purpose, and the loss is a mechanic:

- a different analysis cannot be run later on data that is gone, so a player who suspects
  something the standard pipeline misses has to **keep the log**, and keeping a log costs
  storage — a thing a craft has only so much of;
- a relayed conclusion cannot be re-derived by its receiver, only trusted or not, which is the
  same question as trusting an old key.

A craft can mark a subject **retain raw**, and the pipeline leaves its log alone.

## Persisting what a craft is doing

The checkpoint in `lc_ships` grows to hold what a craft is committed to as well as where it is:
its duty, its reporting marks per recipient, and — once relays exist — the bundles it has
custody of. A shard that restarts resumes all of it.

### As built: reading a log

| what | where |
|---|---|
| the search | `knowledge::transit`: box least squares over period, phase and duration, bands combined by inverse variance, scatter beyond the error bars found from the median absolute deviation and allowed for |
| the probability | a marginal likelihood — the box's likelihood ratio averaged over every period, phase, duration and depth, weighted by a prior measured from the generator's own ladder (`sky::generate::ladder`) over the world's stars, with orientation integrated exactly. A planet the generator cannot make has zero prior and cannot be concluded |
| a lone deep dip | a planet too wide to transit twice in the log fits under a box at almost any period, and would win. Dips far out of the noise that no period repeats are set aside before the search, and a period is only believed if every transit it predicts that the log covers is there |
| when it is read | `Knowledge::due`: after 96 new samples and half as many again as last time, so all the reads of a long watch cost a few times the last. A shard reads one log a tick, taking craft in turn |
| nothing transiting | absence of evidence, not evidence of absence. Its probability is the chance of no planet times the log's **completeness** — the share of the transiting planets the generator makes, at every period, that the log would have found — and the rest is *a planet this log could not have found yet*. Two months on a red dwarf is a few percent; it takes a log as long as the generator's widest orbits to say much more |
| swarm or belts | from moments, which survive consumption whole: the mean dimming across the visible bands, the thermal-infrared glow beyond the star's own, the flicker's covariance at the shortest lag and the lag at which it has halved, which is half a crossing. The probability is a likelihood against the generator's systems seen from spread directions; a swarm's element size and orbit come from `emission::invert_moments`, against the cataloged star most like this one's brightness when there is a distance |
| settled | the leading transit hypothesis at 99%: a planet after three transits, or nothing transiting once completeness allows it |
| consumed | when settled, or when the craft is full — reading is how a full craft keeps watching — unless the subject is retained. Consumed for room with nothing settled, the transit search is lost and the next log is searched afresh |
| what is kept | `Digest`: the moments, the best completeness any log of the star reached, and for a settled planet the search's odds and folds at its period and two either side at its error. The samples go, from memory and, at the next checkpoint, from `lc_samples` |
| what travels | `Conclusion`, in a report's part, with its evidence, its covering and `discarded_s`. A replica drops its copy of the log when it hears its original did |

### As built: room

A byte here is a game unit — a fixed size per record, near what postcard writes — so counting is
cheap and does not move with an encoding. `data_per_module` is one year of a thirty-minute stare
in every band, about 2.1 MB; a craft with no data modules has `ONBOARD_DATA_BYTES`, 1 MiB, which
holds the charts, a first sweep and a couple of months of one star. The shard recounts a craft's
room when its loadout changes, after its log is read, and otherwise one craft a tick; samples keep
the count between recounts. The client shows the room and never enforces it: the shard is what
decides what was kept.

## Open

- **What a busy tick costs.** Measured on 2026-09-22 by `a_busy_tick_is_measured` in
  `lc-server/src/instruments.rs`: a hundred craft sweeping a two-thousand-star sky, each holding
  ten thousand files, all signed in, is 53 ms a tick in a debug build — whose own crates are
  unoptimized — against a 50 ms tick. Reports and client pages read only what changed since their
  mark, and logs due for reading are a queue, so neither grows with files held; a sweep still
  looks at every star in reach for every craft each tick. Measure again in a release build before
  deciding anything from it.
- **Reading is on the tick thread, deliberately for now.** A read is a search over thousands of
  periods, one log a tick. It is the first thing on the shard whose cost grows with how long a
  player has watched, and analyses will get more expensive; when they do it moves to a worker.
- **A craft with no shard does not read its logs.** The offline client runs the instruments but
  not the pipeline.

- **How often processing runs**, and whether it costs anything. Compute is free on the server
  but it need not be free in the game; a probe with no processor might only be able to forward
  logs, not conclude from them.
- **The hypothesis set.** Built: planets by size class, swarms, belts. Still to come as the
  generator makes them: eclipsing binaries (a binary's companion does not yet occult in the
  photometry) and swarms by structure (the generator makes only isotropic shells). A hypothesis
  for something that does not exist is a probability permanently near zero.
- **The catalogue in the client.** The client still holds every star's true position, because the
  sky view draws the light that arrives. A modified client can read it. That was true before this
  document and is not made worse by it, but it is the next thing to fix once knowledge is
  server-side: the sky could be drawn from arriving light without the client ever holding a
  distance.
