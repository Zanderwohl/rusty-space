# Ship rendering

How a form becomes a picture: the hull, a refit being built, the drones doing it, and the field
around all of it.

**Status: designed, not built.** [29-ship-form.md](29-ship-form.md) is what is drawn,
[30-the-field.md](30-the-field.md) is the field's physics, and [31-directed-energy.md](31-directed-energy.md)
is what beams do.

## Two looks

- **From TNG: parts you can read.** Each part is one kind, and each kind looks like what it is.
  A drive section, a habitat and a bay are recognizable from outside.
- **From the Culture: a smooth field with the ship inside it.** Not the surface clutter of a Star
  Destroyer. Light does most of the work: lit living areas on the night side, the field's glow,
  the plume.

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

### Details are sized in meters

The eye judges size by how small repeated detail is. So **every repeated detail has a fixed size
in meters, whatever the hull's size**: windows every few meters, panels tens of meters across,
girders at a fixed pitch. A 50 km ship reads as enormous because its girders are fine against its
length.

At a distance where detail would be smaller than a pixel, it fades into a texture that averages
it, to stop the shimmer. A distance-field mesh has no UV coordinates, so materials are mapped
**triplanar**, from world position in the ship's frame.

### Materials by kind

One texture-graph graph per kind, delivered like the planet graphs, with the region set by which
part a point is nearest. Fillets blend between the two regions.

| kind | look |
|---|---|
| storage | dark and smooth, faint seams. The mass of the ship |
| drone | hangar doors in rows, docks lit when drones are home |
| living | window bands. Lit on the night side, where they are the brightest thing on the hull |
| engine | an emitter grid on the open face, glowing with exhaust power |
| data | fine dense panels |
| mind | a small dark cube with one faint light. Drawn only when nothing encloses it, and always in the editor |
| spar | plated structure, with a row of bolt heads along every line where it meets a neighbor. The line is where the spar's distance and the neighbor's grown distance are both near zero, so the shader finds it with no geometry of its own |
| bay | a shell with a mouth, and a lit interior grid of decks and gantries |

Living lights are emitters with a real (small) power, through the same exposure as everything
else, so they show on a night side and vanish in sunlight as they should.

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

- **Dismantle** runs a build's phases in reverse order but is not a rewind: scaffold goes up, the
  plating comes off, which shows the truss, and then the truss is taken down.
- **Move** slides the subtree from its old anchor to its new one along a smooth path over the
  step. No construction, and drones swarm the joint.
- **Rebuild** is a dismantle of the whole part to nothing in the round's first phase, and a build of
  the new one in its last.
- **Cancel** runs the step in progress backward from the fraction it had reached, which is what the
  server does to its energy.

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
docked. Haze is a mote's light spread over a disc about the spacing between drones. Each drone keeps
the light it had when it became haze, since true point sources that far off would add up to nothing,
until the haze disc is narrower than a few pixels too. From there the haze dims as the hull does.
`crates/lc-client/examples/drones_void.rs` photographs it.

## The field

The envelope from [29-ship-form.md](29-ship-form.md), meshed coarsely, drawn as **two layers**
whatever is decided about air:

- **Inner: clear.** A fresnel rim, faint, with the ship plainly visible through it. If air is ever
  held, this is where the haze goes. `haze.rs` already scatters sunlight through a planet's
  atmosphere, and a park's sky is the same problem in a smaller volume.
- **Outer: the radiator.** Its color is a blackbody at the field's temperature through
  `em_spectra::blackbody`, and its brightness is **physical**: σT⁴ over its area, through the same
  exposure as the stars. The hull's plume already works this way ([`plume.rs`](../../crates/lc-client/src/plume.rs)):
  the color is a consequence, not a setting. At 400 K the outer layer is invisible in the visible
  bands. At 2 400 K on a dive it glows red-orange. At 4 600 K it is the brightest thing on screen.

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

**Collapse** is a white flash and a sphere of hot debris expanding and cooling through the colors
of the afterglow over `collapse_afterglow_s`. Nearby fields brighten when the spike lands on them,
each at its own retarded time, so a cascade is seen spreading at c. From a distance, a collapse is
drawn by the photometry: a new point in the sky, as bright as [30-the-field.md](30-the-field.md)
says.

## Beams and plumes

- **A beam is invisible**, because vacuum scatters nothing. The one exception is an observer inside
  the cone, who sees the emitter as a blinding point in the beam's band. The map draws your own
  beams and the bearings of beams landing on you ([31-directed-energy.md](31-directed-energy.md)).
- A **fore** emission lights the bow's apertures as a drive lights the stern's.

### The exhaust cone

A photon drive's exhaust has no gas in it. Seen from the side, it is invisible; seen from inside, it
is a blinding point. `plume.wgsl` draws a reaction drive: a glowing column of fuel-rich gas 1.5 hull
lengths long, with soot lanes, heated by the jet power `½ F v`. None of that exists here. And the
cone that matters is thousands of times longer than any hull: 27 km of courtesy radius behind a
starting ship, 27 000 km behind a GSV.

The current shader is not the cost problem it might look like. It is one draw per burning ship: a
proxy cone, with each covered pixel marching 24 samples back along its ray. Cost goes with the pixels
covered and not with the length. What does not survive a longer cone is its content, so it is
replaced by two things:

- **The aperture.** The engine's open face glows as a blackbody at the flux leaving it, `F c` over the
  aperture's area, not `½ F v`. It is white-hot at any real thrust, and it is what a burning ship looks
  like from the side. A short near-field glow off the face, a few aperture widths, keeps the direction
  of thrust readable at a glance.
- **The cone**, drawn as an indicator rather than as light: a long, faint cone along the exhaust, out
  to the courtesy radius, shaded by how much heat lands at each distance. Heat falls as `1/d²` along
  the axis, so the shading is **closed form per pixel**: take the point where the view ray passes
  closest to the axis, and read the flux there. There is no march and no loop, one draw per cone,
  and the fragment works in the proxy's own coordinates from the surface back, as `plume.wgsl` does,
  so `f32` holds at 27 000 km.

The cone is drawn for your own ship whenever it burns, for any ship whose courtesy radius you are
inside, and for a selected ship. It uses the hazard color from [18-ui-style.md](18-ui-style.md)'s
palette, and is brightest where it would cook. The map draws the same cone as lines.

An observer inside someone's cone gets the blinding point, from the photometry, as for a beam
([31-directed-energy.md](31-directed-energy.md)).
- God view may draw every beam's cone, as a debug overlay, like the causality lines of
  [07-rendering.md](07-rendering.md).

## Temporary assets

Before the mesher, the construction pass or the field shader exist, everything above has a
placeholder, so the rest can be built and a refit visibly changes the ship straight away:

- **Each part as a Bevy primitive mesh**, scaled to its solved size: `Sphere` scaled for an
  ellipsoid, `Capsule3d`, `Cuboid` for a slab and for the Mind, `Cylinder`, `Torus`,
  `ConicalFrustum`. No blends.
- **A flat color per kind.**
- **Construction as scale plus wireframe:** the growing part drawn in `BodyWireframeMaterial`
  during the truss phase, crossfading to solid.
- **Drones as instanced motes** on straight lines.
- **The field as a fresnel sphere** around the bounds, tinted by temperature once there is one.
- **Spars uncut**: the plain primitive. The saddles and straps arrive with the distance-field
  mesher.

Every placeholder is replaced independently. None of them is on the server's side of anything.

## Photographing it

WGSL cannot be asserted from a test ([AGENTS.md](../../AGENTS.md)), so each piece gets a way to be
photographed:

| flag | shows |
|---|---|
| `--form <plate\|spindle\|cluster\|default>` | the ship in a preset form |
| `--view form` | the editor ([29-ship-form.md](29-ship-form.md)) |
| `--demo refit` | a staged refit, with `--refit-at <fraction>` to freeze it at a point |
| `--demo collapse` | a ship collapsing beside two others, one close enough to follow it |
| `--field-k <kelvin>` | the player's field held at a temperature, for the shader |

`--burst` is the only way to see the saturation flicker, as it is for any flicker.

## Where it goes

| crate | new | changed |
|---|---|---|
| `em-render` | `hull_material` (triplanar, kind regions, reveal mask, living lights), `field_material`, `drone_material`, `exhaust_cone_material` | `plume_material` becomes the aperture glow |
| `lc-client` | `hull_mesh.rs` (surface nets, finishes, caching), `construction.rs` (the function of recipe and `t`), `drones.rs`, `field.rs` | `hull.rs` draws the form instead of the ovoid. `plume.rs` draws the aperture glow at `F c` and the cone |
| `lc-client/assets` | texture-graph graphs per kind. `field.wgsl`, `hull.wgsl`, `drones.wgsl` | |

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
