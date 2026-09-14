# Rendering

Bevy 0.17, wgpu, WGSL. One shader set for native and browser.

## View modes

| mode | what it draws | who gets it |
|---|---|---|
| observer view | every object at its **retarded** state, as seen from the player's ship | players, default and usually only |
| god view | every object at coordinate time `t`, no delay | development, replays, spectators |

Observer view is the game. Each drawable is evaluated at its own retarded time, solved
against the camera's worldline, so a distant ship appears where it was when its light left
and a distant star shows the brightness it emitted years ago. Two objects in the same frame
are drawn at different source times, and that is correct.

God view removes the delay. It is necessary for debugging, for authoring, and for watching a
replay, and it destroys the game's premise if a player has it.

**Decided: never exposed to players, and compiled out of the WASM build.** Two gates, because
they do different jobs:

| gate | stops |
|---|---|
| `#[cfg(feature = "godview")]`, off for `wasm32` | the code shipping to browsers at all — smaller binary, and nothing to find |
| server capability check | a modified desktop binary asking for the data |

The compile gate is about binary size and about not handing anyone the client half of the
mechanism. The server gate is the one that actually enforces it, because a desktop binary can
be patched and the server is the only thing that decides what data leaves it. Treat a client
that requests god-view data without the capability as a client to disconnect.

**God view draws causality explicitly.** For every event still in flight, draw a line from the
event's coordinate to each observer it is currently travelling toward, with the fraction
travelled shown along it. The single most common class of bug in this design is an observer
learning something early or late, and it is invisible in any view that only shows positions.
Rendered this way it is obvious: a line that reaches an observer before the client reacted, or
a reception with no line feeding it, is the bug drawn on screen.

Supporting overlays in the same mode:

| overlay | shows |
|---|---|
| in-flight event lines | which observers each live event is heading for, and how far along |
| expanding shells | the light cone of a selected event as a sphere of radius `c (t - t_emit)` |
| reception log, spatially anchored | what each observer received and when, at their position |
| knowledge diff | what observer A knows that observer B does not, at the same `t` |

These are debug instruments, so they may be expensive. Correctness of the delay model is worth
more than the frame rate of the mode nobody ships.

Both modes share one scene graph. The difference is the time at which each worldline is
sampled, which is a parameter of the extract step, not a separate renderer.

## Retarded-time sampling

**Decided: per object nearby, per system at distance.** Sampling every drawable's retarded time
individually is correct and unnecessary; sampling one per spatial cell is cheap and good enough
almost everywhere. The cell size scales with distance:

| region | granularity | error |
|---|---|---|
| the observer's own system | per object | none; this is where real-time play happens and where light delay is seconds to hours |
| any other system | one retarded time for the whole system | bounded by the system's light-crossing time |
| interstellar objects | per object | there are few of them and they are what the player is watching |

The bound on the second row is what makes it safe. A planetary system out to 100 AU is
0.6 light-days across, so one retarded time for the whole system misplaces a body by at most
`v * dt`: a planet at 30 km/s over half a day is 1.3e9 m, which at 10 light-years subtends
1.4e-7 radians. Sub-pixel by four orders of magnitude, for a system that is itself a few pixels
wide.

The rule is not "per cell" uniformly, because the observer's own system is the one place the
error would be visible and is also the cheapest place to be exact.

## Scale tiers

Positions are `f64` in the simulation and `f32` on the GPU. WGSL has no `f64` at all, so all
reduction happens on the CPU before upload. `f32` has a 24-bit mantissa: at 1e11 m the ulp is
8 km, so absolute world coordinates are unusable and everything is camera-relative.

`src/presentation/render_space.rs` already implements this as
`ToRender::to_render_relative(scale, camera_render_pos)`, subtracting in `f64` and narrowing
after. It moves to `em-render` unchanged. Do not open-code the swizzle or the subtraction
anywhere else — the Z-up to Y-up conversion is a proper rotation, not a mirror, and getting
it wrong silently flips handedness.

Three tiers, each with its own scale factor and camera:

| tier | range | unit on the GPU | contents |
|---|---|---|---|
| surface | < 1e4 m | metres | a ship, a station, a structure's geometry |
| system | 1e4 to 1e14 m | scaled metres | planets, orbits, trajectories, in-system traffic |
| interstellar | > 1e14 m | light-years | stars as points, systems as markers, light cones |

Render tiers back to front into the same target with separate depth ranges, or composite
separate passes. Reverse-Z with an infinite far plane, per tier, keeps depth precision
usable; the standard forward-Z projection wastes almost all of its precision near the near
plane, which at these scale ratios means z-fighting on everything past a planet.

## Star rendering

![The sky at rest](../images/starfield.png)

Stars are points with a physically-derived colour and brightness, not billboards with a
fixed sprite. The existing path is `src/presentation/local_starfield*.rs` with
`src/catalog/spectral_color.rs`, driven by the HYG catalogue.

`em-render::relativistic_starfield_material` is the descendant that the game uses, with
`crates/lc-client/assets/shaders/starfield.wgsl`. It keeps the technique — one mesh, four
vertices per star, the quad expanded in the vertex stage, `w = 0` on the direction transform so
the sky is translation-invariant, `clip.z = clip.w` for background depth — and changes what is
baked.

| | app starfield | game starfield |
|---|---|---|
| baked per star | linear RGB, brightness, size | temperature, radius, position |
| derived per frame | nothing | direction, aberration, Doppler, bands, exposure, size |
| brightness from | catalogue apparent magnitude | `L / d^2` through the band mapping |
| positions | unit directions, fixed | light-years from a bake origin that follows the ship |

### The corona

![A local star](../images/corona.png)

A local star's glare carries radial filaments. This is not physical and is deliberate: a real
corona is a millionth of the photosphere and invisible without occulting it. This ship occults
it. It has no eyes, only a pipeline, and the band matrix is already handed to the player on
exactly those grounds — a corona it chooses to render is the same kind of decision.

The structure is ridged fractal noise, and the one idea that makes it work is **sampling on the
normalised offset in the plane of the sky**. Normalising discards the distance out and leaves
only the angle around the star, so the field is constant along every ray and every feature
comes out radial without being asked for. The sample direction leans along the line of sight as
it goes out, so threads evolve rather than being straight spokes.

It is a function of a per-star seed and a world-space direction and of nothing else. So it does
not swim when the camera turns, it is identical for every client, and flying around a star shows
its other side.

Length and brightness come from two fields, not one, on the same angular scale and different
seeds. Driving both from one field made every long streamer also the brightest, which the eye
picks up at once. Sampling the length at half the frequency was worse again: it replaced the
streamers with half a dozen broad lobes, because the thing being varied was no longer a
streamer.

`corona_frequency` is streamers per radian and is the lever that matters — halving it halves
their number and doubles their width. Three octaves of squared ridges, not four of cubed: the
fine octaves read as fur, and every extra power narrows the crease.

Two things that looked like tuning and are not:

- **The halo is a power law out from the source, not a fade in from the edge of the quad.**
  `pow(1 - r, n)` is a property of where the quad happens to end, and it renders as a ball.
- **The outer fade must finish inside the quad.** Letting the threads push the boundary past
  `r = 1` means the `discard` at the edge cuts it, and the ragged silhouette becomes a hard
  circle — the exact artifact it was added to remove, only sharper.

### Lit bodies reuse the star shader

![The solar system from Earth](../images/planets.png)

A planet is drawn by the starfield shader, as a blackbody at **its star's** temperature with a
smaller radius. Reflected light has the star's spectrum, so the only thing that differs is how
much of it arrives, and that is a radius:

```
R_eff = R_star * R_body * sqrt(p * phase) / d
```

from equating `pi R_eff^2 B / D^2` with the standard `L p R^2 / (4 pi d^2 D^2)`. Exact for a grey
reflector, and checked against a measured magnitude rather than asserted: Jupiter at opposition
comes out at -2.72 against an observed -2.70.

The phase term is not optional. Venus at a tenth of an astronomical unit is a razor crescent, and
a full-phase calculation puts it three magnitudes too bright.

A lit body is drawn at very nearly the background's size, and that is the point. A planet looks
like a star — it is why they were called wandering ones — and what distinguishes it is that it
moves, not that it is bigger. Drawn larger, the quad stops reading as a point source and starts
reading as a disc with a square behind it.

The body's own thermal emission goes in the same slot a swarm's does, because it is the same
physics: it absorbs starlight and re-radiates at the temperature its orbit sets. So a planet is
warm in the thermal preset without any separate machinery.

What this does not carry is a coloured albedo. Mars comes out the Sun's colour rather than its
own, because the model says reflected light has the star's spectrum. A per-body albedo colour is
the next thing this wants, and the preset carries a visualisation colour rather than a measured
albedo, so it wants a real table too.

### Where the bodies come from

Nothing is modelled here. `em-sim` holds and propagates systems, `em_sim::presets` already
carries the solar system — 230 bodies, moons and comets, fitted against JPL — and
`lc_world::sky::generate` already emits a generated system in the form `em-sim` consumes. The
client joins them: the real data where there is real data, a generated system otherwise.

The end-to-end check is that from where the Earth is, the brightest things in the sky are Luna,
Venus, Jupiter, Mars and Saturn. Real elements, real propagation, and photometry that agrees
with what a person standing outside would see.

### Two passes, two laws

The sky is drawn three times, and the passes obey different rules. This is the arrangement Exotic Matters
arrived at with `starfield` and `local_starfield`, and it is right for the same reason here.

| | background | local star | lit bodies |
|---|---|---|---|
| what it is | a dome at infinity | an object at a distance | objects in orbit |
| size | from brightness, 1 to 3 pixels | the angle it subtends, floored at 4 pixels | from reflected flux, 1 to 3.6 pixels |
| glare | slight | allowed to fill the screen | slight |
| corona | none | yes | none |
| rebuilt | when the ship crosses a shell | likewise | every frame, because they move |

A star joins the local pass when the ship is inside its Oort shell, 1.6 light-years, which
[03-world-model.md](03-world-model.md) already makes the partition boundary: being inside it is
the same statement as being in the system.

One law cannot serve both. The drawn size that makes arriving at a star look like arriving is
the size that makes a field of six thousand unreadable, and the first attempt here collapsed
them into one pass and got a sky of dust, then a sky of balloons.

Within a pass the source and its glare are also separate: the quad covers the glare, and the
core is a fraction of it. A single filled disc made a star a hundred and sixty pixels across
into a flat white ball with its actual disc swamped inside.

The reason for the split is that the observer moves. Baking a colour is right when the only
input is a catalogue magnitude and wrong when aberration, Doppler shift, the band matrix and the
exposure all change while the ship flies: re-uploading four `vec4`s per star per frame does not
scale to the target count, and computing them from a temperature costs nothing.

The blackbody is a table — `log2` of band radiance against `log2` of temperature, 2048 samples
by 7 bands, generated at startup by `em_spectra::blackbody`. That is what "the starfield and the
science instrument must not be two implementations" reduces to in practice: one Planck integral,
tabulated for the shader. The table spans 16 K to 4 million K because the lookup happens at the
*shifted* temperature, and at the drive's 0.999c cap the shift factor is 44.7 in both
directions.

Positions are baked relative to an origin that follows the ship, with the ship's offset from
that origin as a uniform, so the shader differences two small numbers instead of two
interstellar ones. The mesh is rebuilt once per light-year of travel.

For the game, the brightness fed to that shader is not the catalogue's apparent magnitude.
It is `L(n, t_r) / d^2` from [04-stellar-photometry.md](04-stellar-photometry.md), evaluated
for the camera's direction and retarded time. A star that a rival's swarm is occluding is
dimmer in the sky, and it is dimmer because of the same function the telescope samples. The
starfield and the science instrument must not be two implementations.

Note the existing catalogue rotation: HYG is equatorial, the sim is ecliptic. That
conversion already exists and is the sort of thing that silently puts everything 23.4 degrees
out of place if duplicated.

## The boundary and the skybox

The playable volume terminates at a boundary well inside the `2^60` coordinate invariant. What
lies beyond it is a skybox of distant galaxies: rendered, never simulated, never a source, never
in the BVH. It exists so the sky is not empty at the edge and so the boundary reads as distance
rather than as a wall.

The skybox is the one thing in the renderer exempt from retarded-time evaluation, because it has
no worldline and no state. Keep it in its own pass so that exemption is structural and cannot
leak into anything that does have a worldline.

## Light cones as geometry

Drawing a light cone is drawing an expanding sphere: radius `c * (t - t_emit)` about the
emission point. In observer view the visible intersection of a cone with the camera is a
single point, so cones are drawn only as an overlay, in the god view, or as a deliberate
UI affordance showing where a transmission has reached.

Implementation is an instanced unit sphere with a per-instance radius and a thin shell
shader. Cost is negligible and the spatial reasoning it supports — "has my message reached
them yet" — is central enough to justify a dedicated overlay.

## Shader portability

Target WebGPU as the baseline. WebGL2 as a fallback costs enough to be a separate decision:

| feature | WebGPU | WebGL2 |
|---|---|---|
| compute shaders | yes | no |
| storage buffers | yes | no; uniforms only, 64 KB typical limit |
| instancing | yes | yes |
| `f16` textures | yes | with extension |
| multiple render targets | yes | limited |

The emission-shell bake and any large per-instance data want storage buffers. Under WebGL2
they become textures, which is a real second code path.

**Decided: WebGPU only.** Detect WebGL2 and refuse it with a clear message pointing at the two
supported routes — a browser with WebGPU, or the desktop binary. Browser WebGPU support is
still arriving unevenly, and the desktop binary is the answer for anyone it has not reached
yet. Carrying a second renderer to cover the gap costs more than the gap does.

Rules for every shader written:

- No `f64`. WGSL does not have it.
- No platform-specific extensions without a WGSL-level fallback.
- Precision-critical arithmetic on the CPU, results uploaded as small `f32` deltas.
- Bevy's material abstraction over hand-written pipelines, so the native and WASM backends
  stay one code path.

## WASM specifics

| constraint | consequence |
|---|---|
| no threads without cross-origin isolation | ship `COOP`/`COEP` headers, or run Bevy single-threaded |
| no blocking IO | all asset loading async; no `std::fs` anywhere in the client's dependency tree |
| download size | `--release` with `opt-level = "s"`, `wasm-opt -Oz`, assets streamed not bundled |
| canvas sizing and input focus | Bevy handles it, but test the embedded case explicitly |

Single-threaded Bevy is the safe default. Cross-origin isolation breaks third-party embeds
and is worth taking only after profiling shows the render or extract stages are the
bottleneck.

## Rendering populations

![A belt from outside](../images/belt.png)

**Decided: a shell of proxy geometry whose opacity at each latitude is the population's own sky
density.** One shape covers every case: a belt's inclinations are narrow, so the density is a
band near the plane and it reads as a ring; an isotropic swarm's is flat and it reads as a
sphere. Nothing special-cases either, and the mesh is built from the inclination distribution
alone, so it is static — what changes as the ship moves is the transform.

### Resolved bodies

![Saturn](../images/planet.png)

Past a couple of pixels of angular radius a body stops being a point and becomes a sphere. Below
that it is drawn at its *effective* radius, which is a photometric fiction — the radius a
blackbody would need to deliver the same flux — and that is right for anything unresolved and
wrong the moment a body has a shape. It is why Saturn's rings had nothing in the middle.

A resolved body is the only thing in this renderer that writes depth, which is what puts the far
half of a ring behind its planet and the near half in front.

**No textures and no authored appearance.** What a body looks like follows from its radius, mass
and equilibrium temperature, which every body in every system already carries, and everything
past that is a seed. `lc_world::surface` sorts the solar system the way a person would, and two
of its thresholds are set between specific pairs rather than chosen:

| | | |
|---|---|---|
| Europa 3013 kg/m³ against Io 3528 | bulk density | ice shell against no water at all |
| Saturn 5.7e26 kg against Neptune 1.0e26 | mass | an ice giant is one because it never got the hydrogen |

Sorting the giants by *temperature* was the first attempt and put Saturn, at 90 K, in with
Uranus. True about its temperature and wrong about everything a person would recognise.

Two families of surface cover it: latitude bands for anything gaseous, mottling for everything
solid. The band warp has to stay well under the band spacing — at a quarter of a period it stops
perturbing the bands and starts destroying them, and the planet reads as blobs.

The classification also supplies a per-body albedo, which replaces the flat 0.3 the photometry
had been using. Ice reflects six times what bare rock does.

The exposure meters bodies along with the stars — see [Metering](#metering) — so a planet
large enough to be the picture is what the window is placed on, and the star field behind it
drops away as it does in any photograph of a planet.

### Rings

![Saturn](../images/rings.png)

**Opacity composites, it does not add.** A sightline crossing the shell at a grazing angle
passes through more of it, and the shader carries that as a limb term — but as `1 - (1 - t)^n`,
not `t * n`. A sightline running *along* a sheet has unbounded path length, and under a linear
law it paints unbounded light: seen edge-on from close up, Saturn's rings came out at twice
full white. It is the same exponential the photometry already applies to the star's light,
where past a covering fraction of a few tenths the elements begin shadowing one another, and it
leaves the face-on case untouched because at `n` of one it is `t`.

A ring is not a shell. It has radial structure and no latitude, so it gets a flat annulus with
each vertex's opacity from the optical depth *at that radius*. Saturn's rings span a factor of
1.8 in radius with a division in the middle that is the most recognisable thing about them;
drawn as a shell at one radius they would be a circle.

Both shapes go through one material, and the difference is a vertex normal: a shell's is its own
direction, a ring's is the pole. Passing it rather than deriving it is the whole of the change,
and it gets the limb term right for both — an edge-on ring has a normal across the view and
lights up, which is correct.

The data is in `lc_world::rings`: band radii from IAU and Cassini results, optical depths
area-averaged per band. Poles are *not* there — `em-sim`'s presets already carry every body's
IAU rotation, and a second copy of a pole is a second chance to have it wrong.

Rings take no display gain, unlike a population. The gain exists because even a Kuiper belt is a
trace; Saturn's rings cover a third of their own annulus and need no help. Applied to them it
rendered Jupiter's — three parts per million, and it took Voyager to find them — at a fifth
opacity.

Rings also reflect, which is why `effective_radius` takes an *area* rather than a radius. Saturn
plus rings is one source with one effective radius, and the ring contribution carries two
cosines: how much of it is turned toward the star, and how much toward the observer. Edge-on it
contributes nothing, which is the real 1.1 magnitude swing over Saturn's ring cycle.

### Two numbers that had to be found by looking

**The opacity mapping is a fourth root.** Covering fraction spans fourteen decades: an asteroid
belt covers 2.6e-12 of its star's sky, a Kuiper analogue 3e-8, a half-built swarm 0.4. Linearly,
everything natural is exactly zero and only a technosignature shows — true photometrically and
useless as a picture, for the same reason a linear tone map of sixty stops renders a black sky.

A logarithm was the first attempt and overcorrected badly: it put a Kuiper belt at 0.46, and
since the ship is *inside* that shell the result was a grey wash over the whole sky. A fourth
root gives 0.001, 0.013 and 0.80 — a trace, a haze and a structure, which is the right reading
of all three.

**A shell the ship is inside is dimmed to about a fifth.** From outside a belt is a ring and the
eye reads it as structure; from inside it covers the entire sky, and the opacity that made the
ring legible buries the star field. The same number cannot serve both. Dimmed, the inside case
is what it should be: a faint band along the plane, the way the zodiacal light is.

An envelope is a visualisation either way — an orbit line, not a photograph. What it carries
honestly is the ordering.

### The rest

**Decided: a layered-swarm surface shader.** A population has no
members to draw — that is the entire point of
[04-stellar-photometry.md](04-stellar-photometry.md) — so the renderer cannot instance from
world state and should not invent world state to instance from.

Instead, draw the population's envelope: a torus for a belt, a shell for a swarm, sized from
the distribution's `a` and inclination spread. Shade it with a procedural texture that reads
as many small bodies, with two or three parallaxing layers to give it depth and a density that
tracks the population's actual `Sigma`. Where the baked emission shell exists, sample its `m`
channel so the visible density and the photometric deficit are the same number.

This is good enough, it costs one draw call per population regardless of element count, and it
cannot drift out of agreement with the physics because it is reading the physics.

Individual elements are drawn only when they have been promoted out of the population — when a
player selects specific members for a manoeuvre — at which point there are a handful of them
and they are ordinary bodies.

## Performance shape

The object count per frame is bounded by what is *visible*, not by what exists. The
light-cone cursor from [02-event-store.md](02-event-store.md) already yields sources in
arrival order and prunes by strength, so the renderer takes the first N and stops. A sky
with 120 000 catalogue stars draws as one instanced point cloud. A swarm is not drawn from
its members, because it has none: the renderer samples the population's distribution to
generate as many representative instances as the current LOD calls for, seeded so the same
swarm looks the same every frame and from every client. Instance count is a rendering
budget, not a world-state quantity.

`PERF_AUDIT.md` in the repo root records the existing app's measured hot spots. Read it
before assuming where time goes in the shared crates.

## Relativistic camera

**Decided: aberration and Doppler are applied to the rendered sky when the camera is moving
fast.** Not an option, not a debug toggle. A view from a ship in transit is supposed to look
wrong, and it is one of the few places the player sees relativity directly rather than reading
about it in a number.

Both effects are exact and cheap, being per-star operations in the vertex stage:

```
aberration:  tan(theta'/2) = sqrt((1 - beta) / (1 + beta)) * tan(theta/2)
Doppler:     D = 1 / (gamma * (1 - n . beta))
beaming:     bolometric intensity scales as D^4
```

At `beta = 0.5`, a star 90 degrees off the bow appears at 60 degrees — the whole sky compresses
forward.

![The sky at 0.99c](../images/relativistic-sky.png)

**The forward sky brightens without limit and never goes dark.** An earlier version of this
document said otherwise — that head-on light blueshifts out of the visible band and forward
sources vanish while brightening — and the renderer disproved it. That argument holds for a
monochromatic source and a star is a continuum. The band an observer looks through is fed by
whatever the star emitted at `lambda / D`, and a hot star has plenty there.

Put exactly: a blackbody seen with Doppler factor `D` is a blackbody at `D T`, and
`B_lambda(lambda, T)` is strictly increasing in `T` at fixed `lambda`. So every band brightens
going forward, monotonically, forever. For a sun-like star the B band rises 11 times at
`D = 1.73` and 193 times at `D = 6.4`.

Astern the same identity runs the other way and the sky really does go out: at `D = 1/6.4` the
B band is down by thirteen orders of magnitude.

So at high `beta` the sky is **a brilliant blue disc ahead and darkness everywhere else** — not
a ring. It is unusable for navigation, which is correct, and it is the most striking image the
renderer can produce. Capturing a still from a ship in transit is worth making an explicit
affordance; `--shot` does it.

## Checking a renderer without a window

![Observer snapshot](../images/observer-snapshot.png)

The client's session state drawn straight to a PNG by `em-plot`: four thousand catalogue
stars shaded through the current band mapping, and the light curve the telescope has
accumulated. The galactic plane is visible as the band across the sky map, star colours come
from their own temperatures, and the curve is labelled with what it actually is — light from
Proxima that left 4.2 years ago, plotted against **emission** time rather than arrival.

```bash
cargo run -p lc-client --bin snapshot -- snap.png assets/catalogs/hygdata_v42.csv
```

A window is the obvious way to check a renderer and it is not the only one. Anything that
decides *what* to draw can be exercised headlessly, and separating that from the drawing is
what phase 5 found a renderer needs anyway.

## Bands and the display mapping

The renderer computes radiance in the seven bands of
[04-stellar-photometry.md](04-stellar-photometry.md) and ends in three. The interesting part is
the ending, not the computing.

### Channel count is not the constraint

Carrying seven bands through a fragment shader is free — they are registers. If a deferred path
ever needs them in a G-buffer, WebGPU's base limits allow `maxColorAttachmentBytesPerSample` of
32, which at `rgba16float` is four attachments, or **16 float channels per pass within the
guaranteed limits**, against seven needed. Half-float render targets are core; the
`shader-f16` extension is only needed for f16 arithmetic, which this does not require.

The constraint is the display: three primaries and a trichromat viewer. Wide-gamut and HDR
panels give more saturated primaries, not more dimensions.

So the design question is the mapping, and the mapping belongs to the player.

### The player configures it

**Decided: the band-to-display matrix is user-controlled, with presets.** The player is a ship
with no eyes, looking at the output of its own processing pipeline. Letting them reconfigure it
is characterisation, not a compromise — and it is exactly what observational astronomy does,
where every published image is a choice of filters mapped to three channels.

```rust
pub struct BandMapping {
    /// 3 x BANDS. Rows are display R, G, B; columns are physical bands.
    pub matrix: [[f32; BANDS]; 3],
    /// Bands the viewing instrument cannot sense are masked to zero and shown as
    /// unavailable, not as black.
    pub available: BandMask,
    pub bloom_band: Option<BandIndex>,
    pub bloom_gain: f32,
}
```

| preset | mapping | shows |
|---|---|---|
| natural | R, V, B to display R, G, B | what a human would see, measured rather than inferred |
| deep natural | R+I, V, B | natural colour with M dwarfs at their real brightness |
| thermal | 10 um, K, V | industry and waste heat; a rival's swarm becomes a colour |
| dust penetration | 21 cm, 10 um, K | through clouds that are opaque in V |
| composition | K, V, B | the grey-versus-reddening diagnostic, made visible: dust reads orange, a swarm reads neutral |
| survey | V as luminance, 10 um as chroma | a monochrome sky in which only excess heat is coloured |

### A wide mapping has to be normalised

A preset whose three bands are far apart in wavelength cannot use weight 1 on each. A sun-like
star delivers **88 times** more band-integrated radiance in V than at ten microns, so an
unweighted thermal mapping renders every ordinary star blue, and a swarm's thermal excess has to
beat its own star's visible light before it shows at all. Everything under about half coverage
stays invisible. The physics was right and the mapping could not show it.

`BandMapping::direct_normalised` weights each channel by `1 / B_band(5772 K)`, so a sun-like star
comes out neutral and an excess in any band is a colour. That is what a false-colour astronomical
image does and why they are readable. `thermal`, `dust_penetration` and `composition` all use it.

`natural` does not, and must not: B, V and R sit close enough together that a blackbody is
already nearly neutral across them, and the small departure from neutral is the star's real
colour.

**The natural preset is the default and exists for the player, not for the science.** It buys
no information the others do not, and a human looking at a sky that looks like a sky is worth
a band. B, V and R are close enough to the display primaries that a direct assignment works.

Strictly it is not exact, and the error is concentrated where it is most visible.
Photometric B, V and R are narrower than the CIE matching functions and `x-bar` has a blue
secondary lobe a direct map misses, so direct assignment **fails to converge to neutral near
white**: measured against the CIE route, a 5772 K star comes out about 1.5 times as saturated
as it should be. At the extremes the two agree within a few percent — a 2500 K or 20 000 K
star is strongly coloured either way.

So the CIE route — integrate the optical bands against the matching functions, convert XYZ to
sRGB — is worth it for the natural preset, where a colour cast on a sun-like star is exactly
what a player would notice, and not worth it anywhere else.

The composition preset is the one worth building first. It turns the photometric diagnostic
into something the player sees rather than reads, and the whole point of computing occlusion
chromatically is that the difference is visible.

### Bloom is a fourth channel

The tone-mapping decision below already routes overflow into glow, so halo radius is a display
dimension that reads independently of pixel colour. Assigning it a band of its own is nearly
free, and thermal IR is the obvious candidate: a structure radiating waste heat gets a halo
that a cold body of the same brightness does not.

Realistic ceiling for simultaneously legible channels is about five — three colour, one bloom,
one riding in fine luminance detail, since acuity is far higher in luminance than in chroma.
Past that, viewers stop reading it as information. Temporal cycling of channels is excluded: it
is nauseating and it destroys the ability to read a static frame, which is most of what this
game asks.

### Sensors are hardware

**Decided: the available band set is a property of the viewing instrument.** The ship's own
suite starts narrow — V alone, a greyscale sky — and widens as sensors are built. Looking
through a remote telescope uses that telescope's bands, so the view changes depending on which
instrument the player is looking through.

This is progression that is diegetic and costs nothing: the mechanism already exists because
instruments already carry a band mask for observation purposes, per
[05-observation.md](05-observation.md). The view and the science read the same field.

It also means two players can look at the same star and be working from genuinely different
data. That is correct and intended; see the same document for why it is load-bearing rather
than a UI hazard.

### What is deliberately not built

Full spectral rendering — dozens of bins, spectral transport, dispersion — buys nothing here.
There is no refraction worth modelling and the occlusion model is band-integrated by
construction. Generating more spectral resolution than the photometry has is inventing data.

Integration cost stays low because the scene is emissive-dominated: stars, point sources,
glowing structures. Emissive sources never enter Bevy's RGB-centric PBR path at all — compute
per-band radiance, apply the 3xN matrix at the end, done. **Collapse to three channels at the
end of the lighting calculation, never in the middle.**

## Tone mapping

**Decided: 2-3 stops of displayed luminance, with everything above that driving glow.**

Two things that only became clear once it was implemented:

- **The window has no absolute reference; it has to be placed by the scene.** A star's band
  radiance at interstellar range is of order 1e-10 in SI units, so any fixed reference is
  thirty stops out and the picture is uniformly black or white. Exposure is set from a high
  percentile of the sky rather than its maximum, because one star can be arbitrarily nearer
  than the rest — the catalogue puts the Sun about an astronomical unit away, and it outshines
  a star four light-years off by some thirty-six stops. Letting the brightest couple of percent
  clip is what a star map does anyway, and it is why daylight hides the sky.
- **Below the window, a point source is small rather than black.** The two-or-three-stop
  window is for surface brightness. A star field spans far more than that, so a shaded value
  carries its true signed offset from the reference, and the renderer maps that to size across
  about fourteen stops while colour stays inside the window.

Physical flux in this game spans something like sixty stops, from a star at 1 AU to the
faintest thing worth drawing. No tone curve maps that to a display. The mapping is therefore
strongly logarithmic — which is what magnitudes already are — compressed into a 2-3 stop
window, and the overflow is routed into bloom radius and intensity rather than into pixel
value.

The in-fiction justification is that the player is a ship with superhuman sensors and is
looking at a processed view, not a photograph. The practical justification is that it is the
only mapping that shows a star and a rock at the same time.

Glow carries the dynamic range the pixels cannot: a very bright source is not a brighter white,
it is a wider halo. That reads correctly, matches how bright points look through any real
optic, and leaves the 2-3 stop window free for the things that have detail in them.

### Metering

**Decided: the percentile is over area, not over count.**

![Saturn, metered](../images/exposure.png)

*The same view metered by the star field alone put Saturn flat against the top of the window and
left the sky at full brightness.*

Two kinds of thing are drawn and they are not measured in the same unit. A point delivers a
*flux*, in W/m^2, into however many pixels the renderer decides to spread it over. A surface has
a *radiance*, in W/m^2/sr, which does not change as the ship approaches it — only its size on
screen does. Exposing both from one number means reducing each to the brightness it has per
unit of sky it covers: a surface's own radiance, and for a point the flux divided by the solid
angle it is drawn at. Weighting each sample by that same solid angle makes the sum the power
actually collected, so the rule reads as a light meter does — expose so that `1 - fraction` of
the frame clips.

Everything the old count percentile did survives, because points all carry the same weight and
the solid angle divides out of both the samples and the total. A sky with no bodies in it meters
exactly as it did, which is what lets the star field keep its tuning. A resolved planet does
not carry a point's weight: against six thousand stars a body takes the exposure once it is
about twelve pixels across, which is roughly where it stops being a dot.

The tone map therefore holds two references a solid angle apart — one flux, one radiance —
placed by the same pass and moved together by the exposure control.

Two practical constraints:

- **The metering is over the whole sky, not the frame.** A light meter sees what the lens sees;
  this one sees every star in the session, so a body's share of the picture is understated by
  the ratio of the sphere to the field of view. Restricting it to the frame would also make the
  exposure change as the camera turns, which needs the star half of the pass cached before it
  can run at frame rate.
- **Re-placing the window costs a pass over the catalogue**, so it happens when the bodies'
  contribution has moved a quarter of a stop rather than every frame. A body's surface radiance
  does not depend on the ship's distance at all, so an approach crosses that a few dozen times
  rather than continuously. Unresolved bodies are ranked by a Stefan-Boltzmann proxy and only
  the brightest thirty-two are shaded: shading all two hundred and thirty solar system bodies
  cost 2.5 ms a frame, and all but a handful sit thirty stops under the cut.

## Open

- `u-v` plane and correlation displays for interferometry, which are charts rather than scenes
  and therefore belong to [11-plotting.md](11-plotting.md), but need a place in the UI.
- Whether the layered-swarm shader needs a second appearance for dust, given that dust is
  chromatic and a swarm is grey. Probably yes, and it is the visual form of the diagnostic in
  [04-stellar-photometry.md](04-stellar-photometry.md).
- How to present an unavailable band. Masking it to zero makes a scene look dark rather than
  uninstrumented, and the difference matters when the player is deciding what to build.
