# Ship form

What a ship is made of, what shape it is, and what it costs to change either.

**Status: designed, not built.** This replaces the loadout of [19-ship-fitting.md](19-ship-fitting.md).
Module counts and hull slots are gone: **a ship is its shapes**, and each shape's volume is how much
of its kind the ship has. The energy, mass and drive rules of 19 are unchanged.
[29-the-field.md](29-the-field.md) and [30-directed-energy.md](30-directed-energy.md) are what the
shape does in play, and [31-ship-rendering.md](31-ship-rendering.md) is how it is drawn.

## Why shapes and not counts

The first version of this doc kept the loadout and had the form arrange it. That meant every edit
was done twice: move a slider to have more of something, then place or resize a shape to hold it.
Two sources of truth that the player had to keep in step. With the counts gone, the player builds
what they want where they want it, and **energy is the only limit on what can be placed.**

Placing individual blocks was never an option. A hull's volume goes as the cube of its length, so
a 50 km GSV is a million times the volume of a 500 m ship. An editor has to work in parts a
player can see.

## Parts

A `Form` is a tree of **parts**. Each part is one primitive of one kind, with a continuous volume.

| primitive | proportions | closed-form volume |
|---|---|---|
| ellipsoid | three semi-axis ratios | `4/3 π a b c` |
| capsule | length over radius | cylinder plus a sphere |
| slab | three edge ratios, corner radius | rounded box |
| cylinder | length over radius | `π r² h` |
| torus | major radius over minor | `2 π² R r²` |
| frustum | length, and the two end radii over each other | `π h (r₁² + r₁ r₂ + r₂²) / 3` |

**Volume and proportions are stored. Scale is solved** from them in closed form. Overlaps and
fillets are ignored when sizing: the volume of a blended union depends on the neighbors, which
would make one part change size when another moved, and would need a numerical solve two
machines might round differently. Each part alone is exact.

### Kinds

Every kind but the Mind turns volume into a capacity at a density. The densities are converted
from 19's per-module values over its 392 699 m³ slot, so the numbers a ship flies by are unchanged.

| kind | per cubic meter | mass | notes |
|---|---|---|---|
| **mind** | nothing | module density | the root. See below |
| **storage** | 1.27 × 10⁻⁵ ME of capacity | module density | |
| **drone** | 5.88 × 10¹³ W of building power | module density | at least `min_drone_m3` must remain |
| **engine** | 5.53 × 10¹³ W of aperture | module density | points fore or aft. See [30-directed-energy.md](30-directed-energy.md) |
| **living** | 1.13 × 10¹⁰ W of drain | module density | parks and population later |
| **data** | 7.5 bytes | half | three times as slow to build |
| **bay** | a mouth, whose smaller dimension is the largest hull it can launch | a tenth | a shell with a procedural interior. After construction exists |
| **spar** | nothing | a twentieth | truss, for holding parts apart: cluster spokes, booms. Pays for its area like anything else |

Module density stays 395.8 kg/m³. **ME stays the unit of energy**, 1.397 × 10²⁵ J, which is what
one 19-era module weighed. It no longer means anything but the number.

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

The Mind is usually enclosed by the first part built around it, so it is inside the ship. The
editor shows it through the hull.

## Placement is relative

Parts change size, so absolute coordinates would leave a child floating or buried after a refit.
Each part but the Mind hangs from a parent, and it is placed in one of two ways:

- **Attached**: it sits on the parent's surface.
- **Enclosing**: it is centered on the parent and contains it. This is how a core wraps the Mind,
  and how a shell wraps a core.

| field | meaning |
|---|---|
| `parent` | a part id |
| `mode` | attached or enclosing |
| `anchor` | attached only: a direction in the parent's frame. The attachment point is where a ray from the parent's center along it last leaves the parent's surface, so on a torus a child hangs off the rim |
| `twist` | rotation about the surface normal, or about the parent's axis when enclosing |
| `tilt` | the child's axis relative to that normal or axis, as a small rotation |
| `standoff` | attached only: distance along the normal, in multiples of the child's size. Negative embeds it |
| `blend` | smooth-union radius with the parent, as a fraction of the smaller |
| `mirror` | the subtree is repeated, reflected through the ship's port–starboard plane |

An attached child's surface meets its parent's at the anchor, offset by `standoff`. When the parent
grows, the anchor point moves out with its surface. When the child grows, it grows away from the
parent.

The Mind's frame is the ship's frame: its axis is the nose, `lc_world::motion::facing`.

Part ids are small integers assigned by whoever adds the part, checked for uniqueness by the
server, and stable across refits. Steps and animation refer to parts by id.

## What the server computes from a form

A shape matters to play only once it becomes numbers. Everything below is in `lc-world`, is
engine-free, and is **stated by the server in `Fitted`**, as `Balance` already is. The client
computes the same numbers for its preview and takes the server's when they arrive.

**Capacities** are sums of volume × density per kind. They replace everything 19 read from the loadout.

**Geometry** comes from a **voxel grid of fixed resolution**: `FORM_GRID = 64` cells along the longest
side of the bounding box, whatever the ship's size. Each cell is filled by evaluating the union's
signed distance at its center. That is 262 144 samples, which is nothing at the rate refit steps
complete.

| quantity | how | read by |
|---|---|---|
| **shadow table** | area of the grid's projection along each of the 162 vertices of a twice-subdivided icosahedron, interpolated between them | starlight and beams arriving, brightness |
| **broadside** | the direction of largest shadow, and the roll that presents it | the idle attitude of [20-solar-power.md](20-solar-power.md) |
| **envelope** | the union's distance field offset by `envelope_margin` and blended with a large radius | the field's area and volume, [29-the-field.md](29-the-field.md) |
| **moments of inertia** | the filled cells, weighted by each part's density | slew rate |
| **extent** | the envelope's longest dimension | `length_m`: the camera, the zoom limits, `Presence` |

**The shadow handles concave shapes**, which the ovoid formula could not. A stack of plates shades
itself and collects about what one plate would. A ship spread out collects more and turns more
slowly, because spreading out also raises its moment of inertia.

The analytic ellipsoid `A(ŝ)` in `lc_world::solar` retires. Its tests turn around: a form that is one
ellipsoid must reproduce it to within the grid's resolution.

### Hull structure follows area

Each part carries structure at `hull_areal_density` per square meter of **its own surface**, from its
primitive's closed-form (or standard approximate) area, again ignoring overlaps. Flattening buys
shadow, radiating area and room on the surface, and pays for them in mass, so in acceleration.
`hull_areal_density` is anchored so the starting form weighs what today's starting ship does.

### Placement rules

A target that breaks one is refused, naming the part.

- **Engine parts point along the nose axis, fore or aft,** and need a clear cone of
  `engine_clear_half_angle` along it, checked by marching rays through the grid.
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
| **proportions or primitive** | dismantle all, then build all | the 5% loss on all of it | both |
| **anchor, mode, twist, tilt, standoff, blend, mirror or parent** | move, carrying its subtree | none | `move_work_factor` of what building the subtree would take |

Data takes `data_work_factor` times as long as its energy says, as in 19. Drone power is measured at
each step's start, so drones built first speed up everything after them.

**The target is refused if the build phase cannot be paid for** from what the dismantle phase leaves in
storage. The target is **not** refused for venting. A player may vent heat on purpose, and
[29-the-field.md](29-the-field.md) says what happens when the vent is too big.

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

Completed steps stay. The step in progress is reversed, and what it had moved comes back at the 95%
rate, or goes to the field if storage has no room. The ship is left partway between its form and the
target, which may be worse than either, and that is intended.

Flying and refitting still exclude each other, as in 19.

## The starting form

- the Mind
- **storage**, an ellipsoid at the old 5 : 3 : 1, enclosing the Mind: 2.36 × 10⁶ m³, 30 ME
- **engines**, one frustum aft: 1.96 × 10⁶ m³, 5 g full
- **drones**, a capsule under the keel: 7.85 × 10⁵ m³
- **living**, a slab across the dorsal face, and **data**, a small capsule forward: one old slot each

That is 19's starting loadout without the five empty slots, so the ship is about 9% shorter than
today's. The anchors of 20 and 29 are re-derived from this form rather than kept at their old values.

The editor also offers **presets**, each rearranging the ship's current volumes:

| preset | shape |
|---|---|
| **Plate** | one wide slab with everything on its faces. Largest shadow for its volume, slowest to turn |
| **Spindle** | capsules on one long axis. Smallest shadow nose-on, quickest to turn |
| **Cluster** | separate bodies on spars about a core, all inside one envelope |

A preset is only a target. Applying it is a round like any other, usually made of moves.

## The editor

**A third mode of the main view**, beside World and Map: `ViewMode::Form`, on `H`. It is not a window,
for the reasons the map is not one ([07-rendering.md](07-rendering.md), [13-client-shell.md](13-client-shell.md)):

- It needs a camera of its own, orbiting the ship at editing distance and ignoring the boom.
- Its handles need the pointer everywhere in the view. In a window, egui and the sky would fight over
  every drag.
- One surface at a time ([18-ui-style.md](18-ui-style.md)).

The sky takes the corner square, as in the map's mode. The clock does not stop.

**The editor is where refits are made.** The refit window (`R`) loses its sliders and becomes the
ledger: the budget, the three phases, progress, and Cancel.

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
| mode | attached or enclosing |
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
| standoff | tenths of the child's size |

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

Every handle emits an `Action` ([13-client-shell.md](13-client-shell.md)). `--view form --shot`
photographs it, and `--form <preset>` stages a draft.

## Protocol and persistence

- `Loadout` is removed from the wire and from saves. `Order::Refit { target: Form }`.
- `Fitted` carries the form, each part's solved scale, the capacities, and the geometry's numbers.
- A craft's form is saved. Old rows are not read: there are no players, so the format simply changes.
- **Other craft's forms reach a client only by being seen.** `Presence` gains the form, and it arrives with
  the light, so a ship seen mid-refit is seen in the shape its light left in.

## Balance

`Balance` loses the per-module fields and `slot_volume_m3`, and gains:

| setting | first guess | meaning |
|---|---|---|
| `storage_density` | 1.27 × 10⁻⁵ ME/m³ | capacity per m³ |
| `drone_density_w` | 5.88 × 10¹³ W/m³ | building power per m³ |
| `engine_density_w` | 5.53 × 10¹³ W/m³ | aperture power per m³, so thrust is this over c |
| `living_density_w` | 1.13 × 10¹⁰ W/m³ | drain per m³ |
| `data_density_b` | 7.5 B/m³ | |
| `mass_fraction` | data 0.5, bay 0.1, spar 0.05, others 1 | of module density |
| `min_part_m3` | 1 000 | the smallest part, and the Mind's size: a 10 m cube |
| `min_drone_m3` | 10 000 | the least drone a ship may keep |
| `move_work_factor` | 0.25 | a move's time over building what it carries |
| `hull_areal_density` | *anchored* | structure per m² of part surface |
| `envelope_margin` | 0.05 | the envelope's offset over the cube root of hull volume |
| `engine_clear_half_angle` | 15° | |

`recovery`, `drive_efficiency`, `module_density_kg_m3` and `data_work_factor` are unchanged.

## Where it goes

| crate | new | changed |
|---|---|---|
| `lc-world` | `form.rs` (parts, tree, sizing, capacities, the starting form, presets), `form_grid.rs` (voxels, shadow, envelope, moments) | `fitting.rs` loses `Loadout` and reads capacities. `refit.rs` plans rounds: three phases, one step per part change. `solar.rs` reads the shadow. `craft.rs` reads extent and moments |
| `lc-proto` | | `Form` replaces `Loadout` in `Order::Refit`, `Fitted` and saves. `Presence` gains the form |
| `lc-server` | | validation and refusals. `persist.rs`. The console's fitting commands |
| `lc-client` | `form_view.rs`, `form_panel.rs`, `snap.rs` | `ui.rs` gains the view mode. The refit window becomes the ledger |

`form_grid.rs` and the client's mesher ([31-ship-rendering.md](31-ship-rendering.md)) both evaluate the
distance field that `form.rs` defines, so the grid the server reasons about and the surface the player
sees are one definition.

## Open

- **Parks**: a living variant that must face outward, and whether it holds air under the field.
- **What the Mind is for**, beyond the root. Whether it carries the craft's knowledge, and survives
  anything, is a question for [22-provenance.md](22-provenance.md) and [29-the-field.md](29-the-field.md)'s
  rule on death.
- **Radiator fins** as a kind, if the square–cube pressure needs one.
- **Whether moves should cost anything at all** once waiting has a price.
- **Other craft's forms at a distance.** A presence carries the whole form. Where the ship is a pixel,
  the shadow table alone would do, and it is smaller.
