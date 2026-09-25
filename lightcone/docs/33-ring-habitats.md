# Ring habitats

Spun rings with a floor on the inside: the Culture's Orbitals and Niven's Ringworld. Players see
them called by those names; the code calls every one a `ring_habitat`, because "orbital" already
means orbital elements, periods and planes everywhere in this repository.

**Status: design only.** `~/rust/vavatch-orbital` is the prototype this starts from: the
parameter set, the floor-and-walls geometry and the look. Its destruction is not part of this.

## One shape, two hosts

Every ring habitat is the same five numbers:

| | |
|---|---|
| radius | to the floor, m |
| width | of the floor, m |
| wall height | rim wall, measured inward from the floor |
| wall flare | how far the wall top leans outward past the floor's edge |
| gravity | at the floor; the spin follows as `ω = √(g / r)` |

What differs is what it is attached to. An **Orbital** is a body in its own right, in orbit about
a star, spinning about an axis of its own. A **Ringworld** is centered on its star and has no orbit
at all.

The three to be built first:

| | radius | width | g | day | rim speed | light-time across | floor, in Earths |
|---|---|---|---|---|---|---|---|
| Orbital, 1.5 AU | 2.2e9 m | 3.5e7 m | 1.2 | 23.9 h | 161 km/s | 14.7 s | 950 |
| Orbital, Sun–Earth L1 | 3e8 m | 4.8e6 m | 1.0 | 9.7 h | 54 km/s | 2.0 s | 18 |
| Ringworld | 1 AU | 1.6e9 m | 0.98 | 9.1 d | 1200 km/s | 998 s | 3,000,000 |

The first is vavatch's own default. Its 24-hour day is no coincidence: at one gravity, a 24-hour day fixes the radius at 1.86e9 m, which is why the Culture's
Orbitals are all about that size. The L1 ring cannot be that size: L1 is only 1.49e9 m from Earth, so a
2.2e9 m ring would enclose it. At 3e8 m it clears the Moon's orbit by 8e8 m and pays for it with a
short day. Its width keeps vavatch's proportion. Both of its numbers are proposals.

The Ringworld is Niven's: one astronomical unit, a million miles wide, thousand-mile rim walls,
1200 km/s.

## Where they come from

**Decided: none are generated.** A ring habitat is something a civilization builds. The only
ones that exist without being built are the demos: two Orbitals in the solar system, and the
Ringworld around HD 189567.

They are records in `lc_world::ring_habitat`, keyed by an `em-sim` body id, the way
`lc_world::rings` hangs ring systems off planets. They are added to the `System` when the world is
built. They are **not** written into `assets/systems/solar_system.em`: that file is generated from
JPL Horizons and pinned by `tests/ephemeris.rs`, and Exotic Matters reads it too.

The body's `em-sim` radius must not be the ring's radius. Collision, crossing and the occluder all
read that radius as a sphere, and a sphere 2.2e9 m across would forbid the one thing everybody
will try first, which is flying through the hole.

### The Orbital at 1.5 AU

A circular Keplerian orbit, period 1.84 years. Mars runs from 1.38 to 1.67 AU, so it crosses 1.5 AU
twice a Martian year, and with a synodic period of 79 years it will eventually pass close to, or through, the ring.
Either incline the orbit a few degrees or choose its phase and pin a minimum-separation test over
a millennium. At 1.5 AU the floor gets 44% of Earth's sunlight. `lc_world::climate` would call that
cold. The surface is authored rather than derived, so that is a fact about the place and not a
constraint on its look.

### The Orbital at L1

`em-sim` has no motive for this. `Fixed` is fixed relative to a parent, not co-rotating with a
pair. It wants a new `MotiveSelection` for a Lagrange point of two bodies. The point itself is
already solved: `em_foundations::lagrange::Collinear` is the root of Lagrange's quintic, and
`lc_world::navigation`'s `Course::Lagrange` puts ships there with it. The pair is the Sun and the
Earth–Moon barycenter, which is the convention the real L1 missions use. The new motive is an
addition, so Exotic Matters keeps building.

L1 is unstable. The habitat is taken to station-keep, the way SOHO does, and is drawn at the point
itself.

**Its orientation is a real choice, because it decides whether Earth gets an eclipse.** L1 lies on
the Sun–Earth line, and from there the solar disc is only 7,000 km in radius. Whenever that line
lies in the ring's plane, it crosses the band, and the band is 4,800 km wide.

- **Axis fixed in space.** The existing `RotationMode::Spinning` does this. The line sweeps through
  the ring's plane at least twice a year, and Earth has **eclipse seasons**. They would be short
  and seen from everywhere on the day side.
- **Axis fixed in the Sun–Earth frame.** This needs a new rotation mode that turns the pole once a
  year. There are no eclipses, as long as the plane is tilted far enough off the Sun–Earth line
  that the band clears the disc. Physically, it is a gyroscope being turned once a year, which is
  handwaving on the Culture's scale.

The ring must also be lit. With the Sun on its axis, the floor only gets grazing light and stays
dark. That is the most photogenic option from Earth, a hoop 23° across around the Sun, and the
worst one to live on.

### The Ringworld

The Ringworld is `Fixed` at its star with zero offset, and its axis is the generated system's
pole, so it lies in the plane its planets orbit in. It has no orbital motion to model. Its mass,
about Jupiter's, is left out of `em-sim`, and the planets inside it keep their Keplerian orbits.
A ring's field is not a point mass, and pulls an interior planet outward, toward the nearest arc,
not inward. Its famous instability is not modeled either. It is held by attitude jets, and so are
we.

**Decided: it goes around HD 189567.** It cannot go around the Sun at one astronomical unit,
because Earth is there. HD 189567 is HIP 98959, a G2V dwarf in Pavo 57.8 ly away, HYG row 98643,
`StarId` `0x6344b3af85c80770`. It is the closest match to the Sun within 25 pc in the catalog's own
numbers:

| | HD 189567 | Sun |
|---|---|---|
| absolute magnitude | 4.83 | 4.83 |
| B−V | 0.648 | 0.656 |
| luminosity | 1.02 | 1 |
| mass | 1.006 | 1 |

At one astronomical unit the floor gets 2% more light than Earth does.

It also has room. The generator gives it six rocky planets inside 0.69 AU, the outermost of which
is at 0.64 AU, e 0.077, and a seventh at 1.25 AU, e 0.037, which comes no closer than 1.20 AU.
Nothing crosses one astronomical unit. From the floor, the inner planets are in the sky, beside the
Arch. The outer one is under the floor. Every other close twin gets a planet that crosses 1 AU:

- **18 Scorpii**, the famous solar twin, gets a planet at 1.13 AU with an eccentricity of 0.12.
- **HD 10307** is clear from 0.13 to 11.6 AU, but it shines at 1.47 suns.

The gap is the generator's output, not the catalog's. A test pins it: HD 189567's generated
planets stay clear of the Ringworld's radius, so a change to the generator's tuning cannot quietly
run a planet through the floor.

The Ringworld is reached with `teleport 0x6344b3af85c80770`, or in singleplayer with a dev flag
beside `--at` that starts the ship at a star by id. That flag does not exist yet.

## Scale is the design constraint

At a ring's own radius, f32 resolves:

| | radius | f32 step |
|---|---|---|
| L1 Orbital | 3e8 m | 32 m |
| 1.5 AU Orbital | 2.2e9 m | 256 m |
| Ringworld | 1.5e11 m | 16 km |

A single mesh in local coordinates is fine from across a system. It is unusable at the surface
tier ([07-rendering.md](07-rendering.md#scale-tiers)). So:

- **Geometry is chunked.** It is cut into columns around the ring, vavatch's decomposition kept
  as render chunks rather than fragments. Each chunk has an f64 origin that is subtracted on the
  CPU before narrowing, as `camera_relative` does for everything else. Near the eye, chunks
  subdivide further. It also keeps the door open to tearing a ring apart later.
- **The spin phase is reduced on the CPU.** `ω t` since J2000 is tens of thousands of radians for
  an Orbital. It is taken modulo 2π in f64 and uploaded. The shader only ever adds small angles to
  it.
- **Surface coordinates are split.** A position along the floor is an integer cell and a fraction,
  never one float. Along the Ringworld's floor, a single f32 cannot hold anything finer than 56 km.

Tessellation around the ring is cheap by comparison. At 4096 columns, the chord of a Ringworld
column stands 44 km off the true arc, invisible against a floor 1.6e9 m wide.

## Levels of detail

**Decided: detail is chosen per chunk, by the size of a texel on screen, never per habitat.**
Standing on the Ringworld's floor, the ground at your feet, the floor running away to the horizon
and the Arch overhead are the same object, 998 light-seconds across, in one frame. They need all
three tiers at once.

1. **Near: a small area, baked finely.** The floor around the eye's footprint is baked by
   texture-graph into a stack of levels, each twice as coarse and twice as wide as the one inside
   it: a clipmap. At 512² a level, twenty-odd levels span one-meter texels up to where the mid tier
   takes over, for about 24 MB. When the footprint moves, only the newly exposed strip of each
   level is re-baked. A level still catching up falls back to the coarser one outside it. **A bake
   never blocks a frame.**
2. **Mid: the whole ring, coarsely.** One baked atlas of the floor, the back and the walls in
   strips. The 1.5 AU Orbital fits vavatch's size, a 12,640 × 32 strip with texels of about
   1,100 km. The Ringworld takes a texture array: 64 MB puts its texels near 10,000 km, which is
   still a fraction of a pixel from anywhere outside it.
3. **Far: vertex color.** Each vertex of the coarse ring carries the mid tier's mean albedo per
   band. No texture is sampled at all.

**All three come from one graph, band-limited.** The mid tier is the near tier's content with
every octave finer than its texel replaced by that octave's mean, and the far tier is the mid
tier averaged. That is what stops a pop at the crossover, and what makes a far ring the same
color as the close one. The hull material already fades detail into its own average
([32-ship-rendering.md](32-ship-rendering.md#details-are-sized-in-meters)), and this is the same
rule at a larger scale.

Two things texture-graph needs for this:

- **Evaluation over an offset domain**, taking the integer cell separately from the fraction, so a
  tile a trillion meters along the floor has the same precision as one at zero.
- **A bake that drops octaves past a given frequency** and substitutes their mean.

It already has the periodic value lattice. The circumference must be a whole number of periods
at every frequency the graph uses, or there is a seam at angle zero.

## Light delay

The whole ring cannot be sampled at one retarded time. Every other object is either smaller than
its light-time error or far enough away that the error is sub-pixel
([07-rendering.md](07-rendering.md#retarded-time-sampling)). A ring habitat is neither.

**The shape does not change.** A spinning ring is a surface of revolution and maps onto itself
under any rotation, so the mesh needs no displacing. What moves is everything *on* it. A point
of the floor at material angle `θ` is seen where it was at the moment its light left, and the
far side is seen earlier than the near side. The lookup is therefore corrected, not the
geometry:

```
θ = ψ − φ_now + ω · d / c
```

- `ψ` is the fragment's angle around the ring in the world frame.
- `φ_now` is the spin phase from the CPU.
- `d` is the fragment's distance from the eye.

Render space is already camera-relative, so `d` is the length of the fragment's own position. The
correction is one length and one multiply-add. In the near tier it is applied in the tile's local
coordinates. It is small there, but it has to be continuous with the mid tier.

What that produces:

| | far side lags the near side by | on the floor |
|---|---|---|
| L1 Orbital | 0.36 mrad | 110 km |
| 1.5 AU Orbital | 1.1 mrad | 2,400 km |
| Ringworld | 8.0 mrad | 1.2e9 m, three quarters of its own width |

The squish is small next to the offset. Features running toward the eye are compressed, and
those running away are stretched, by at most `1 / (1 − v/c)`. For the Ringworld's rim that is
1.004. What the eye sees is the Ringworld's far side with its pattern set back 1.2e9 m against
the near side's.

**Things on the ring are the exception, and those do need the vertex shader.** A structure, a
docked ship, a spill mountain or a shadow square is not symmetric about the axis. It is drawn at
`ψ = θ + φ(t − d(ψ)/c)`. That equation is implicit in `ψ` and settles in a fixed-point iteration
or two, because `ω d / c` is small.

**Lighting needs no correction.** The star, the ring and the ring's shadow on itself are all
symmetric, or fixed in the world frame, so the terminator stands still while the floor turns
through it. The one exception is the Ringworld's shadow squares. Their shadow on the floor comes
from where they were when the light left the star: 499 s before it lands, plus the delay to the
eye.

The Orbital's orbital motion adds a residual skew across the ring of `v · Δd / c`, about 350 km at
1.5 AU. The Ringworld's is its star's motion, which is meters per second. Both are left alone
until something is placed close enough for them to show.

Doppler across the Ringworld's rim is ±0.4%, narrow next to any band. It is ignored.

## Lighting

The look vavatch has comes from Bevy's metallic PBR, a directional light and bloom. Lightcone
has none of those. Hulls and bodies are lit per band through `em_render::body_surface_material`,
and a ring habitat will be too. It starts matte. **The metallic sheen needs a specular term in
the per-band material.** That is the one piece of vavatch that does not come across by porting.

- **Orbital day and night come from a tilt.** The star is near the ring's plane, and the near
  arc shades the far arc except where the tilt lets light over the rim wall. A point has day on
  the far side of the ring from the star and night on the near side. That shadow is analytic, a
  ray from the fragment to the star tested against the band, and it costs a few instructions in
  the fragment shader.
- **Ringworld day and night come from shadow squares.** The floor always faces the star, so the
  squares' shadows are the whole of the day cycle.
- **The night side is lit from inside.** Living areas glow, the same rule as the hull
  ([32-ship-rendering.md](32-ship-rendering.md#two-looks)).

## Photometry

Neither shape is a sphere, and `lc_world::occluder` models only spheres.

- **An Orbital is a moving annulus.** Edge-on it presents up to 1.5e17 m², ten Jupiter discs, and
  face-on almost nothing, so its brightness as a point is strongly orientation-dependent. It is
  given an area, not a radius, the way Saturn's rings already are. A transit is a band across
  the stellar disc, unlike a planet's disc.
- **A Ringworld is fixed.** It intercepts 0.54% of its star's light, and it does so in a band of
  ±0.31° about its plane. An observer in that band never sees the star at all, only the ring's
  back, glowing at the floor's waste-heat temperature. This is the "large fixed occlusion over a
  solid angle" of [03-world-model.md](03-world-model.md), and it belongs on the emission shell as
  a band, not as a transit.

Either one is among the loudest artificial things in a sky, which seems right.

## Order of work

1. `lc_world::ring_habitat`: the record and derived spin. The 1.5 AU Orbital in the solar-system
   demo, drawn as a point at its area.
2. The far tier: a coarse ring, vertex-colored, lit per band, matte. The light-delay correction
   and the phase reduction, from the start, because they are hard to retrofit.
3. The L1 motive, and whichever rotation mode the orientation choice asks for.
4. The mid tier: re-authored vavatch graphs as `.tgraph`, periodic, baked as strips.
5. Chunking with f64 origins, then the near-tier clipmap. This is the texture-graph work.
6. The analytic self-shadow and the specular term.
7. The Ringworld at HD 189567, its shadow squares, and its band on the emission shell.

## Open

- **The L1 ring's size and orientation:** eclipse seasons, or an axis turning once a year.
- **Shadow squares:** count, size, orbit and sense of rotation.
- **Tearing one apart.** Deferred. The chunks are vavatch's columns so that it stays possible.
