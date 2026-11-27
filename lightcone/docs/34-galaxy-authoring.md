# Authoring the galaxy

How designers describe a galaxy of some 10^11 stars without placing any of them, and how that
description becomes stars the shard and the client can load.

**Status: partly built.** Steps E1 to E3 of [Before Phase I](#before-phase-i) are built:
`crates/lc-galaxy` holds the timeline, its tolerant codec, the `variable`, `frame`, `population`
and `plummer` kinds, placement on the fixed grid, attributes and a local publish;
`crates/lc-galaxy-editor` is **Galaxy Forge**, the editor; and `lc_world::sky::galaxy` reads
published chunks into the shard (`--sky <manifest>.galaxy`) and the client (a `.galaxy` asset).
Galaxy Forge's Charts panel (an HR diagram over the zero-age main sequence, and mass, age and
`[Fe/H]` histograms against the IMF and age distributions they were drawn from) is built ahead of
Phase II, with a draft Phase II galaxy in `crates/lc-galaxy/galaxies/phase-2.ron`. Phase II is under way: since 2026-11-25 the dev shard and the promoted browser build run the
Phase II galaxy, 467 stars in chunk format 3 (build `832dbd40` since 2026-11-26). Phase I ran
from 2026-11-24 (build `fbb9423f`). Of Phase II's additions, the Hurley tracks, solo and
difference in both views, per-star shells with their histogram, and authored system contents
([35-system-contents.md](35-system-contents.md)), `lc-galaxy stats` and the separate-process
determinism test (`crates/lc-galaxy/tests/cli.rs`, publishing with one thread and with seven) are
built. Of Phase III, the shape kinds are built (`crates/lc-galaxy/src/kinds/{profiles,regions,spiral}.rs`):
`exponential_disc`, `sersic`, `power_halo`, `king`, `ellipsoid`, `ring`, `bar` and `log_spiral`,
each with a distance whose Lipschitz bound a test checks, then `boolean` (`kinds/boolean.rs`)
and `path` (`kinds/path.rs`), whose control points Galaxy Forge lists in the Parameters panel and
drags in the region view, `noise` (`kinds/noise.rs`, over `f64` gradient noise in
`crates/lc-galaxy/src/noise.rs` whose values a test pins), and a draft Phase III galaxy in
`crates/lc-galaxy/galaxies/phase-3.ron`. Masks, the ray marcher, tier A and the CDN path are not
started. The shipped sky is still the
HYG catalog through `lc_world::sky::HygProvider`, and [03-world-model.md](03-world-model.md) records that the shipped
game is a fictional galaxy and that phase 4 of [12-buildout.md](12-buildout.md) only made one
possible. Every number below is a first estimate.

The design in one paragraph: designers edit a **feature graph** — an ordered list of shapes and
operations, stored as RON in the repository — that defines per-population density and attribute
fields over space. A headless **bake** turns the fields into stars by drawing a Poisson process
on a fixed global grid of cells and thinning it with a hard-core rule that needs only
neighboring cells, so any region can be baked alone and the result does not depend on bake
order or thread count. Stars are **published** by region as immutable files; a published star
never moves again, whatever later happens to the graph or the generator code. Every shape in
the graph also answers a signed distance, which is what lets one ray marcher draw the editor's
previews and the skybox pictures of galaxies on other shards. The editor is a separate native
egui binary, Galaxy Forge, built before any test galaxy, and ships with neither the client nor the shard.
Test galaxies reach players in playtest phases I to III, each of several iterations; see
[Order of work](#order-of-work).

The data model borrows from two feature editors that already exist: Century's timeline
(`~/rust/century/crates/century-core/src/{feature,timeline}.rs`, `project/codec.rs`) and
Grackle's prop modeler (`~/rust/grackle/src/prop/feature.rs`); see [Data types](#data-types).

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

### The coordinate box, the companions and other shards

The global grid's invariant is `|c| < 2^60` per axis, which is ±36 500 ly
([01-spacetime.md](01-spacetime.md)). The main galaxy's center sits at the origin, and its disc
uses a radius of 18 250 ly but only a few thousand light-years of height. The companion galaxies
therefore sit **above and below the disc**, where the sphere of radius 36 500 ly about the origin
is nearly empty: a dwarf centered 25 000 ly off the plane, with a radius of a few thousand
light-years, stays inside both that sphere and the per-axis bound. Each companion is its own
[frame](#feature-kinds) with its own orientation, so it need not share the disc's plane.

Galaxies farther away than that are **on other shards**. A shard's world ends at its boundary
([07-rendering.md](07-rendering.md#the-boundary-and-the-skybox)), and from inside it another
shard's galaxy is a skybox item: rendered, never simulated. Its picture comes from that shard's
own feature graph, drawn from far away by the ray marcher in
[Signed distances and ray marching](#signed-distances-and-ray-marching), so it shows the same arms
and dust lanes a player would find by moving there. Where the shards sit relative to one another
is a small universe-level file — each shard's galaxy origin and orientation in `f64` light-years,
outside the integer grid — that only the skybox bake reads.

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
orbit may be set by the galaxy's rotation curve. References are by feature id, which is never
reused, so reordering or deleting a feature does not silently rebind them; a feature may move
only within the range between what it reads and what reads it, as Century's `legal_range`
computes.

What a timeline buys over a plain node graph:

- A **rollback marker** shows the galaxy as of any feature, which is how a designer sees what one
  feature did.
- **Suppress** turns a feature off without deleting it. A feature that reads a suppressed one
  gets a problem of its own and the rest of the timeline still evaluates.
- **Groups** fold a run of features into one row: "Arm 2" may be a spiral, two noise passes and a
  dust lane.

Expressions are arithmetic over named parameters with units, evaluated once when the graph
loads. There is no constraint solver; a CAD sketch solver earns its cost on closed profiles, and
none of these features has one.

## Data types

A first draft, to be argued with. The names are what the code would use; nothing here exists yet.

### What comes from Century and what from Grackle

| decision | source | why it fits a galaxy |
|---|---|---|
| an open set of kinds behind a trait, registered by name | Century's `FeatureKind` and `Registry` | see below |
| a feature id that is never reused, with `next_id` stored in the file | both | references survive reordering and deletion |
| `depends_on`, and a feature may move only between what it reads and what reads it | Century's `legal_range` | keeps references pointing backward |
| one feature per line of RON | Century's `project/codec.rs` | a bad line damages one feature, not the file, and diffs are one line per change |
| a kind this build does not know, or a newer version, is kept verbatim and never evaluated | Century's `Unknown` | a file from a newer build survives being opened and saved by an older one |
| a line that does not parse is kept verbatim and never evaluated | Century's `Damaged` | a hand-edit typo costs one feature |
| `VERSION` per kind and `migrate(from, ron)` | Century | parameter shapes will change |
| a failed feature becomes a problem and the rest still evaluate, holding its last good output | Century's `Problem` and `Held` | deleting a disc does not blank the view |
| the rollback bar is view state, not saved and not undone | Century | |
| a reference to one of several outputs records how many there were | Grackle's `BodyRef { feature, index, of }` | a spiral that goes from two arms to three makes "arm 2 of 2" name nothing rather than the wrong arm |
| undo as whole-document snapshots | Grackle | a galaxy timeline is a few kilobytes |
| numbers written back as they were typed | Grackle's `nice_f32` | `f64`'s shortest round-trip formatting already does this; a test pins it |

Grackle's prop modeler closes its set of kinds into an enum because a boolean there has to know
the geometry it cuts. A galaxy boolean does not: it combines two weights and two signed distances,
which every kind provides through one trait, so the open set costs nothing and is what lets an
older build carry a newer kind.

### Unsupported fields

Century keeps a line verbatim when it does not decode. One case needs more than that here: serde
ignores fields it does not recognize, so a field written by a newer build would be dropped on read
and lost on the next save, with no warning. So the reader tells three cases apart:

| case | when | kept | evaluated | the editor shows |
|---|---|---|---|---|
| `Unknown` | the kind is not registered, or its version is newer than this build's | the line, verbatim | no | the kind and version |
| `Unsupported` | the kind is known but the params carry fields this build does not have | the line, verbatim | no | the feature's name and kind, and **each field it does not support** |
| `Damaged` | the line is not readable RON, or repeats an id | the line, verbatim | no | the parse error |

`Unsupported` is found by reading the params as a `ron::Value` map and comparing its keys with the
kind's `FIELDS`, so every unfamiliar field is named rather than only the first. A test holds each
kind's `FIELDS` to what its struct actually serializes. The editor shows such a feature in its
place in the timeline with its line as editable text; a designer may delete the fields, which turns
it back into an ordinary feature, or leave it for whoever has the newer build. Features that read
it get a missing-dependency problem.

The tools differ in how much of this they accept. The editor and `lc-galaxy check` open anything
and list what they could not use. `bake` and `publish` refuse a timeline with any `Unknown`,
`Unsupported` or `Damaged` line or any problem, because a bake that silently skipped a feature
would publish a different galaxy than the file describes.

### The timeline

```rust
/// Never reused within a timeline.
pub struct FeatureId(pub u32);

pub trait FeatureKind:
    Clone + PartialEq + Debug + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// The name in the file. Never changed once shipped.
    const KIND: &'static str;
    /// Bumped when the params change shape; the old shape is read by `migrate`.
    const VERSION: u32;
    /// Every field the params serialize, for telling `Unsupported` apart.
    const FIELDS: &'static [&'static str];

    fn new() -> Self;
    fn depends_on(&self) -> Vec<FeatureId> { Vec::new() }
    fn summary(&self) -> String;
    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem>;
    fn migrate(from: u32, ron: &str) -> Result<Self, String>;
}

pub struct Feature {
    pub id: FeatureId,
    pub enabled: bool,
    pub name: Option<String>,
    /// The group row this feature folds under in the timeline.
    pub group: Option<FeatureId>,
    pub params: Box<dyn Params>,
}

/// Unknown, Unsupported and Damaged, each holding its line verbatim.
pub enum Unread { /* … */ }

pub struct Timeline {
    features: Vec<Feature>,
    next_id: u32,
}

/// One of the outputs of a feature that makes several.
pub struct Output { pub feature: FeatureId, pub index: u32, pub of: u32 }
```

`Timeline::compile(cut)` evaluates the first `cut` features into a `Compiled`, which is what the
bake, the ray marcher and the statistics read:

```rust
pub struct Compiled {
    pub variables: BTreeMap<String, f64>,
    pub frames: BTreeMap<FeatureId, Frame>,
    pub populations: BTreeMap<FeatureId, Population>,
    pub shapes: BTreeMap<Output, Arc<dyn Shape>>,
    /// In timeline order, which is the order they apply in.
    pub terms: Vec<Term>,
    pub objects: Vec<PointObject>,
    pub problems: BTreeMap<FeatureId, Problem>,
}

pub trait Shape: Debug + Send + Sync {
    /// Signed distance to the shape's support in ly, negative inside. May underestimate, never
    /// overestimate by more than `lipschitz` allows: the ray marcher steps by it.
    fn distance(&self, p: DVec3) -> f64;
    /// 1 at the profile's peak, falling to 0 at the edge of the support.
    fn weight(&self, p: DVec3) -> f64;
    /// Bound on `|∇ distance|`; 1 for an exact distance, above 1 for noise and warps.
    fn lipschitz(&self) -> f64;
    fn bounds(&self) -> Aabb;
}

pub struct Term {
    pub source: FeatureId,
    pub target: Target,
    pub op: Op,
    pub shape: Output,
    pub mask: Option<Output>,
}

pub enum Target {
    Density(FeatureId),   // a population
    Exclusion,
    Age(FeatureId),
    FeH(FeatureId),
    Dust,
    Gas,
}

/// As written in a feature; `Compiled` holds `Op`, the same with every `Num` resolved.
pub enum OpSpec {
    Add(Num),        // value += a·w
    Scale(Num),      // value *= 1 + c·w
    Subtract,        // value *= 1 - w
    Set(Num),        // value = lerp(value, v, w)
}

/// A number as written: a literal, or a variable defined earlier in the timeline.
pub enum Num { Lit(f64), Var(String) }
```

Terms do not commute — a scale applied before an add leaves the add unscaled — so their order is
the timeline's order and that is what the timeline means.

As built, `Target` has only `Density`, `Exclusion` and `FeH`; the others arrive with the kinds
that need them. Variables resolve when the timeline compiles, so `Term` carries `Op` with plain
numbers. A cluster kind reads `Add` on a density as a total number of stars and converts it to a
peak density from its profile's integral, because the total is what a designer knows.

### Kinds

Most rows of the timeline are a shape that contributes to a field. To keep the common case one
row, every shape kind carries its own `contributes: Vec<(Target, Op)>`, and a shape with an empty
list is construction geometry, drawn grayed, that only booleans and masks read.

| kind | outputs | notes |
|---|---|---|
| `variable` | a named number | Century's `Variable`, numbers only |
| `frame` | a frame | origin in ly, tilt and azimuth in degrees, rotation curve; a galaxy or a companion |
| `population` | a population | IMF, age distribution, age–metallicity relation, binary fraction, age–velocity dispersion |
| `exponential_disc`, `sersic`, `power_halo` | one shape | analytic profiles, with a truncation radius and edge width so the support is finite; `Add` on any shape's density counts stars, across all of a feature's outputs |
| `plummer`, `king` | one shape | clusters |
| `log_spiral` | one shape per arm | `Output { index, of }` names an arm |
| `bar`, `ring`, `ellipsoid`, `path` | one shape | a bar is a Ferrers ellipsoid, `(1 - m²)²`; an `edge_ly` on a ring or ellipsoid is measured along its shortest semi-axis; a path is a tube along a Catmull-Rom spline through its points, each with a radius, which may change by at most 0.3 ly per ly along it |
| `noise` | one shape | warps or modulates another shape's weight; raises its `lipschitz`. Fractal Brownian motion of Perlin's improved noise, in `f64` with integer hashing rather than texture-graph's `f32` GPU port, since published stars depend on its values. Modulating multiplies the weight by between `1 - depth` and 1 and keeps the support; warping reads the shape at `p + reach · N(p)` |
| `boolean` | one shape | union, intersection or difference of shapes, with a blend width in ly. Distances combine by a smooth minimum or maximum over the blend; an intersection or difference weighs the operands' weights by how far inside the combined distance a point is, fading over the blend, so the blend is the soft edge. A difference is the first shape less all the others, and "arms in a disc" is a union of the arms and then an intersection with the disc |
| `object` | a point object | a black hole, a pulsar, a hand-placed star; inserted before thinning |

Angles are stored in degrees and lengths in light-years, with the unit in the field name. That is
the storage convention CLAUDE.md allows above `em-foundations`; evaluation converts once.

### The file

One timeline per frame — the main galaxy, and one per companion — so designers working on
different galaxies do not touch the same file:

```ron
Timeline(format: 1, next_id: 8, seed: 1)
Feature(id: 0, kind: "frame", version: 1, enabled: true, name: "Main", params: (origin_ly: (0, 0, 0), tilt_deg: 0, azimuth_deg: 0, rotation: Flat(speed_km_s: 220, core_ly: 3000)))
Feature(id: 1, kind: "population", version: 1, enabled: true, name: "Thin disc", params: (frame: 0, imf: Kroupa, age: Uniform(min_gyr: 0, max_gyr: 10), feh: (zero_age: 0.1, per_gyr: -0.05, per_kly: -0.018, scatter: 0.15), binaries: Standard, dispersion: (zero_age_km_s: 10, exponent: 0.5)))
Feature(id: 2, kind: "variable", version: 1, enabled: true, params: (name: "h_R", value: 3300))
Feature(id: 3, kind: "exponential_disc", version: 1, enabled: true, name: "Disc", params: (frame: 0, scale_length_ly: Var("h_R"), scale_height_ly: 350, radius_ly: 18250, edge_ly: 1500, contributes: [(Density(1), Add(1.83))]))
Feature(id: 4, kind: "log_spiral", version: 1, enabled: true, name: "Arms", params: (frame: 0, arms: 2, pitch_deg: 12, inner_ly: 2000, outer_ly: 17000, width_ly: 600, contributes: []))
Feature(id: 5, kind: "boolean", version: 1, enabled: true, name: "Both arms", params: (op: Union, blend_ly: 100, of: [(4, 0, 2), (4, 1, 2)], contributes: []))
Feature(id: 7, kind: "boolean", version: 1, enabled: true, name: "Arms in disc", params: (op: Intersect, blend_ly: 300, of: [5, 3], contributes: [(Density(1), Scale(1.5))]))
Feature(id: 6, kind: "ellipsoid", version: 1, enabled: true, name: "Void", params: (frame: 0, center_ly: (6000, 2500, 0), radii_ly: (400, 300, 200), edge_ly: 150, contributes: [(Density(1), Subtract)]))
```

A **graph version** is the content hash of a shard's timeline files together with the hash of the
generator that read them; it is what a bake records and what a publish pins.

Texture-graph graphs already travel as RON over the CDN ([07-rendering.md](07-rendering.md)), and
its node set includes the noise the `noise` kind needs. It evaluates on the GPU, though, and a
star's existence must not depend on which GPU ran the bake. If its evaluators are reused, it is
through a CPU path with the same determinism rules as `lc-world`: `f64`, `libm`
transcendentals, no fused multiply-add.

## Signed distances and ray marching

Every shape answers `distance` as well as `weight`. That gives three uses from one function.

**Skipping empty space in the bake.** Most of the 73 000 ly box is empty. A cell whose center is
farther than the cell's half-diagonal from every shape that adds density cannot hold a star, so
the bake skips it without evaluating any profile, and the same test bounds which chunks a region
bake visits at all.

**Pictures of the galaxy without baking a star.** The ray marcher integrates emission and
absorption along each ray:

```
I = ∫ j(s) · exp(-τ(s)) ds        τ(s) = ∫₀ˢ κ·dust(s') ds'
j(x) = Σ_p n_p(x) · L̄_p(x)        L̄_p: mean luminosity per star of population p at its age there
```

with color carried the same way, per band. Outside every support it steps by the union's signed
distance divided by its Lipschitz bound (sphere tracing); inside, it takes steps short enough to
resolve the narrowest profile it is in. Dust lanes come out dark against the disc behind them
because absorption is in the integral, which is the "distant dusty picture" a skybox wants.

The mean luminosities `L̄_p` are a table per population, built once from its IMF, age distribution
and tracks, so the marcher never draws a star. The same table makes the face-on and edge-on
previews luminosity rather than number density, which is what the galaxy looks like.

| use | evaluated on | notes |
|---|---|---|
| editor previews: face-on, edge-on, a free camera | GPU, over the coarse volumes and a distance volume baked from the graph | the volumes rebuild in parallel on the CPU after an edit; at 100 ly voxels that is about 5e6 graph evaluations |
| stills, tests, and skybox pictures | CPU, straight from the graph, in `lc-galaxy` | deterministic, so a picture is reproducible from a graph version |
| the far sky inside a shard | the client, over the coarse volumes | the same integral starting at the edge of the loaded stars; see [07-rendering.md](07-rendering.md) |

Translating a timeline into WGSL so the GPU marches the graph itself is possible later. Nothing
above needs it.

**Skybox pictures of other shards.** For each pair of shards, the CPU marcher renders the other
shard's galaxy from this shard's origin at the distance and orientation the universe layout gives,
into an image with alpha so its dust dims what is behind it. The image is placed on the skybox in
that direction. A shard is 73 000 ly across, so for a galaxy 500 000 ly away, moving from one edge
of the shard to the other changes the direction to it by about eight degrees and its apparent size
by about fifteen percent; one picture per pair is enough until shards sit closer than that. A
picture is regenerated when either graph version changes and goes to the CDN under its content
hash like any other asset.

**Distances that are only bounds.** Smooth booleans, noise and warps do not give exact distances.
Each shape declares its Lipschitz bound and the marcher divides by it; a test samples random point
pairs for every kind and fails if the distance changes faster than the declared bound allows.
A spiral arm's distance is the radial offset to the nearest winding times the cosine of the pitch
angle, which is exact on the arm's axis and a bound off it.

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
2. Each cell draws a Poisson number of **candidates** at the cell's bounding intensity, each
   candidate's draws hashed from `(seed, cell index, candidate index)`, and keeps each with
   probability `λ(x)/λ_max`. The seed is the timeline header's, fixed across edits, so changing
   a parameter adds or removes the stars it affects instead of reshuffling every star. The
   bound comes from sampling the field on a 1 ly lattice over the cell with a margin; a
   candidate whose field still exceeds it doubles the bound and redraws the cell, which stays a
   pure function of the field, and a cell that exceeds every bound tried is a bake error.
3. Each candidate gets a **mark**, a uniform draw from the same stream, and an exclusion distance
   `r(x)`.
4. A candidate **survives** unless another candidate within `(r_a + r_b) / 2` has a smaller mark.

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

When `r(x)` varies, the surviving density departs from the formula, and which pair radius the
rule uses decides by how much. Measured over 1000 seeds of Phase I and 150 of Phase II, the larger
of the two radii left 4.6% fewer stars than the field promises (a sparse edge candidate's wide
radius removes dense neighbors), the smaller left 4–9% too many, and their mean came within 0.1%.
So the rule uses the mean, and a test bakes a uniform ball and the graded Phase II cluster and
holds the surviving count to the field within a stated tolerance.

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
| evolutionary state, luminosity, `T_eff`, radius | initial mass, age and metallicity through analytic tracks: Tout et al. (1996) for the zero-age main sequence and Hurley, Pols & Tout (2000) for the main sequence and Hertzsprung gap, as built in `lc_world::sky::galaxy::tracks`, through the Hertzsprung gap, giant branch, core helium burning with its blue loop, and AGB, to Mestel-cooling white dwarfs, neutron stars (eq. 92) and black holes (Belczynski et al. 2002 fallback). The tracks model no winds, so the AGB ends at an initial–final mass relation rather than at envelope loss |
| velocity | the frame's rotation curve at `x`, plus a seeded dispersion that grows with age |

This reverses the direction `lc_world::sky::metallicity` uses for HYG. There, metallicity is
inferred from velocity because the catalog has velocity and lacks `[Fe/H]`. Here, age is drawn
first and both metallicity and velocity follow from it, so fast stars still come out old and
metal-poor. `metallicity::from_speed` stays as the HYG path.

Pulsars and magnetars come out of the tracks as young neutron stars; a pulsar the design wants
in a particular place is a point object. Solar-system contents are not generated by the bake; how
the graph steers them by region is [35-system-contents.md](35-system-contents.md).
[26-system-generation.md](26-system-generation.md) already turns a star's seed, mass and
metallicity into a system on demand, and a galaxy star feeds it the same `CatalogStar`.

### What a published star stores

The rule `sky::chunk` follows for HYG applies: store what cannot be recomputed. Identity and
position are what knowledge records, sightings and the event store refer to, so they are stored
and frozen. Physics derived from the stored draws may be retuned like any other generator:

| field | bytes | notes |
|---|---|---|
| cell within chunk | 2 | a chunk is 32³ cells, 256 ly on a side |
| index within cell | 2 | with the cell, the id |
| position within cell | 6 | `u16` per axis over 8 ly: 7.7 AU resolution |
| initial mass | 2 | logarithmic over 0.08–150 M☉ |
| age | 2 | linear over 0–13.8 Gyr |
| `[Fe/H]` | 1 | 0.02 dex steps |
| population, tier | 1 | four bits of population index into the chunk's header, one of tier |
| **total** | **16** | a companion adds a record of its own; multiplicity is not stored yet |

As built (`lc_world::sky::galaxy`), a chunk file is a header — the graph version, the chunk's
coordinates, and each population's frame, rotation curve and dispersion — then its records,
brightest first. Files are named by their content hash, and a `manifest.galaxy` in RON lists
them with their hashes; the loader refuses a chunk whose bytes do not match. Remnants that emit
nothing are left out of the sky on load.

Velocity, luminosity, temperature, radius and `mu` are computed on load, by the same code the
bake uses (`sky::galaxy::tracks` and `Kinematics`), and the bake derives its stars from the
quantized record, so Galaxy Forge shows a star exactly as the game will load it. A retune of the tracks
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

1. Load the timelines, refuse any `Unknown`, `Unsupported` or `Damaged` line or any problem,
   compile them, and compute the graph version.
2. Evaluate coarse volumes: per-population density, luminosity and color per voxel, dust, and
   the signed distance to everything that adds density, on a sparse grid. 100 ly voxels over the
   disc's slab is 365 × 365 × 40, about 5e6 voxels and on the order of 100 MB across all
   channels. These drive previews, statistics and the far sky, never star placement.
3. For each cell in the requested region that the signed distance does not rule out:
   candidates, then thinning against neighbors, then attributes.
4. Group cells into **chunks** for storage — an octree over the cell grid whose leaves hold about
   2^15 stars — and sort each chunk's stars by luminosity, so that the first bytes of a chunk file
   are its brightest stars and a client can fetch a prefix with an HTTP range request.
5. Write chunk files and a manifest naming each chunk's key, graph version and content hash.

Throughput is unmeasured. At an assumed 1e7 stars per second per core, the whole of tier B is
about fifteen minutes on sixteen cores before I/O, and tier A is seconds. A designer's working
loop bakes boxes of a few hundred light-years, which is interactive.

Determinism is tested the usual way for this repo: the same graph version gives byte-identical
chunk files twice, in separate processes, with one thread and with many (`lc-galaxy publish
--threads`); a region baked alone equals the same region cut from a larger bake.

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

**Galaxy Forge**: a native Bevy binary, `lc-galaxy-editor`, over an engine-free library. It is a
design tool and never ships to players.

```bash
cargo run -p lc-galaxy-editor -- [timeline.ron] [--view region|sky] [--select <row>] [--pick <star>] [--shot <png> [--frames <n>]]
```

With no path it opens `crates/lc-galaxy/galaxies/phase-1.ron`. As built in E2, the region and
sky views draw through egui's painter, which is cheap below the live-bake budget; the client's
starfield material and the GPU marcher replace it when the views draw fields.

### Views

| view | shows |
|---|---|
| face-on and edge-on | a ray-marched picture of the galaxy, or projected density, `[Fe/H]`, mean age or dust, per population or summed, from the coarse volumes |
| slice | the same fields on an arbitrary plane, at full resolution, evaluated from the graph |
| 3D | the ray-marched galaxy from a free camera, with each shape's support as a surface at `distance = 0`, point objects and feature gizmos |
| region | the baked stars of a selected box at their true positions, colored by any attribute |
| sky | the view from a chosen point, drawn with the client's starfield material from baked tier A, baked tier B nearby and the luminosity volume beyond, which is what a player actually sees |

Any view can **solo** one feature or population, or show the difference between two graph
versions.

### Panels

- **Timeline**: the ordered feature list with groups, suppression and the rollback marker.
  `Unknown`, `Unsupported` and `Damaged` features keep their place in it, marked, with the
  unsupported fields named.
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

egui, decided. [18-ui-style.md](18-ui-style.md) puts forms and dense readouts in egui, and this
editor is mostly both. Transform and falloff handles are drawn in the viewport, through the same
camera controls as the client's World and Map views.

### The CLI

The editor is the first front end. The CLI follows for work that wants scripting: CI checks, the
determinism tests, and the runbook's bake and publish.

```bash
lc-galaxy check  galaxy/            # parse, resolve references, report the graph version
lc-galaxy render galaxy/ --face-on out.png --field density
lc-galaxy stats  galaxy/ --box -500,-500,-200:500,500,200
lc-galaxy bake   galaxy/ --region <box> --out chunks/
lc-galaxy diff   galaxy/ --against <manifest>
lc-galaxy publish chunks/           # CDN upload and manifest rows; see 15-runbook.md
```

As built, `check`, `stats` and a local `publish` exist. `stats` prints what Galaxy Forge's
Statistics panel shows for the stars inside `--box` (the whole graph without it): counts against
the field, the nearest-neighbor and shell-radius summaries, and the systems table. Stars just
outside the box still count as neighbors.

```bash
cargo run -p lc-galaxy --bin lc-galaxy -- stats crates/lc-galaxy/galaxies/phase-2.ron --box -8,-8,-8:0,8,8
cargo run -p lc-galaxy --bin lc-galaxy -- publish crates/lc-galaxy/galaxies/phase-1.ron --out crates/lc-client/assets/galaxies/phase-1
cargo run -p lc-server --bin lightcone-server -- --open --sky crates/lc-client/assets/galaxies/phase-1/manifest.galaxy
cargo run -p lc-client --bin lightcone -- galaxies/phase-1/manifest.galaxy --local
```

The published directory is generated and not committed.

The RON stays readable by hand, which matters when the editor opens a file and marks a feature
it cannot read.

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

The editor is built first, and every test galaxy is made in it. If the bake came first, the
editor's UI would be judged only after the generator had settled. Built first, the editor is
used at each scale the galaxy passes through, and its UI changes alongside the generator.

### Playtest phases

Phase I to III, in Roman numerals to keep them apart from the numbered phases of
[12-buildout.md](12-buildout.md), are rounds of play and feedback. In each, a test galaxy is
published to the dev shard in place of HYG, players fly in it, and their reports go back into
the generator and the editor. A phase holds several iterations. Each iteration is a change to
the generator, the graph or the editor, followed by a rebake and a republish. Before launch,
republishing is free ([Publishing and freezing](#publishing-and-freezing)), so an iteration
costs a bake and a manifest row.

| phase | the galaxy | stars | first exercises |
|---|---|---|---|
| **I** | one cluster, one population, 1.5–2 ly spacing | about 10, in a radius of about 2.3 ly | the file format and its tolerant reader, the timeline, `Compiled`, thinning inside one cell (a 0.86 ly floor at 1.75 ly spacing), attributes, the chunk reader, `GalaxyProvider` on the shard |
| **II** | a cluster of two populations, one old and one young | 500, in a radius of about 8.6 ly | thinning across cell faces (it spans some 27 cells), the determinism tests, populations, IMF and tracks, an HR diagram |
| **III** | a spiral about 100 ly across: disc, bulge, two arms, a void, a boolean | about 10 000 at 2 ly spacing | arms, booleans, masks, signed distances, the ray-marched preview, the tier split, chunks on the CDN, publishing and freezing |
| full | the reference galaxy and its companions | 1.5e11 | tier A for the whole galaxy, publishing tier B by region, the far sky, and the skybox picture of a second shard |

Phase I's count is a Poisson draw, so "about 10" is whatever its seed gives; the tests pin that
seed.

The editor runs locally and needs no players, so its UI can change on any day. A phase sets when
players see a galaxy, not when the editor changes.

### Before Phase I

| step | built | tried by |
|---|---|---|
| E1 | `lc-galaxy` core: the timeline, the tolerant codec, `Compiled`, the `variable`, `frame`, `population` and `plummer` kinds, the sampler on the fixed grid, attributes from a placeholder track table | unit tests |
| E2 | `lc-galaxy-editor`: timeline and parameters panels, undo, open and save with `Unknown`, `Unsupported` and `Damaged` features marked, the region and sky views, the inspector, and statistics with a nearest-neighbor histogram against the hard-core distance | the designer, locally |
| E3 | the chunk writer, `lc-world::sky::galaxy`, `GalaxyProvider`, a region query on `StarProvider`, and a local publish: chunk files and a manifest that the dev shard loads (built) | Phase I begins |

### What each phase adds

Cumulative, on top of E1–E3:

| phase | added |
|---|---|
| I | whatever its iterations call for; expected to be the track table, the thinning parameters and the editor's panels |
| II | the real IMF and tracks; the age–metallicity and age–velocity relations; the HR diagram and the metallicity and shell-radius histograms in Statistics; solo and difference in every view; determinism tests in separate processes and thread counts; `lc-galaxy check` and `stats` |
| III | the shape kinds, `boolean` and `noise`; signed distances and the CPU ray marcher behind the face-on, edge-on and 3D views; coarse volumes; tier A; the manifest on the CDN, `publish` and `diff`; the Published panel; transform and falloff handles in the viewport |
| full | the luminosity volume in the client; the GPU marcher in the editor; the universe layout and skybox pictures |

### Live bake and scale

The editor rebakes on every edit while the stars in view stay under a budget, 100 000 to start.
Above it, the views draw fields — projected density and the ray-marched picture — and baking is
limited to a selected region. Phases I to III all fit under the budget. Phase III is where the
ray-marched views arrive, so its iterations show the field views beside a bake of the same graph,
which checks the marcher against the stars and gives switching between the two modes its UI
before the full galaxy forces it.

## Open

| question | notes |
|---|---|
| `L_A` and tier A's cell size | Set by how far the telescopes in [05-observation.md](05-observation.md) resolve a star of a given luminosity. 100 L☉ is a placeholder. |
| Cell side | 8 ly keeps a dense cell near a thousand candidates and a sparse one near four. Globular cores may want a finer grid, which would be a second grid level rather than a change to this one. |
| Chunk delivery | Range requests on immutable CDN files are assumed to work through the CDN in [14-hosting.md](14-hosting.md); unverified. |
| Shell overlap at 1 ly spacing | Shells overlap wherever stars are closer than 3.2 ly, which Phase I's 1.3–1.9 ly spacing already does. A position belongs to the *nearest* star within the shell (`lc_world::system::holding`), which the shard and the client share; the first-in-catalog-order rule it replaced put a ship beside one star into its neighbor's system. Whether a shell this much larger than the spacing still suits [03-world-model.md](03-world-model.md)'s other uses of it is open. |
| Published tier-B volume | How far ahead of play to publish. Sets CDN storage, which grows with exploration rather than with the galaxy. |
| Nebulae | Authored as dust and gas fields with a young population inside. How they render and occlude is a separate design. |
| The universe layout | Who owns the file that places shards relative to one another, and whether a shard's galaxies can be seen from another in more than a skybox picture. |
| Lipschitz bounds for noise | A warped shape's bound may be loose enough to make sphere tracing slow; measured in Phase III. |
