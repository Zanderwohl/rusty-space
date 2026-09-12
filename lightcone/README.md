# Lightcone

Design and architecture documents for a relativistic sandbox MMO built on this repo's
orbital mechanics crates.

`Lightcone` is a working codename. It appears in directory names, crate prefixes (`lc-`)
and type names throughout these documents. Renaming costs one `git mv` and one
find-and-replace; nothing in the Exotic Matters app depends on it.

## Relationship to Exotic Matters

This monorepo holds two products.

| | Exotic Matters | Lightcone |
|---|---|---|
| what | trajectory tool for a TTRPG | multiplayer sandbox game |
| root | `.` (workspace root package), `docs/` | `crates/lc-*`, `lightcone/` |
| shared | `crates/em-foundations`, `crates/em-sim`, `crates/em-render` (to be extracted) | same |

The `em-*` crates are shared libraries. Neither product owns them; changes to them must
keep both building. Everything under `lc-*` is game-specific and Exotic Matters must never
depend on it.

`docs/` is Exotic Matters documentation. `lightcone/docs/` is this project's. They do not
cross-reference except where a shared crate is the subject.

## Documents

| file | subject |
|---|---|
| [00-overview.md](docs/00-overview.md) | what the game is, scope, non-goals |
| [01-spacetime.md](docs/01-spacetime.md) | coordinates, units, the SR model, what relativity does to gameplay |
| [02-event-store.md](docs/02-event-store.md) | PostgreSQL schema, light-cone queries, delivery scheduling |
| [03-world-model.md](docs/03-world-model.md) | systems, bodies, structures, the Oort shell as a partition boundary |
| [04-stellar-photometry.md](docs/04-stellar-photometry.md) | emission shells, occlusion, why transits are analytic and swarms are baked |
| [05-observation.md](docs/05-observation.md) | telescopes, light curves, FFT detection, radio and tight-beam |
| [06-crate-layout.md](docs/06-crate-layout.md) | crate boundaries, dependency rules, what gets extracted from the app |
| [07-rendering.md](docs/07-rendering.md) | Bevy, scale tiers, WASM and native shader constraints, view modes |
| [08-networking.md](docs/08-networking.md) | transport, authority, tick model, interest management |
| [09-open-questions.md](docs/09-open-questions.md) | decisions not yet made |

## Status

Nothing is implemented. These are planning documents. Every number in them is a first
estimate; where a number drives a decision, the derivation is shown so it can be rechecked.
