# Ship form

What a ship is made of, what shape it is, and what it costs to change either.

**Status: designed, not built.** It replaces the loadout of [19-ship-fitting.md](19-ship-fitting.md):
**a ship is its parts**, and each part's volume is how much of its kind the ship has. 19's energy,
mass and drive rules stand.
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

Building one takes about 20 ms for the starting form and 60 ms for the Cluster with `lc-world`
optimized, and half a second to two seconds unoptimized in the dev profile.

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
  `engine_clear_half_angle_rad` along it, checked by marching rays through the grid.
- **A bay's mouth must be clear** out to its own width.
- **Every attached part touches its parent**, and every enclosing part contains its parent.
- **The envelope's extent stays inside `LENGTH_RANGE_M`.**
- **Drones stay at or above `min_drone_m3`**, and every part at or above `min_part_m3`.

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
- **Egui side panels**: the tree of parts, and the selected part's primitive, kind, volume and
  placement as editable numbers. Every handle has a field, so anything done with the mouse can be typed
  exactly.
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

`Balance` loses the per-module fields and `slot_volume_m3`, and gains:

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
