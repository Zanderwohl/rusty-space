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
| shared | `crates/em-foundations`, `crates/em-sim`, `crates/em-render`, `crates/em-plot`, `crates/em-spectra` | same |

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
| [09-open-questions.md](docs/09-open-questions.md) | decisions made and still outstanding |
| [10-superluminal.md](docs/10-superluminal.md) | analysis only: what FTL trajectories would cost, and why the design is already paradox-free |
| [11-plotting.md](docs/11-plotting.md) | `em-plot`: the shared charting crate, and why decimation is the hard part |
| [12-buildout.md](docs/12-buildout.md) | **the plan.** Ten phases, each written to be started cold |
| [13-client-shell.md](docs/13-client-shell.md) | states, menus, windows, and why the game never pauses |
| [14-hosting.md](docs/14-hosting.md) | the website, the blog, and how the WASM build and its assets reach a browser |
| [15-runbook.md](docs/15-runbook.md) | the commands: build, publish, deploy, promote, roll back, and TLS |
| [16-identity.md](docs/16-identity.md) | accounts as a broker neither product owns, and how a socket proves who it is |
| [17-reconciliation.md](docs/17-reconciliation.md) | the four ways a client can differ from the server, and which of them is a mechanic |

## Status

Nothing is implemented. These are planning documents, and planning is finished — the blocking
decisions are made and [12-buildout.md](docs/12-buildout.md) is the order of work. Every number in them is a first
estimate; where a number drives a decision, the derivation is shown so it can be rechecked.
