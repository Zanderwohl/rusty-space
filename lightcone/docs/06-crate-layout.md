# Crate layout

## Target state

| crate | what it is | may depend on |
|---|---|---|
| `crates/em-foundations` | orbital mechanics, reference frames, epochs | glam, serde, num-traits, scilib |
| `crates/em-sim` | simulation state and propagation | em-foundations; `bevy_ecs` behind the `bevy` feature |
| `crates/em-render` | **new.** reusable Bevy rendering for orbital scenes | em-foundations, em-sim, bevy |
| `crates/em-plot` | **new.** charts, curves, heat maps; see [11-plotting.md](11-plotting.md) | glam; bevy and egui behind features |
| `crates/em-spectra` | **new.** bands, blackbody, extinction, colour, stellar relations, band-to-display mapping | serde only; no engine, no glam |
| `crates/lc-spacetime` | event coordinates, intervals, retarded time, worldlines | glam, serde; no engine |
| `crates/lc-world` | game rules, systems, structures, ships, resources, photometry | em-foundations, em-sim, em-spectra, lc-spacetime |
| `crates/lc-proto` | wire messages, serialisation, versioning | serde, lc-spacetime, lc-world types |
| `crates/lc-store` | Postgres schema, migrations, queries, the light-cone cursor | sqlx, lc-spacetime, lc-world |
| `crates/lc-server` | authoritative server binary | lc-store, lc-world, lc-proto, tokio |
| `crates/lc-client` | Bevy client, native and WASM | em-render, lc-world, lc-proto, bevy |
| `.` (`exotic-matters`) | the existing TTRPG app | em-foundations, em-sim, em-render, bevy |

The workspace already globs `crates/*`, so new crates are picked up with no manifest change.
The root package stays Exotic Matters, because `assets/` must resolve against
`CARGO_MANIFEST_DIR` for Bevy's default `AssetPlugin`. `lc-client` needs its own `assets/`
directory and its own asset path configuration; it cannot inherit the root's.

## Naming rule

`em-*` is a shared library used by both products. `lc-*` is game-specific.

**`exotic-matters` must never depend on an `lc-*` crate.** Check it the same way the
existing invariants are checked:

```bash
cargo tree -p exotic-matters | grep -i '^\s*lc-'            # must be empty
cargo tree -p em-foundations | grep -i bevy                 # must be empty
cargo tree -p em-sim --no-default-features | grep -i bevy   # must be empty
cargo tree -p em-plot --no-default-features | grep -i bevy  # must be empty
cargo tree -p lc-spacetime | grep -i bevy                   # must be empty
```

`lc-spacetime` stays engine-free for the same reason `em-foundations` does: the server links
it and the server must not link Bevy.

## What moves into `em-render`

Extracted from the app's `src/presentation/`, `src/camera/` and `src/catalog/`. The test of
whether a module belongs: **does it know any game or TTRPG rule?** If not, it moves.

| source | moves | notes |
|---|---|---|
| `src/presentation/render_space.rs` | yes | the Z-up to Y-up boundary and `ToRender`; already the only converter, and both products need exactly it |
| `src/presentation/body_mesh.rs`, `body_material.rs` | yes | sphere meshes and the body shader |
| `src/presentation/body_point.rs`, `body_point_material.rs` | yes | distant bodies as points |
| `src/presentation/local_starfield.rs`, `local_starfield_material.rs` | yes | background stars from a catalogue |
| `src/presentation/labels.rs` | yes | screen-space labels for world positions |
| `src/presentation/lights.rs` | yes | star as a light source |
| `src/presentation/rotation.rs` | yes | body spin applied to transforms |
| `src/presentation/chain_path.rs` | yes | trajectory polylines from `em_sim::trajectory::Path` |
| `src/presentation/celestial_markers.rs` | yes | generic orbital markers |
| `src/presentation/encounter_marker.rs`, `encounter_marker_material.rs` | judgment | encounter markers are patched-conic concepts, which `em-sim` owns, so they move |
| `src/camera/freecam.rs`, `planetarium.rs` | yes | controllers parameterised by scale |
| `src/catalog/` | **no, revised** | see below |
| `src/gui/` | no | egui panels encode Exotic Matters' workflows; the game needs different ones |

Everything that moves keeps its public API and gains a `Plugin` per subsystem, so the app
composes what it wants rather than getting an all-or-nothing plugin group.

### `src/catalog/` does not move

An earlier draft sent it to `em-render`. Phase 4 made that wrong: catalogue parsing is not a
rendering concern, and putting it in the render crate would have given the project two HYG
parsers, one for the starfield and one for world generation.

The responsibilities split three ways instead:

| concern | home |
|---|---|
| colour, temperature, blackbody, extinction | `em-spectra` |
| catalogue parsing, star identity, world data | `lc-world::sky`, behind `StarProvider` |
| drawing a list of stars it is handed | `em-render` |

`em-render` therefore parses nothing. Exotic Matters keeps `src/catalog/` as its own loader
and hands the result over, which is one fewer crate boundary to move and leaves the app
working unchanged.

Extraction order, one commit each, app building at every step:

1. `render_space` and `ToRender` — no dependents outside the app, smallest blast radius.
2. Materials and meshes.
3. The starfield and catalogue.
4. Cameras.
5. Markers and paths.

## `em-spectra`

Everything about light that is physics rather than game rule. Shared, because Exotic Matters
has stars to colour too and because a physics toolkit is worth more than a game feature.

```
bands.rs       the seven-band definition, centres, widths, BandMask
blackbody.rs   Planck, Wien, Stefan-Boltzmann, band-integrated emission
colour_index.rs  B-V to Teff (Ballesteros) and back; the reddening degeneracy
extinction.rs  A_lambda/A_V curves, reddening vectors, colour-colour geometry
cie.rs         CIE matching functions, XYZ, sRGB, the direct-assignment shortcut
mapping.rs     BandMapping: the 3 x BANDS display matrix, presets, bloom assignment
```

No engine, no ECS, no rendering — `em-render` and `em-plot` consume it, and so does
`lc-world`. `BANDS` is a compile-time constant here and nowhere else, so a change to the band
set is one edit and a format version bump.

The occultation integral of [04-stellar-photometry.md](04-stellar-photometry.md) is a
migration candidate. It is pure physics and nothing about it is game-specific, but it is new
and unproven, so it stays in `lc-world` until it has been used enough to know its shape.

## `lc-spacetime`

Deliberately small and dependency-light, because both the server and a WASM client link it
and it is on every hot path.

```
coord.rs       Coord, the 2^60 invariant, construction and conversion
interval.rs    interval2, Separation, precedes
worldline.rs   the Worldline trait, retarded and advanced time solvers
frame.rs       system-local <-> global conversions
units.rs       light-microsecond and microsecond newtypes
doppler.rs     shift and aberration
proper_time.rs gamma, tau integration, hyperbolic motion
```

No `bevy`, no `tokio`, no `sqlx`. `#![forbid(unsafe_code)]`, matching the other libraries.

## `lc-world`

Game rules and state transitions, engine-free so the server can run it headless and the
client can run the same code for prediction.

```
sky/           StarProvider, synthetic ids, the HYG importer, generation, metallicity
system.rs      System, wrapping em_sim::System, plus structures and ships
photometry.rs  the emission model of 04-stellar-photometry.md
shell.rs       Oort shell geometry, crossings, baking
ship.rs        worldlines, orders, proper time
economy.rs     resources, energy, construction
observer.rs    known-world snapshots, per-observer folds over received events
apply.rs       the single `apply(event) -> state delta` entry point
```

`apply.rs` matters more than the rest. **One function turns an event into a state change,
and both server and client call it.** Any rule implemented anywhere else is a rule the
client and server can disagree about.

## Shared determinism

The client predicts between events. Where a prediction affects a rule, it must match the
server bit-for-bit.

Rust's `f64` arithmetic is IEEE-754 and deterministic across platforms, but the standard
library's transcendentals are not: `sin`, `cos`, `exp`, `atan2` come from the platform libm,
and x86-64 Linux, aarch64 macOS and WASM disagree in the last ulp. Kepler solving uses
`sin` and `cos` in a loop, so the divergence compounds.

**Rule: `lc-world` and `lc-spacetime` use the `libm` crate for every transcendental, not
`std`.** It is a pure-Rust software implementation, identical everywhere. `em-foundations`
currently uses `std`; it either gains a `deterministic` feature that swaps in `libm`, or
`lc-world` wraps the calls it needs. Prefer the feature — duplicating Kepler solving to get
determinism would create exactly the second source of truth the project avoids elsewhere.

Cost is roughly 2-3x per transcendental call against a good platform libm. Measure before
assuming it matters; Kepler solves are not the bottleneck at these object counts.

## Build targets

| target | crates |
|---|---|
| `lc-server` native | lc-store, lc-world, lc-spacetime, lc-proto, tokio, sqlx |
| `lc-client` native | lc-client, em-render, em-plot, lc-world, lc-spacetime, lc-proto, bevy |
| `lc-client` wasm32-unknown-unknown | same, minus anything that touches the filesystem or threads |
| `exotic-matters` | unchanged, plus em-render |

The WASM build is the constraint that shapes the client. Keep `lc-client` free of
`std::fs`, `std::thread`, blocking IO, and any dependency that pulls them in. See
[07-rendering.md](07-rendering.md) and [08-networking.md](08-networking.md).

## Do not build release

The root `Cargo.toml` sets `opt-level = 3` for all dependencies in the dev profile, so a
debug build already runs Bevy and glam fully optimised. Release adds `lto = true` and
`codegen-units = 1`, which costs minutes of link time. This applies to the new crates too.
The exception is the WASM build, where size matters and `--release` with `opt-level = "s"`
plus `wasm-opt` is the only configuration worth shipping.
