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

## Blocking the first line of code

| question | options | blocks |
|---|---|---|
| Project name | `Lightcone` is a codename. Renaming later costs a `git mv` and a find-and-replace. | crate names, directory names |
| Coordinate unit | integer light-microsecond grid as specified, or `f64` light-seconds, or `i64` nanoseconds plus separate spatial units | `lc-spacetime`, the Postgres schema, everything downstream |
| Whether `em-foundations` gains a `deterministic` feature | feature-gate `libm`, or wrap the calls in `lc-world` | shared prediction; see [06-crate-layout.md](06-crate-layout.md) |

The coordinate unit is the one that is expensive to change later, because it is in the
database schema, the wire format and every stored coordinate. Decide it first.

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
