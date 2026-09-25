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
| [18-ui-style.md](docs/18-ui-style.md) | which toolkit a surface belongs to, and what it may do to the one behind it |
| [19-ship-fitting.md](docs/19-ship-fitting.md) | modules, energy as mass, the drive as a rocket, and refits |
| [20-solar-power.md](docs/20-solar-power.md) | hulls collect starlight: the shadow-area integral, the anchor, and segments of constant income |
| [21-library.md](docs/21-library.md) | the shelf of public-domain books, and the ereader window that costs the player nothing |
| [22-provenance.md](docs/22-provenance.md) | what a craft knows and how it came to know it: records, lineage, parallax, and the blind spots in a sky |
| [23-factions.md](docs/23-factions.md) | factions as keys rather than lists, rotation and traitors, relays over a network that moves, and what anything is called |
| [24-standing-instruments.md](docs/24-standing-instruments.md) | instruments that run while nobody is watching, what a craft's knowledge is stored as, and logs consumed into conclusions |
| [25-system-knowledge.md](docs/25-system-knowledge.md) | planets, their orbits and a system's plane as knowledge: how each is learned, what the System panel, the map and courses read, and how a player reports one system to another craft |
| [26-system-generation.md](docs/26-system-generation.md) | what a star's seed turns into: the disc, the ladder of feeding zones, air and water from one retention chain, moons of two origins, and the belts that are what never assembled |
| [27-console.md](docs/27-console.md) | commands typed after `/`, parsed and level-checked on the shard, and teleport as a worldline that jumps |
| [28-beauty-shots.md](docs/28-beauty-shots.md) | a photograph through the ship's telescope every ten seconds, beside the map: a second camera on the sky's own scene |
| [29-ship-form.md](docs/29-ship-form.md) | a ship as parts whose volumes are its capacities, the Mind at the root, refits as rounds paid for in energy, the voxel grid the server reasons with, and the editor as a third view |
| [30-the-field.md](docs/30-the-field.md) | the field as collector, radiator and shield: heat that goes as `T⁴`, the anchors, collapse, and what dies beside a ship that fails |
| [31-directed-energy.md](docs/31-directed-energy.md) | engines, radios, weapons and power lines as one order: apertures, exhaust from heat, spread against lead, and the star's gain |
| [32-ship-rendering.md](docs/32-ship-rendering.md) | the hull from a distance field, construction as a function of time, stateless drones, the field shader, and the order of work across 29–32 |
| [33-ring-habitats.md](docs/33-ring-habitats.md) | Orbitals and the Ringworld: one shape on two hosts, the solar-system demo, levels of detail at astronomical scale, and light delay across a spinning ring |

## Status

Phases 1a through 9 of [12-buildout.md](docs/12-buildout.md) are built — `crates/lc-*` is some
59 000 lines of Rust. `lc-spacetime` and `lc-world` are engine-free and tested, `lc-store` holds
the schema and the light-cone cursor, `lc-proto` is at wire version 27, `lc-server` is
authoritative, and `lc-client` runs native and in a browser. What is left of phase 9 is delivery
rather than the build: [14-hosting.md](docs/14-hosting.md) W4.

Phase 10 — the game — has not started. There are no resources, no deposits, no construction and
no replication; survey regimes and proper time are library code nothing calls yet. Much of what
exists instead was never in the plan — ship fitting, solar income, radio, the scenarios,
accounts, the library — which is why [12-buildout.md](docs/12-buildout.md) is the order of work
and not a record of it. It carries no completion markers; the code is the only status.

Every number in these documents is a first estimate until a phase has measured it; where a
number drives a decision, the derivation is shown so it can be rechecked.
