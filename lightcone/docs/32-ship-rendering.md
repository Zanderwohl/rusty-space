# Ship rendering

How a form becomes a picture: the hull, a refit being built, the drones doing it, and the field
around all of it.

**Status: partly built.** The hull material, the mesher, the drones and the field shader are in, each in a void. Construction is drawn on the placeholders in the game, and truss and plating on the meshed hull under `--demo refit`; see the "As built" notes. [29-ship-form.md](29-ship-form.md) is what is drawn,
[30-the-field.md](30-the-field.md) is the field's physics, and [31-directed-energy.md](31-directed-energy.md)
is what beams do.

## Two looks

- **From TNG: parts you can read.** Each part is one kind, and each kind looks like what it is.
  A drive section, a habitat and a bay are recognizable from outside.
- **From the Culture: a smooth field with the ship inside it.** Not the surface clutter of a Star
  Destroyer. Light does most of the work: lit living areas on the night side, the field's glow,
  the drive's open face.

The construction look is Cities: Skylines and SimCity 4: a skeleton goes up, is closed in, is
fitted out, and the scaffolding comes down. It is mechanical on purpose. The one time the ship
looks like machinery is while it is being built.

## The hull

### From distance field to mesh

The client evaluates the same signed distance field that `lc_world::form` defines, so the surface
drawn is the surface the server reasons about.

- **Surface nets**, on a grid sized to the ship's pixels on screen: coarse when it is a smudge, fine
  when it fills the view. Surface nets are smooth, cheap, and have no ambiguous cases to get wrong.
- Meshed **off the main thread** on Bevy's async compute pool, cached by a hash of the form and
  each part's solved scale, and swapped in when ready. A new mesh is needed only when a step
  completes. Within a step, the growing part is drawn by the construction pass below, not
  remeshed.
- **Finish** is a per-form choice: *smooth* (surface nets as they come), *faceted* (flat normals),
  or *blocky* (occupied cells drawn as cubes, for anyone who wants a brutalist ship). It changes
  extraction, not the shape, so it touches nothing the server computes.

As built (`lc_client::hull_mesh`): one cell per four pixels along the ship's longest side, as a
power of two from 16 to 256, held until the ideal is more than three quarters of a doubling away.
Before extraction the grid is cleaned of every lattice square whose corners alternate in sign by
turning one outside corner inside, so each cell face carries at most one segment of surface; a
vertex per loop of crossings in a cell, not per cell, then makes the mesh closed and manifold
whatever the form. Blocky is the same topology with each vertex moved to its cell's center, which
is exactly the cubes' faces. A feature thinner than a cell is kept only where a sample lands in
it: a strap on a coarse grid comes out holed or gone, never torn. The cache key is FNV-1a over a
canonical encoding of the form and each part's solved shape, since `Form` holds `f64`s. A
256-cell mesh takes a few seconds in a dev build and never holds up a frame; the old mesh stays up
until the new one lands. `cargo run -p lc-client --example mesh_void` photographs the fixtures.

![The fixture forms, meshed and drawn in the hull material](../images/mesh-forms.jpg)

![Smooth, faceted and blocky, at the grid the screen asks for and at 32 cells](../images/mesh-finishes.jpg)

![A saddle's bolt row, and a strap at 16, 64 and 256 cells: holed on the coarse grid, never torn](../images/mesh-seams.jpg)

![Consecutive frames across a remesh from 32 to 128 cells: the old mesh stays up until the new one lands](../images/mesh-remesh.jpg)

### In the game

As built (`lc_client::ship_hull`, R10): every craft with a form is drawn this way, the player's
from `Fitted` and everyone else's from the form its `Presence` stated, as its light left it. Only
a craft stated with no form is still the ovoid. Meshes are shared between craft of one design at
one band, and a hull measures its band from its own pixels on screen, so a distant ship is 16 cells
and a close one up to 256.

- **Nothing blinks.** A new form or a new band keeps the mesh on screen until its replacement lands.
  Until a craft's first mesh lands it is drawn as placeholders, which is the only place they are
  still drawn.
- **Another ship's roll** is its form's, from the same grid the server measures its broadside on,
  worked out on the async pool. A new form's roll is taken as its mesh lands, so the old shape
  never turns to the new one's roll.
- **A refit is drawn over it.** `refit_hull` stands a craft's hull aside once a step's meshes are
  up, the player's and anyone else's alike (R15). When the round is over, its last meshes stay until
  the real hull is the form the round left, so the craft never flashes back to placeholders or to an
  earlier shape.
- **Memory.** A vertex is 48 bytes and 24 of indices, held in the main world and on the GPU. The
  starting form is 45 kB at 16 cells, 0.7 MB at 64 and 11 MB at 256; Cluster is a quarter larger.
  A sky of a hundred distant designs is a few MB, and 32 meshes nothing wants are kept for zooming
  back.

![Two clients on one shard, each photographing the other: A rebuilt as Cluster, seen by B](../images/real-hull-b-sees-a.jpg)
![And B in the starting form, seen by A](../images/real-hull-a-sees-b.jpg)
![Cluster by Saturn, which was placeholders between refits until R10](../images/real-hull-cluster.jpg)
![Consecutive frames across remeshes in the game, 16 to 32, 32 to 64 and 64 to 128 cells: a burst with the camera dollying in](../images/real-hull-remesh.jpg)

### Details are sized in meters

The eye judges size by how small repeated detail is. So **every repeated detail has a fixed size
in meters, whatever the hull's size**: windows every few meters, panels tens of meters across,
girders at a fixed pitch. A 50 km ship reads as enormous because its girders are fine against its
length.

At a distance where detail would be smaller than a pixel, it fades into a texture that averages
it, to stop the shimmer. A distance-field mesh has no UV coordinates, so materials are mapped
**triplanar**, from world position in the ship's frame.

As built (`em_render::hull_material`): each kind's graph is baked as one repeating tile of 64 m,
and the fade is the tile's mip chain, built on the CPU in linear light, so a window too small to
see becomes its own average — a lit window's power spread over the pixel, not lost from it.
`cargo run -p lc-client --example hull_void` photographs it on a sphere of any size.

![The hull material on spheres of 500 m and 50 km, and the shimmer the mips remove](../images/hull-material.png)

### Materials by kind

One texture-graph graph per kind, delivered like the planet graphs, with the region set by which
part a point is nearest. Fillets blend between the two regions.

The mesher hands the material each vertex's weight for every region, a byte apiece, read off the
per-part distances it already evaluates, and the shader draws the two heaviest. Weights rather
than two indices and a share, because a triangle whose corners name different pairs cannot
interpolate a share, and every triangle crossing the edge of a fillet is one: that was tried, and
drew the edges as stairs. The material knows regions only as indices
into the caller's palette of graphs; which kind is which is Lightcone's. A graph's output color is
albedo, and a layer named `lights` beside it is the lit share of each texel, which the caller
scales by a power per region.

**Which face is open is the form's, not the distance field's.** As the spar's seams are, it is
handed per vertex: the mesher reads each engine's open face off the part (`capacity::apertures`,
the frustum's wide end or the cylinder's end) and writes how much of the vertex lies on it and
which face it is, a byte each. A vertex is on a face when it is on that engine's own surface, in
the face's plane, inside its rim, and turned along the exhaust both as the engine's primitive is
and as the hull is. The first keeps a fillet onto a neighbor off the face, the second a neighbor's
surface passing within a cell of it. Each is eased over a cell, and the rim's inside its edge,
because the vertex on the rim is the corner and belongs to the flank as much as the face. A vertex
off every face names the nearest, so a triangle at a rim names one face at all three corners and
the index can be read flat. The material then draws one region, the engine's, as itself only on
open faces, lit there by a color per face rather than per region, and as another layer, the
flank's, everywhere else. The switch is chosen, not blended, so the tile is sampled once; it falls
on the rim, which is a crease anyway. `em_render::hull_material::HullUniform::open` and `faces`.
The uniform holds 16 faces. A form with more engine copies than that draws the grid on the first 16
in placement order and machinery on the rest, which still take their aperture glow when lit; the
client warns when it meshes one.

| kind | look |
|---|---|
| storage | dark and smooth, faint seams. The mass of the ship |
| drone | hangar doors in rows, docks lit when drones are home |
| living | window bands. Lit on the night side, where they are the brightest thing on the hull |
| engine | an emitter grid on the open face, glowing at whatever leaves through it, and greebled machinery on its flanks: housings, louvered recesses, fittings and pipe runs, unlit. The grid is the only thing drawn on the face and the machinery the only thing drawn off it, so an engine reads as one at any distance even when nothing is lit |
| data | fine dense panels |
| mind | a small dark cube with one faint light. Drawn only when nothing encloses it, and always in the editor |
| spar | plated structure, with a row of bolt heads along every line where it meets a neighbor. The line is where the spar's distance and the neighbor's grown distance are both near zero, so the shader finds it with no geometry of its own. As built, the mesher hands each vertex its signed distance to the nearest seam and meters along it, and the shader puts a head every 1.5 m, 0.8 m in from the seam. Along is the one number a distance field does not hand over. The mesher classes each seam whole, from the two primitives' gradients along it, as a ring about the spar's axis or a line along it, and measures it as meters around at the seam's mean radius or meters along. A boom's end and a rib's edge are both in meters, and a seam that climbs spreads its heads only by the cosine of its climb. Chosen per vertex, the heads shear where the choice changes |
| bay | a shell with a mouth, and a lit interior grid of decks and gantries |

Living lights are emitters with a real (small) power, through the same exposure as everything
else, so they show on a night side and vanish in sunlight as they should.

As built (R15, `lc_client::ship_hull::lamp_of`, `lc_client::hull::lamp`): each kind's lights are a
luminance and a color temperature, a blackbody scaled in V against white in full sun at 1 AU
(40 000 cd/m²) and put through the band mapping as starlight is. A lit window is 300 cd/m² at
3000 K, as are drone docks and a bay's decks at their own temperatures; the Mind's light is 90;
the engine's grid has no lamp of its own: it is lit at the blackbody of what leaves each face
(below), and the flanks are unlit. The girders' work lights are
a floodlit yard's 2000 lux. Two things had to change for a night side to show them:

- **A real hull's night fill is 0.3% of its starlight**, not the ovoid's 10%, which outshone a
  window on the hull's paint anywhere inside about 2 AU. Its own lights give its night side a shape instead.
- **The exposure meters a hull by the share of its disc the eye sees lit**, plus a lit window. On
  its day side that is starlight, and the windows are lost; on its night side it is the windows,
  and the exposure opens for them. Between the stars the same rule exposes for the lights alone.

![By day and by night: Cluster at Venus's L1 (top) and Jupiter's (bottom)](../images/lights-day-night.jpg)
![Its living quarters close: lost by day at Venus; lit at night at Venus and at Jupiter](../images/lights-living-close.jpg)
![Between the stars, `--demo refit` exposed for its lights](../images/lights-between-stars.jpg)

![Every kind by day and by night](../images/hull-kinds.png)

![The spar's bolt row on both spheres, and the plating reveal mask partway](../images/hull-seams.png)

## Building, as a function of time

**What is drawn is a pure function of the refit's recipe and the time `t`.** No animation state is
stored, and nothing is sent. Each planner step is one part's whole change, and says which part, which
phase of the round, and the interval it occupies ([29-ship-form.md](29-ship-form.md#refits)).

Each part's volume at `t` is its volume at the round's start plus completed steps plus the completed
fraction of the current one. The step in progress acts on a **sliver**: the shell between the part at
its volume before the step and after it, or the whole part when it is new.

### A build step is a frontier

Every point of the sliver goes through four phases in turn. **When** a point starts depends on its
distance from where the part meets its parent, so the phases sweep across the sliver as bands, one
behind another:

| phase | drawn |
|---|---|
| **truss** | a lattice of girders |
| **plating** | panels close over the lattice |
| **fitting-out** | the kind's material fades in, windows light, emitters appear |
| **scaffold down** | the outer truss comes apart, and drone traffic carries it off |

The band widths and how far the sweep leads are client constants. The step's duration is the
planner's, so the last band reaches the far edge as the step ends.

- **The truss is a distance field too**: a repeating lattice of capsules at a fixed pitch in meters,
  intersected with the sliver's shell. It is meshed once at the step's start, since the sliver is
  known then.
- **Plating is a reveal mask** on the hull material: each panel has a hashed threshold offset by its
  distance from the attachment point, and a uniform sweeps across them.

One rule covers every size. Drone power and part volume both go as the ship's volume, so a
proportionate change takes about as long on any hull: a quarter more storage is about a real minute on
a starting ship with its drones, and about a real minute on a GSV with the same share of drones. What
differs is what that minute looks like. On the starting ship, the frontier crosses a new pod at a
glance. On a GSV it is a band of scaffolding kilometers wide, sweeping a face tens of kilometers long
with drone traffic streaming to it: a shipyard that is also the ship.

### The other steps

- **Dismantle** runs a build's phases in reverse, from the far edge in: scaffold goes up, the
  fitting-out comes out, the plating comes off, which shows the truss, and then the truss is taken
  down. Point by point that is the build played backward; what is not a rewind is the drone traffic,
  which carries loads home (R9).
- **Move** slides the subtree from its old anchor to its new one along a smooth path over the
  step. No construction, and drones swarm the joint.
- **Rebuild** is a dismantle of the whole part to nothing in the round's first phase, and a build of
  the new one in its last.
- **Cancel** runs the step in progress backward from the fraction it had reached, at its own pace.
  The ledger is back at the moment of cancel for every kind of step
  ([29-ship-form.md](29-ship-form.md#cancel)), so the backward run is only the picture. A dismantling
  that storage could not pay back finishes at once, in the picture as in the ledger.

As built (`lc_client::construction`): `Frame::at` takes the plan and the round's clock and reads
`Plan::at` for what is finished and what is under way, so no timing is derived twice. A form partway
through a round need not place, since a part may hang from one taken apart or not built yet; such a
parent stands in from the start's form while dismantling and from the target's once moving and
building, and is not drawn. Growth and shrinkage re-place the form at the interpolated volume, so
children ride out on a growing parent. A part appearing or going whole is scaled about its foot, which
is where the planner's placement would put it at that volume. A copy gained or lost is its own piece,
so both copies of a mirrored part build together. Each point's phase is `Working::look`, `d` of the
way across the sliver from the joint; each point spends 30%, 15%, 15% and 10% of the step in the four
phases, and the rest is the front's travel, so the far edge finishes as the step does.

On the placeholders the working part is solid at its volume at `t`. They draw a round only for the
moment before its first step's meshes land. Until R15 a cage of tubes at the sliver's outer size stood
for the truss there; it retired once every craft's round was drawn on the meshes.

As built for truss and plating (`lc_client::refit_hull`, `lc_client::truss`): **while a craft has
a round, the whole craft is drawn on the mesher and the hull material**, and the placeholders
stand aside once the first step's meshes are shown. Plating is a mask on R3's material and means
nothing on a Bevy primitive, so construction could not wait for R10. A
step is meshed once, as it starts: the ship it leaves alone (`Frame::standing`, placed as a form, or as
its bare copies where it hangs from a part not built yet), each copy it works on at its larger size,
and each copy's truss. Within the step only uniforms and poses move: what hangs from a part being
resized rides out on it, each copy posed every frame from `Frame::pieces`, as a move's carried copies
are, and the truss keeps out of where those copies will stand. Once the clock passes a step, its copies
are drawn as the step left them until the next step's meshes land, so a part taken apart stays gone. `Working::sweep` turns `Working::look` into
meters from the joint, a front and a width a band, which is all the shader needs to give every point
its own phase; a test holds the two to agreement across every step. A step's meshes are shown only
once all of them have landed, and the last step's stay up until then. A carried or riding copy is meshed in its own frame, bare,
so a fillet it has with the resized part is missing until the step is done.

- **The truss is whole girders, not a distance field meshed.** The lattice is the one described, at
  8 m square to the ship's frame, girders 0.35 m in radius, and it is kept where both of a girder's
  nodes lie between two pitches inside the copy's surface and one outside it, and outside the
  standing ship. The girders outside the finished surface are the scaffold. Meshing the lattice's
  field by surface nets would cost the shell's volume over the girder's radius cubed, about twenty
  million samples on a starting ship's hull; whole girders cost what is kept, 15 600 of them, and a
  truss stops at a node anyway. Each girder carries its distance from the joint and a hashed
  threshold, and stands while its band's share is past it, so the truss goes up and comes down a
  girder at a time. The mesher's core is shared all the same: `surface_nets` takes any Lipschitz
  field, and the working copies are meshed through it as bare shapes.
- **Past 60 000 girders there is no truss mesh**, and the shader draws the same lattice on the
  sliver's surface, in the face's two axes at the same pitch. That is every GSV. Close up it is
  girders over the gaps, which are discarded. Once a girder is under a pixel nothing is discarded,
  and plating, girders and what is behind them are drawn as their shares of the pixel, counting the
  three layers a line of sight crosses; a band of scaffolding kilometers wide is then a band of
  that color. The girders are safety yellow with their own work lights, so it reads as construction.
- **Plating** is R3's reveal mask with the joint as its origin, a panel a lattice cell. **Fitting-out**
  fades the kind's material in over bare plating and lights its emitters. **Scaffold down** takes the
  outside girders away. A dismantle is the same uniforms with the fraction run backward, so it goes in
  reverse. Where nothing is up yet the working surface is discarded, and the ship it grows from shows.

![A starting ship's hull growing: truss, plating, fitting-out, scaffold down; then the data core taken apart, scaffold up and truss down](../images/truss-starting.jpg)
![The same round on a 50 km GSV](../images/truss-gsv.jpg)
![From 150 m, the lattice on both: meshed girders on the starting ship, the shader's on the GSV, both 8 m](../images/truss-pitch.jpg)
![A burst at the demo's pace, frames 0 to 3 above and 20, 21, 38 and 39 below: panels close one at a time, nothing jumps, and the hull runs on through scaffold down to finished](../images/truss-burst.jpg)

`--demo refit` stages one of each step on the starting form (the data core taken apart, the deck
moved aft, the hull grown by half, a mirrored pair of pods on spars) as a client fixture beside
`--form`, not a scenario: it is only the player's ship. The
Cluster preset would be the obvious target, and the planner refuses it from the starting form.

![0.1: the data core being taken apart](../images/refit-10.jpg)
![0.2: further through the dismantle](../images/refit-20.jpg)
![0.3: the deck sliding aft](../images/refit-30.jpg)
![0.5: the hull partway grown, on the placeholders and their cage before R8](../images/refit-50.jpg)
![0.9: the mirrored pods being built](../images/refit-90.jpg)

### Rounds in the game

As built (`lc_client::construction`, R14): the game draws the player's round exactly as the demo does,
from a `Refit` on the coordinate clock rather than a frozen or looping one. `follow` takes each
statement of the round in `Fitted`, a pure function of what was stated: a new recipe starts a `Refit`, the same one stated
again changes nothing, and **a round that stops being stated before it is done was canceled** at the
statement that dropped it, with the form that statement carried, unless that form is the target:
then the round was finished at once, and the drawing settles on it. A round replaced by some other
form, as the console's refit at once does, is taken for a cancel and settles at once too, since the
form it left is not where the step stood and nothing runs backward. The wire says nothing more about a
cancel, and needs to say nothing more. From then the `Refit` draws `Frame::canceled`, and once
`Refit::is_over` (the round done, or the reversal run back) it is stood down: the hull meshes go in the
frame the placeholders come back, and nothing is left over.

- **The player's round** is the plan the ledger reads, `Fitting::refit` from `Fitted`, so the picture
  and the ledger agree step for step. The cancel's moment is the fitting's settlement, which is exact.
  The camera is framed on both ends of the round, as the demo's is, until the `Refit` is stood down,
  so a reversal is not reframed under it.
- **Another craft** states no round, only the step its light shows, in its `Presence`
  ([29-ship-form.md](29-ship-form.md#protocol-and-persistence)). `construction::sighted` draws that step
  on the stated form, reckoned on at the step's pace to `Contact::emitted_s`, never to now, so a
  distant ship is seen mid-build as it was; running back after a cancel, it runs back. Past the step's
  end it draws the step finished until the next statement says what came next, at most a tick
  later. Knowing one step, it stands in a missing parent from only the step's two ends, so a part
  hanging from one the round has not built yet is not drawn. `refit_hull` draws that frame over the
  craft's real hull as it draws the player's (R15), under the craft's own root, so it is placed and
  rolled as its hull is. Its drones (R16) are drawn from the same frame, under the same root, on a
  clock that is the time its light left, counted from when its swarm was spawned.
- The player's drones are placed in the ship's frame directly rather than under the placeholders' root,
  which is gone while the hull meshes draw; under `--demo refit` since R8 they had not been drawn at
  all. Their clock is the round's, from its start, in the game as in the demo.

![One client watching another's applied round, and closer: its drones at the frontier of the growing hull](../images/drones-other-craft.jpg)

`--apply` pins the view back to the world once the round is under way, so a real refit can be
photographed where it is drawn. From R15 it draws another craft's round too, light-delayed:

![Applied in the game and `--demo refit`, both from their night sides by Venus](../images/refit-game-and-demo.jpg)
![One client watching another's applied round: its hull growing in its truss](../images/refit-other-craft.jpg)

![Applied in the game: the drones grown first, inside their truss](../images/refit-game-applied.jpg)
![Canceled mid-step: the step running backward, truss coming down](../images/refit-game-cancel-early.jpg)
![Later in the same reversal](../images/refit-game-cancel-late.jpg)
![Run back: the placeholders again, and nothing left over](../images/refit-game-settled.jpg)

## Drones

Drones are **stateless particles**: each one's position is a closed-form function of its index, a
seed and `t`, evaluated in the vertex shader over one quad per drone. That is the same rule as the
hull: the picture is a function of the recipe and the clock. It also means no particle simulation
to keep in step, and no dependency such as `bevy_hanabi` to check against Bevy 0.19 and the browser
(WebGPU) build.

- **Count** follows drone volume, `particles_per_m3`, capped.
- **Working:** arcs from the drone part's docks to points on the sliver, a dwell at the frontier,
  and the arc back. On a dismantle they carry glowing pieces home, which reads as energy going back
  into storage.
- **Moving a part:** they swarm the joint.
- **Idle:** most docked, a thin patrol drifting over the hull.
- **At a distance:** the swarm fades into a soft haze over the frontier before individual motes
  would fall below a pixel.

As built (`em_render::drone_material`, `drones.wgsl`): the quads are one mesh, each carrying its
drone's index in a vertex, since Bevy's shared vertex buffers offset `vertex_index`. Docks and
targets are fixed arrays in the material's uniform, which the host fills. A hash of the index picks
each drone's role against two fractions, working and patrolling, so raising either adds drones
without reshuffling the rest. A working drone takes a new target every trip, switching while it is
docked. Haze is a mote's light spread over a disc about the spacing between drones, and never
narrower than a few pixels, because a quad under a pixel lands on no pixel center and sparkles. The
light is conserved, so the haze has the swarm's true brightness per pixel, as the hull does, and a
sparse swarm makes a faint haze. The player's clock is seconds since the refit round began (R4's
`t`), or since the view was spawned when idle. Another craft's is the time its light left, counted
from when its swarm was spawned, so its traffic does not restart with the round: the client never
learns when that began. The host takes that difference in `f64` and only then narrows
it to the shader's `f32`, which resolves a clock since J2000 only to seconds.
`crates/lc-client/examples/drones_void.rs` photographs it.

What the material is told (`lc_client::drones`) is a pure function of R4's `Frame`: the player's
from its `Refit`, another craft's from `refit_hull::buildings`, the step its light shows. Each other
craft has a swarm of its own, seeded by its id so no two move in step, and none while it has no round.
Its mesh is the population rounded up to a power of two, shared by crafts of a size, rather than the
player's full cap, and a craft spanning fewer than 24 pixels draws none: below that its haze, never
narrower than a few pixels, would be wider than the craft, and no traffic is computed for it.

The count is `PARTICLES_PER_M3` = 10⁻³ per m³ of drone part, 785 on the starting ship, capped at `MAX_DRONES` = 8192 quads, since the vertex shader runs over all of them every frame.
Past the cap a mote stands for several drones: a fixed share of the width of the cube of drone part
it stands for, with the rest of their light in its brightness. Widening it enough to carry all the
light in area drew a GSV's swarm as a few hundred blobs. Docks are points just off the drone parts' surfaces, where no other
part covers them. A build or a dismantle sends drones to its sliver, and a dismantle's come home
glowing. A move sends them to ring the moved part's rim, from its foot to its middle. With no step
working, a few patrol and the rest stay docked.

The shader gives each drone a new target every trip, so a target that jumped would make every
drone on its way there jump with it. Instead each target is a continuous function of the step's
fraction: it sits on a fixed meridian of the part, where `Working::across` is a fixed share of the
way through the band. It is stood off along the meridian's ray rather than the surface normal,
because the normal turns at once over a cylinder's rim. Where the band crosses a flat stretch, such
as the hull's belt seen from its center, the targets move fast, because the band does reach all of
that stretch at once. Targets still change from one step to the next, so traffic fades in over the
first tenth of each step, or one trip if that is shorter, and out over the last. That is also why
no drone flies before a step starts or after it ends.

The drones' clock is the round's while it loops or runs in the game. `--refit-at` freezes the construction but not the
traffic, so a burst of one step moves; `--rate 0` freezes both. Four consecutive frames of the
dismantle at `--refit-at 0.1 --rate 3`, each mote a little further along:

![burst frame 0](../images/drones-burst-0.jpg) ![burst frame 1](../images/drones-burst-1.jpg)
![burst frame 2](../images/drones-burst-2.jpg) ![burst frame 3](../images/drones-burst-3.jpg)
 The shader guards its distance to
the eye against zero and nothing else. It once clamped at 10⁻⁶, which was a micrometer in the
example and 150 km in the client, whose unit is an AU, and turned every mote into haze.

![0.1: the data core taken apart, loads going home](../images/drones-dismantle.jpg)
![0.3: the deck moving aft, its rim swarmed](../images/drones-move.jpg)
![0.5: the hull growing](../images/drones-grow.jpg)
![0.9: the mirrored pods being built](../images/drones-build.jpg)
![the starting form idle: a thin patrol and nothing else](../images/drones-idle.jpg)
![a GSV-sized form idle, its motes capped](../images/drones-gsv-idle.jpg)
![`--demo refit` at 0.5 with its drones, which it had not drawn from R8 to R14](../images/refit-demo-drones.jpg)

## The field

The envelope from [29-ship-form.md](29-ship-form.md), an ellipsoid, drawn as **two layers**
whatever is decided about air:

- **Inner: clear.** A fresnel rim, faint, with the ship plainly visible through it. If air is ever
  held, this is where the haze goes. `haze.rs` already scatters sunlight through a planet's
  atmosphere, and a park's sky is the same problem in a smaller volume.
- **Outer: the radiator.** Its color is a blackbody at the field's temperature through
  `em_spectra::blackbody`, and its brightness is **physical**: σT⁴ over its area, through the same
  exposure as the stars. A drive's open face works this way ([`plume.rs`](../../crates/lc-client/src/plume.rs)):
  the color is a consequence, not a setting. At 400 K the outer layer is invisible in the visible
  bands. At 2 400 K on a dive it glows red-orange. At 4 600 K it is the brightest thing on screen.

**How it is drawn.** One mesh, three draws: the far wall, the inner rim, then the near wall, which
alone has alpha and so is the only one that can hide anything. By Kirchhoff each mode's emissivity is
its absorptivity, and a thin shell's grows toward one along a grazing path, so a Clear field is
limb-brightened and a Black one glows evenly. The same number is how much of what is behind a wall it
takes out: Clear shows the ship, Black hides it. Clear takes out only half of it
(`CLEAR_VEIL_PERCENT`), its own glow left physical, so the ship reads plainly through a hot Clear field. The shader takes kelvin and fractions and a table of
blackbody colors the host has already put through the observer's bands, so nothing in
`em_render::field_material` knows a `Balance`.

**The mode sets the surface.** Clear is a shimmering, mostly transparent skin: thin-film color bands
that drift across it like a soap bubble's, over the fresnel rim, with the heat glow showing through
as a tint. Black is matte and dark, and the heat glow is all there is to see. A switch sweeps the new
surface across the envelope over `field_switch_s`, from the Mind outward.

| field state | drawn |
|---|---|
| idle | a faint rim, a slow shimmer |
| warm | a colored glow, even over the surface |
| beamed | a **hot spot** on the envelope toward each incoming beam's bearing, from `Outbound::Illuminated`, spreading as the field fills |
| past 80% of `Q_max` | the glow goes uneven and begins to flicker, faster as it nears the limit |
| collapse | below |

![A field at 400, 2 400 and 4 600 K, Clear left and Black right](../images/field-temperatures.png)

At 400 K the glow is nothing in the visible: Clear shows the ship and the world behind it through
the sheen, and Black is a hole in the world. At 2 400 K it is red-orange, Clear's limb the brighter.
At 4 600 K it is the brightest thing in the frame, and uneven.

![Four consecutive frames of a field at its limit](../images/field-flicker.png)

![A beam's hot spot, and a switch from Clear to Black half swept from the Mind](../images/field-beams-and-switch.png)

**Collapse** is a white flash and a sphere of hot debris expanding and cooling through the colors
of the afterglow over `collapse_afterglow_s`, reaching its full size in the first half of it
(`DEBRIS_SPREAD`) and fading over the whole. Nearby fields brighten when the spike lands on them,
each at its own retarded time, so a cascade is seen spreading at c. From a distance, a collapse is
a new point in the sky: the spike's flash, then the afterglow cooling (below).

![A collapse: the flash, then the debris at 40, 180 and 270 s of a 300 s afterglow](../images/field-collapse.png)

### In the game

`lc_client::field` draws every craft with a form so, under the root its hull hangs from, and every
collapse whose light has arrived.

- **One mesh per design.** The envelope is fitted from the form's parts without the grid
  (`form::grid::Envelope`), and a UV sphere scaled to it is the mesh, made again only when a craft's
  stated form changes. The switch sweeps from the Mind.
- **Your field is the account's** (`Fitted`): the bar's temperature and fill, the shade, and a switch
  with its progress. **Anyone else's is `Presence.glow`**, as its light left it: temperature and
  shade, the fill worked back as `(T/T_limit)⁴`, and no switch, which an observer learns of only once
  it is done.
- **The heat is the envelope's, not the hull's.** Once a craft's envelope is drawn its hull carries
  only what it reflects and its windows (`hull::lit`'s `enveloped`), so the heat is drawn once. The
  metering is unchanged: `hull::Sent` sums the thermal term once whoever draws it. A craft too far
  to resolve has no envelope drawn: its point carries the heat instead
  ([The exhaust cone](#the-exhaust-cone), From a distance).
- **The bar is the envelope's color.** It reads the ramp the shader interpolates
  (`field::color_linear`), not the exact blackbody, which some mappings put a few percent off the
  ramp between its entries. A test holds the two together in every mapping.
- **Hot spots** are the beams on your ship from `Outbound::Illuminated`, by beam, each restatement
  replacing the last and zero power removing it, as the power landing over what the field radiates,
  drawn no stronger than twenty times it. Nothing tells an observer about beams on anyone else.
- **A collapse is drawn from its `kind::COLLAPSE` sighting**, at the place and in the shape its craft
  was last seen, and the console names it as its presence did. It starts on the frame the sighting
  is taken, which is when its hull leaves the contacts: the shard sends it once its light has arrived
  by the shard's clock, and this client's may be behind. Its flash lasts half a real second, and its
  debris spreads and cools from its arrival in coordinate time or as the afterglow plays at the
  design rate, whichever is further on, so a clock slowed for watching does not hold it still. A
  wreck's light still in flight shows it facing as it ended (`Craft::end` keeps the nose), not along
  its last order's attitude. **The spike lands on each neighbor** as a hot
  spot toward the wreck when the light of it, off that neighbor, reaches this ship:
  `arrive + (|w − n| + |n − o| − |w − o|) / c`, from what the client knows of where each was.
- **A collapse too far to resolve is a point** where its debris at full spread would be under two
  pixels, and so is one of a craft this ship never saw, placed where the sighting's direction and
  its light's delay put it. The flash is the spike taken as a second long, for half a real second;
  then H10's afterglow, `T_limit (1 − τ/D)^¼` over a fixed area, both closed form in `globals.time`
  from the frame the light arrived, so nothing is written while it plays. The afterglow plays over
  its game duration at the clock's rate, a multiple of the design rate (R11 had floored it against
  the coordinate seconds a real second is, and so played it at the design rate at every speed).
- **The spike cheats.** At `collapse_spike_k`, 10⁷ K, a blackbody puts a millionth of itself in any
  band an eye has, and the most violent thing in the game would be a faint star. So the eye is shown
  its flux flat, the same in every band: its energy is kept, and it is white in every mapping. The
  photometry an instrument reads (`lc_world::afterglow`) is still the blackbody.
- **How it looks a light-year off**, a ship ten times the starting size, 4 × 10²⁹ J: the flash is
  V −10 for its half second, a new star brighter than any planet and about a tenth of the full
  Moon, as white as the brightest in the frame at the automatic exposure. The afterglow starts near V 8, below the automatic
  exposure's reach; opened ten stops, it is an orange point that reddens and fades over its month,
  five seconds at a year a minute. The point is drawn in the lit bodies' style, which caps a
  point's size at a few pixels, so even V −10 is a very bright star rather than a burst: a beauty
  shot of one would want a glare of its own.

![before: Beacon, a light-year off, at the automatic exposure](../images/r18-before-auto.jpg)
![its collapse arrives: the flash, at the automatic exposure](../images/r18-collapse-flash-auto.jpg)
![the flash with the exposure opened ten stops](../images/r18-collapse-flash.jpg)
![the afterglow a second in, orange](../images/r18-collapse-afterglow-early.jpg)
![and three seconds in, reddening](../images/r18-collapse-afterglow-late.jpg)

![The player's field at 400, 2 400 and 4 600 K, Clear left and Black right: Black hides the design](../images/r11-temperatures.jpg)

![A beam's hot spot from beside the camera, and a switch to Black half swept from the Mind](../images/r11-beam-and-switch.jpg)

![`--demo cascade` partway: Aster's debris, Bramble's flash as its light arrives, and three ships whose ends have not](../images/r11-cascade.jpg)

![Diving at 2 400 K: the field and the bar the same orange](../images/r11-dive.jpg)

## Beams and plumes

- **A beam is invisible**, because vacuum scatters nothing. The one exception is an observer inside
  the cone, who sees the emitter as a blinding point in the beam's band. The map draws your own
  beams and the bearings of beams landing on you ([31-directed-energy.md](31-directed-energy.md)).
- A **fore** emission lights the bow's apertures as a drive lights the stern's. A face glows at
  what leaves through it, whatever lit it, so a drive, an emit from either end and a balanced emit
  from both are drawn by one path.

### The exhaust cone

A photon drive's exhaust has no gas in it. Seen from the side, it is invisible; seen from inside, it
is a blinding point. The game used to draw a reaction drive: a glowing column of fuel-rich gas 1.5
hull lengths long, with soot lanes, heated by the jet power `½ F v`, marched 24 samples a pixel. None
of that exists here, and R12 retired it. The cone that matters is thousands of times longer than any
hull: 21 km of courtesy radius behind a starting ship, 21 000 km behind a GSV. It is drawn as two
things:

- **The aperture.** The engine's open face glows as a blackbody at the flux leaving it, `F c` over the
  aperture's area, not `½ F v`. It is white-hot at any real thrust, and it is what a burning ship looks
  like from the side. A short near-field glow off the face, a few aperture widths, keeps the direction
  of thrust readable at a glance.
- **The cone**, drawn as an indicator rather than as light: a long, faint cone along the exhaust, out
  to the courtesy radius, shaded by how much heat lands at each distance. Heat falls as `1/d²` along
  the axis, so the shading is **closed form per pixel**: take the point where the view ray passes
  closest to the axis, and read the flux there. There is no march and no loop, one draw per cone,
  and the fragment works in the proxy's own coordinates from the surface back, so `f32` holds at
  21 000 km.

  "Closest" is measured as an **angle from the apex**, not as a distance from the axis line. From
  beside the two are the same point. From behind the ship, the nearest point to the line can fall
  behind the apex, outside the cone, while the ray still crosses the cone further out, which leaves
  a hole. The angle along a ray has one minimum, and it solves as a linear equation. When the ray
  runs along the axis, the view from inside or from past the end, that solution cancels to noise.
  So the ends of the ray's segment are always tried as well, and the smallest of the three
  candidates wins.

The cone is drawn for your own ship whenever it burns, and for another only when you are inside its
courtesy radius and it glances you: you are inside it, or within `SCATTER_RAD` of its edge, as its
light left ([31](31-directed-energy.md)). Nothing else can be known of it, so selecting a ship
does not draw it. Everyone sees the faces glow. A drive's cone runs down the hull as drawn, an
emit's along the axis it was aimed at, which only those it glances are told. A spread of zero draws
nothing. It uses the hazard color from [18-ui-style.md](18-ui-style.md)'s palette,
and is brightest where it would cook. The map draws the same cone as lines: eight generators, the
rim at the courtesy radius, and a ring at the cooking distance.

The hazard color is 18's red-orange. The material takes both of its colors as uniforms, so it
stays free of either product's palette. The aperture glow is a second material in
`exhaust_cone_material`. A drive's face is `lc_world::emit::aperture_temperature_k`: all of
1.1 × 10²⁰ W through a face 100 m across is 7.0 × 10⁵ K, and through the starting form's bell,
whose open face is 176 m across, 5.3 × 10⁵ K.

**In the game** every open face an emission leaves through glows, whatever lit it
([`lit_faces.rs`](../../crates/lc-client/src/lit_faces.rs)). What leaves each end is
`lc_world::emit::Ends`: the main drive's `F c` aft, and an emit's power from the ends it lit, an
emit flown as a burn from the end it was ordered from and a balanced one from both. Each face takes
its share of its end by engine volume, as the rating divides, and its temperature is
`aperture_temperature_k` of that share through its own area. Your own are
`lc_world::emit::faces_w` now. Another craft's are what its `Presence` states as its light left:
`drive_w`, and `emit_fore_w` and `emit_aft_w` beside it, which `lc_world::emit::emit_w` gives at the
retarded instant. A balanced emit is remembered by its craft once it is out, as its field is, so
light that left while it was lit still shows it lit. No exhaust speed is assumed for anyone; a
photon drive has none but `c`.

The faces are worked out once a frame, before the hulls: the engine's grid is lit on each face at
its color ([Materials by kind](#materials-by-kind)), and the aperture glow
([`plume.rs`](../../crates/lc-client/src/plume.rs)) sits over each lit face under its craft's hull
root. A craft too far to resolve reads the same faces and temperatures for its point rather than
work them out again.

A burn is an emit and an emit a burn, so each thing lit is one cone, `plume::Jet`: the main drive's
at `drive_spread_rad` from `drive_w`, and an emit's at its own spread from what leaves each end, out
of the end it leaves. Its apex is that end's faces' power-weighted middle, or a formless craft's
stern or bow, and it runs to the courtesy radius at its own spread. A balanced emit is two cones,
one from each end. The thrusters spread wider and draw no cone. A diffraction-limited emit's cone
is its axis: nanoradians are under a pixel at any length, as the map's lines say. Another craft is
drawn where its light shows it, from its `Presence` and the `DRIVE` events since, and whether you are
inside its radius is measured to that place.

`emit <fore|aft|both>` at the console ([27-console.md](27-console.md#emit)) lights them for a
photograph, and `refit-magic plate fore:1` makes a ship with engines at both ends.

![coasting in sunlight: the bell's open face is the emitter grid, its flanks the machinery](../images/r13-grid-and-flank.jpg)
![`--demo closing`, your own drive: the face white-hot under its glow, the flanks unlit](../images/r13-drive.jpg)
![an emit flown as a burn from the aft face, your own and another's seen by ship 1](../images/r13-own-aft.jpg)
![](../images/r13-other-aft.jpg)
![a balanced emit from the plate turned two-ended, both ends lit, your own and another's](../images/r13-own-balanced.jpg)
![](../images/r13-other-balanced.jpg)

![your own burn from beside: the bell's face white-hot, the cone running aft](../images/r12-own-beside.jpg)
![from behind, just off the axis](../images/r12-own-behind.jpg)
![the same cone on the map](../images/r12-map.jpg)
![`--demo kzinti`: a Direct approach burning toward you from outside its radius draws no cone](../images/r12-kzinti-outside.jpg)
![and its brake, with you inside its radius and its cone](../images/r12-kzinti-inside.jpg)

Photograph it in a void with `cargo run -p lc-client --example cone_void -- --view
beside|behind|inside --length <m>`. Its flags are listed in the example's module doc.

An observer inside someone's cone gets the blinding point, from the photometry, as for a beam
([31-directed-energy.md](31-directed-energy.md)).

**From a distance** a burn is a point in the sky, not a cone and a face. Its light is what the
`Presence` it left in says: power, facing, place and velocity. With `a` the exhaust axis and `u` the
direction from the emitter to the observer, `cos θ = a · u` says which source the observer sees:

- **Inside the cone**, `θ` within `drive_spread_rad`: the exhaust itself, the top-hat's
  `P / (Ω d²)` that `lc_world::emit::flux_w_m2` gives, in the face's blackbody spectrum through the
  observer's bands. The client does not work it out: a drive is an emission like any beam
  ([31-directed-energy.md](31-directed-energy.md#exhaust-lands-on-whatever-is-behind)), and the one
  fan-out hands every observer inside its cone a `Glare` on the emitter's `Presence`. Its `EMIT`
  sightings carry the same emission, and between two presences the client reads the glare from
  them at the same `flux_w_m2`, so a burn that came and went between them still shows. So a beam, a
  dump and a drive reach the eye by one path. That is the blinding point, and it carries: the starting drive at its rating
  lands 5 × 10⁻¹¹ W/m² a light-year away, a bolometric seventh magnitude, a telescope star in the next
  system.
- **Outside it**, only the face, seen obliquely as the aperture glow draws it up close, falling off
  with the angle.

So a burn in the next system is a moving star that brightens by orders of magnitude as its cone
sweeps over you, Doppler-shifted and aberrated at the craft's velocity as a star is.

**The meeting at the cone's edge, settled.** Outside, the face is what the aperture glow draws near
to: a flat face of radiance `B(T)` over its area, seen at `cos θ`, so `P cos θ / (π d²)`. That is
the resolved picture summed, so stepping back from a burn never changes its brightness. Inside, the
glare is the whole of the point and the face is dropped: the face seen straight down the beam *is*
that light, and adding it would count it twice. The two are never summed, and neither is a second
budget: the account and the heat landing on anything are the cone's alone. So the edge is a step,
by `π / (Ω cos α)`, about 130 for the drive's 5° (seven stops), and that step is the flare. Faces at
several temperatures are drawn as one blackbody carrying their sum; seen from the front, `cos θ`
is negative and a face sends nothing, so a beam out of the bow seen from astern is not drawn at all.

**As built** ([`distant.rs`](../../crates/lc-client/src/distant.rs)):

- **Where a hull stops and a point starts** is the bodies' crossover: a craft whose half-length is
  under two pixels is a point and nothing else. Its hull root is hidden, and with it the envelope
  and the aperture glows, and it is metered as a point. So its heat is drawn once, by the envelope
  or by the point, never both. The craft the camera is behind is never a point.
- **Every unresolved craft gets one**, lit or not: `hull::Sent`'s reflected light and windows over
  the hull's disc, its heat over a quarter of its envelope, and one term for whatever it has lit.
  All four are seen at the source's own Doppler factor. The shader adds the observer's, and the
  aberration, as it does for a star.
- **One mesh for all of them**, a fourth pass of the starfield sharing its uniforms. It is written
  only when a point moves or changes by a part in a thousand, not every frame; its uniforms carry no
  clock, so the material is written only when the exposure or the view is. It is baked about an
  origin of its own, moved to the eye once the eye has come a ten-thousandth of the nearest point's
  distance from it: the sky's moves only every light-year, and in `f32` a point a few hundred
  kilometers off would land anywhere in the frame.
- **A flash is not metered**, as a wreck's debris's is not: it overflows rather than stopping the
  sky down for half a second.
- **Burns are read from their sightings too.** A `kind::DRIVE` or `kind::EMIT` sighting starts a
  flare that is shown until its going out arrives, and for half a real second at least. So a burn
  that lit and went out between two presences is still seen. Inside a beam, the glare is the
  presence's or the sightings', whichever is brighter.

**How bright, in V**, from `--demo distant`:

| | V |
|---|---|
| the starting ship's drive, inside its cone, a light-year off | about 20 |
| a ship ten times its size at 2 g, inside its cone, a light-year off | 13 |
| the same, 10° outside it | 18 |
| a Clear ship at rest, sunlit, 10⁵ km off | 5 |

A drive's face at 7 × 10⁵ K puts about a millionth of its light in V, so a burn a light-year off is
a telescope star, as 31 says, and not one the eye's exposure shows. `--exposure 10` opens the
exposure ten stops, a long one, and the in-cone burn appears as a blue point inside its marker.

![a burn a light-year off, 10° outside its cone: its face obliquely, too faint to show](../images/r18-burn-outside.jpg)
![the same ship turned away from you, its cone on you, a hundred times brighter: a blue point in Lantern's ring, `--exposure 10`](../images/r18-burn-inside.jpg)
![a Clear ship at rest a hundred thousand kilometers off, sunlit: Tern, at the automatic exposure](../images/r18-unlit-auto.jpg)

Photograph it with `--demo distant --first-person --exposure 10` and the catalog. The scene runs a
year a minute, so the light of what it stages arrives about a minute in, and `--burst` catches it.
- God view may draw every beam's cone, as a debug overlay, like the causality lines of
  [07-rendering.md](07-rendering.md).

## Temporary assets

Before the mesher, the construction pass or the field shader exist, everything above has a
placeholder, so the rest can be built and a refit visibly changes the ship straight away. Since R10
the hull's placeholders are drawn only until a craft's first mesh lands, and a refit's only until
its first step's meshes do:

- **Each part as a Bevy primitive mesh**, scaled to its solved size: `Sphere` scaled for an
  ellipsoid, `Capsule3d`, `Cuboid` for a slab and for the Mind, `Cylinder`, `Torus`,
  `ConicalFrustum`. No blends, and a slab's corners square.
- **A flat color per kind.**
- **Construction as scale plus wireframe:** the growing part drawn in `BodyWireframeMaterial`
  during the truss phase, crossfading to solid.
- **Drones as instanced motes** on straight lines.
- **The field as a fresnel sphere** around the bounds, tinted by temperature once there is one.
- **Spars uncut**: the plain primitive. Retired in the game with the placeholders; the mesher cuts
  the saddles and straps.

Until the grid gives a form its extent, the orbit camera frames the smallest sphere about the Mind
holding the corners of the form's bounds, so its stops and standoff follow the form's size as the
ovoid's follow its length.

![the starting form as placeholder parts](../images/parts-default.jpg)
![the cluster preset, from ahead](../images/parts-cluster.jpg)
![the cluster preset, from the side](../images/parts-cluster-side.jpg)
![the plate preset](../images/parts-plate.jpg)
![the spindle preset](../images/parts-spindle.jpg)

Every placeholder is replaced independently. None of them is on the server's side of anything.

## Photographing it

WGSL cannot be asserted from a test ([AGENTS.md](../../AGENTS.md)), so each piece gets a way to be
photographed:

| flag | shows |
|---|---|
| `--form <plate\|spindle\|cluster\|default>` | the ship in a preset form |
| `--view form` | the editor ([29-ship-form.md](29-ship-form.md)) |
| `--demo refit` | a staged refit, with `--refit-at <fraction>` to freeze it at a point, or `--refit-from <fraction>` to run it from there. With `--form default*k` every part is `k` times larger |
| `--demo-cam-at <x:y:z:m>` | orbit a point of the ship's frame from `m` meters, past the boom's stops. Aimed with `--demo-cam`; how the truss's pitch is photographed on a GSV |
| `--demo collapse` | a ship collapsing beside two others, one close enough to follow it |
| `--field-k <kelvin>` | the player's field held at a temperature, as drawn, metered and on the bar. `--field-mode clear\|black` holds its shade, `--field-switch <progress>` a switch into it, and `--field-beam <watts>` a beam from beside the camera |

Until the player has a field, the shader is photographed in a void: `cargo run -p lc-client
--example field_void -- --field-k <kelvin> --mode clear|black`, around a stand-in hull, with its
own `--burst`, `--spot`, `--switch` and `--collapse`. Its flags are in the example's module doc.

`--burst` is the only way to see the saturation flicker, as it is for any flicker.

## Where it goes

| crate | new | changed |
|---|---|---|
| `em-render` | `hull_material` (triplanar, kind regions, reveal mask, living lights), `field_material`, `drone_material`, `exhaust_cone_material` (the cone, and the aperture glow beside it) | `plume_material` retired in R12 |
| `lc-client` | `hull_mesh.rs` (finishes, painting, caching) over `surface_nets.rs` (any field), `ship_hull.rs` (every craft's steady hull), `truss.rs`, `refit_hull.rs` (the construction overlay), `construction.rs` (the function of recipe and `t`), `drones.rs`, `field.rs` | `hull.rs` draws only a craft with no form. `plume.rs` draws the aperture glow at `F c` and the cone, and `map_cone.rs` the cone's lines |
| `lc-client/assets` | texture-graph graphs per kind. `field.wgsl`, `hull.wgsl`, `drones.wgsl`, `exhaust_cone.wgsl`, `aperture_glow.wgsl` | |

Materials go in `em-render` because nothing in them is specific to Lightcone. A hull with regions
and a reveal mask is as much Exotic Matters' as anyone's.

## Order of work, across 29 to 32

Each step leaves the game playable and adds one thing a player can see or feel.

1. **The form replaces the loadout.** `lc_world::form`: parts, the Mind, densities, the starting
   form. The planner plans rounds. `Form` on the wire and in saves. On the client, the placeholder
   meshes, construction as scale and wireframe, and instanced drones. Until the editor exists, the
   refit window is a list of parts with snapped size fields, and adds parts at default placements.
   A refit visibly changes the ship.
2. **The form has geometry, and an editor.** The grid, the shadow table, the envelope and moments.
   Solar reads the shadow. Hull mass by area. The editor, as a third view, with the budget and
   snapping.
3. **The field.** The heat account, collapse and proximity. `HULL_K` retires. Photometry of fields.
   The field shader, and the HUD's countdown.
4. **Directed energy.** Exhaust from heat, `Order::Emit`, beams fanned out and delivered, reciprocity,
   the gain moved onto stars, radio charged. The emit window.
5. **The real hull.** Surface nets, the truss and plating passes, materials by kind, scale-true
   detail. The placeholders go.
6. **Later:** bays and what is built in them, air under the field, parks, Dyson swarms.

Step 3 comes before 4 because heat has to exist before anything can dump it, and proximity heating
from a collapse needs no beams.

## Open

- **Refraction** of the stars behind the field. It needs the sky behind the ship, which is drawn by a
  different camera. It is a nice-to-have, and the rim carries the look without it.
- **Meshing a GSV.** A 50 km hull at fine detail is far too many triangles at once. It will want
  levels of detail per part, and the frontier meshed finer than the rest.
- **Looking into a bay.** The interior grid is procedural, but a ship being built inside a bay is the
  construction pass applied to a second form inside the first. That is the screenshot, and it waits
  for bays.
