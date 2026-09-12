# Open questions

Decisions not yet made, grouped by what they block. Each topic document carries its own
"Open" section; this is the index and the ordering.

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
| Multi-star systems | The catalogue is full of binaries. `em-sim` propagates about a primary; close binaries need a hierarchical decomposition. Excluding them removes a large fraction of real stars. |
| Star proper motion | Frozen stars make interstellar retarded-time solves exact and cheap. Moving stars are correct, and Barnard's Star moves visibly within a single session at 8766x. |
| Playable volume | How many stars, out to what distance. Sets the source count, the BVH size, and the maximum light delay. |
| Shell radius rules | Fixed by stellar mass, or mutable by sufficiently large construction. |
| Deposit granularity | Per-body totals, or per-site with surface positions. |

## Blocking the event store

| question | notes |
|---|---|
| `cube` versus PostGIS | `cube` is contrib and adequate for bounding-box pruning; PostGIS is heavier and gives real spatial operators. |
| Source index location | In Postgres, or an in-memory BVH rebuilt at start with Postgres holding only durable positions. |
| Partition cadence | One in-game month is 3.6 real hours. Partition maintenance must be automated either way. |
| Retention and archival | When an event becomes unreachable, and whether archival must stay reversible for replays. |

## Blocking photometry

| question | notes |
|---|---|
| Analytic/baked threshold | 2 degrees of angular half-width is a guess; it should be measured. |
| Limb darkening | Bundle Claret tables, or fit a two-parameter function of `Teff`. |
| Band set | Four bands assumed, including a thermal IR band so waste heat is visible. More bands cost linearly in baked shell size and nothing on the analytic path. |
| Reflected light and phase curves | 1e-5 of stellar flux at best. Probably out; it is an information channel if in. |

## Blocking the client

| question | notes |
|---|---|
| WebGL2 support | Recommended: no. Storage buffers and compute are wanted, and the fallback is a second code path. |
| Threading in WASM | Cross-origin isolation breaks embeds. Single-threaded Bevy until profiling says otherwise. |
| Retarded-time sampling granularity | Per object is correct; per spatial cell is cheaper and the error is sub-pixel at distance. |
| God view exposure | Development-only, spectator-only, or never for players. |
| Tone mapping | Stellar flux spans many orders of magnitude; a linear mapping shows the Sun or the sky, not both. |

## Blocking the server

| question | notes |
|---|---|
| Transport | WebTransport availability decides whether browser and native share one transport or two. |
| Wire format | `postcard` or `bincode`, plus a versioning scheme, since clients will lag deploys. |
| Where the light-cone cursor lives | `lc-store` needs a database; the client wants the traversal logic. Likely splits into `lc-spacetime`. |
| Rate limiting | A scripted client can emit intents at any rate. |

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
