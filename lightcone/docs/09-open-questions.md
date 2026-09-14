# Open questions

Decisions not yet made, grouped by what they block. Each topic document carries its own
"Open" section; this is the index and the ordering.

## Resolved

| question | decision |
|---|---|
| Star proper motion | Planned, not implemented. Catalogue frozen; stars still get a `Worldline`, currently constant, so nothing reads a position as a field. |
| World boundary | Terminates. The galaxy fits inside 36 500 ly; a skybox of distant galaxies lies beyond it. A third coordinate tier is possible and not expected. |
| Multi-star systems | A binary is a barycentre with two children — a hierarchical decomposition `em-sim` already propagates. One shell per system, two emission sources, and coherent mutual eclipses on the analytic path. Contact binaries excluded from generation. |
| Shell radius | Immutable world geometry. |
| Swarm sub-populations | They exist and nest. One record per wave or per band; deficits add, so a meta-population needs no separate representation. |
| Oort and Kuiper generation | Scaled by stellar generation and metallicity, with metallicity synthesised from galactic kinematics since the catalogue lacks `[Fe/H]`. |
| Deposit granularity | Per-body totals. |
| Event ID allocation | `(shard, coordinate_time, sequence)`, snowflake-style. Locally generated and time-ordered whether or not sharding happens. |
| Von Neumann termination | Replication orders carry a generation TTL. Drift can corrupt the counter, producing self-perpetuating drifters, which is a mechanic rather than a bug. |
| Superluminal travel | Not built, not foreclosed. Three cheap signature decisions keep the option open; see [10-superluminal.md](10-superluminal.md). |

## Blocking the prototype

The test applied here is narrow: **would a wrong answer force a rewrite rather than an edit?**
That is true only of things named by everything else — types, identifiers, and stored formats.
Everything else is discoverable by building, and planning it further is waste.

Four items pass that test.

**All four are now decided. This section is kept as the record of why they were the ones that
mattered.**

### 1. The coordinate unit — RATIFIED

Integer light-microsecond grid with `c = 1`, as specified in
[01-spacetime.md](01-spacetime.md), versus `f64` light-seconds or some other pairing. It is in
the schema, the wire format, every stored coordinate, and the type that everything else names.
Nothing else on this list is as expensive to change.

Ratified: the integer light-microsecond grid with `c = 1`.

### 2. One time representation — DECIDED: one

Currently the design implies three: `em_foundations::Instant` as `f64` seconds since J2000 for
propagation, local system time as `f64` seconds from a per-system epoch, and `Coord::t` as
`i64` microseconds from the world origin.

Three origins for one quantity is exactly the shape of the failure that CLAUDE.md records —
"mixing them once put the whole solar system 28 days out of position" — and a fictional galaxy
makes it worse, because a fictional galaxy has no J2000 and `Instant`'s origin stops meaning
anything.

There is also a precision argument. `f64` absolute seconds holds up for decades and stops
holding up for millennia: at 1e9 s the ulp is 36 m of light travel, comfortably under the
300 m grid, but at 1.15e12 s — the 36 500-year world horizon — it is 73 km, well over it. A
long-lived server reaches that; at 8766x, 36 500 in-game years is 4.2 real years.

**Recommended: collapse to one.** `i64` microseconds is the only stored time. `f64` seconds
exists only as a locally computed *difference*, which is what Kepler propagation actually
consumes and where `f64` is precise. That removes the per-system epoch entirely and leaves one
conversion, in `lc-spacetime`, in the pattern `em_foundations::time` already uses.

Decided as recommended, on the grounds that this game runs far longer than Exotic Matters
does. `em-foundations` is not modified; `lc-spacetime` converts at the boundary and passes
`em-sim` an offset rather than an absolute epoch.

### 3. Star data behind a provider interface — DECIDED

Created by the decision that the shipped game is a fictional galaxy. World generation must not
parse a catalogue directly, and an HYG number must never become a `source_id`. See
[03-world-model.md](03-world-model.md). Free now; a data migration and a schema change later.

### 4. `BANDS` as a compile-time constant — DECIDED

One line, but it belongs in exactly one crate (`em-spectra`) and it is baked into the shell
file format, so the format needs a version field from its first write. Going from five bands
to seven was free in a document and would not have been free in a serialised asset.

## Explicitly not blocking

Listed so they do not get planned. Each is a local edit whenever it is faced.

| item | why it can wait |
|---|---|
| `deterministic` feature on `em-foundations` | a feature flag and a swap of `f64::sin` for `libm::sin`. It only matters once client prediction gates a rule, which is not the prototype. Previously classified as blocking; that was wrong. |
| `cube` versus PostGIS | an index choice behind a query interface |
| source BVH in Postgres or in memory | durable positions are needed either way |
| transport and wire format | a prototype runs in one process |
| every open item in [04-stellar-photometry.md](04-stellar-photometry.md) | refinements to a model whose shape is settled |
| path extinction for interstellar dust | **Deferred deliberately.** Nurseries and dense clouds are special zones and want their own thought. Occlusion stays on a source's own shell until then; the omission is recorded rather than papered over. |
| globular cluster shell overlap | the clamp already exists; how well it behaves at cluster densities is measurable, not predictable |

## Blocking the world model

| question | notes |
|---|---|
| Playable volume | How many stars, out to what distance. Sets the source count, the BVH size, and the maximum light delay. Bounded above by the 36 500 ly coordinate invariant, and expected to be far smaller. |
| Drift rate, TTL depth, branching factor | The three numbers that tune von Neumann expansion and drifters. All need calibrating against how long a player should take to notice and to respond. |
| Metallicity synthesis | The correlation between kinematics and `[Fe/H]` is real and strong; the mapping still needs writing and the resource yield curve calibrating. |

## Blocking the event store

| question | notes |
|---|---|
| `cube` versus PostGIS | `cube` is contrib and adequate for bounding-box pruning; PostGIS is heavier and gives real spatial operators. |
| Partition cadence | One in-game month is 3.6 real hours. Partition maintenance must be automated either way. |
| Retention and archival | When an event becomes unreachable, and whether archival must stay reversible for replays. |

## Blocking photometry

| question | notes |
|---|---|
| Coherent/statistical threshold | Currently "when `m` approaches 1 the events stop being separable". It is also observational — a better telescope separates more — and the two criteria have not been reconciled. |
| Caustic edge term | A thin ring's turning latitudes carry an integrable spike that a level-5 shell smears. An analytic correction is cheaper than raising the whole shell's level, and is unwritten. |
| Optical depth | Deficits are summed linearly, which fails as a swarm approaches full coverage — exactly the end state the game is about. Switch to `1 - exp(-tau)` past a few percent. |
| Non-Poisson flicker | Real swarms have resonances, gaps and clumps, so the noise is correlated and its spectrum says more than the current model admits. |
| The `m ~ 1` transition band | Neither the Gaussian nor the Poisson branch of the flicker synthesiser is right there, and a Kuiper analogue sits in it. Generating the true event train is affordable at `m ~ 1`; decide whether to. |
| Limb darkening | Bundle Claret tables, or fit a two-parameter function of `Teff`. |
| Band set | Four bands assumed, including a thermal IR band so waste heat is visible. More bands cost linearly in baked shell size and nothing on the analytic path. |
| Reflected light and phase curves | 1e-5 of stellar flux at best. Probably out; it is an information channel if in. |

## Blocking the client

| question | notes |
|---|---|
| Threading in WASM | Cross-origin isolation breaks embeds. Single-threaded Bevy until profiling says otherwise. |
| Interferometry displays | `u-v` coverage and correlation views are charts, so they belong to `em-plot`, but they need a place in the UI. |
| Dust versus swarm appearance | Dust is chromatic and a swarm is grey. The shader probably needs two looks, which is the visual form of the photometric diagnostic. |
| Presenting an unavailable band | Masking to zero makes a scene look dark rather than uninstrumented, and the player needs the difference to decide what to build. |

## Blocking the server

Deferred by decision: these are settled when the features they belong to are designed, not in
advance. Each is local to one component.

| question | notes |
|---|---|
| Transport | WebTransport availability decides whether browser and native share one transport or two. |
| Wire format | `postcard` or `bincode`, plus a versioning scheme, since clients will lag deploys. |
| Where the light-cone cursor lives | `lc-store` needs a database; the client wants the traversal logic. Likely splits into `lc-spacetime`. |
| Rate limiting | A scripted client can emit intents at any rate. |

## Observation, still open

| question | notes |
|---|---|
| Interstellar VLBI data shipping | Recordings travel as cargo, as transmitted signal, or both. Radio is recordable and optical is not, so 21 cm is the only band this works in. |
| `u-v` coverage model | How partial coverage degrades an image, and what a correlator costs to build and run. |
| Frequency as a game surface | 1420 MHz costs 15x in power and is where everyone listens; the 3-30 GHz window is free and private. How much of this to expose as a player choice rather than a constant. |

## Design questions with no technical blocker

These change what the game is, not what the code must do first.

- **Does a player ever get more than one ship they directly inhabit?** The premise says one.
  Fleets are autonomous. Confirm before building a UI that assumes either.
- **What stops a von Neumann fleet from consuming the world?** Energy and mass conservation
  first; a hard object cap as a backstop. An expansion 40 ly out cannot be recalled in under
  40 in-game years, which is the interesting part, but it has to terminate.
- **Is there combat, and does it matter?** Interception falls out of the physics. Whether it
  is a loop the game supports deliberately is separate.
- **What does a new player do in their first hour?** One hour is one in-game year: enough to
  establish a presence in one system and see a signal leave it, not enough to reach another
  star. The onboarding has to work inside that budget.
- **Persistence across wipes.** A world whose events accumulate forever eventually has a
  light horizon full of dead civilisations, which is either the best feature or an
  unmaintainable archive.

## Method

Where a number drives a decision, the derivation belongs in the document next to the
decision, so it can be rechecked when an assumption changes. Every number currently in these
documents is a first estimate and none has been measured against an implementation.
