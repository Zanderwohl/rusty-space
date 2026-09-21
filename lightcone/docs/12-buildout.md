# Buildout plan

Eleven phases. The organizing constraint is not effort but **context**: this project is larger
than any one working session can hold, so each phase is written to be started cold.

Every phase states what must already exist, what it delivers, how you know it is done, what it
explicitly does not do, and which documents to read. A phase's reading list is the whole
reading list — a session working on phase 3 should not need phases 7 through 10 in its head.

Phases 1 to 6 are the prototype. **Phase 3 is the go/no-go**: it is the smallest program that
either demonstrates the premise or shows that it does not work.

## Dependency shape

```
  1a lc-spacetime ─┐
                   ├─> 2 photometry ─> 3 PREMISE ─┐
  1b em-spectra ───┘                              ├─> 6 client ─> 7 store ─> 8 server ─> 9 wasm ─> 10 game
                   └─> 4 the sky ─────────────────┤
  5 em-render + em-plot ──────────────────────────┘
```

1a and 1b are independent of each other. 5 is independent of everything and touches only the
existing app, so it can run in parallel throughout.

---

## Phase 1a — `lc-spacetime`

**Before:** nothing. First code written.

**Deliver:** `crates/lc-spacetime`, engine-free.

| module | contents |
|---|---|
| `coord.rs` | `Coord { t, x, y, z }` in `i64` microseconds and light-microseconds, the `2^60` construction invariant |
| `interval.rs` | `interval2` in `i128`, `Separation`, `precedes` |
| `worldline.rs` | the `Worldline` trait, `retarded_times` returning a `SmallVec`, `is_subluminal` |
| `frame.rs` | system-local `f64` meters to and from the global grid; `propagation_time` to `em_foundations::Instant` |
| `units.rs` | newtypes, and no arithmetic that can mix a duration with an instant |
| `doppler.rs`, `proper_time.rs` | shift, aberration, gamma, hyperbolic motion |

**Done when:**

- `cargo test -p lc-spacetime` passes, including: the interval classification table; the
  single-root property over randomised sub-luminal worldlines; the straight-line retarded-time
  closed form checked against bisection; `i128` non-overflow at the `2^60` bound.
- `cargo tree -p lc-spacetime | grep -i bevy` is empty.
- `em-foundations` is untouched.

**Do not:** build the light-cone cursor, touch a database, or add superluminal support beyond
the signature already specified.

**Read:** [01-spacetime.md](01-spacetime.md). Skim [10-superluminal.md](10-superluminal.md) for
why `retarded_times` returns a collection.

---

## Phase 1b — `em-spectra`

**Before:** nothing. Independent of 1a.

**Deliver:** `crates/em-spectra`, engine-free, per [06-crate-layout.md](06-crate-layout.md).
Bands, blackbody, extinction curves, color index, CIE conversion, `BandMapping` and presets.
`BANDS` is defined here and nowhere else.

**Done when:**

- Ballesteros returns 5778 K for `B-V = 0.65` and 3950 K for `+1.40`.
- Extinction ratios reproduce the documented transmission table.
- XYZ to sRGB round-trips; the direct-assignment shortcut is measurably more saturated than
  the CIE path, and both are available.
- A reddening vector and a stellar locus can be computed and their slopes compared.
- No engine in the dependency tree.

**Do not:** render anything, or implement the occultation integral.

**Read:** the Bands section of [04-stellar-photometry.md](04-stellar-photometry.md).

---

## Phase 2 — Photometry

**Before:** 1a, 1b.

**Deliver:** the photometry half of `crates/lc-world`. `Population`, `EmissionModel`, the
occultation integral, both flicker regimes, the analytic occluder path with quadratic limb
darkening, and the baked shell with its versioned format.

**Done when:**

- The occultation integral agrees with direct Monte Carlo orbit sampling within Poisson error,
  isotropic and inclination-banded. **The verification already run in the design work is the
  test suite** — port it rather than reinventing it.
- The moment inversion recovers its inputs: from mean deficit, rms flicker and knee
  frequency, return element size and count. The documented case must return 1.0e12 m² and
  1.50e6 elements.
- Both flicker branches are exercised: Gaussian at `m = 8.1`, Poisson event train at
  `m = 1.4e-3`, and the sparse branch reports the single-event depth rather than the mean.
- A shell bakes, round-trips through its format, and interpolates.

**Do not:** add observers, telescopes, or noise. This phase produces true radiance, not
measurements.

**Read:** [04-stellar-photometry.md](04-stellar-photometry.md) in full. It is the densest
document and this is the phase it exists for.

---

## Phase 3 — The premise

**Before:** 2.

**Deliver:** the observation half of `lc-world`. Observers with worldlines, instruments with
apertures and band masks, photon noise from a seeded generator, the received light curve.

**Done when one headless integration test passes**, and it is the game:

1. Build a system with a star, a planet and a swarm.
2. Place an observer 30 light-years away.
3. Advance the clock. The observer's measured curve matches the system's state at the
   **retarded** time, not the current one.
4. Change the swarm at time `T`. The observer's curve is unchanged until `T + 30 years` and
   changes after.
5. An observer with a V-only instrument cannot distinguish the swarm from a dust cloud of the
   same optical depth. One with two bands can.

If step 4 or step 5 will not pass, the design is wrong and this is where it is cheapest to find
out.

Step 5 was originally written as "B/V/R/I usually can, and fails for a red star; one with K
can", and both halves were wrong. Phase 1b measured that K does not rescue the blind zone, and
phase 3 found that the step conflates two different measurements:

| measurement | what breaks the ambiguity |
|---|---|
| the occultation **varies**, so there is an unobscured baseline | the deficit *ratio* between any two bands: 1.0 for a solid occulter, 1.32 in `B/V` for dust. One band cannot form a ratio at any exposure; two can, and no locus is involved |
| the obscuration is **steady**, so there is no baseline | the stellar-locus degeneracy, which has a blind zone at late K to early M that more bands do not fix |

Both are real and they are different problems. The first is the phase 3 test; the second lives
in `em_spectra::extinction` and was settled in phase 1b.

**Do not:** render, network, or persist. Everything here is a function call.

**Read:** [05-observation.md](05-observation.md), and [01-spacetime.md](01-spacetime.md) for
the retarded-time solver.

---

## Phase 4 — The sky

**Before:** 1a, 1b. Can overlap 2 and 3.

**Deliver:** the star data provider interface, the HYG importer behind it, synthetic stable
IDs, seeded procedural system generation producing `em_sim::system::BodyDef`, metallicity
synthesised from galactic kinematics, and the Oort, Kuiper and belt populations that come with
every system.

**Done when:**

- 120 000 catalogue stars load through the provider with synthetic IDs, and no HYG number
  appears anywhere but a provenance field.
- The same seed produces a byte-identical system, twice, in separate processes.
- Binaries generate as a barycenter with two children and propagate.
- A second provider implementation exists, even if it only returns three hand-written stars.
  The interface is not proven by one implementation.

**Do not:** author a fictional galaxy. That is later, and this phase exists to make it possible.

**Read:** [03-world-model.md](03-world-model.md).

---

## Phase 5 — `em-render` and `em-plot`

**Before:** nothing. Runs in parallel with everything; touches the existing app.

**Deliver:** the five-step extraction in [06-crate-layout.md](06-crate-layout.md), and
`em-plot` core with min/max decimation, scales, color maps and an SVG test backend.

**Revised during the phase.** The five-step list assumed materials, meshes, cameras, markers
and paths all extract. Measuring the coupling showed they do not: material and geometry
definitions have zero app imports, and every Bevy *system* has three to ten, because each one
knows how this application lays out its ECS. `em-render` therefore ships definitions and each
host writes its own systems; the contract between them waits for `lc-client` to exist so it
has two callers to be shaped by. See [06-crate-layout.md](06-crate-layout.md).

**Done when:**

- Exotic Matters builds and runs unchanged after every extraction commit.
- `em-render` contains no game or TTRPG rule.
- A two-million-sample curve renders into 800 pixels with the envelope preserved, verified
  against a reference SVG, and a one-sample-wide transit survives every zoom level.
- `cargo tree -p em-plot --no-default-features | grep -i bevy` is empty.

**Do not:** build charts the game has not asked for.

**Read:** [06-crate-layout.md](06-crate-layout.md) and [11-plotting.md](11-plotting.md).

---

## Phase 6 — The client

**Before:** 3, 4, 5.

**Deliver:** `crates/lc-client`, single process, no server, no network. Scale tiers,
camera-relative rendering, retarded-time sampling per the distance rule, band-to-display
presets, the population envelope shader, tone mapping with glow, and a light-curve panel.

**Done when you can show it to someone:** fly to a star, point a telescope, watch a transit in
the curve, switch to the thermal preset, and see a swarm that was invisible in natural color.

**Do not:** add WASM, networking, or god view. God view is compiled out of this build from the
start rather than added and later removed.

**Read:** [07-rendering.md](07-rendering.md).

---

## Phase 7 — The event store

**Before:** 3. Can overlap 6.

**Deliver:** `crates/lc-store`. Schema, partitioning, the causality functions, delivery
scheduling, and the light-cone cursor.

**Done when:**

- Events survive a restart and a partition rollover.
- Delivery lookup is a single B-tree range scan, demonstrated by an execution plan rather than
  by assertion.
- The cursor yields receptions in arrival order without sorting the full result, and early
  termination does the work of the terminated case and not more.
- `lc_precedes` agrees with `lc-spacetime`'s `precedes` on a randomised corpus. Two
  implementations of causality is one too many, and this is the test that keeps them honest.

**Do not:** shard.

**Read:** [02-event-store.md](02-event-store.md).

---

## Phase 8 — Server and protocol

**Before:** 7.

**Deliver:** `lc-proto`, `lc-server`, the tick loop, intent validation, and the single send
gate.

**Done when:** two clients connect to one server, one acts, and the other learns about it at
light delay and not before — with a test that asserts the negative case, that nothing arrives
early. Every outbound message passes through one function and there is no second emit path.

**Do not:** optimize. Correctness of the filter is the whole deliverable.

**Read:** [08-networking.md](08-networking.md).

---

## Phase 9 — WASM

**Before:** 6, 8.

**Deliver:** the browser build. WebGPU only, single-threaded Bevy, transport over WebTransport
or WebSocket, god view absent from the binary.

**Done when:** it runs in a browser, refuses WebGL2 with a message that names the two supported
routes, and the download is small enough to be worth measuring.

**Read:** the WASM and portability sections of [07-rendering.md](07-rendering.md), and
[08-networking.md](08-networking.md). The build pipeline, the CDN layout and the four
things in the client that are not wasm-ready are in [14-hosting.md](14-hosting.md), whose W3
and W4 are this phase's delivery half and can be worked separately.

---

## Phase 10 — The game

**Before:** 9.

**Deliver:** resources and per-body deposits, energy, construction, ships with orders and
proper time, telescope survey regimes, and von Neumann replication with generation TTL and
drift.

This phase is deliberately least specified. By the time it starts, six phases of contact with
the design will have changed what it should contain, and planning it now would be planning the
wrong thing.

**Read:** [03-world-model.md](03-world-model.md), and whatever the previous nine phases have
added to it.

---

## Phase 11 — What a craft knows

Knowledge, factions and relays. Seven steps, in this order because each changes something the
next one stores or sends:

```
  11a subjects ─> 11b server instruments ─> 11c persistence ─┬─> 11d conclusions
                                                             └─> 11e factions ─> 11f relays
                                           11g naming and notes from the map (after 11a; best after 11e)
```

The reporting, naming and survey machinery these build on already exists and is described in
[22-provenance.md](22-provenance.md).

### 11a — Subjects

**Built.** Notes, which the done-when below mentions, arrive in 11g; everything else is in
`lc_world::knowledge`. Protocol version 29, because a report's shape changed.

**Before:** `lc_world::knowledge`, keyed by `StarId`.

**Deliver:** a `Subject` key — star, planet, small body, population, craft — and knowledge keyed
by it. Assigned names as **rules** evaluated against the craft's own namings, so "Kettle b"
follows the star when the star is renamed. Planet letters by expected slot, frozen once assigned,
with gaps left and second letters appended when a slot is taken. Report entries carry a star's bodies with it.

**Done when:** a report about a star carries its planets' namings and notes in the same entry, a
receiver sees the planets named after *its* name for the star, and renaming the star renames
them.

**Do not:** move anything to the server, or persist. This is a change of key, and it goes first
because every later step stores or sends it.

**Read:** [22-provenance.md](22-provenance.md), and "What things are called" in
[23-factions.md](23-factions.md).

### 11b — Instruments on the server

**Built.** Protocol version 30. Notes are still 11g; persistence is 11c, so a shard restart
still loses every craft's knowledge.

**Before:** 11a.

**Deliver:** `Order::SetDuty`; duties advanced by the server for every craft each tick; each
craft's `Knowledge` held by the server; the client's knowledge a replica fed by a report from
itself. Naming, notes and reporting become orders. Reports are sent and folded by the server.

**Done when:** a craft set to sweep, whose client then disconnects for an in-game month,
reconnects to find the month's detections — and a report sent to it while it was away has been
folded in.

**Do not:** persist across a shard restart. That is 11c, and keeping it separate means this step
is testable against `journal::Memory` alone.

**Read:** [24-standing-instruments.md](24-standing-instruments.md), and `lc-server`'s `radio.rs`
for how an order becomes an event.

### 11c — Persistence

**Built.** Save format 6; schema step `0010_knowledge`. A checkpoint writes only what changed.

**Before:** 11b.

**Deliver:** a row per `(craft, subject)` holding the file as a versioned blob; an append-only,
time-partitioned table of photometric samples; duties and reporting marks in the ship
checkpoint. Everything restored on shard start.

**Done when:** a shard restarted mid-sweep resumes the sweep, and a craft's map after the restart
is the map it had before.

**Do not:** consume logs. Samples accumulate until 11d.

**Read:** [24-standing-instruments.md](24-standing-instruments.md), [02-event-store.md](02-event-store.md)
for how `lc-store` applies and tests its schema.

### 11d — Conclusions

**Before:** 11c.

**Deliver:** the data module and knowledge fullness — a `Loadout` count, `data_per_module`, and
logs that stop recording when full. A processing pass that turns a subject's log into conclusions — hypotheses with
probabilities, parameters and evidence — and keeps sufficient statistics while deleting the
samples consumed. A per-subject "retain raw" flag. Conclusions travel in reports.

**Done when:** a planet the generator placed around a nearby star comes out as the most probable
hypothesis after enough transits, with its period within error, and the samples that produced it
are gone.

**Do not:** invent hypotheses the generator cannot produce.

**Read:** [05-observation.md](05-observation.md) for the analyses, [04-stellar-photometry.md](04-stellar-photometry.md)
for what the signals look like, [24-standing-instruments.md](24-standing-instruments.md).

### 11e — Factions

**Before:** 11c.

**Deliver:** founding and naming a faction; membership of several at once, with a channel per
faction held; key generations and rotation, each rotation sent per member sealed to their ship
key; certificates for `invite`, `grant`, `rotate` and `generation`; invitations in the chat log;
the faction channel, redacted by generation held at arrival. The minimum generation an asset
accepts is modelled now, on the craft themselves, so that assets inherit it rather than
retrofitting it.

**Done when:** a traitor excluded by rotation can still read everything sealed to the old
generation, cannot read anything sealed to the new one, and a loyal member far away is seen
sending in the old generation until the new key reaches them.

**Do not:** build relays, or anything about commanding assets.

**Read:** [23-factions.md](23-factions.md), [05-observation.md](05-observation.md) on keys.

### 11f — Relays

**Before:** 11e.

**Deliver:** first, transmission cost — every transmission draws stored energy by link-budget
physics times `transmit_gain`, at a range the sender chooses, and that range sets its fan-out.
Then bundles with origin, TTL and path; manual relay of any received report; automatic
flooding of faction bundles with echo-only-new and split horizon; custody; routing tables of
light-time cost advertised in reports, aged as they travel.

**Done when:** a report from one member reaches a member it was never in range of, through two
relays, exactly once — and the lineage on its records names every hop.

**Do not:** build contact-graph routing from predicted trajectories until distance-vector has
been measured and found wanting. Do not relay across factions automatically, ever: a craft in
two factions forwards each one's traffic only to that faction. Swarms as relay nodes wait for
non-player craft to exist.

**Read:** "Relays" in [23-factions.md](23-factions.md).

### 11g — Naming and notes, from anywhere

**Before:** 11a; best after 11e, so notes can be shared.

**Deliver:** a subject card — name, notes thread, provenance — opened from a pick in the map, the
sky, the telescope panel or the system panel, for every kind of subject including craft outside
the faction. Transcript lines for reports: "Kestrel: told you about 64 stars". Every transmission
carries the name its sender calls itself; `Presence` stops carrying account names.

**Done when:** a player can click a planet on the map, rename it, leave a note, and a faction
member elsewhere sees both arrive — at light speed — with the note marked as written before they
could have known of it.

**Do not:** build dictionaries.

**Read:** [23-factions.md](23-factions.md).

---

## What is deferred past all of this

| item | why |
|---|---|
| path extinction and nurseries | special zones, wanting their own design pass |
| an authored fictional galaxy | phase 4 makes it possible; the content is its own project |
| sharding | phase 8 does not preclude it; nothing should until measurements demand it |
| interstellar VLBI | a late-game mechanic that needs the whole stack first |
| superluminal travel | not built, not foreclosed; see [10-superluminal.md](10-superluminal.md) |
| dictionaries between factions | sits on top of namings carrying their witness; see [23-factions.md](23-factions.md) |
| key theft and forged identity | breaks the one guarantee factions keep, so it wants designing as its own mechanic |
| commanding assets | certificates carry the capability already; there are no assets to command yet |
| navigation on believed positions | deferred by decision; see [22-provenance.md](22-provenance.md) |
| data rate limits | deferred by decision; the transmission-cost formula already takes the size |
| out-of-game invitations | need the identity broker to carry an invite between accounts |

## Where a design error is most likely to surface

| phase | risk |
|---|---|
| 3 | the premise. If light-delayed observation is not fun, nothing later fixes it. |
| 7 | the event store under real volume. The source-not-event decomposition is sound on paper and has never met a million rows. |
| 6 | scale tiers and retarded-time granularity. The `f32` reduction is well understood; whether the three tiers compose visually is not. |

Phase 3 is cheap and answers the largest question. That is why it is third and not tenth.
