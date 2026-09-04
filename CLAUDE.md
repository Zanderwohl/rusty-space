# Exotic Matters

A Bevy app plus two engine-free libraries.

## Layout

| crate | what it is | may depend on |
|---|---|---|
| `crates/em-foundations` | orbital mechanics, reference frames, epochs | glam, serde, num-traits, scilib |
| `crates/em-sim` | simulation state and propagation | em-foundations; `bevy_ecs` only behind the `bevy` feature |
| `.` (`exotic-matters`) | the app: rendering, egui, persistence | anything |

The app is the workspace **root** package, so `assets/` resolves against `CARGO_MANIFEST_DIR`
as Bevy's default `AssetPlugin` expects.

Two invariants worth checking after any structural change:

```bash
cargo tree -p em-foundations | grep -i bevy          # must be empty
cargo tree -p em-sim --no-default-features | grep -i bevy   # must be empty
cargo test -p em-sim --no-default-features           # the headless suite
```

## Don't build release

`Cargo.toml` sets `opt-level = 3` for **all dependencies** in the dev profile:

```toml
[profile.dev.package."*"]
opt-level = 3
```

So a debug build already runs Bevy, glam and the rest fully optimised — only this
workspace's own code is unoptimised, and it is a thin layer over them. A release build
adds `lto = true` and `codegen-units = 1`, which costs many minutes of link time for a
speedup you will not notice while testing a change.

Use `cargo run` (and `cargo run --bin exotic-matters`). Reach for `--release` only when
actually profiling the simulation at high body counts or high time-warp.

## Conventions

- **`em-foundations` is radians-only, without exception.** Degrees are a storage and
  display convention that stops at that crate's boundary. Element structs above it store
  degrees; callers convert once, on the way in.
- **Simulation space is right-handed and Z-up**, ecliptic of J2000, +X toward the vernal
  equinox. Bevy renders Y-up. `src/presentation/render_space.rs` is the only place that
  converts — do not open-code the swizzle.
- **Time is typed.** `Instant` counts seconds since J2000; `JulianDate` counts days from a
  different origin. They convert only through `From`. This exists because mixing them once
  put the whole solar system 28 days out of position.
- **`System` is the single source of truth for body state.** Entities are views holding a
  `BodyRef(BodyId)`; nothing per-body is also an ECS component.

## Orbital data

The bundled system is generated, not hand-maintained. Elements are least-squares fits
against JPL Horizons series, and every body records its own fit residual. The pipeline and
its pitfalls are in `docs/horizons-golden-vectors.md`, with scripts in `docs/scratch/`.
`tests/ephemeris.rs` pins positions and velocities against JPL.
