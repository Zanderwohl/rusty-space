# Working in this repository

Read [CLAUDE.md](CLAUDE.md) first — layout, conventions, the comment policy and the file-size
cap live there and are not repeated here. This file is the rest: how to check your work, and
the things that have already cost someone a day.

Design documents are the readable form of the argument and are kept current:
`lightcone/docs/` for the game, `docs/` for Exotic Matters. Read the one for the area you are
touching before changing it, and update it when the answer changes.

## Housekeeping

- **`cargo clean` when you finish.** `target/` reaches **70 GB** in this workspace. Nothing
  warns you.
- **Never `--release`.** Dependencies are already `opt-level = 3` in the dev profile; a release
  build costs many minutes of link time for a speedup you will not notice.
- **Commit promptly.** Worktrees get recycled and uncommitted work is gone for good.

## Two invariants, both enforced by CI

```bash
cargo tree -p em-foundations | grep -i bevy     # must be empty
cargo tree -p exotic-matters | grep -E '^\s*lc-' # must be empty
```

`em-foundations` is engine-free. Exotic Matters and Lightcone are two products on the same
shared crates (`em-*`); Lightcone's own crates (`lc-*`) may never be reached from the app.

## Checking your work

`python3 tools/api_surface.py crates/<name>` prints a crate's public surface and its line
counts. Run it at the end of a piece of work; the printout is the review artifact.

**WGSL cannot be asserted from a test, and a window nobody is watching proves nothing.** The
client photographs itself through the real pipeline:

```bash
cargo run -p lc-client --bin lightcone -- assets/catalogs/hygdata_v42.csv \
    --station rings:Saturn --panel system --rate 0 --shot /tmp/shot.png --frames 90
```

| flag | for |
|---|---|
| `--shot <path> --frames <n>` | photograph and quit |
| `--burst <n>` | photograph `n` **consecutive** frames — the only way to see a flicker |
| `--at <body>` / `--station <course>` | stand off a body, or start on a station |
| `--panel <name>` / `--tune` | open a panel |
| `--menu` | hold at the main menu, so `--shot` photographs that instead of the sky |
| `--signin` | hold at the sign-in modal, which draws over the menu and no action can reach |
| `--password` | hold at the password form, the one egui surface inside the menu |
| `--turn <deg>` / `--pitch <deg>` | turn the view, the only way to put something off screen |
| `--rate <n>` | clock multiplier; `0` freezes it, which makes frames comparable |

Most of what has gone wrong in the renderer was found this way and could not have been found
any other way.

The store's tests need PostgreSQL (`createdb lc_store`; `LC_STORE_URL` overrides). They
**skip** when they cannot reach one — keep it that way, so the suite passes without it.

## Traps

Each of these cost real time. None of them are visible from the code that hits them.

**Rendering**

- Projecting a *path* by projecting each point and dropping the ones behind the camera draws a
  **chord**: the two survivors either side of the gap get joined, and a ring seen from inside it
  acquires a straight line across the view that no ring has. Cut the segments at the camera
  plane instead — `em_ui::reticle::project_path`.
- `Camera::world_to_viewport` **errors** for anything behind the camera, so nothing built on it
  can point at what is behind you. Work in clip space and keep `w`: `clip.w` is `-view.z`, so
  behind the camera it is negative while `clip.x` keeps the sign of `view.x`. Dividing anyway
  mirrors the point through the centre. See `em_ui::reticle::place`.

- Depth is **reversed**. `clip.z = clip.w` is the *near* plane. Background geometry wants a
  tiny positive value, not zero — the buffer clears to zero and the test is strictly greater.
- `AlphaMode::Add` is *premultiplied*: `src + dst*(1-alpha)`. For pure additive the fragment
  must return **alpha 0**, or it overwrites and two coplanar meshes flicker on sort order.
- Render positions are f32 relative to the camera: about **six metres** at a hundred thousand
  kilometres. Never place the camera on a surface — an infinitely thin sheet containing the
  camera swings wildly from frame to frame.
- Two runs stopped at frame `n` and frame `n+1` are **not** consecutive frames. They have
  accumulated different wall time. Use `--burst`.

**`em-sim` and `em-foundations`**

- Derived columns — `parent`, `position`, `mu` — are empty until the first propagation.
  Reading them straight after `System::from_contents` gives zeros and no hierarchy.
- `System::mu(i)` is the `mu` of the orbit body `i` is *on*, i.e. `G(M_parent + M_i)`. To orbit
  *around* `i`, use `gravitational_constant() * mass(i)`.
- `Instant::to_j2000_seconds()`, not `seconds_since_j2000()`.
- The arena holds **one instant**. `System::position(i)`, `LocalSystem::body_position_ly` and
  friends read it; `propagate::state_at` and `LocalSystem::body_state_at` answer for any time
  without touching it. Mixing the two — a craft read at `t`, the body it orbits read out of the
  arena — turns a circular orbit into a wild ellipse. Nothing in `lc-world` or `lc-client`
  propagates any more; if you need a position, say which instant you mean.
- **A planet's frame is not inertial.** Earth turns eight degrees in nine days, so a "straight
  line past Earth" posed at J2000 is a curve by the time it arrives, and a flyby slower than
  30 km/s is Earth running into the craft rather than the reverse. Any test that predicts a
  chord, a miss distance or an impact angle has to be fast enough that the frame holds still —
  see `FLYBY_SPEED` in `em_sim::collision`.
- Sphere-of-influence radii scale with the **live** separation, so they breathe over an
  eccentric year: Earth's L2 is 1.476 million km at J2000 (near perihelion) and 1.501 at the
  mean distance. A published figure is the mean one.
- At a patched-conic join the craft is *exactly* on a boundary, so `influence::containing` is a
  coin toss and it comes up "the sphere you are leaving". Take the new primary from the
  crossing, as `em_sim::patch` does.

**egui**

- Interface rules live in `lightcone/docs/18-ui-style.md`: which toolkit a surface belongs to,
  one surface at a time, and why anything over another panel is opaque.
- **Bevy UI is retained**: a surface rebuilt every frame loses `Interaction`, so its buttons
  never show a hover. Key the rebuild on *what is drawn*, not on `Res::is_changed` — the menu
  backdrop writes `ResMut<Ui>` every frame, so that flag is always true.
- **Bevy UI orders by spawn**, so two systems spawning into one frame have no order between
  them — an overlay drawn by one lands *behind* the screen drawn by the other, interleaved with
  it. `em_ui::MenuUi::overlay` sets a `GlobalZIndex` for this reason. And a translucent panel
  over another of the same size reads as one muddled thing: a modal's panel wants full alpha.
- An overlay on `Order::Background` is painted *under* every panel and floating area, so the
  interface covers it. `Order::Foreground` is over all of them — keep such an overlay inside
  `ctx.available_rect()` so it does not draw on top of a docked panel.
- The default font has no U+2715 `✕` — it renders as a tofu box. U+00D7 `×` is fine.
- `add_enabled` wrapping a `SelectableLabel` reports clicks nobody made. A plain
  `selectable_label` does not.

**PostgreSQL**

- `numeric ^ 2` is exact but goes through `numeric_power`, which picks a display scale: `-1`
  comes back as `-1.0000000000000000`. Multiply instead — scale zero, and cheaper.
- Migrations need a **session advisory lock**. Without one, two processes both find the step
  table missing, both create it, and one dies on a duplicate key in `pg_type`.
- An index-only scan still checks the heap for every row until a **vacuum** marks pages
  all-visible, and the planner correctly prices it as no better than a bitmap scan until then.
  A plan that should be index-only and is not usually means no vacuum has run.
- `tokio-postgres` has no `jsonb` conversion for a Rust string. Send `text[]` and cast in the
  statement.

## When a test disagrees with the code

Assume the test premise is wrong about as often as the code is — most of the disagreements in
this repository so far have been the assertion, not the implementation. Numbers written from
memory (an orbital period, a threshold, a formula) are the usual culprit.

The exception is worth knowing: if you write a brute-force reference to check a fast path,
**derive it independently**. A reference written from the same mistaken idea as the thing it
checks will agree with it and prove nothing. One here did not agree only because the two
filtered their results differently, which is luck.
