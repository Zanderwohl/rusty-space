# Exotic Matters

A Bevy app plus two engine-free libraries.

See [AGENTS.md](AGENTS.md) for how to check your work and for the traps that have already cost
someone a day. This file is the conventions.

## Layout

| crate | what it is | may depend on |
|---|---|---|
| `crates/em-foundations` | orbital mechanics, reference frames, epochs | glam, serde, num-traits, scilib |
| `crates/em-sim` | simulation state and propagation | em-foundations; `bevy_ecs` only behind the `bevy` feature |
| `crates/em-ui` | Bevy-native menu widgets, in a palette the caller picks | bevy |
| `.` (`exotic-matters`) | the app: rendering, egui, persistence | anything |

The app is the workspace **root** package, so `assets/` resolves against `CARGO_MANIFEST_DIR`
as Bevy's default `AssetPlugin` expects.

`auth/` is a separate cargo workspace too, holding the two services that are neither product:
`lc-identity`, the identity broker, and `lc-admin`, the administration console over its
database. `lightcone/docs/16-identity.md` is both of them.

`web/` is **a separate cargo workspace** with its own lockfile, and is not a member of this
one — so `cargo test --workspace` does not reach it and `cargo update` here does not touch it.
That is deliberate: the site shares no code with either product, and a second lockfile is what
makes "the site and the game version independently" a mechanism rather than an intention. It
has its own CI job.

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

So a debug build already runs Bevy, glam and the rest fully optimized — only this
workspace's own code is unoptimized, and it is a thin layer over them. A release build
adds `lto = true` and `codegen-units = 1`, which costs many minutes of link time for a
speedup you will not notice while testing a change.

Use `cargo run` (and `cargo run --bin exotic-matters`). Reach for `--release` only when
actually profiling the simulation at high body counts or high time-warp.

## Comments

Comments are a cost. Write the minimum a competent reader with the code in front of them
actually needs.

Delete:

- restatements of the code — `// increment i`, `/// Returns the name.` on `fn name()`
- section banners and decorative rules
- narration of obvious control flow
- commented-out code; git remembers it
- design-document prose. The docs exist; link to them instead of inlining them.

Keep:

- **why**, where the why is not derivable: a constraint, a discarded alternative, a bug this
  shape prevents
- units, ranges and frames the type does not carry
- numerical hazards — cancellation, overflow, saturation, tolerance choices
- invariants a caller must uphold

One line is usually enough. A paragraph needs a reason. Module docs carry shared context so
items do not repeat it. Prefer making the code say it: a named constant, a smaller function,
or a better type removes the comment that would have explained it.

## File size

Cap a module at **1000 lines of code**, tests excluded. Past that, split by responsibility.

```bash
python3 tools/api_surface.py crates/<name>      # public surface, and the line counts
```

Run it at the end of any phase of work — the printout is the review artifact.

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

## The website

`web/` is the Lightcone Frontier site: axum, maud, SCSS compiled by `grass` at boot. Two
conventions that are easy to violate by habit:

- **Semantic classes only.** A class names what a thing *is* — `.post-meta`, `.tag-list` — never
  what it looks like. A class used once is a review item; most pages should add none, because
  rendered markdown is bare tags and `_base.scss` styles those.
- **No JavaScript.** Every page is a document a browser renders on arrival. `/play` is the one
  exception and all it does is hand over to the game client. Anything that wants real
  interactivity belongs in the client, which already has a WebGPU context.

The browser build of the game is staged by `tools/build-wasm.sh` and delivered from a CDN that
never overwrites anything — a new build is a new directory, so rollback is a pointer change.
Promotion is a row in the site's database, not a deploy.

[lightcone/docs/14-hosting.md](lightcone/docs/14-hosting.md) is why any of it is shaped that
way; [15-runbook.md](lightcone/docs/15-runbook.md) is the commands.

## The other project

`lightcone/` holds the design documents for a separate product — a relativistic sandbox MMO
built on the same shared crates. Players know it as **Lightcone Frontier**, and every
user-facing string says so: the site, the identity broker's pages, window titles, the menu.
Inside the repo it is `lightcone` — crates, binaries, containers, directories, the keychain
service and the config directory. Its code is `crates/lc-*`: `lc-spacetime`, `lc-world`,
`lc-store` and the `lc-client` app. Exotic Matters must never depend on anything from it:

```bash
cargo tree -p exotic-matters | grep -E '^\s*lc-'     # must be empty
```

Game-specific crates stay `lc-*`. Shared libraries stay `em-*` and must keep both products
building. Start at [lightcone/README.md](lightcone/README.md).
