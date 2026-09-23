# Server tick lag on `claude/lightcone-system-knowledge`

The shard spikes on this branch and not on master. The cause is work the branch added to the
tick thread: an angles-only orbit fit that costs many ticks and never leaves its queue, a
checkpoint whose volume the survey multiplied, a report backlog that only grows, and a survey
that rebuilds a whole system's light every tick to measure seven bodies of it.

Paths below are as they stand on that branch. Estimates are from reading the code; nothing was
profiled. Phase 0 exists so every later phase is judged by a number rather than by feel.

The tick is 50 ms (`TICK_MS`, 20 Hz). At the design rate one tick is about 438 coordinate
seconds, so a survey (`SURVEY_DWELL_S` = 60) visits about seven bodies a tick.

## Status

Measured with `a_surveying_shard_is_measured` (`crates/lc-server/src/instruments.rs`): ten craft
surveying one system for 1,400 ticks, workspace code optimized.

| after | mean tick | ticks over 50 ms | worst |
|---|---|---|---|
| Phase 1 only | 269 ms | 1,308 | 3,544 ms, all `fit_orbits` |
| Phase 2 | 3.0 ms | 0 | 3.5 ms, survey |
| Phase 6 | 1.15 ms | 0 | 1.6 ms, survey |

Done: 0, 1, 2, 4, 5 (backlog and empty pages), 6, and from 7 `next_due`, the prior off the
tick, surveyed systems pinned, and indexed star lookups.

Open: 3 (a cheaper fit — off the tick now, so this is throughput and CPU, not lag); from 5,
refreshing once per subject in `receive` and paging in one pass; from 7, a maintained
`retained_subjects`; and the pre-existing list at the end.

Taking the checkpoint snapshot costs about 1 ms for ten surveying craft (721 files, 1.2 MB).
It stays on the tick, so the checkpoint is still the shard at one tick.

## Phase 0 — Measure the tick

Nothing here fixes anything. It makes the rest checkable.

- Time each stage of `Server::tick` (`crates/lc-server/src/server.rs:~405`): `resync_systems`,
  `run_instruments` (split into the survey loop, `read_logs`, `fit_orbits`), `land_reports`,
  `tell_learned`, `journal.write`, and `checkpoint` in `bin/lightcone-server.rs`.
- Keep a rolling max and p99 per stage; report them in the status readout (`status.rs`) and log
  any tick over `TICK_MS` with its stage breakdown.
- Confirm the shard is built with `lc-world`, `em-sim` and `em-spectra` optimized. The dev
  profile only optimizes dependencies outside the workspace, so a debug shard runs all of the
  code below at 10–50× its release cost. If deployment uses a dev build, add
  `[profile.dev.package.lc-world] opt-level = 3` (and the other two) before judging anything.

**Done when** a surveying craft on a local shard shows which stage owns each overrun.

## Phase 1 — Stop the fit queue running forever

The single change most likely to end the spikes.

`Knowledge::unfitted` (`crates/lc-world/src/knowledge/primary.rs:55`) offers a body while
`newest > stated`. A fit that fails states nothing, so the body stays eligible with identical
inputs on every tick; `tried` only reorders. And every survey visit re-arms some body, so the one
fit per tick is spent permanently while anyone surveys.

- Eligible only when `newest > stated.max(tried)`: a failed body waits for a new look.
- Require real growth before a refit: at least `REFIT_LOOKS` new sightings since the last
  attempt, or the arc lengthened by some fraction of itself. Store the look count at the last
  attempt beside `tried`.
- Drop the `self.fit(id, true)` after `fit_orbit` (`instruments.rs:220`). A fit changes no
  samples, and that call is a full pass over every file.

Test: a body whose fit is rivalled is tried once, then not again until sighted; a system whose
bodies all fail goes quiet.

**Done when** Phase 0 shows `fit_orbits` idle on most ticks of a long survey.

## Phase 2 — Take the fit off the tick thread

Even run rarely, one `arc::fit` is 0.2–1.5 s: a 256×256 range grid (`arc.rs:45`), per candidate
primary (up to four), each point scored against up to 16 bearings with a Kepler solve apiece and
no early exit, times up to 25 unwrappings, then `spread`'s 5 × 40 walks.

- Split `fit_orbit` into *gather* (tick thread: the looks, `star_ly`, the resolved primary
  orbits, as owned values) and *solve* (pure, `Send`).
- Run the solve on `tokio::task::spawn_blocking` or a small worker pool. Keep at most one in
  flight per craft; a completed result is filed by the tick that finds it, through the same
  path `fit_orbit` files by today. Discard a result whose inputs a newer fit has overtaken.
- Deterministic across sides: the solve takes only its snapshot, so the result does not depend
  on which tick files it. Stamp it with the snapshot's time, not the filing time.
- Persist nothing extra: a fit in flight at a checkpoint is simply redone after a restart.

**Done when** no tick's `fit_orbits` stage exceeds a millisecond regardless of fit outcome.

## Phase 3 — Make the fit cheaper

Off the thread it no longer lags the tick, but it still decides how fast a system is learned and
how many cores a busy shard burns.

- **Warm start.** When the craft already holds its own astrometric orbit for the body, skip the
  grid and `settle` from it. Fall back to the grid only if the settled residual is worse than
  the old one.
- **Coarse grid, then refine.** 64×64, then refine the best few cells, instead of 256×256.
- **Bound the grid.** Pass the current eighth-best residual as the bound to `score` once eight
  are held, so the first unwrapping stops early on a hopeless point.
- **De-duplicate in O(1).** Replace the linear `found.iter().position(..)` (`arc.rs:840`) with a
  `(i / APART, j / APART)` cell array holding the best candidate per cell.
- **`spread`.** Start each walk at the previous sigma, or at `residual / sqrt(weight)`, rather
  than `1e-9` (about twenty wasted steps each), and bisect; or replace the walks with a
  finite-difference Fisher matrix.
- **Hoist `basis(pole)`** out of `Fitted::at` so it is built once per `residual`.
- **Frames.** `members(star)` (`mod.rs:485`) scans every file held. Use
  `files.range(..)` over that star's subjects. Resolve each candidate primary's orbit once per
  fit, not per look, through a lean `placed_offset` that reads only the orbit chain (no name,
  kind, colors or mass).

Pin the fit's answers before touching it: the existing arc tests plus a snapshot of fitted
elements for a few fixed look sets, compared within tolerance.

**Done when** a fit from a warm start is a few milliseconds and a cold fit well under 100 ms,
with the pinned answers unchanged.

## Phase 4 — Checkpoint off the tick

`checkpoint()` (`bin/lightcone-server.rs:326`) runs every 400 ticks and is awaited inside the
tick loop. A survey touches every body file each rotation, so it encodes and upserts up to ~234
files per surveying craft, and `save_files` clones every blob again.

- Take the changes on the tick thread (cheap: `take_changes` is a drain), then move encoding and
  the transaction onto a spawned task. Hand rows over by value; drop the `.clone()`s in
  `lc_store::knowledge::save_files`.
- Keep failure semantics: the task reports failure back over a channel and the tick calls
  `untake_knowledge` / `redirty`. Never start a second checkpoint while one is in flight.
- The final checkpoint at shutdown still awaits.

**Done when** the checkpoint tick costs no more than its neighbors.

## Phase 5 — Bound the report backlog

`Backlog` (`crates/lc-world/src/knowledge/report.rs:228`) never removes an entry. Each survey
visit adds about two, the star measurement one a tick. `report_upto` from `Mark::default()`, on
every connect and every report to a new recipient, walks all of it, and `page()` / `report_for`
rebuild and re-serialize from scratch on each of up to eight halvings.

- Key the backlog by record, keeping only its latest time: remove the old entry when a record is
  replaced or decimated, as `Recent::touch` already does.
- Page in one pass: serialize entries in order and stop at `PAGE_BYTES` / `REPORT_LIMIT`, instead
  of halving the entry count and rebuilding.
- In `page()` (`instruments.rs:95`), check `through` before serializing, so an empty report is
  not encoded for every client every tick.
- `receive` / `fold` (`report.rs:336`) refreshes the belief after every record. Use
  non-refreshing inner variants and refresh once per subject.

**Done when** backlog size is flat over a long survey, and a connect after an hour of surveying
costs the same as after a minute.

## Phase 6 — The survey's steady cost

`survey_between` (`observatory.rs:262`) calls `visit::all` → `LocalSystem::drawables_at`
(`system.rs:187`) for every body every tick. Per craft per tick, over ~234 bodies: a
propagation each, `worlds::of`, `climate::of` (client-only), a linear `rings::for_body`, a name
clone, and 7 bands × 2 Simpson integrals of Planck (about 108k `exp`); then two sorts.

- Cache per system, once, what never changes for a body: world, rings, kind, spin, name, and
  each body's thermal band radiance at its effective temperature. Cache the star's per-band
  `flux_from` on `LocalSystem`, not per body.
- Give the survey its own lean path that propagates positions and applies phase and range to the
  cached terms. `drawables_at` stays as it is for the client.
- Sort once.
- The brightness order needs every body's flux, so this path still touches every body each
  tick; after the caching that is a propagation and a few multiplies each. If Phase 0 still shows
  it, reorder only once per rotation.

Pin: the survey tests in `observatory.rs` and `visit.rs` must produce identical sightings.

## Phase 7 — Smaller per-tick waste

- `Knowledge::due()` (`conclusion.rs:330`) builds a Vec for `.first()`; add `next_due()`.
- `retained_subjects()` scans every file per client per tick and is cloned per page attempt
  (`instruments.rs:328`). Keep a set updated in `retain_raw`.
- `Prior::measure` (`prior.rs:69`) runs 1,500 full `generate::system_for` for `.populations`
  alone, on the tick thread at the first log read. Add `generate::populations_of(star)` and
  build the prior eagerly in `load_world` on a blocking thread.
- `found_by_transit` (`planets.rs:123`) builds a whole system to read planet periods; use
  `generate::planets_of` for a generated star.
- `World::sweep` pins only occupied systems. Pin surveyed stars too, so more than
  `MOST_LOADED` surveys cannot evict and rebuild each other every second.

## Not regressions, still worth doing

Unchanged from master, but they grow with the fleet:

- `World::system_at` scans every catalog star per craft per tick. Check the craft's current
  system first, then a spatial grid.
- `World::star_at` should use `star_by_id`; `Sky::target` should use `Sky::index_of`.
- `fix()`, under Stare and Watch, builds a source for every catalog star per turn, up to 64 turns
  a tick. The survey already avoids this with `Sky::source_of`; the glare test needs only stars
  near the target's line of sight.

## Order

Phase 0, then 1 (small, likely enough on its own), then 2 and 4 (the two things that hold the
tick for a long time), then 5, 6, 3 and 7 as Phase 0's numbers rank them.
