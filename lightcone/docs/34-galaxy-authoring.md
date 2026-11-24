# Authoring the galaxy

How designers describe a galaxy of some 10^11 stars without placing any of them, and how that
description becomes stars the shard and the client can load.

**Status: design exploration.** Nothing here is built. Today's sky is the HYG catalog through
`lc_world::sky::HygProvider`, and [03-world-model.md](03-world-model.md) records that the shipped
game is a fictional galaxy and that phase 4 of [12-buildout.md](12-buildout.md) only made one
possible. Every number below is a first estimate.

The design in one paragraph: designers edit a **feature graph** — an ordered list of shapes and
operations, stored as RON in the repository — that defines per-population density and attribute
fields over space. A headless **bake** turns the fields into stars by drawing a Poisson process
on a fixed global grid of cells and thinning it with a hard-core rule that needs only
neighboring cells, so any region can be baked alone and the result does not depend on bake
order or thread count. Stars are **published** by region as immutable files; a published star
never moves again, whatever later happens to the graph or the generator code. The editor is a
separate native binary and ships with neither the client nor the shard.

## The size of the thing

The brief: one spiral galaxy 36 500 ly across, a few smaller galaxies, and 1–5 ly between stars,
closer in the core and in globular clusters. A reference model that meets it:

| component | profile | parameters | stars |
|---|---|---|---|
| thin disc | exponential in `R` and `z` | `h_R` = 3300 ly, `h_z` = 350 ly, cut at `R` = 18 250 ly | 8.6e10 |
| thick disc | exponential | `h_R` = 4000 ly, `h_z` = 1500 ly, 8% of the thin disc's central density | 4.1e10 |
| bulge | exponential sphere | `r_b` = 700 ly, 0.7 ly spacing at the center | 2.5e10 |
| halo | power law | sparse | 1e9 |
| **total** | | | **1.5e11** |

Spacing here is `n^(-1/3)`. The thin disc's midplane runs from 0.8 ly at the center to 1.8 ly at
`R` = 8000 ly and 5 ly at the edge. A globular cluster of 1e5 stars in 30 ly, or 1e6 in 50 ly,
has a mean spacing near 1 ly.

That is about as many stars as the Milky Way has in a galaxy a third as wide, so roughly thirty
times the real density on average. Four consequences follow from the count alone:

- **Storing every star costs 2–5 TB** at 16–32 bytes each. The design therefore never stores the
  whole galaxy as stars. It stores the graph (kilobytes), coarse volumes for rendering and
  preview (tens to hundreds of megabytes), and stars only where they have been published.
- **A hashed `u64` id collides.** `StarId::synthesize` hashes a provider tag and a key. The
  birthday bound for 1.5e11 values in 64 bits is `n²/2^65` ≈ 600 expected collisions. Galaxy
  stars need ids that are unique by construction; see [Identity](#identity).
- **The shell clamp is no longer an edge case.** A system's nominal shell radius is 1.58 ly, and
  [03-world-model.md](03-world-model.md) lists globular clusters as the place where the
  clamp against neighbors would "run constantly". At 0.8–5 ly spacing it runs for nearly every
  star in the disc. The hard-core distance in [Placing stars](#placing-stars) is what gives the
  clamp a floor.
- **The shard's linear nearest-star scan and the `stars() -> &[CatalogStar]` signature of
  `StarProvider` do not survive.** Both assume the whole sky is one slice in memory. The
  provider becomes a spatial query; see [What changes outside the tool](#what-changes-outside-the-tool).

### The coordinate box and the satellites

The global grid's invariant is `|c| < 2^60` per axis, which is ±36 500 ly
([01-spacetime.md](01-spacetime.md)). Putting the main galaxy's center at the origin uses
±18 250 ly of that for the disc and leaves the rest of the box for satellites. A satellite
galaxy must sit within about 35 000 ly of the origin along each axis, so the satellites are near
neighbors — about one disc radius beyond the edge, like the Sagittarius dwarf rather than the
Magellanic Clouds. Anything farther away belongs to the skybox in
[07-rendering.md](07-rendering.md#the-boundary-and-the-skybox), which is rendered and never
simulated.

The galaxy's frame is the global frame with `+Z` toward galactic north. Each system's own pole is
already drawn from its seed (`generate::pole_for`), so nothing depends on the ecliptic of J2000
except Sol's fitted bodies, which keep `+Z` through `CatalogStar::system_pole` wherever Sol is
placed.

## The feature graph

### What the graph defines

The graph defines fields, not stars. At any point `x` it can answer:

| field | units | used for |
|---|---|---|
| `n_p(x)` per population `p` | stars per ly³ | how many stars, and which population each belongs to |
| `exclusion(x)` | fraction of local spacing | the hard-core distance, see below |
| age distribution per population | Gyr, as a mean and spread or a named star-formation history | evolutionary state, velocity dispersion, metallicity |
| `[Fe/H]` offset per population | dex | added to the population's age–metallicity relation |
| dust density | arbitrary, calibrated later | path extinction, which [09-open-questions.md](09-open-questions.md) defers; carried now so nebulae can be authored before they render |
| gas emission | arbitrary | nebula rendering, not in the game yet |

A **population** is a named set of stellar statistics: an initial mass function, an age
distribution, an age–metallicity relation, a binary fraction by mass, and an age–velocity
dispersion relation. "Thin disc", "thick disc", "bulge", "halo", "young arm", "globular cluster"
are populations. A feature contributes density *to* a population. A star's population is drawn
in proportion to the populations' densities where it falls, and its attributes are then drawn
from that population's statistics evaluated at that point.

Keeping density per population, rather than one scalar density, is what lets an arm be "more
young stars" instead of "more stars", and lets a globular cluster be old and metal-poor inside a
disc that is neither.

### Feature kinds

| kind | examples | parameters |
|---|---|---|
| profile | exponential disc, sech² disc, Sérsic bulge, power-law halo | scale lengths, central density, truncation radius and its softness |
| cluster profile | Plummer, King | total stars, core radius, tidal radius |
| spiral arm | logarithmic spiral | pitch angle, start and end angle, width as a function of radius, density contrast over the disc |
| bar, ring, torus | central bar, resonance ring | length, axis ratio, orientation |
| path | stellar stream, filament, dust lane | a spline in 3D, a cross-section profile along it |
| region | void, nebula envelope, star-forming complex | an ellipsoid or a union of ellipsoids, a falloff width |
| noise | fBm, domain warp | octaves, scale, amplitude; breaks the symmetry every analytic profile has |
| point object | a supermassive black hole, a specific pulsar, a hand-placed star, Sol | position, and whatever record the object needs |
| frame | a galaxy | origin, orientation, rotation curve; every feature inside it is placed in its frame |

Point objects are the one kind that is not a field. The bake inserts them before anything else
and they suppress any generated star within their exclusion distance, so a hand-placed star is
never crowded by a generated one.

### Operations

Each feature applies an operation to one or more fields within its support:

| operation | effect | typical use |
|---|---|---|
| add | `n += f` | a disc, a bulge, a cluster |
| scale | `n *= 1 + a·f` | an arm as density contrast over the disc |
| subtract | `n *= 1 - f` | a void, a gap between arms |
| smooth union, intersection, difference | smooth `max` / `min` with a blend width | "this arm, but only inside the disc", "this void, but not through the bulge" |
| mask | restrict another feature to where a field exceeds a threshold | young stars only where gas is dense |
| set | replace an attribute field | metallicity of a stream that came from a satellite |

Every edge is soft. A feature's support is where its profile exceeds a cutoff, and booleans take
a blend width in light-years, so "vague edges" is a parameter rather than a special case. Most
features end up as multiplicative probability modifiers on a population's density, as the brief
expected.

### The timeline

The graph is an ordered list, the way a parametric CAD model's history is. Features evaluate in
order, and each may reference parameters of the ones before it: an arm's width may be a multiple
of the disc's scale height, a void may sit at a fraction of the way along arm 2, a cluster's
orbit may be set by the galaxy's rotation curve. References resolve by feature name, not by
index, so reordering does not silently rebind them.

What a timeline buys over a plain node graph:

- A **rollback marker** shows the galaxy as of any feature, which is how a designer sees what one
  feature did.
- **Suppress** turns a feature off without deleting it, and suppressing a feature that others
  reference is an error the editor names.
- **Groups** fold a run of features into one row: "Arm 2" may be a spiral, two noise passes and a
  dust lane.

Expressions are arithmetic over named parameters with units, evaluated once when the graph
loads. There is no constraint solver; a CAD sketch solver earns its cost on closed profiles, and
none of these features has one.

### Storage

The graph is RON files in the repository, one file per galaxy frame plus one for populations, so
two designers editing different galaxies do not touch the same file and every change is a
reviewable diff. A **graph version** is the content hash of those files together with the hash
of the generator that read them; it is what a bake records and what a publish pins.

Texture-graph graphs already travel as RON over the CDN ([07-rendering.md](07-rendering.md)).
Its node set includes the noise this graph needs, but it evaluates on the GPU, and a star's
existence must not depend on which GPU ran the bake. If texture-graph's evaluators are reused, it
is through a CPU path with the same determinism rules as `lc-world`: `f64`, `libm`
transcendentals, no fused multiply-add.

## Placing stars

### Why not Bridson's Poisson-disc sampler

The brief proposes Poisson-disc sampling within subregions because the cost grows quadratically
with the area. Bridson's algorithm is linear in the number of points when it uses a background
grid, so cost is not the reason to chunk. The reasons are that it is **serial** — it grows from
an active list, one point at a time — and that its output in one region depends on what was
placed in every region before it. Two chunks sampled independently put points too close together
along their shared face; resolving that by sampling them in a fixed order makes each chunk
depend on all of its predecessors, so a region cannot be baked alone and editing one region
rebakes the world. Parallel variants (phase-colored grids) are deterministic but have the same
non-local dependence.

Bridson's output is also too regular for a sky. A maximal Poisson-disc set has a minimum
separation near 0.9 of the mean spacing, which reads as a crystal from inside it. Real stars
clump.

### A hard-core process on a fixed grid

Recommended: **Matérn type II thinning of an inhomogeneous Poisson process**, seeded per cell of
a fixed global grid.

1. Space is divided into cubic **cells** of fixed side, 8 ly as a first estimate, on a grid
   anchored at the origin. The grid is part of the format and never moves.
2. Each cell draws a Poisson number of **candidates** at the cell's bounding intensity, from a
   generator seeded by `(graph version, cell index)`, and keeps each with probability
   `λ(x)/λ_max`. The bound comes from sampling the field over the cell with a margin; a
   candidate whose field exceeds the bound is a bake error, not something to clamp.
3. Each candidate gets a **mark**, a uniform draw from the same stream, and an exclusion distance
   `r(x)`.
4. A candidate **survives** unless another candidate within `max(r_a, r_b)` has a smaller mark.

Step 4 is the whole of the interaction between points, and it is symmetric and local: whether a
candidate survives depends only on candidates within `r_max` of it, which live in its own cell
and its 26 neighbors. So a cell's stars are a pure function of the graph version and a 3×3×3
block of seeded draws. Any region can be baked alone, in any order, on any number of threads,
and produces the same stars a whole-galaxy bake would. There are no seams because the rule
applied across a cell face is the rule applied inside a cell.

The cost is a lower density ceiling than dart throwing. With `V = 4/3·π·r³`, the surviving
density is `(1 - e^(-λV))/V`, which saturates at `1/V`. So to reach a target density `n`, draw
candidates at

```
λ = -ln(1 - nV) / V        requires nV < 1
```

At `nV = 0.5`, the bake draws 1.39 candidates per surviving star, and the guaranteed separation
is `r = 0.49·n^(-1/3)`: about half the local mean spacing. A star in a 3 ly neighborhood has no
neighbor closer than 1.5 ly, and its shell clamps to no less than 0.75 ly. Raising `nV` to 0.8
takes two candidates per star and only raises the floor to 0.58 of the spacing, so `exclusion` defaults
to the `nV = 0.5` value and is a field so a designer can tighten it in a cluster core.

Between the hard-core distance and the mean spacing the process is Poisson, so stars clump the way
real ones do. Clustering beyond that — open clusters, associations — is authored as features,
or as a cluster-population feature that places a parent process and spawns members around each
parent (a Neyman–Scott process), whose members the hard-core rule then thins like any others.

When `r(x)` varies, the surviving density departs from the formula by a few percent. The bake
corrects for it by measurement rather than by derivation: a test bakes uniform and graded boxes
and holds the surviving count to the target within a stated tolerance.

### Two tiers

A bright star is visible from thousands of light-years away — Deneb is first magnitude at
2600 ly — so the sky from anywhere needs the luminous stars of the whole galaxy, while the
dwarfs that are almost every star matter only within a few hundred light-years. The bake
therefore draws two tiers from the same fields with separate generator streams:

| tier | what | cell side | count, first estimate |
|---|---|---|---|
| **A** | stars whose luminosity at their age exceeds `L_A`, about 100 L☉: O and B stars, giants, supergiants, and named point objects | 64 ly | about 1e-3 of all stars, 1.5e8 |
| **B** | everything else | 8 ly | 1.5e11 |

Tier A's intensity is `n(x)·P(L > L_A | populations at x)`, and its stars draw mass and age from
their population conditioned on being luminous; tier B draws the complement. The union has the
populations' statistics. Tier A is placed first, and a tier-B candidate within exclusion distance
of a tier-A star does not survive.

The split exists so the two can be published at different times. Tier A is small enough —
about 2 GB at 14 bytes a star — to bake and publish for the whole galaxy at once, which fixes
every distant bright point in every player's sky. Tier B is published region by region as play
reaches it, and baking it never moves a tier-A star.

## Attributes

Each surviving star draws, from its own seed and its population's statistics at its position:

| attribute | from |
|---|---|
| population | in proportion to `n_p(x)` |
| initial mass | the population's IMF (Kroupa by default) |
| age | the population's age distribution at `x` |
| `[Fe/H]` | the population's age–metallicity relation, a radial gradient, the graph's offset field, and seeded scatter |
| multiplicity | the population's binary fraction at that mass; a companion's mass ratio and separation |
| evolutionary state, luminosity, `T_eff`, radius | initial mass, age and metallicity through analytic tracks — the Hurley, Pols & Tout fits are a candidate — including white dwarfs, neutron stars and black holes as remnants |
| velocity | the frame's rotation curve at `x`, plus a seeded dispersion that grows with age |

This reverses the direction `lc_world::sky::metallicity` uses for HYG. There, metallicity is
inferred from velocity because the catalog has velocity and lacks `[Fe/H]`. Here, age is drawn
first and both metallicity and velocity follow from it, so fast stars still come out old and
metal-poor. `metallicity::from_speed` stays as the HYG path.

Pulsars and magnetars come out of the tracks as young neutron stars; a pulsar the design wants
in a particular place is a point object. Solar-system contents are not generated by the bake.
[26-system-generation.md](26-system-generation.md) already turns a star's seed, mass and
metallicity into a system on demand, and a galaxy star feeds it the same `CatalogStar`.

### What a published star stores

The rule `sky::chunk` follows for HYG applies: store what cannot be recomputed. Identity and
position are what knowledge records, sightings and the event store refer to, so they are stored
and frozen. Physics derived from the stored draws may be retuned like any other generator:

| field | bytes | notes |
|---|---|---|
| index within cell | 2 | with the cell, the id |
| position within cell | 6 | `u16` per axis over 8 ly: 7.7 AU resolution |
| initial mass | 2 | logarithmic |
| age | 2 | |
| `[Fe/H]` | 1 | 0.02 dex steps |
| population, multiplicity, tier | 1 | |
| **total** | **14** | a companion adds a record of its own |

Velocity, luminosity, temperature, radius and `mu` are computed on load. A retune of the tracks
therefore changes how bright a published star is, which is the same bargain system generation
already makes; it never changes where a star is or whether it exists.

### Identity

A galaxy star's id is built, not hashed: the cell's index on the fixed grid (73 000 ly / 8 ly is
under 2^14 per axis, so 42 bits), the candidate's index within the cell (16 bits), and the tier
and a galaxy-provider tag in the remaining 6. Unique by construction, and independent of how
stars are later grouped into files. The catalog rule in [03-world-model.md](03-world-model.md)
still holds: the id is the bake's own, and no outside catalog's numbering reaches it.

## Baking

The bake is a library function over cells with no shared mutable state, so it parallelizes over
cells with `rayon` and over machines by region:

1. Load and validate the graph; resolve references; compute the graph version.
2. Evaluate coarse volumes: per-population density, luminosity and color per voxel, plus dust,
   on a sparse grid — 100 ly voxels over the disc's slab is 365 × 365 × 40, about 5e6 voxels
   and on the order of 100 MB across all channels. These drive previews, statistics and the far sky, never star placement.
3. For each cell in the requested region: candidates, then thinning against neighbors, then
   attributes.
4. Group cells into **chunks** for storage — an octree over the cell grid whose leaves hold about
   2^15 stars — and sort each chunk's stars by luminosity, so that the first bytes of a chunk file
   are its brightest stars and a client can fetch a prefix with an HTTP range request.
5. Write chunk files and a manifest naming each chunk's key, graph version and content hash.

Throughput is unmeasured. At an assumed 1e7 stars per second per core, the whole of tier B is
about fifteen minutes on sixteen cores before I/O, and tier A is seconds. A designer's working
loop bakes boxes of a few hundred light-years, which is interactive.

Determinism is tested the usual way for this repo: the same graph version gives byte-identical
chunk files twice, in separate processes, with one thread and with many; a region baked alone
equals the same region cut from a larger bake.

## Publishing and freezing

Players' knowledge refers to stars by id and position. Once a star has been published to a shard,
nothing may move it, but designers must still be able to edit the galaxy. The two are reconciled
by making **published files the truth** and the graph only the way unpublished stars are made.

- **Publish** bakes a region under the current graph version, uploads its chunk files to the CDN
  under their content hashes, and records them in the shard's manifest table. The CDN never
  overwrites ([14-hosting.md](14-hosting.md)), so a published chunk is immutable by construction.
- **A published chunk is never rebaked.** A later graph version applies only to unpublished
  cells. Where a new cell borders a published one, its thinning reads the published neighbor's
  candidates from that neighbor's own pinned version, so both sides of the face still agree.
- **Regenerating published stars from their pinned version is not the mechanism.** That would
  make every change to the generator code — an IMF bug fix, a different random stream — move
  stars players have already charted, which is the class of error `tests/ephemeris.rs` exists to
  catch for the solar system. The stored file is what is kept.
- **The editor shows the conflict.** Where the current graph would bake a published region
  differently — in count, populations or metallicity beyond a tolerance — the editor highlights
  it, so a designer knows an edit has stopped describing what players see.

Tier A for the whole galaxy is published once, at launch. Tier B is published ahead of play by
region, as a runbook step at first; publishing on demand as craft approach is possible later and
needs the shard to run the bake library as a job.

Before launch there are no players, and the whole galaxy can be republished whenever the graph
changes. The freeze rules matter from the first public shard on.

## The editor

A native Bevy binary, `lc-galaxy-editor`, over an engine-free library. It is a design tool and
never ships to players.

### Views

| view | shows |
|---|---|
| face-on and edge-on | projected density, luminosity, `[Fe/H]`, mean age or dust, per population or summed, from the coarse volumes |
| slice | the same fields on an arbitrary plane, at full resolution, evaluated from the graph |
| 3D | the coarse volume as a volume render, with point objects and feature gizmos |
| region | the baked stars of a selected box at their true positions, colored by any attribute |
| sky | the view from a chosen point, drawn with the client's starfield material from baked tier A, baked tier B nearby and the luminosity volume beyond, which is what a player actually sees |

Any view can **solo** one feature or population, or show the difference between two graph
versions.

### Panels

- **Timeline**: the ordered feature list with groups, suppression and the rollback marker.
- **Parameters**: the selected feature's fields, with units, expressions and references.
- **Statistics**: total stars by population and tier, the integral of the field rather than a
  count of a bake; for a baked region, a nearest-neighbor histogram against the hard-core
  distance, a shell-radius histogram, a metallicity histogram and an HR diagram drawn with
  `em-plot`. The HR diagram is already the cheapest check that a star list is right
  ([03-world-model.md](03-world-model.md)).
- **Inspector**: for a picked star, its attributes, its population, and which features contributed
  to the density at its position.
- **Published**: which regions are published, under which versions, and where the current graph
  disagrees with them.

### Toolkit

[18-ui-style.md](18-ui-style.md) puts forms and dense readouts in egui and keeps Bevy UI for the
menu and the ship editor, whose handles sit on the rendered ship. The galaxy editor is mostly
forms and readouts, so its panels are egui. Its transform and falloff handles are drawn in the
viewport the way the ship editor's are, through the same camera controls as the client's World
and Map views. Whether the panels should instead follow the ship editor into `em-ui` is the
owner's call; this document assumes egui because the editor is internal.

### Without the editor

The library and a CLI come first, and are useful alone:

```bash
lc-galaxy check  galaxy/            # parse, resolve references, report the graph version
lc-galaxy render galaxy/ --face-on out.png --field density
lc-galaxy stats  galaxy/ --box -500,-500,-200:500,500,200
lc-galaxy bake   galaxy/ --region <box> --out chunks/
lc-galaxy diff   galaxy/ --against <manifest>
lc-galaxy publish chunks/           # CDN upload and manifest rows; see 15-runbook.md
```

A designer can edit RON in a text editor and rerun `render` under a file watcher well before the
editor exists.

## Crates

| crate | what | depends on | linked by |
|---|---|---|---|
| `lc-world::sky::galaxy` | the chunk format reader, the record, the id, `GalaxyProvider` | what `lc-world` already has | shard, client |
| `lc-galaxy` | graph types, evaluator, sampler, attributes, tracks, writer, the CLI | `lc-world`, `em-plot`, `rayon`, `ron` | nothing that ships |
| `lc-galaxy-editor` | the editor | `lc-galaxy`, `bevy`, `bevy_egui`, `em-render` | nothing |

The reader lives in `lc-world` because both the shard and the wasm client need it, and the
producer lives outside it so neither links the bake. A new invariant beside the existing ones:

```bash
cargo tree -p lc-galaxy | grep -i bevy               # must be empty
cargo tree -p lc-client --prefix none | grep -E '^lc-galaxy'   # must be empty
cargo tree -p lc-server --prefix none | grep -E '^lc-galaxy'   # must be empty
```

## What changes outside the tool

- **`StarProvider`** gains a region query — stars within a box or within a radius of a point — and
  `stars()` becomes something only small providers implement. The shard's nearest-star scan
  (`lc_server::status::nearest_within_shell`) becomes a query on that index.
- **The systems table** ([09-open-questions.md](09-open-questions.md), "What owns the systems" and
  "More than one galaxy"): both open questions are answered by the manifest. A system belongs to
  a galaxy frame and a chunk, and the published chunk is what owns it.
- **The client's sky**: near stars as points from loaded chunks, distant tier A from chunk
  prefixes, and the rest as light integrated from the luminosity volume into the background — a
  new pass, since today every star in the sky is a point.
- **Path extinction**: the dust field gives the deferred extinction model something to integrate.
  It stays deferred; the graph only carries the data.
- **Proper motion** stays off ([01-spacetime.md](01-spacetime.md)). Velocities are stored for
  when it is switched on. A 36 500-year game is about 1/2000 of an orbit at 8000 ly from the
  center, so arms are static for the game's lifetime.

## Order of work

| phase | what | done when |
|---|---|---|
| G1 | graph types, RON, evaluator, `check` and `render` | the reference model renders face-on and edge-on, and its integrated star count matches the table above |
| G2 | sampler, thinning, tier split | determinism tests pass; nearest-neighbor histograms show the hard-core floor; a region baked alone equals a region cut from a larger bake |
| G3 | populations, IMF, tracks, attributes | a baked region's HR diagram shows a main sequence, giant branch and white dwarfs in proportions that change with population age |
| G4 | chunk format, `GalaxyProvider`, region query in `StarProvider` | the shard runs on a baked region instead of HYG |
| G5 | the editor: timeline, parameters, projections, region view | a designer adds an arm and sees it without editing RON |
| G6 | sky view in the editor, luminosity volume in the client | the sky from inside the disc shows the band of the galaxy |
| G7 | publish, manifest, freeze, diff | an edit that touches a published region is reported and does not change it |

## Open

| question | notes |
|---|---|
| `L_A` and tier A's cell size | Set by how far the telescopes in [05-observation.md](05-observation.md) resolve a star of a given luminosity. 100 L☉ is a placeholder. |
| Cell side | 8 ly keeps a dense cell near a thousand candidates and a sparse one near four. Globular cores may want a finer grid, which would be a second grid level rather than a change to this one. |
| Chunk delivery | Range requests on immutable CDN files are assumed to work through the CDN in [14-hosting.md](14-hosting.md); unverified. |
| Shell overlap at 1 ly spacing | The hard-core floor bounds the clamp from below. At the 0.8 ly central spacing the floor is a 0.2 ly shell; whether that is still a usable partition boundary for [03-world-model.md](03-world-model.md) is measurable once G2 exists. |
| Published tier-B volume | How far ahead of play to publish. Sets CDN storage, which grows with exploration rather than with the galaxy. |
| Nebulae | Authored as dust and gas fields with a young population inside. How they render and occlude is a separate design. |
| Editor panels in egui or `em-ui` | See [Toolkit](#toolkit). |
