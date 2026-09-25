# Ship form

What a ship is made of, what shape it is, and what it costs to change either.

**Status: partly built.** It replaces the loadout of [19-ship-fitting.md](19-ship-fitting.md):
**a ship is its parts**, and each part's volume is how much of its kind the ship has. 19's energy,
mass and drive rules stand. Since F9 `lc-world` has no loadout: a craft's account is kept on its
form (§What a craft reads). The wire and saves still carry the loadout until S1, and **refits are
refused between F9 and S1** (§Protocol and persistence).
[30-the-field.md](30-the-field.md) and [31-directed-energy.md](31-directed-energy.md) are what the
shape does in play, and [32-ship-rendering.md](32-ship-rendering.md) is how it is drawn.

## Why parts

Module counts beside a form would be two sources of truth, and every edit would be made twice: once
to the count, once to the shape holding it. Without counts, **energy is the only limit on what can be
placed.**

Blocks do not scale. A 50 km hull is a million times the volume of a 500 m one.

## Parts

A `Form` is a tree of **parts**. Each part is one primitive of one kind, with a continuous volume.

| primitive | proportions | closed-form volume |
|---|---|---|
| ellipsoid | three semi-axis ratios | `4/3 π a b c` |
| capsule | straight length over radius | cylinder plus a sphere |
| slab | three edge ratios, corner radius over the shortest edge | rounded box |
| cylinder | length over radius | `π r² h` |
| torus | major radius over minor | `2 π² R r²` |
| frustum | length over the first end's radius, and the second end's radius over the first's | `π h (r₁² + r₁ r₂ + r₂²) / 3` |

**Volume and proportions are stored. Scale is solved** from them in closed form. Overlaps and
fillets are ignored when sizing: the volume of a blended union depends on the neighbors, which
would make one part change size when another moved, and would need a numerical solve two
machines might round differently. Each part alone is exact.

### Kinds

Every kind but the Mind turns volume into a capacity at a density. Each density is 19's per-module
value over its 392 699 m³ slot.

| kind | per cubic meter | mass | notes |
|---|---|---|---|
| **mind** | nothing | module density | the root. See below |
| **storage** | 1.27 × 10⁻⁵ ME of capacity | module density | |
| **drone** | 5.88 × 10¹³ W of building power | module density | at least `min_drone_m3` must remain |
| **engine** | 5.47 × 10¹³ W of aperture | module density | points fore or aft. See [31-directed-energy.md](31-directed-energy.md) |
| **living** | 1.13 × 10¹⁰ W of drain | module density | parks and population later |
| **data** | 7.5 bytes | half | three times as slow to build |
| **bay** | a mouth, whose smaller dimension is the largest hull it can launch | a tenth | a shell with a procedural interior. After construction exists |
| **spar** | nothing | a twentieth | structure for holding parts apart: cluster spokes, booms, straps. Conforms to what it touches; see below |

Module density is 395.8 kg/m³. **ME is the unit of energy**, 1.397 × 10²⁵ J.

**Every kind but drones may go to zero.** A ship with no engines cannot move and a ship with no
storage holds nothing, and both are allowed. A ship with no drones could never refit again, so the
server refuses any target with less than `min_drone_m3` of drone.

**Nothing is smaller than `min_part_m3`.** Volumes are continuous, but a part below that is refused,
which stops dust.

### The Mind

Every ship has exactly one **Mind**: a cube of `min_part_m3`, the root of the tree. It **cannot be
deleted, resized, reshaped or moved.** It has no capacity and no role in the physics: the form is
perfectly rigid, and nothing hangs from anything in the way a structure in *Kerbal Space Program*
does.

It is there for two reasons:

- **The tree needs a root that never changes.** Everything is placed relative to something, so one
  thing must be placed relative to nothing. That fixed thing is the ship's center and the origin of
  its frame.
- **Lore.** A Culture ship is its Mind, and the rest is what the Mind has built around itself.

**The Mind's stored primitive and volume are ignored.** Everything that sizes, weighs or draws it uses
the cube of `min_part_m3`, so a form cannot carry a larger Mind.

The Mind is usually enclosed by the first part built around it, so it is inside the ship. The
editor shows it through the hull.

### Spars conform

A spar is shaped by the parts it joins, as if it were bolted to them. Its distance field is its own
primitive combined with its **tree neighbors** (its parent and its children) by boolean operations,
in one of two ways, carried on the kind as `Kind::Spar(SparMode)`:

- **Saddle.** Each neighbor, grown by `spar_gap`, is subtracted from the spar. Where a boom meets a
  hull, its end is cut to the hull's curve and sits flush against it. The spar is embedded into its
  neighbors a little so there is something to cut.
- **Strap.** The spar is intersected with a shell around its parent, from its surface out to
  `spar_thickness` of the parent's smallest dimension, so it follows the parent's surface wherever its
  primitive passes: a band around a tank, a rib along a hull.

A spar joins its neighbors **hard, never blended**. `blend` is ignored on a spar and on its children's
joint with it, so the join reads as bolted rather than welded.

Only tree neighbors cut a spar. That keeps each spar's evaluation local, and it means a spar never
changes shape because an unrelated part moved past it.

A neighbor is cut by its primitive, never its own cut shape, so a spar hung from a spar does not
depend on how its parent was cut. An ellipsoid's distance is a bound rather than exact (see below), so
off an ellipsoid the gap and the strap's depth are exact only across its shortest axis and grow with
the semi-axis along the others.

**A spar's volume is its uncut primitive's.** The cut shape has no closed form. It is charged as the
stock it was cut from, which slightly overstates a strap's mass at a twentieth of module density, and
never understates it. A strap thinner than a grid cell does not show up in the shadow table. At that
size it does not matter.

## Placement is relative

Parts change size, so absolute coordinates would leave a child floating or buried after a refit.
Each part but the Mind hangs from a parent, and it is placed in one of two ways:

- **Attached**: it sits on the parent's surface.
- **Enclosing**: it is centered on the parent and contains it. This is how a core wraps the Mind,
  and how a shell wraps a core.

| field | meaning |
|---|---|
| `parent` | a part id |
| `mount` | attached or enclosing. `Mount::Attached` carries `anchor` and `standoff`, so an enclosing part cannot have either |
| `anchor` | attached only: a direction in the parent's frame. The attachment point is where a ray from the parent's center along it last leaves the parent's surface, so on a torus a child hangs off the rim. A ray that misses a torus's tube takes the point of the tube nearest it; along the axis, where every point of the inner equator is nearest, the one toward the parent's +y |
| `twist` | rotation about the surface normal, or about the parent's axis when enclosing |
| `tilt` | the child's axis relative to that normal or axis, as a small rotation: a two-component rotation vector across it, in radians |
| `standoff` | attached only: distance along the normal, in multiples of the child's **reach**, the distance from its center to its foot. Zero rests the child on the surface, −1 centers it there, and negative embeds it |
| `blend` | smooth-union radius with the parent, as a fraction of the smaller part's smallest dimension |
| `mirror` | the subtree is repeated, reflected through the ship's port–starboard plane, y = 0 |

An attached child's surface meets its parent's at the anchor, offset by `standoff`. When the parent
grows, the anchor point moves out with its surface. When the child grows, it grows away from the
parent.

What meets the anchor is the child's **foot**: the end of its axis at −x, one **reach** from its
center — the ellipsoid's first semi-axis, half the capsule's straight section plus its radius, half the slab's
first edge, half the cylinder's or frustum's length, the torus's minor radius. On every primitive but
the torus the foot is on its surface. A torus's foot is the center of its hole: the hole sits over the
anchor and the tube's lowest circle lies in the plane tangent to the parent there. On a curved parent
the tube therefore clears the surface by the parent's sag across the major radius, and touches it
nowhere on a convex one. A negative `standoff` or a `blend` closes the gap. Seating the tube on the
surface itself has no closed form for a general parent. Tilt pivots the child about its foot, so a tilted boom leans from where it is
bolted on rather than sliding its base across the parent. Reach is also the unit of `standoff`,
because it is the length the placement is already made of: F1's scale means a different dimension on
each primitive, and the extent along the normal would change under tilt.

The Mind's frame is the ship's frame: its axis is the nose, `lc_world::motion::facing`. Space is
Z-up, so the ship's z is up, its y is port, and the port–starboard plane a mirror reflects through is
y = 0.

**A mirrored copy keeps its part's id** and is named by `(PartId, Side)`, `Side::Original` or
`Side::Mirror`. A part has a mirror when its own `mirror` or any ancestor's is set; a mirror inside a
mirrored subtree adds nothing, since reflecting twice is the original. Refits, steps and animation
name the part, and both copies change together. Every primitive is symmetric in its own y, so a
mirrored copy is still a rigid transform: the reflection composed with that flip.

**A part's volume is per copy.** Each copy is solved from the same volume and proportions, so a
mirror repeats the part at its size. If the volume were split between the copies instead, switching a
mirror on would halve each copy and pull its children in, and a part would change size because of a
flag. So everything that totals volume counts every copy: capacities, dry mass and its structure, the
drone minimum, and each kind's total that a layout keeps. `min_part_m3` applies to each copy.
`Form::copies` gives the count.

**Placement is closed form**, so the server and every client compute it to the bit. A ray from a
torus's center lies in a plane through its axis, which cuts the tube in two circles, so even its last
exit is a quadratic; a rounded slab's is a quadratic on each of at most four pieces of the ray.
Trigonometry is `libm`'s, as [06](06-crate-layout.md) §Shared determinism asks.

**A part's axis is its local x**: a capsule's, cylinder's or frustum's length, a torus's axis of
symmetry, an ellipsoid's first semi-axis and a slab's first edge. A frustum's first end is at −x.
Every part is centered on its own origin, a frustum at half its length rather than its centroid. At
zero twist and tilt, a child's axis lies along the normal (attached) or its parent's axis (enclosing),
and its y along the parent's y projected across that, or the parent's z where the y is parallel. Twist
turns it about its axis. Tilt then rotates it by a rotation vector whose two components are along the
twisted y and z.

Part ids are small integers assigned by whoever adds the part, checked for uniqueness by the
server, and stable across refits. Steps and animation refer to parts by id.

`Form::validate` also refuses any number that is NaN, infinite, or of a sign its meaning forbids,
naming the part and the field, because forms arrive from clients. Two ranges are narrower than a
sign: a slab's corner is at most half its shortest edge, past which opposite roundings cross, and a
torus's major radius is at least its minor, below which the tube crosses the axis and the closed
forms count that part twice. And proportions extreme enough that the part's solved dimensions
underflow to zero or overflow to infinity are refused, since a grid sized from them would be too.

## What the server computes from a form

A shape matters to play only once it becomes numbers. Everything below is in `lc-world`, is
engine-free, and is **stated by the server in `Fitted`**, as `Balance` already is. The client
computes the same numbers for its preview and takes the server's when they arrive.

**Capacities** are sums of volume × density per kind, with every copy of a mirrored part counted. They
replace everything 19 read from the loadout.

**The distance field** is `form::sdf::Sdf`, built once from a form and then evaluated at any point in
the ship's frame, negative inside. Every primitive's distance is exact except the ellipsoid's, which
has no closed form off its surface and uses the bound `(|p/r| − 1) · r_min` instead: zero on the
surface, the right sign everywhere, and Lipschitz 1. Blends are the quadratic polynomial smooth
minimum, whose gradient is a convex combination of its arguments', so the whole field is Lipschitz 1:
a lower bound on the distance, and zero on the surface. That is all a grid or surface nets need.
The same `Sdf` names the part nearest a point, which is how the grid weights cells by density and the
hull material finds its kind regions. It also gives a spar's distance to its seams, for the bolt rows
of [32](32-ship-rendering.md) §Materials by kind.

**Geometry** comes from a **voxel grid of fixed resolution**: `FORM_GRID = 64` cells along the longest
side of the bounding box, whatever the ship's size. Each cell is filled by evaluating the union's
signed distance at its center. That is 262 144 samples, which is nothing at the rate refit steps
complete.

| quantity | how | read by |
|---|---|---|
| **shadow table** | area of the grid's projection along each of the 162 vertices of a twice-subdivided icosahedron, interpolated linearly across the face a direction passes through | starlight and beams arriving, brightness |
| **broadside** | the direction of largest shadow, and the roll about the nose that carries +z onto the direction across the nose with the largest shadow | the idle attitude of [20-solar-power.md](20-solar-power.md) |
| **envelope** | the union's distance field offset by `envelope_margin` and its two nearest parts blended over `ENVELOPE_BLEND = 0.5` of the cube root of hull volume; area and volume by marching tetrahedra | the field's area and volume, [30-the-field.md](30-the-field.md) |
| **inertia tensor** | the filled cells, weighted by each part's density, as the full symmetric tensor per kilogram, scaled to the ship's whole mass: a form is symmetric only port to starboard, so the xz product is generally not zero | slew rate |
| **extent** | the envelope's longest dimension: its widest width along the axes and the table's 81 directions, within about 1.2% of its diameter | `length_m`: the camera, the zoom limits, `Presence` |

The grid is `form::grid::FormGrid`. Its box is the field's bounds padded by half as much again as
the envelope can reach, so the hull spans about 51 of the 64 cells for the starting form and every
preset, and about 57 for a form of one part, which blends with nothing and reaches only its offset.
The pad doubles until the envelope closes inside the grid, and a form whose envelope never does is
refused as `Extent`; no preset needs a second pass. Four choices the table leaves open:

- **The envelope offsets a truer distance than the field.** Off an ellipsoid the field is a bound
  that falls short by up to the ratio of its axes, so an offset of it would put the starting hull's
  tips five margins out. The envelope reads `Sdf::estimate_each_with`, which replaces the bound with
  `k₀(k₀ − 1)/k₁`: exact along the axes and to first order at the surface. Hull volume, for the
  margin, is the parts' closed forms summed, so the offset does not move with the resolution.
- **The blend is between the two nearest parts**, and never with a part something encloses, which
  would raise a blister over its encloser. It adds at most a quarter of its radius anywhere.
- **A shadow is cast by the cells either side of the surface**, each cut by the plane its field's
  gradient gives. Whole cubes stand out past the rim by up to half a diagonal, which on a hull a few
  cells thick is a tenth of its shadow. A one-ellipsoid form is within 0.9 of a cell of rim of `A(ŝ)`
  in every direction. Broadside and roll are fitted, not climbed to: a projection is noisy to a few
  parts in a thousand, and the shadow is flatter than that near its peak.
- **The cells carry contents only.** Structure, stored energy and heat are taken to lie where the
  contents do, so the tensor is kept per kilogram and scaled by whatever the ship weighs. A cell's
  density is its nearest part's by the envelope's truer distance, which the same pass has computed.

Building one takes about 20 ms for the starting form and 50 ms for the Cluster with `lc-world`
optimized, and a quarter to three quarters of a second unoptimized in the dev profile.

### What a craft reads

`lc_world::fitting::Hull` is what the account and the craft read off a form, worked out when the form
changes and never per tick: at creation and load, and when a refit step finishes.

| number | from | read by |
|---|---|---|
| capacities | volumes, as above | storage, drain, building, data |
| dry mass | contents and structure | mass, and so every rating |
| thrust | the aperture of the engines **firing aft**, over `c`: those whose open face points to −x | the rated acceleration. An engine firing fore pushes the other way, and is a weapon ([31](31-directed-energy.md)) |
| extent | the grid | `length_m` |
| gyration | the grid's tensor: the square root of the larger eigenvalue of its block across the nose, per kilogram | the slew rate |

**Slew goes as one over the gyration.** Attitude thrust is sized to the ship as its drive is, so
torque over mass is the same for every ship, angular acceleration goes as `1/k²` and the rate as
`1/k`. For one shape `k` is a fixed fraction of the length, so this is the old `1/L` law, and it is
anchored on the same hull: the 500 m ovoid turns at π/60 rad/s. The starting form's `k` is
139 m against the ovoid's 130, so it flips in 64 s rather than 60. A craft with no form is still
that ovoid and turns by its length. The flip's axis is the slower of those across the nose, since
a flip is about one of them and the nose never turns about itself.

A process builds each form's grid once: every ship today is the starting form, so a shard fitting a
hundred builds one. A step partway through a round, whose form may not place, keeps the last
measured extent and gyration until it finishes. The starting form's extent is **571 m**, where its
twenty slots made 19's ship 500 m long. Until F10 reads the shadow, collection still takes the
ovoid of the craft's length, so the starting form collects (571/500)², about 1.3 times, what
[20](20-solar-power.md)'s anchor says.

**The shadow handles concave shapes.** A stack of plates shades
itself and collects about what one plate would. A ship spread out collects more and turns more
slowly, because spreading out also raises its moment of inertia.

The shadow table replaces the analytic ellipsoid `A(ŝ)` in `lc_world::solar`. A form that is one ellipsoid must
reproduce that formula to within the grid's resolution.

### Hull structure follows area

Each copy of a part carries structure at `hull_areal_density` per square meter of **its own surface**, from its
primitive's closed-form area (Thomsen's approximation for an ellipsoid), again ignoring overlaps. Flattening buys
shadow, radiating area and room on the surface, and pays for them in mass, so in acceleration.
`hull_areal_density` is anchored so the starting form weighs what 19's starting ship does. The
starting form and the built-in presets use no mirrors, so how a mirror is counted cannot move the
anchor.

### Placement rules

A target that breaks one is refused, naming the part.

- **Engine parts point along the nose axis, fore or aft,** and need a clear cone of
  `engine_clear_half_angle_rad` along it, checked on the grid's filled cells.
- **A bay's mouth must be clear** out to its own width.
- **Every attached part touches its parent**, and every enclosing part contains its parent.
- **The envelope's extent stays inside `LENGTH_RANGE_M`.**
- **Every part stays at or above `min_part_m3`**, and drones at or above `min_drone_m3`, every copy
  counted.

The rules are `form::rules::check`, which the server and the editor both call. It returns **every**
fault, in the order above and by part id within a rule, so the editor can mark each part; the server
refuses with the first. A form that fails the structural checks gets only that fault, since nothing
else can be judged on it. On success it hands back the grid, which the caller was going to build
anyway. The size rules alone are `rules::sizes`, which needs no grid, and the refit planner refuses on
them itself. Geometry is judged on the grid's cell centers, in its fixed order, so the server and a
client agree to the bit, and to its resolution, **half a cell's diagonal**:

- **An engine's or a bay's open face** is where its axis leaves it: a frustum's wide end, as 31 makes
  its aperture, and every other primitive's +x end, away from the foot it hangs by. A torus opens
  through its hole. Its radius is the disk inscribed in that end: the lesser cross semi-axis of an
  ellipsoid, half the lesser end edge of a slab, so a slab's corners reach outside it.
- **The cone** widens at the half-angle from the face's rim, not from its center, since the exhaust
  leaves the whole aperture. A filled cell in it that lies inside any other part blocks the engine,
  the engine's own mirrored copy included. Every filled cell is tested rather than rays marched
  between them, which would leave gaps between rays that widen with distance and need a step.
- **A bay's mouth** is a disk of its face's radius, cleared out to its width, twice that radius: a
  hull that fits through the mouth has room to leave.
- **Touching** is a cell center within the tolerance of both parts' primitives, uncut and unblended,
  plus a quarter of the joint's blend radius, the most its fillet reaches. Both fields are Lipschitz 1,
  the ellipsoid's bound too, so a part that touches its parent always passes. The bound is short
  beside an ellipsoid's long axes, so there a gap up to its axis ratio times the tolerance passes too.
- **Containing** samples the parent's surface along the shadow table's 162 directions and a cube's 26,
  which find a slab's corners, and reads the encloser by the envelope's truer estimate, whose sign is
  right everywhere. A part the grid cannot resolve, such as the Mind inside anything a cell across,
  is contained as far as the server can tell.

## Refits

A refit takes the ship from its form to a target form. It is one **round** of edits, and a round runs
in three phases, strictly in order:

1. **Dismantle.** Every part that shrinks, disappears or is being reshaped gives up what it loses.
   Drones go last. Each return is 95% of the mass-energy removed and lands in storage **if there is
   room**, where "room" means capacity left after this phase's own storage losses. **What does not fit
   is vented into the field as heat**, a burst at the end of the step that frees it.
2. **Move.** Parts whose placement changed slide to their new place, root first. Free in energy.
3. **Build.** Every part that grows, is new or is being reshaped is built, drones first. Each takes its
   added mass-energy, structure included, from storage.

| change to a part | phase | energy | time |
|---|---|---|---|
| bigger, same proportions | build the difference | its mass-energy | energy ÷ drone power |
| smaller, same proportions | dismantle the difference | 95% back | energy ÷ drone power |
| new | build | its mass-energy | energy ÷ drone power |
| removed | dismantle | 95% back | energy ÷ drone power |
| **proportions, primitive, or a spar's mode** | dismantle all, then build all | the 5% loss on all of it | both |
| **anchor, mount, twist, tilt, standoff, blend or parent** | move, carrying its subtree | none | `move_work_factor` of what building the subtree would take |
| **mirror**, or a new parent, that adds a copy or takes one away | build or dismantle that copy, for each part in the subtree whose count changes | as new or removed | as new or removed |
| **kind**, other than a spar's mode | removed, then new | as those two | as those two |

**Every row counts every copy.** A mirrored part that grows builds the difference on both copies, and
a move carries both copies of whatever it moves. Mirroring is priced as matter because it is matter:
if it were a move, a player could double a part for a fraction of its cost. A part whose count of
copies changes in the same round as its size resizes the copies both forms have, then builds or
dismantles the other whole, so the 5% loss cannot be dodged by trading a copy for size.

Data takes `data_work_factor` times as long as its energy says, as in 19. Drone power is measured at
each step's start, so drones built first speed up everything after them.

**The target is refused if the build phase cannot be paid for** from what the dismantle phase leaves in
storage. The target is **not** refused for venting. A player may vent heat on purpose, and
[30-the-field.md](30-the-field.md) says what happens when the vent is too big.

The planner is `lc_world::refit::rounds`. What it settles that the table does not:

- **Within a phase, parts go in id order**, drones apart as above, and moves by depth in the tree
  the round began with.
- **A move carries what hung from the part when the round began**, as much of it as the dismantle
  phase left, weighted by each part's work factor. A part both reshaped and moved still moves,
  carrying its children, and is rebuilt in its new place.
- **A store that shrinks while fuller than its new capacity spills the excess** as part of its own
  step's vent. The ledger is the round's own: drain and income are the ship's, and are not in the
  energy check.
- **A round is refused if the dismantle phase would leave no drone standing**, whether the drones
  are removed, replaced by new parts or reshaped, since the build phase would begin with none.
- The Mind's stored shape and volume may change freely, since nothing reads them.
- **A copy gained or lost is its own step**, an `Add` or `Remove` of the part, which carries the
  part's `mirror`. The copy lost goes at the size it had, and the one gained comes at the size it
  will have. A mirror inside a mirrored subtree, which changes no count, is a move that takes no
  time. Counts in the form partway through follow the flags, so a subtree follows its root's step.

### Why strict phases

The planner could interleave dismantles and builds so that energy in transit never piles up. It does
not, because the strict order is predictable, and because it makes one rule do a lot of work:
**energy in transit has to be stored somewhere.** A large transformation in one round vents the
difference into the field, which may kill the ship. Done properly it takes rounds:

1. Take the old section apart, and grow storage to hold what comes back.
2. Spend that storage on the new section.
3. If you like, shrink the extra storage away again.

Each round is visible and survivable, and the editor says so before each Apply. Emptying a full
storage part before reshaping it follows from the same rule, with nothing special added.

### Cancel

Completed steps stay. The step in progress is reversed. A build's energy comes back at the 95%
rate, or goes to the field if storage has no room. A dismantling is put back: what it had returned to
storage and what it had radiated are paid back out of storage, and the part is left as it was. The
radiated loss is not recovered: putting a part back costs its whole mass-energy. If storage cannot
pay, the dismantling finishes at once instead, and its return goes to storage as far as there is
room. A move snaps back. Whatever the reversal or the finish loses, and whatever has no room, is a
burst at the moment of cancel. The ship is left partway between its form and the
target, which may be worse than either, and that is intended.

Flying and refitting still exclude each other, as in 19.

## The starting form

- the Mind
- **storage**, an ellipsoid at 5 : 3 : 1, enclosing the Mind: 2.36 × 10⁶ m³, 30 ME
- **engines**, one frustum aft: 1.96 × 10⁶ m³, 5 g full
- **drones**, a capsule under the keel: 7.85 × 10⁵ m³
- **living**, a slab across the dorsal face, and **data**, a small capsule forward: 3.93 × 10⁵ m³ each

Each volume is 19's module count of its kind times the 392 699 m³ slot, so the form holds exactly
what 19's starting ship does. Placed, it is about 545 m long, 200 m across and 190 m deep:

- the storage is 335 × 201 × 67 m;
- the engine flares aft, 116 m wide where it meets the hull and 175 m at its open face, which is the
  aperture ([31](31-directed-energy.md));
- the drone capsule, 169 m long, lies fore and aft under the keel with its top sunk into the hull, so
  it is tilted off the normal it is attached along;
- the living slab is 145 m across the beam, 96 m fore and aft and 29 m deep;
- the data capsule, 124 m long, is sunk into the nose.

`hull_areal_density` is 1 215 kg/m², so this form weighs 19's dry starting ship, 2.65 × 10⁹ kg: every
module, the data module at half, and 19's frame over all twenty slots, the five empty ones included.
Its one bell fires aft, so it pulls 5 g full and 13.8 g empty, as 19's ship did.

The anchors of 20 and 30 are derived from this form.

The editor also offers **built-in presets**, each rearranging the ship's current volumes:

| preset | shape |
|---|---|
| **Plate** | one wide slab with everything on its faces. Largest shadow for its volume, slowest to turn |
| **Spindle** | capsules on one long axis. Smallest shadow nose-on, quickest to turn |
| **Cluster** | separate bodies on spars about a core, all inside one envelope |

A preset is only a target. Applying it is a round like any other, usually made of moves.

### Your own presets

A player can save the draft as a preset, name it, and apply it to any ship they fly later. A saved
preset is a `Form` with its part ids, and it applies in one of two ways:

| applied as | what the target is | typical round |
|---|---|---|
| **layout** | the preset's arrangement, filled with this ship's volumes. Each part keeps its share of its kind's total, so the ship keeps what it has and changes shape | moves and reshapes, little energy |
| **design** | the preset exactly, volumes included | builds and dismantles, paid for as usual |

The built-in presets are layouts written in code, filled with the starting form's volumes so each is
also a design. Applying either kind replaces the draft, not the ship, so the result is one undoable
edit and nothing is spent until Apply. A preset whose kinds the ship lacks gives those parts nothing in
a layout, and the editor names them.

A layout's other rules:

- **Spars are not shared out.** They hold nothing, so a ship's spar volume says nothing about where
  its parts go. They are the arrangement's own structure and grow by the ratio its other parts grew
  by, so a cluster applied to the starting ship, which has no spars, keeps its spokes.
- **A share below `min_part_m3` is dropped**, smallest first, and the rest of its kind take its volume.
  It is named with the parts given nothing.
- A part given nothing is left out, and its children hang from its nearest ancestor that is kept.
- A kind the ship has and the preset has no part for is left out of the layout, and the editor names
  that too.

- **Presets belong to the account, not to a craft.** They are designs a player carries in their head,
  not facts about the world, so they are not knowledge and do not travel at the speed of light. They
  are kept in `lc-store`, so a player sees the same list on the desktop and in a browser, which has
  nowhere good to keep files.
- **They can be shared as text.** Export copies the preset as a RON string and Import reads one from
  the clipboard. Sharing designs between players then needs no server feature and nothing to moderate.
- An imported or stale preset is only a draft: the server validates the target when it is applied,
  as it validates any other.
- Limits, so a store row stays small: `MAX_PARTS` parts to a form, `MAX_PRESETS` presets to an
  account, and a name of at most `PRESET_NAME_LIMIT`, 64 bytes. Both preset limits are in
  `lc_proto::form`, because both ends check them.
- Each limit is refused by name: `TooManyPresets`, `PresetName` for a name that is empty or too
  long, and `Form(TooManyParts)`. Saving under a name already kept replaces it, so it is never one
  too many. Nothing else about the form is checked on save.
- The shard sends an account its whole list, by name, after `Welcome` and after every save or
  delete. There is one connection to send it to: signing in displaces the account's earlier one,
  so a player moving from the desktop to a browser gets the current list with the welcome. It keeps the lists in memory, checkpointed
  beside the bookmarks, and reads them at every boot, including a shard starting a new world. A row holds the form in
  postcard, the wire's encoding, so its shape is pinned by `lc-proto`'s goldens.

## The editor

**A third mode of the main view**, beside World and Map: `ViewMode::Form`, on `H`. It is not a window,
for the reasons the map is not one ([07-rendering.md](07-rendering.md), [13-client-shell.md](13-client-shell.md)):

- It needs a camera of its own, orbiting the ship at editing distance and ignoring the boom.
- Its handles need the pointer everywhere in the view. In a window, egui and the sky would fight over
  every drag.
- One surface at a time ([18-ui-style.md](18-ui-style.md)).

The sky takes the corner square, as in the map's mode. The clock does not stop.

**The editor's camera carries a zero-sized marker, `FormCamera`,** as `SkyCamera`, `MapCamera` and
`UiCamera` do, and every query that wants it filters on that marker. Bevy gives no order between
cameras spawned by separate systems, and a fourth camera would otherwise turn every `.single()`
camera query into an early return, the failure [AGENTS.md](../../AGENTS.md) describes for the map's.
Existing queries already filter on `SkyCamera`, so adding the editor's should break none of them.
Check any that do not before it lands. The editor's camera must also never be the first one created,
or `bevy_egui` gives it the primary context.

**The editor is where refits are made.** The refit window (`R`) is the ledger: the budget, the
three phases, progress, and Cancel.

### Getting in and out

| key | from World | from Map | from the editor |
|---|---|---|---|
| `H` | the editor | the editor | back to where it was entered from |
| `M` | the map | the world | the map |
| `Escape` | closes the top window, or opens the menu | the same | closes the top window, or leaves to where it was entered from |
| click on the corner square | the map | the world | the world |

The editor is the one mode with somewhere to go back to, so `Escape` goes there before it opens the
menu. `M` always means the map; from the map it means out of it.

### Its camera

An orbit about a focus on the ship's nose axis (x in the ship's frame). It keeps every control the
other two modes have, with the same sensitivities, so a drag means the same turn in all three:

| control | World | Map | editor |
|---|---|---|---|
| right-drag (the cursor is locked) | turns the view | turns the camera | orbits the ship |
| held arrow keys | turn the view | nothing | orbit the ship |
| wheel, `=` / `-` | the boom, in hull lengths | zooms toward the pointer | in and out, in the form's own size |
| left-drag | picks | pans the plane | on empty space, slides the focus fore and aft; on a part, nothing yet: reserved for selection and handles |
| Shift+wheel | the boom | zooms | slides the focus fore and aft |
| held `PgUp` / `PgDn` | nothing | nothing | slide the focus toward the nose / the stern |
| right-drag on the corner square | — | turns the ship's view | turns the ship's view |

- **Fore and aft is the spaceplane hangar's move.** The focus slides along the nose axis and stops
  at the stem and the stern. Long ships are the point: a 50 km hull is navigable end to end, and
  zoomed in on one end the rest is simply further along the slide.
- **Everything is in the form's own size**, the diagonal of its bounds: the distance, and how far
  the focus is along. A 500 m ship and a 50 km one open framed alike and a notch means the same on
  both. The slide is in stand-offs, so the same drag moves the ship as far across the screen at any
  zoom, and the point under the cursor follows it.
- **The near stop is outside every part** however the camera is turned: outside the form's bounds,
  grown by 15%, along the line of sight. Looking nose-on at a long hull it is off the bow, not on the
  axis inside it. The far stop leaves the whole form a small thing in the middle.
- **Up tilts the view up** on the arrows and the drag alike, as it does over the sky, which on an
  orbit is the camera sinking under the ship. The map's turn has the same signs.
- **The press decides.** A drag that starts on a window, on the corner square or on one of the
  editor's own controls belongs to it wherever it goes, and one that starts on a part belongs to
  the part. `form_view::drag_of` is that rule, and it is tested without a window.
- The page keys are the reader's while a book is open, as the arrows are, and the slide stands down
  then.

### How it is drawn

- **Its own camera, `FormCamera`, on its own layer**, drawing copies of the parts in the ship's
  frame, in meters about the render origin, into an image. Copies, because the sky's pieces are
  placed relative to the eye in astronomical units and lit by the star, and an entity has one
  transform and one material; the draft will differ from the ship anyway. An image, because two
  cameras on the window's own texture clear and tone-map over each other, which is why the map
  renders into one too.
- **The image is laid out in Bevy UI**, under the readout and around the corner square, where the
  sky's camera draws exactly as it does in the map's mode.
- **The editor's own controls are Bevy UI**, in `em_ui`'s widgets, so C2's handles, tree and fields
  composite over the rendered view. egui draws over Bevy UI, so every window still floats over the
  editor. Bevy UI has its own pointer: the look button, the wheel and the slide all stand down over
  an `em_ui` control as they do over an egui one (`em_ui::Controls`).
- **A hangar's light, not the star's.** A key light over the camera's shoulder and a fill under it,
  so the side being looked at is always lit. The star is honest and leaves half the ship black, and
  what is being judged here is shape: which part is where, and how big. How the ship looks under
  its own star is one key away, and in the corner square the whole time.
- **Until the draft exists it shows the ship's own form**, or the starting form for a ship that has
  none yet.

`--view form` starts in the editor; `--turn`, `--pitch`, `--zoom` and `--slide` move its camera, and
`--form spindle*100` stages the preset a hundred times larger, a hull 92 km long.

### The budget

Always on screen, and updated with every handle:

- **available**: what is stored, plus 95% of everything the draft removes
- **spent**: the mass-energy of everything the draft adds
- **peak in storage** at the end of the dismantle phase, against capacity, and **what would be vented**,
  with its effect on the field

A part that cannot be paid for cannot be placed, and a handle stops at the size the budget allows.
Venting is shown in the field's own terms, as the peak temperature it would reach. It is allowed, but
Apply asks once more when the vent would collapse the field.

### Handles

| handle | does |
|---|---|
| drag the part | slides its anchor over the parent's surface |
| ring | twist |
| size | grows or shrinks it **at fixed proportions**, snapped |
| arrows on each axis | stretch the proportions **at fixed volume**, snapped. This is a reshape, so the part is rebuilt |
| standoff arrow | out along the normal, or in to embed |
| mount | attached or enclosing. While a part is enclosing, the editor keeps its last anchor and standoff so switching back restores them. They are editor state, not part of the form |
| spar mode | saddle or strap. A reshape: the cut changes, the charged volume does not |
| mirror | for the subtree |
| add | a primitive and a kind, attached where the pointer is |
| delete | the part and its subtree. Refused for the Mind and for the last drones |

Handles show **what a part does, not how big it is**: "drive section 1.1 × 10²⁰ W, 5 g on this ship",
not a count of cubic meters. The volume is in the side panel for anyone who wants it.

### Snapping

Volumes are continuous in the data. **The editor snaps them**, and its preferred steps are a table in
the client, so tuning the feel is a data change:

| what | default steps |
|---|---|
| volume | the R10 series: ten steps per decade, each about 26% larger than the last, anchored at `min_part_m3`. A modifier gives R40 |
| proportions | ratios on the same series |
| twist and tilt | 15°, and 5° with the modifier |
| anchor | the parent's axes and their diagonals, and free with the modifier |
| standoff | tenths of the child's reach |

A ladder of ratios rather than fixed amounts means the same handle is fine on a 500 m ship and on a
50 km one. The server never snaps anything. A console command or a hand-built target may ask for any
volume above the minimum.

### What else it shows

- **The draft**, drawn as structure over the ship as it is, drawn as a faint ghost. Parts to be built,
  dismantled, moved and rebuilt each get a mark from [18-ui-style.md](18-ui-style.md)'s palette.
- **Side panels**, in `em_ui`'s widgets like the rest of the editor's controls: the tree of parts,
  and the selected part's primitive, kind, volume and placement as editable numbers. Every handle
  has a field, so anything done with the mouse can be typed exactly.
- **The preview**, a pure function of `Session` and `Ui`: capacities, acceleration, broadside shadow,
  envelope area, slew rate, the field's rated load and headroom, brightness at the ship's current
  distance from its star, and the round's duration.

### Undo and redo

The editor keeps a **history of edits to the draft**: a list of entries and a cursor into it. Each
entry records what changed, before and after, on the parts it touched:

| edit | before | after |
|---|---|---|
| add | nothing | the new part |
| remove | the part and its subtree | nothing |
| resize | volume | volume |
| reshape | primitive and proportions, or a spar's mode | the same |
| move | placement: parent, mount, anchor, twist, tilt, standoff, blend | placement |
| mirror | on or off | on or off |
| apply a preset | the whole draft | the whole draft |
| reset the draft to the ship | the whole draft | the whole draft |

- **Undo** writes an entry's *before* back and moves the cursor back one. **Redo** writes its *after*
  and moves forward. A new edit made with the cursor behind the end drops everything after it. That
  is the ordinary linear history, without branches.
- **Part ids make it exact.** Entries name parts by id, and a removed part keeps its id in its entry,
  so undoing a removal puts back the same part in the same place with the same children.
- **One gesture is one entry.** A drag records on release, not on every frame, and a typed field
  records when it is committed. The list reads as what the player did.
- **The history is a panel**, listing entries by what they say ("resized storage core, 2.4 → 3.0 ×
  10⁶ m³"). Clicking one moves the cursor there, undoing or redoing everything between.
- **It is temporary.** It lives in `Ui` beside the draft, survives leaving the view and coming back,
  and is cleared when a round is applied, because the ship is then the new starting point. It is not
  saved and not sent. It holds `MAX_HISTORY` entries and drops the oldest.
- `Cmd`/`Ctrl`+`Z` undoes, and `Shift` with it redoes. Both are `Action`s like any handle, so undo is
  tested without a window, as the preview is.

Every handle emits an `Action` ([13-client-shell.md](13-client-shell.md)). `--view form --shot`
photographs it, and `--form <preset>` stages a draft.

## Protocol and persistence

- `Loadout` is removed from the wire and from saves. `Order::Refit { target: Form }`.
- **Between F9 and S1** `lc-world` has no loadout and the wire still does. A ship created or loaded
  from a loadout is the starting form with each kind scaled by its count over the starting ship's,
  a count of zero leaving the part out. A loadout the wire asks for is read back off the
  capacities, a slot of each density a module, and the balance's per-module fields are each
  density over 19's slot. A loadout refit on the wire or in a save is dropped, as one whose recipe
  no longer planned always was. **The shard refuses `RefitLoadout` and `refit-magic` as
  `NotBuilt`**, as it refuses `Order::Refit`, and the refit window is a ledger with no draft.
  `lc_world::fitting`'s conversions are S1's to delete.
- `Fitted` carries a `Hull`: the form, each part's solved scale, the capacities, and the geometry's
  numbers.
- An invalid target is `Refusal::Form(FormFault)`, naming the part: the structural checks, then each
  placement rule.
- The wire's form types are `lc_proto::form`, mirrors with arrays for glam's vectors, converted in
  `lc_world::form`.
- A craft's form is saved. Old rows are not read: there are no players, so the format simply changes.
- Presets: `Inbound::SavePreset { name, form }`, `Inbound::DeletePreset { name }`, and
  `Outbound::Presets`, the account's whole list, sent after `Welcome` and after each change. A
  `presets` table in `lc-store`, keyed by account and name.
- **Other craft's forms reach a client only by being seen.** `Presence` gains the form, and it arrives with
  the light, so a ship seen mid-refit is seen in the shape its light left in.

## Balance

`Balance` loses the per-module fields and `slot_volume_m3` (F9), and gains the fields below. 19's
slot survives as `form::presets::SLOT_M3`, the volume a module-energy and the first guesses below
are quoted per.

| setting | first guess | meaning |
|---|---|---|
| `storage_density` | 1.27 × 10⁻⁵ ME/m³ | capacity per m³ |
| `drone_density_w` | 5.88 × 10¹³ W/m³ | building power per m³ |
| `engine_density_w` | 5.47 × 10¹³ W/m³ | aperture power per m³, so thrust is this over c |
| `living_density_w` | 1.13 × 10¹⁰ W/m³ | drain per m³ |
| `data_density_b` | 7.5 B/m³ | |
| `data_mass_fraction`, `bay_mass_fraction`, `spar_mass_fraction` | 0.5, 0.1, 0.05 | of module density. Every other kind is 1 |
| `min_part_m3` | 1 000 | the smallest part, and the Mind's size: a 10 m cube |
| `min_drone_m3` | 10 000 | the least drone a ship may keep |
| `spar_gap` | 0.5 m | how far a saddle stands off the neighbor it is cut to |
| `spar_thickness` | 2% of the parent's smallest dimension | a strap's depth |
| `move_work_factor` | 0.25 | a move's time over building what it carries |
| `hull_areal_density` | *anchored*: 1 215 kg/m² | structure per m² of part surface |
| `envelope_margin` | 0.05 | the envelope's offset over the cube root of hull volume |
| `engine_clear_half_angle_rad` | 15° | |

Limits, which are constants rather than balance: `MAX_PARTS` 256, `MAX_PRESETS` 64, and on the client
`MAX_HISTORY` 256.


## Where it goes

| crate | new | changed |
|---|---|---|
| `lc-world` | `form.rs` (parts, the tree and its structural checks) and its submodules `form/{primitive, place, sdf, capacity, presets, grid, rules}.rs`: sizing, placement, the distance field, capacities, the starting form and presets, the voxel grid (shadow, envelope, moments), the placement rules | `fitting.rs` loses `Loadout` and reads capacities. `refit/rounds.rs` plans rounds: three phases, one step per part change. `solar.rs` reads the shadow. `craft.rs` reads extent and moments |
| `lc-proto` | `form.rs`: the form's mirror types, `Hull`, `FormFault`, `Preset` | `Form` replaces `Loadout` in `Order::Refit`, `Fitted` and saves. `Presence` gains the form |
| `lc-store` | `presets.rs` | |
| `lc-server` | | validation and refusals. `persist.rs`. The console's fitting commands. Preset save, delete and list |
| `lc-client` | `form_view.rs`, `form_panel.rs`, `snap.rs`, `form_history.rs`, `presets_panel.rs` | `ui.rs` gains the view mode. The refit window becomes the ledger |

`form/grid.rs` and the client's mesher ([32-ship-rendering.md](32-ship-rendering.md)) both evaluate the
distance field that `form/sdf.rs` defines, so the grid the server reasons about and the surface the player
sees are one definition.

## Open

- **Parks**: a living variant that must face outward, and whether it holds air under the field.
- **What the Mind is for**, beyond the root. Whether it carries the craft's knowledge, and survives
  anything, is a question for [22-provenance.md](22-provenance.md) and [30-the-field.md](30-the-field.md)'s
  rule on death.
- **Radiator fins** as a kind, if the square–cube pressure needs one.
- **Whether moves should cost anything at all** once waiting has a price.
- **Other craft's forms at a distance.** A presence carries the whole form. Where the ship is a pixel,
  the shadow table alone would do, and it is smaller.
