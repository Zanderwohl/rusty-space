# Ship form

What shape a ship is, who decides it, and which numbers the shape feeds.

**Status: designed, not built.** [19-ship-fitting.md](19-ship-fitting.md) says how much of each
module a ship has. This says where those modules are. [29-the-field.md](29-the-field.md) and
[30-directed-energy.md](30-directed-energy.md) are what the shape is *for* in play, and
[31-ship-rendering.md](31-ship-rendering.md) is how it is drawn.

## The rule

**The loadout sizes; the form arranges.** A player chooses proportions and placement. The
module counts set every volume, so no editor can make a ship bigger or smaller than what it has
built. `slots` stays the truth, as it is today, and the silhouette is what the player owns.

Placing individual modules was ruled out by scale. Slots go as the cube of length:

| hull | slots |
|---|---|
| 500 m | 20 |
| 5 km | 20 000 |
| 50 km | 20 000 000 |

A block editor that is fun at twenty is a chore at twenty thousand and impossible at twenty
million. Placing modules also implies caring about corridors, which is avatar-scale play and
out of scope ([00-overview.md](00-overview.md)).

## Shapes

A `Form` is a tree of **shapes**. Each shape is one primitive holding **one module kind**. That
is the simplest rule, and it keeps a ship readable from outside: the drive section, the habitat
and the bays are each visibly one thing.

| primitive | proportions the player sets | closed-form volume |
|---|---|---|
| ellipsoid | three semi-axis ratios | `4/3 π a b c` |
| capsule | length over radius | cylinder plus a sphere |
| slab | three edge ratios, corner radius | rounded box |
| cylinder | length over radius | `π r² h` |
| torus | major radius over minor | `2 π² R r²` |
| frustum | length, and the two end radii over each other | `π h (r₁² + r₁ r₂ + r₂²) / 3` |

Proportions are dimensionless. **Scale is never stored.** It is solved from the shape's volume.

### Kinds

Every module kind in `Loadout` gets shapes, plus two that are new:

- **frame**: the hull's free slots. These are drawn as bare truss, the open framework a ship
  grows into. Building a module takes a slot's volume out of a frame shape and adds it to a
  shape of that module's kind. Growing the hull adds frame.
- **bay**: a new module kind. It has a slot's volume and `bay_mass_fraction` of a module's mass,
  so it is large and light, and costs energy in proportion to its mass. A bay shape is a shell
  with a **mouth**, the opening on one face. Its interior holds a procedural grid of sub-bays,
  decks and gantries. The mouth's smaller dimension is the largest hull that can be launched
  from it. Nothing is built in bays until construction exists (phase 10), so bays come after
  the rest of this doc.

### Sizing

A shape's volume is **its own count of modules times `slot_volume_m3`**, and its scale follows
in closed form from its primitive and proportions.

- **Overlaps and fillets are ignored.** The volume of a blended union depends on its neighbors,
  so sizing against it would make one shape change size when another moved, and the answer
  would need a numerical solve that server and client might round differently. Each shape
  alone is closed form and exact.
- **A kind's count is shared between its shapes** by a weight each shape carries. Whole modules
  go to each shape by largest remainder, so every shape holds a whole number of slots. A shape
  with no modules is drawn as frame at its last size, or as nothing if it never had any.
- A kind with a nonzero count must have at least one shape. The editor will not delete the
  last one.

## Placement is relative

Shapes change size with every refit, so absolute coordinates would leave a child either
floating or buried. Each shape but the root hangs from a parent:

| field | meaning |
|---|---|
| `parent` | a shape id |
| `anchor` | a direction in the parent's frame. The attachment point is where a ray from the parent's center along it leaves the parent's surface |
| `twist` | rotation of the child about the surface normal there |
| `tilt` | the child's axis relative to that normal, as a small rotation |
| `standoff` | distance along the normal, in multiples of the child's own size. Negative embeds it |
| `blend` | smooth-union radius with the parent, as a fraction of the smaller shape |
| `mirror` | the subtree is repeated reflected through the ship's port–starboard plane |

The child is placed so its own surface meets the parent's at the anchor, offset by `standoff`.
When the parent grows, the anchor point moves out with its surface and the child moves with it.
When the child grows, it grows away from the parent.

For a torus, the ray from the center along `anchor` meets the inner surface first. Placement
takes the outermost crossing, so children hang off the rim.

The root sits at the craft's center with its long axis along the nose. The nose is
`lc_world::motion::facing`, as now.

Shape ids are small integers assigned by whoever adds the shape and checked for uniqueness by
the server. Steps and animation refer to shapes by id, so ids stay put across refits.

## What the server computes from a form

The shape is presentation until it becomes numbers. Everything below lives in `lc-world`, is
engine-free, and is **stated by the server in `Fitted`**, as `Balance` already is. The client
computes the same things for its preview and takes the server's numbers when they arrive.

The geometry is a **voxel grid of fixed resolution**, `FORM_GRID = 64` cells on the longest side
of the bounding box, whatever the ship's size. It is filled by evaluating the union's signed
distance at each cell center. Sixty-four cells on a side are 262 144 samples, which is nothing
at the rate refit steps complete.

| quantity | how | read by |
|---|---|---|
| **shadow table** | area of the grid's projection along each of the 162 vertices of a twice-subdivided icosahedron, interpolated between them | solar and field intake, beams arriving, brightness |
| **broadside** | the direction of largest shadow, and the roll that presents it | the idle attitude of [20-solar-power.md](20-solar-power.md) |
| **envelope** | the union's distance field offset by `envelope_margin` and blended with a large radius, so it covers the hull and smooths over what sticks out | the field's area and volume in [29-the-field.md](29-the-field.md) |
| **surface area** | the hull's, from the grid's boundary faces with a standard correction for the staircase | hull structure mass |
| **moments of inertia** | from the filled cells, weighted by each shape's density | slew rate |
| **extent** | the envelope's longest dimension | `length_m`, which the camera, zoom limits and `Presence` read |

**The shadow handles concave shapes**, which the ovoid formula could not. A stack of plates shades
itself and collects about what one plate would. A ship spread out collects more light and turns
more slowly, because the same spread raises its moment of inertia.

The analytic ellipsoid `A(ŝ)` in `lc_world::solar` retires. Its tests turn around: a form that is
one ellipsoid must reproduce it to within the grid's resolution.

### Hull mass follows area

[19-ship-fitting.md](19-ship-fitting.md) charges hull structure by slot volume. Once shape
varies, structure should follow **hull surface area** instead: `hull_areal_density` kg per square
meter of hull. That turns flattening into a real trade. It buys shadow, radiating area and
room on the surface, and pays for them in mass, so in acceleration.

`hull_areal_density` is anchored so that the default form of the starting loadout weighs what
it does today. The trade starts at zero, and only a ship reshaped away from that default feels it.

### Placement rules

A target form that breaks one is refused, naming the shape, as the planner's other refusals do.

- **Engine shapes point along the nose axis, fore or aft,** and need a clear cone of
  `engine_clear_half_angle` along it. Checked by marching rays through the grid. What the fore
  and aft engines are each for is [30-directed-energy.md](30-directed-energy.md).
- **A bay's mouth must be clear** out to its own width.
- **Every shape touches its parent.** A standoff that opens a gap is refused.
- **The envelope's extent stays inside `LENGTH_RANGE_M`**, so the camera and the reticle can
  still cope.

## Changing a form

A refit's target becomes `(Loadout, Form)`. The planner gains one kind of step, and every step
now names the shape it acts on.

| change | what the planner does | energy | time |
|---|---|---|---|
| more or fewer modules of a kind | builds or dismantles, in the shapes the weights say | as now | as now |
| weight moved between two shapes of a kind | dismantles from one and builds in the other | the 5% loss on each module moved | as now |
| **anchor, twist, tilt, standoff, blend, mirror or parent** | a **move** step | none | `move_work_factor` of what building the moved modules would take |
| **primitive or proportions** of a shape | dismantles every module in it, then builds them all again | the 5% loss on all of them | two refits' worth |

- **Rearranging is free and slow.** A move carries the whole subtree hanging from the shape, and
  its time counts every module in that subtree. Energy is untouched, so a ship can try a layout
  with nothing to lose but the wait.
- **Reshaping is rebuilding.** A shape with new proportions is a new structure. While it is being
  rebuilt the ship does without what was in it: a drone shape rebuilt is fewer drones for the
  rest of the refit, and a storage shape rebuilt vents what it held unless there is room
  elsewhere.
- Moves go after builds and before dismantles in the planner's order, so a subtree is not carried
  twice. Within that, the planner orders them root first.
- A refit that changes only the form has zero energy cost. It still needs a drone, which every
  loadout keeps.

## Default forms

A ship with no form, and every ship loaded from a save that predates forms, gets
`Form::default_for(loadout)`:

- storage as the root, an ellipsoid at the old 5 : 3 : 1
- engines as one frustum aft
- living as a slab across the dorsal face
- drones and data as capsules under the keel
- frame as a truss band at the waist

This is close enough to today's ovoid that nothing looks like it jumped.

The editor offers **presets**, each a function of the current loadout:

| preset | shape |
|---|---|
| **Plate** | one wide slab with everything on its faces. Largest shadow for its volume, slowest to turn |
| **Spindle** | capsules on one long axis. Smallest shadow nose-on, quickest to turn |
| **Cluster** | a ring of separate bodies about a core, each blended in, all inside one envelope |

## The editor

**A third mode of the main view**, beside World and Map: `ViewMode::Form`, on `H`. It is not a
window, for the reasons the map is not one ([07-rendering.md](07-rendering.md),
[13-client-shell.md](13-client-shell.md)):

- It needs a camera of its own, orbiting the ship at editing distance and ignoring the boom.
- Its handles need the pointer everywhere in the view. In a window, egui and the sky would fight
  over every drag.
- One surface at a time ([18-ui-style.md](18-ui-style.md)). A shape editor in a window over a
  moving sky is two dense things at once.

The sky takes the corner square, as it does in the map's mode. The clock does not stop.

### What it shows

- **The draft**, drawn as structure, over the ship as it is, drawn as a faint ghost. Shapes to
  be built, dismantled, moved and rebuilt each get their own mark. Palette entries come from
  [18-ui-style.md](18-ui-style.md), not from this doc.
- **Egui side panels.** The tree of shapes, and the selected shape's primitive, kind, weight and
  placement fields as numbers. Every handle has a field, so anything done with the mouse can be
  typed exactly.
- **The preview**, a pure function of `Session` and `Ui` like the refit panel's. It adds, beside
  the planner's energy and duration: broadside shadow, envelope area, slew rate, mass, the
  field's rated load and headroom, and brightness at the ship's current distance from its star.

### Handles

| handle | does |
|---|---|
| drag the shape | slides its anchor over the parent's surface |
| ring | twist about the normal |
| arrows on each axis | stretch the proportions **at constant volume**: lengthening one axis narrows the others |
| standoff arrow | out along the normal, or in to embed |
| mirror toggle | on the shape, for its subtree |
| add | a primitive and a kind, attached where the pointer is on the selected shape |
| delete | its weight goes to the kind's other shapes |

The loadout's sliders stay in the refit window. The window and this view edit **one draft**, and
either can Apply. The refit window is still the quick way to change counts without looking at
shapes. This view is where the shapes are.

Every handle emits an `Action` ([13-client-shell.md](13-client-shell.md)). `--view form --shot`
photographs it, and `--form <preset>` stages a draft for the shot.

## Protocol and persistence

`Form` goes on `Order::Refit`'s target and in `Fitted`, alongside the loadout and each shape's
solved scale. `Fitted` also gains the numbers the server computed. A craft's form goes in `Saved`.
A save with no form loads with the default. Wire and save formats change directly.

Other craft's forms reach a client **only by being seen**. `Presence` gains the form, and it
arrives with the light like everything else in a presence, so a ship seen mid-refit is seen in
the shape its light left in. Up close, that is the shape drawn. Far away, it is a shadow table
driving a point's brightness ([29-the-field.md](29-the-field.md)).

## Balance

| setting | first guess | meaning |
|---|---|---|
| `bay_mass_fraction` | 0.1 | a bay module's mass, and so its cost, over any other module's |
| `move_work_factor` | 0.25 | a move's time over building the modules it carries |
| `hull_areal_density` | *anchored* | kg per m² of hull, so the default starting form weighs what today's ship does |
| `envelope_margin` | 0.05 | the envelope's offset, as a fraction of the cube root of hull volume |
| `engine_clear_half_angle` | 15° | the cone an engine shape needs clear |

## Where it goes

| crate | new | changed |
|---|---|---|
| `lc-world` | `form.rs` (shapes, tree, sizing, defaults, presets), `form_grid.rs` (voxels, shadow table, envelope, moments) | `refit.rs` gains the move step and shape ids on steps. `solar.rs` reads the shadow table. `fitting.rs` charges hull by area. `craft.rs` reads extent and moments |
| `lc-proto` | | `Form` on `Order::Refit`, `Fitted` and `Presence` |
| `lc-server` | | validation and refusals. `persist.rs` |
| `lc-client` | `form_view.rs`, `form_panel.rs` | `ui.rs` gains the view mode. The refit panel shares the draft |

`form_grid.rs` is written against the SDF evaluation in `form.rs`, and the client's mesher in
[31-ship-rendering.md](31-ship-rendering.md) reads the same evaluation. The grid the server
reasons about and the surface the player sees then come from one definition of the shape.

## Open

- **Parks.** A living variant that must face outward, and whether it holds air under the field;
  see [29-the-field.md](29-the-field.md).
- **Radiator fins** as a shape kind, if the square–cube pressure turns out to need one.
- **Whether moves should cost anything at all** once there is something to lose by waiting.
- **Other craft's forms at the edge of resolution.** A presence carries the whole form. At a
  distance where the ship is a pixel, the shadow table alone would do, and it is smaller.
