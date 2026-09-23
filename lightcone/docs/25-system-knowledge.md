# What a craft knows about a system

A system's planets, their orbits and the plane they share are **knowledge**, with provenance,
exactly as a star's distance is ([22-provenance.md](22-provenance.md)). Nothing a player sees about
a system reads the generator. This extends the transit search in
[24-standing-instruments.md](24-standing-instruments.md) from "there is probably a planet" to a
body with a name, an orbit and a place in a plane that was itself worked out.

Status: **design**, with phases 1 to 5 built, bar a deferred item in each of 3 and 5. Nothing
below is built except where it says so, and what is carries a mark. Every claim about what
exists was checked against the code on 2026-09-22, and the symbols named are real; where a draft
of this document guessed wrong, the correction is in the text rather than quietly removed,
because the wrong guess was usually "that already exists" about something that does not.

## Where it is today

- The System panel lists `LocalSystem::inventory()`: every body the generator made, with its true
  orbit radius. The map draws the same truth, and courses resolve against it.
- The transit search concludes *a rocky or giant planet on a P-day orbit* about a **star**. No
  body is ever created from it: `Knowledge::found_planet` exists, letters a planet, and has no
  caller outside tests, not one. `Orbit` carries a single element, `semi_major_au`.
- ~~The map's **Ecliptic** plane is `+Z`, the ecliptic of J2000, in every system~~ **Fixed,
  phase 1.** It was `+Z` in `normal()`, `basis()` and a comment, so a generated system's planets
  and belts — which orbit `generate::pole_for(seed)` — were tilted out of the plane drawn under
  them, and the star's map sphere was pinned to `+Z` besides. That is the bug that started this.
  `Plane` is now a selector and `Plane::about(system_pole)` yields a `Datum` carrying the
  resolved basis; see phase 1 below for what it changed.
- Truth already holds a good deal of what the survey below wants to measure. See *What the truth
  has to hold first*, which lists it against what was absent and is now built.

## The rule, for systems

1. A body exists for a craft when the craft holds evidence of it, and not before.
2. Everything about a body is its own record, from a witness, with lineage: that it exists, its
   period, its distance, its shape, its orientation, what kind of thing it is.
3. A system's plane is a **belief drawn from its orbits**, as a star's distance is a belief
   drawn from bearings. It has an error, and two craft that solved different data hold slightly
   different planes. Sol's plane, solved, is close to the ecliptic of J2000 and not equal to it.
4. What cannot be known is shown as unknown, not guessed. An orbit of known size and unknown
   orientation is a sphere of that radius, not a ring in some plane.

## Records

On `Subject::Body { star, body }`, keyed by the `BodyId` the generator's key hashes to, which
never reaches a player — except for a body no generator made, which is *Which body a transit is*
below.

| record | holds | from |
|---|---|---|
| `Sighting` | a bearing to the body and its reflected flux, as for a star | imaging inside the system |
| `Orbit` | elements with errors, and how much of the orientation is known (below) | a transit fit, an astrometric fit, or a claim |
| `Naming` | as now: a planet letter from `found_planet`, and any given name | the crew, other craft |
| `Conclusion` | what kind of body: rocky or giant, and later temperature and albedo | transit depth, reflected and thermal flux |

`Orbit` grew from one number to what a fit actually yields — this is the built shape, in
`knowledge/record.rs`:

```rust
pub struct Orbit {
    pub witness: Witness,
    pub period_s: (f64, f64),            // value, sigma
    pub semi_major_au: (f64, f64),
    pub eccentricity: Option<(f64, f64)>,
    pub orientation: Orientation,
    /// A time at which the body was at a known place on the orbit: a transit's mid-time, or an
    /// astrometric fit's epoch. With a full orientation this places the body now.
    pub epoch_s: Option<f64>,
    pub method: Method,                  // Transit, Astrometric, or Claim: stated by a craft that sent no raw data
    pub stated_s: f64,
    pub lineage: Lineage,
}

pub enum Orientation {
    Unknown,
    /// Seen to transit from `toward`: the orbit's pole is perpendicular to that line of sight,
    /// somewhere on a great circle, and the body was on that line at `epoch_s`.
    EdgeOnTo { toward: DVec3 },
    /// `pole` and `node` are in **simulation axes**, never in the system plane. Longitude
    /// measured from the system plane's zero is a display quantity, derived at read time: the
    /// plane is a belief drawn from these orbits, so storing a longitude in it would define
    /// each orbit against a frame that its own value helps determine, and refining one orbit
    /// would silently move every other one's stored number.
    Known { pole: DVec3, sigma_rad: f64, node: f64, periapsis: f64 },
}
```

**Frames.** Every direction in a record is in simulation axes, which is what truth and
`position_ly` already use: right-handed, Z-up, the ecliptic of J2000. That is a storage
convention and not a claim that a craft knows where J2000's ecliptic is — a craft knows its own
attitude, and the axes are the arena's. Display longitudes are computed from the plane belief
when the panel asks, which is also what keeps them stable as the plane refines.

`Orbit` is changed in place. The old shape gets no back-reader: the game has no players, so a
stored knowledge file is worth less than the ceremony of keeping it. `FILE_FORMAT` is 4, decoded
natively, and its three back-readers `FileV3`, `FileV2` and `FileV1` each embed `Orbit` by name,
so keeping them would mean freezing an `OrbitV4` beside them and repointing all three. Delete
those readers with the shape they read, and `OLDEST_FILE_FORMAT` becomes the new one.

## How each thing is learned

### From outside: transits (the search is built)

A settled transit conclusion (`SETTLED`, three transits) **creates the body**: `found_planet` is
called with it, the planet gets its letter, and the conclusion's class moves onto the body.

- **Year length** is the period, measured directly.
- **Distance from the star** comes from Kepler's third law, with the star's mass from its
  luminosity class via the generator's prior. The error is mostly the mass's.
- **Kind** comes from the depth, which gives the radius ratio. That is today's rocky-or-giant
  split.
- **Orientation** is `EdgeOnTo`: the pole lies on the great circle perpendicular to the line of
  sight. The mid-transit time puts the planet on that line at a known instant, one point of the
  orbit in 3D.
- Two craft that saw the same planet transit from different directions have two great circles.
  They cross at the pole, up to its sign, and the direction of motion settles the sign. A
  planet's orientation from outside is a **cooperative** measurement, which suits reports.

### Which body a transit is

`BodyId::of(star, key)` hashes the generator's key for the body, and a settled `Candidate` has no
key: it has a period, a depth and an epoch found in a light curve. Nothing in the search can mint
the `BodyId` the imaging path would use for the same planet, and a settled false positive has no
true body at all. So:

- **At settle time the shard matches the candidate's period to the system's planets** and, on a
  match within the period's sigma, uses that planet's `BodyId`. This is a truth lookup through
  `generate::system_for`, it happens shard-side where truth already lives, and it is the same
  kind of read the transit search already does to generate a light curve.
- **A candidate that matches nothing gets a `BodyId` derived from `(star, witness, period
  bucket)`.** It is a body this craft believes in, drawn on its map, and nobody else can confirm
  it. Two craft with the same false positive get different ids and never merge, which is correct:
  they have no shared object to agree about.
- The period bucket, rather than the raw period, is what makes a craft's own later transits of
  its own false positive land on the same id.

**Cross-identification** — imaging later finding the real body and having to be recognized as the
same thing — is 22-provenance's open question, and this is where it stops being theoretical. The
truth match above is what avoids it in the normal case. A craft that imaged a planet before
anyone's transit settled still merges by `BodyId`, because both paths went through the truth
lookup. The case with no answer yet is two craft merging catalogues where one holds a false
positive at the same period as the other's real planet, and the honest outcome is that they hold
two bodies.

### The star's mass, and the ship's distance to it

Everything above and in the next section needs the star's `mu` with an error, and **nothing holds
a believed mass today**. Kepler's third law turns a period into a radius with it; the angles-only
fit needs it as a parameter; a moon's orbit measures it. It cannot come from truth. Needed:

- **A mass record on the star's subject,** with a sigma and a lineage, like a distance. First from
  the luminosity prior, then refined once two orbits are held, then again from any moon.
- **The ship's distance to its own star as a belief,** because without it the mass prior cannot
  run at all. The whole chain is visible in one line, `conclusion.rs:359`:

  ```rust
  let host = belief.and_then(|b| prior.host_like(b.band, b.luminosity_w()?));
  ```

  `Belief::luminosity_w` is `Option` only because of the distance, and `Distance::from` returns
  `Some` only for `Distance::Measured` — `Unknown` and `AtLeast` both give `None`. So no measured
  distance means no luminosity, no `host_like`, no host mass, no Kepler radius. For a ship that
  knows nothing, that chain is broken at the first link about its own sun.

  Inside a system the distance is a parallax against the ship's own motion, short-baseline but
  very close, and `astrometry::triangulate` already computes exactly that from bearings. It is the
  **first** thing a new ship measures, because every other number in the system hangs off it.

  **Two things block that, both found by reading rather than by running.** The plan is right; it
  does not work on the code as it stands.

  1. **A ship at rest measures nothing, and this one is at rest exactly.** `ShipState::at` is
     documented "At rest at a point" and sets `beta: DVec3::ZERO` with `Motive::Drifting`;
     `World::start` hands a new craft a position and nothing else; and `Sky::sources` reads the
     *catalogue* position for every star, which for the host is the system's origin and never
     moves. So the baseline is zero forever and so is the parallax.

     That is not a defect to fix. Arriving at rest is a decision already taken and already
     pinned: `crossing_to_a_star_arrives_in_its_system_at_rest_and_not_in_an_orbit` says
     "arriving puts the ship in the new system at rest, and choosing an orbit there is a second
     order", and `Course::Orbit` is that order. Nor is it one to work around — that a ship has
     to move before it can measure anything is the rule this whole document rests on, and the
     host star is not an exception to it.

     What is wrong is the word **parked** in the done-when below. A ship holding still 5 AU out
     can never learn how far away its own sun is, and should not. The scenario has to be a ship
     **in orbit** at 5 AU, which is `Course::Orbit`. Measured, with sixteen bearings round the
     orbit and a centroid at the `resolution * CENTROID_FLOOR` floor of 3e-10 rad: a sigma of
     6.2e-15 ly, a part in 1.3e10 of 5 AU. The 1% planet radii the done-when asks for need
     nothing like that much, and nothing else comes close to it.

     An angular diameter was considered as a route that works at rest, and is not one. The
     disc is 6,244 resolution elements across at 5 AU, so the *diameter* is easy; turning it
     into a distance needs the star's radius from a main-sequence relation whose scatter is 5 to
     10%, and 10% on the distance is 10% on every planet radius. It is worth having for its own
     sake — every planet needs one — but not as a distance to the host.
  2. **The bearings that are kept are the wrong ones for a host star.** `File::decimate` keeps
     the `BEARINGS_KEPT` sightings whose *observer positions* are farthest apart, which is right
     for a star light-years off and wrong for one 5 AU away: a ship going round it keeps bearings
     spread over the whole orbit, so the kept bearings point over as much as 360°.
     `astrometry::triangulate` then takes a weighted mean direction `z` and drops every bearing
     with `toward.dot(z) <= 0.5`. Bearings spread around a circle have a mean direction near
     zero, so most are dropped and the distance silently returns to `Unknown` — the ship would
     measure its own sun early, then stop being able to.

     ✅ **Built** (2026-09-22), and decimation was not what had to change. This is the same
     conditioning `triangulate`'s own doc warns about, approached from the other end: there the
     parallax is too small to invert the 3x3 system `sum (I - u u^T) x = sum (I - u u^T) p`,
     whose smallest eigenvalue is the parallax squared. Here it is too large for the projection
     the regression works in. **So the two solvers fail in each other's regime and nowhere
     else**, and for a source seen from around, the 3x3 system is not the unstable one — it is
     the right one, because what makes that matrix invertible is exactly the bearings pointing
     in genuinely different directions.

     `astrometry::intersect` is that solver, and `triangulate` hands over to it when the
     *widest* bearing is more than `WIDE_RAD` from the mean. Widest rather than an average,
     because one bearing past the 60° gate is one the regression drops in silence. Two passes,
     since the measurement is an angle and the residual is a length: the first finds roughly
     where the source is, the second weights each bearing by `sigma_rad * range`, which is what
     its miss distance is worth. Every interstellar case is far below the threshold and goes to
     the regression exactly as before.

     Keeping the bearings whose *observer positions* are farthest apart then turns out to be
     right for a host star after all, so `File::decimate` is untouched: a wide spread is what
     the new solver wants.
- ~~**A transit `Conclusion` records the host mass it used.**~~ **Dropped in phase 4.** The
  argument was that a receiver could not otherwise turn a relayed period into the same radius the
  sender did. It does not have to: the `Orbit` carries the axis itself, so a receiver reads the
  distance rather than recomputing it, and the two cannot disagree. The mass would matter only
  for re-deriving an axis, which nothing does. What the prior's error does feed is the sigma on
  that axis, and `Prior::host_mass` supplies it at the moment the orbit is minted.

### From inside: surveying a system

**The target: a few months in a system tells you what its planets are, beyond doubt.** The shard
runs at 8766 times real time (`server::TICK_US`), so a game year is a real hour:

| real | game | |
|---|---|---|
| one tick, 50 ms | 7.3 minutes | |
| one second | 2.4 hours | |
| one minute | 6.1 days | |
| 15 minutes | 3 months | everything below, for Venus, Earth, Mars, Jupiter and Saturn |
| 30 minutes | 6 months | |

A player who parks and surveys for a quarter of an hour of real time should get tremendous
returns, and the instrument allows it. Inside a system the major planets are the brightest things
in the sky after the star, and the ship's telescope resolves them outright. The ship sensor's
`aperture_m2` is 4.0, which `astrometry::diameter_m` makes a 2.26 m mirror, so the Rayleigh limit
at 550 nm is 0.06 arcseconds. From 5 AU, Venus is a disc of about 3 arcseconds and Jupiter about
40. So this is not the transit search's statistics at the noise floor. It is looking.

**The duty.** ✅ **Built** (2026-09-22). `Duty::Survey { star, started_s }`, which takes the
bodies of one system in turn, **brightest first**, for `SURVEY_DWELL_S` each. What it settled:

- **It does not name its targets,** unlike a `Duty::Watch`, because the craft does not know
  them: finding them is the duty. It rotates over whatever the system holds, so a body too faint
  or too close to the star is simply not detected on that pass and is there on a later one when
  the geometry has moved. Sweeping for new ones and revisiting what you hold turned out to be the
  same loop — you do not know what you hold until you look.
- **Brightest first is what the done-when needs.** Seven bodies come round per tick at the design
  rate, so ordering by brightness is what gets every major planet a position inside the first
  real second rather than four game hours in. From 5 AU the first turn is Jupiter.
- **Turns are read from the clock, not from a cursor,** so two sides that ticked differently
  agree, and each turn is stamped at the *end* of its dwell, as a sweep's fields are. Counting
  from elapsed time instead skipped turn zero on the first tick and did not come back to it for a
  whole cycle — caught by a probe that asked which planets were held after one tick and found
  Jupiter, the brightest thing in the system, missing.
- **One game hour was wrong about the preset.** Sol carries 221 bodies, so a full round is about
  four game hours; a generated system of eight planets and their moons takes minutes. The figure
  in this paragraph was written before anybody counted.
- **The system reaches the tick as an `Option`,** the way `motion::state_at` takes it, and the
  server loads the system the *duty* names rather than the one the craft is in. A survey ordered
  from outside is then refused by the physics — the bodies are points in the star's glare —
  rather than by a silent special case.
- **The sky the survey looks at is the system's bodies plus its own star, and not the catalogue.**
  The star belongs *in* that list because it is the only meaningful glare in the system and
  `look` can only be asked about it if it is a source like any other. A star light-years off
  cannot outshine a planet at 5 AU, so it can neither glare on one nor hide behind one.

- **The star is measured every tick, and does not take a turn.** It is not a target of the
  survey; it is the reference the survey is measured against, in every frame because it is the
  brightest thing in the sky and what every phase angle is reckoned from. So it costs no dwell
  of its own, and its parallax accumulates with the ship's motion tick by tick — without which
  nothing in the system has a distance and no mass prior runs. Putting it in the source list for
  its glare and never pointing at it is what the first version did, and a test asking for its
  triangulated distance is what caught that.

Measured: a ship 5 AU out holds 7 bodies and its star after one tick, and 213 of Sol's 221 after
forty, each with a bearing, a brightness and a disc, under the `BodyId` a navigation order names.
Mars is not in the first seven, which is the physics and not a fault: the Galilean moons and
Titan are all brighter than it is from there.

**A body is never given a distance by being watched move.** `astrometry::triangulate` fits a
static point to whatever bearings it is handed, and every bearing to a body is taken from inside
its own system, where it moves appreciably between them. So it returns a place the body was never
at, with the error bar of a fit that converged: a ship on a 5 AU orbit surveying Sol put **Jupiter
at 1.63 AU plus or minus 9e-7**, sixteen million sigma from where it was, which is far worse than
no answer. `Knowledge::believe` now returns `Distance::Unknown` for a `Subject::Body` and a body's
distance comes from its orbit, which is what this section always said it would. The ship's own sun
is measured from the same bearings, because it is the one thing in the system that holds still —
and that is the whole difference.

**The local star is in the way.** ✅ **Built** (2026-09-22), and the diagnosis it was built from
was half wrong, so both halves are recorded here.

The sweep has no special case for the craft's host star: `Sky::sources` iterates the whole
catalogue and gives each star a flux of `L / 4πd²`, and at `START_OFFSET_AU` — 5 AU, about
7.9e-5 ly — the host's flux is enormous. The old `glare_radius_rad` was
`resolution_rad * (SCATTER * bright / faint).sqrt().max(1.0)`, a hard disc: one radius for one
pair, floored at the resolution so it could only ever widen.

What that actually came to, measured rather than assumed:

| what is being looked for, from 5 AU | old blind spot | new one |
|---|---|---|
| Jupiter, reflecting in V | 0.0083° | one resolution element |
| Venus | 0.012° | one resolution element |
| Earth | 0.021° | one resolution element |
| Saturn, at 9 AU range | 0.035° | one resolution element |
| Mars | 0.088° | one resolution element |
| a Sun-like star 100 ly off | 0.68° | 1.6 resolution elements |
| the faintest source the instrument reaches at all | the whole sky | 4.3° |

A planet's reflected flux there is `incident_in_band * geometric_albedo * (radius / range)^2`,
which is what a geometric albedo is defined against. Getting that relation wrong in either
direction is easy and the check is a known magnitude: Jupiter comes out at 2.6e-8 W/m^2 in V
from 5 AU, against 2.7e-8 read off its -2.7 at opposition. See [`visit`].

So **Jupiter was never hidden,** and neither was any other major planet: the radius already had
the faint source's own brightness in it, and a planet at 5 AU is a billion times brighter than a
background star. What was hidden was the far end of the catalogue — the faintest stars, out to
every angle, because `sqrt(bright / faint)` has no bound and nothing clamped brightness anywhere.

That is a defect in the **shape** of the model, not in its scale. A hard disc says a source is
either seen perfectly or not at all, and says it from a ratio that grows without limit. Replaced
with the thing the disc was standing in for: **the wings of the bright source's own image**, with
a surface brightness falling as the cube of the separation, whose integral outward from one
resolution element is `SCATTER` of the source. Three properties follow, and none of them had to
be put in by hand:

- **The hole is sized by what is being looked for.** A cube root, so nine orders of contrast cost
  three of separation. From 5 AU the sun now loses a planet only where the two are one image, and
  loses the faintest source the instrument reaches — 25 counts, which is `DETECTION_SNR` against
  its own photons — out to 4.3°. That is the "small inner radius plus a falloff" this section
  asked for, and it is one expression rather than two rules.
- **Glare is background, not a veto.** `survey::look` adds the wings of everything brighter to the
  noise instead of consulting `hidden_by`, so a source beside something bright comes back with a
  worse bearing and a worse flux, and only disappears once that noise swallows it. `hidden_by`
  stays, and now answers a different question — *which* source is in the way, for a reader who
  wants to be told — and is no longer in the measuring path at all.
- **A bearing to the host star needs no special case.** Nothing outshines it, so its own glare is
  zero and it is always measured. The rule that would have had to be written down turned out to be
  a consequence.

One thing the wings could not say, and the hard disc had been covering for. Two sources closer
together than one resolution element are **one image**, whatever their contrast: the fainter's
photons land inside the brighter's own image, so there is one measurement to make and not two,
and no exposure and no contrast separates them. The wings start where that ends, so they say
nothing about it, and replacing the disc with them alone made the three stars of
`AuthoredStars::sample` — which sit at *exactly* the same bearing — all separately detectable.
`survey::blended_with` is that rule on its own, named for what it is rather than folded into
glare, and `hud::an_unnamed_star_still_gets_a_label` is what caught it.

**The ceiling is on the calibration, not on the well.** A saturation ceiling was asked for here so
that the host's counts would stop scaling the glare radius. With the wings in place that reason is
gone — the sun's halo 45° out is a fiftieth of one count — but a ceiling is still right, for a
better reason: without one the ship measures its own sun's flux to a part in 1e11. What stops real
photometry is never the photons. It is the flat field, the filter and the gain, and
`instrument::PHOTOMETRY_FLOOR` is a part in a thousand of an **absolute** flux.

Two floors it is deliberately not:

- **Not on a transit deficit.** A deficit is the star measured against itself, so every one of
  those systematics is common to both halves and divides out. `observation::observe` stays photon-
  limited, and differential photometry beating absolute calibration is the whole reason the transit
  search works at all. Applying the floor there was tried, and it put the swarm in
  `one_band_cannot_tell_a_swarm_from_dust_and_two_can` — a deficit of 6.9e-6 — under the noise.
- **Not on a centroid.** Astrometry really does improve with every photon, and has its own floor in
  `astrometry::CENTROID_FLOOR` for its own reasons: the optics and the pointing, not the gain. So
  the host's *position* is as good as it ever was while its *brightness* is honest, which is what
  this section wanted from a saturation ceiling and did not get from one.

Mercury and Venus at inner elongations stay hidden, which is correct and is the same reason they
are hard from Earth.

### Proximity is the instrument

✅ **Built** (2026-09-22). There is no lidar module any more than there is a telescope module:
a ship has one sensor, and what it can do with a body depends on how much of the sky that body
fills. Past `survey::CLOSE_ELEMENTS` — a thousand resolution elements across the disc — the disc
is a map rather than a dot, and three things come for nothing that no amount of watching from
across a system gives:

- **A range.** A bearing with a range is a *position*.
- **A rotation period,** from following features across the disc. The rate, not a whole
  revolution: features move a measurable fraction of the way round in a dwell, so a first close
  look already states a period and nothing has to remember how long a body has been watched.
- **A radius,** which is the angular diameter and the range multiplied, from the one look. Both
  ride on the `Sighting` for that reason: carried apart, one would have to be propagated across
  time while the body moved.

A thousand elements is chosen so that a survey from `START_OFFSET_AU` does **not** get any of it
and a visit does. From 5 AU a ship's telescope puts Jupiter at 630 elements, Saturn at 290, Earth
at 57 and Mars at 30 — discs, all of them, and none of them close. Jupiter has to be approached
inside 3.1 AU, Earth inside 0.29, Mars inside 0.15. From an orbit about any of them it is
millions. **So flying somewhere is worth something,** and that one number is what decides it.

**What ranging does to the orbit fit is the largest part of this.** Ranged looks are positions,
and positions are an orbit outright: no grid, no polish, nothing searched. Three game months of
Saturn — three degrees of arc, refused outright from bearings — becomes the plane to under a
degree, the size to 2% and the period to about a tenth. Two things had to change for that:

- **The plane is the sum of `r_i x r_{i+1}` over every position,** twice the area each step
  sweeps, which points along the orbit's own normal. Not `(r2 - r1) x (r3 - r1)` from three of
  them: on a short arc that cross product is the arc's *curvature*, a part in ten thousand of
  the same magnitudes, and it fell under the degeneracy guard for every three-degree arc —
  exactly the case ranging exists to rescue.
- **A circle where the conic is singular.** `1/r = A + B cos + C sin` needs the arc to bend to
  separate those three, and over a few degrees it does not, whatever the ranges are worth: the
  rows are the same row three times over. So the size survives and the shape does not, and the
  record states its eccentricity as `None` — which rule 4 distinguishes from stating a circle.
  What it calls the axis is then the radius the body is *at*: Saturn at 0.0565 runs from 9.0 to
  10.1 AU, and three degrees near the near end reports 9.0.

**A circle is only an answer when the positions were known,** and finding that out cost a real
defect. Assuming one drops two parameters, so an arc too short to shape a conic can still be
*fitted* by a circle — and from bearings alone that circle can be anywhere, because nothing pins
the range. On the shard, ten hours of bearings on a **moon at 0.0097 AU fitted a circle at 63
AU**, agreed with by every separated start, implying a star of 299 suns and so squeaking past the
mass bound by one. `knowledge::arc` now refuses a circle that nothing ranged.

### The primary is at a focus

✅ **Built** (2026-09-23), and the whole of it follows from one fact the design had not used: a
Keplerian orbit puts its primary **at a focus**. So the candidate that works as a focus *is* the
primary, and the same test finds the star for a planet, the planet for a moon and the moon for a
moon's moon. **There is no moon case in the code.** `Orbit` carries `about: Option<BodyId>`,
`None` being the star, and `fit_orbit` fits against each candidate in turn and keeps whichever
explains the bearings best.

- **Candidates are ranked by the mean bearing,** because the mean of a body's directions over its
  orbit points at what it goes round: seen from outside, a satellite's apparent path is a closed
  loop about its primary and the middle of that loop is the primary. That ranks correctly at both
  levels — a planet's mean bearing points at the star, a moon's at its planet — and the star is
  always tried besides, which is what stops a planet being handed to a neighbor.
- **A satellite's frame moves.** The looks go into the frame of where the primary was *at each
  look's own time*, not now: a moon's planet moves between one look and the next, and a frame
  that ignored that would be fitting the planet's orbit and the moon's at once.
- **Places compose.** `Placed` is still an offset from the **star** even for a moon, because one
  reader should not have to know how deep a body sits. `body_belief` walks the chain and adds
  each step, errors in quadrature, which is why a moon is always placed worse than its planet.
  A cycle guard bounds the walk: two bodies each fitted as the other's primary would otherwise
  recur until the stack ran out.
- **And this is what weighs anything.** A satellite's orbit is its primary's mass by Kepler's
  third law, `mu = 4 pi^2 a^3 / P^2`, and nothing a telescope does measures a mass directly. So
  `BodyBelief::mass_kg` comes from whatever goes round a body and a planet with no moon has none
  — which is the honest answer and the one this document already gives for Venus. The errors come
  in cubed in the axis and squared in the period, so a percent on the axis is three on the mass.

**Three guards turned out to be about stars rather than about orbits,** and each would have
thrown away every satellite there is:

- The plausibility bound ran 0.02 to 200 **AU**; Io's orbit is 0.0028 AU. It is now the band the
  ranges were actually searched in, carried on the fit, which means the same thing at every level.
- The mass bound ran 0.02 to 300 **solar**; Jupiter is 0.00095. Only the ceiling was a real
  statement, since a primary can be as light as a rock, so the floor is gone.
- The range grid ran the whole system, log-spaced. **The geometry hands over a better band**: a
  ray's nearest point to the primary is at `-from . toward`, and how far it misses by there is
  the smallest the orbit can be — so the largest such miss across the arc *is* the orbit's scale.
  That gives 0.003 AU for Io where the old grid stepped 2.7% of a decade, of which Io's entire
  orbit is a twentieth of one step.

  Across the arc and not per ray: a body near opposition has a ray passing almost through its
  primary, and a band built on that one look collapses to a point. And the narrow band is used
  only where the reading is unambiguous, because the scale is a *lower* bound and a weak one when
  a body sits near conjunction — two bodies at 5 and 5.2 AU have nearly the same period and so
  sit in near-permanent conjunction, which measured gives a scale of 0.46 AU for a 5.2 AU orbit.
  The two cases are three orders apart with nothing between them: that Jupiter reads 11 and Io
  about Jupiter reads 3500.

**A survey can resonate with a satellite.** The middle anchor is now whichever look points
furthest from both ends rather than whichever sits in the middle of the list. A survey revisits
on a fixed cadence and a satellite has a short period, so twenty-four looks over exactly two of
Io's orbits put the first and the middle at the *same orbital phase* — two coincident points, and
a conic through them singular however much the arc bends.

**Open: a satellite's orbit from bearings alone at survey range.** The depth along the line of
sight is observable — measured at nine hundred sigma, since the observer's own motion is larger
than the moon's orbit — but its basin is about thirty times narrower than one step of the range
grid, so the search cannot land in it and the near-equal candidates it finds are refused as
rivals. From a close pass the ranges are measured and there is no search at all, so **a planet is
weighed by visiting it**, which is a fair price and of a piece with the rest of this section. The
fix, if it is wanted, is the classic visual-binary one: fit the *projected* ellipse in the plane
of the sky, where the size and the inclination survive and only the depth's sign is lost.

**Not oblateness.** The arena's bodies are spheres, so there is no figure to measure and none is
invented. Same decision as rings for generated planets in phase 5, for the same reason.

**And the velocity is not measured at all.** An orbit and a time *are* a velocity: `placed_at`
throws that half away because only geometry is wanted there, and `body_belief` keeps it. The one
thing needed is the real `mu`, which the orbit states — `n^2 a^3`, Kepler's third law read
backwards, as `knowledge::arc` measures it.

**What one visit measures,** per body. ✅ The *truth* side is **built** (2026-09-22) as
`lc_world::visit`: `Visit { body, toward, range_m, diameter_rad, phase_rad, flux }` for every
body of a system as its light arrives at a point, brightest first. What it settled:

- **Reflected light is `incident_in_band * geometric_albedo * (radius / range)^2`,** which is
  what a geometric albedo is defined against — a flat disc of the body's own radius, no factor
  of pi. Easy to get wrong by four or by pi in either direction, so the test is an independent
  number rather than the formula again: Jupiter comes out at 2.6e-8 W/m² in V from 5 AU, against
  2.7e-8 read off its V of -2.7 at opposition. An earlier pass here was four times low and put
  four times too small a number in the glare table above.
- **The band flux, not the bolometric one.** `survey::flux_from` already gives it. The Sun is
  153 W/m² at 1 AU in V against 1361 bolometric, and using the wrong one is a factor of nine.
- **Thermal emission carries no phase factor,** because it comes off the whole sphere rather
  than the lit crescent. It is the one thing a body still gives at conjunction, and the reason
  the radio band can find a surface under cloud at all.
- **A giant's temperature is its own.** `Drawable::effective_k` already held this from phase 5,
  and Jupiter radiates above what its distance would allow.
- **Rings reflect,** over whatever of their cross-section is turned toward both the star and the
  ship, at their own albedo. The geometry was already worked out in `drawables_at` for the
  renderer's gray path; not carrying it over would have made the one body in Sol with rings the
  one whose brightness is wrong.
- **The join key is `Drawable::name`,** which is the same expression `build_inventory` puts in
  `Target::Body` — so `BodyId::of(star, &visit.body)` names the same body the navigation list
  does. It is *not* the `em-sim` arena id that `worlds` and `rings` are keyed by, which differs
  for Luna among others. Both keys are live and a test asserts every visit matches an inventory
  entry, because the wrong one would silently name nothing.

Still to come is the knowledge side: turning these into sightings under `Subject::Body`.

Per body:

- a bearing, to the centroid precision the disc's brightness allows (`astrometry`);
- the angular diameter, once resolved, and its oblateness;
- the flux in every band the sensor sees: reflected light in B to K, thermal emission in the
  thermal infrared, and radio;
- anything extended about it: rings, and points moving with it, which are its moons.

**What reading those gives,** and how soon in real time:

| learned | how | real time |
|---|---|---|
| existence and position | the first visit | the first second |
| size | angular diameter times distance; the distance comes from the orbit fit | seconds, then as good as the orbit |
| rotation period | the periodogram of its flux: Earth's clouds and continents, Jupiter's bands | Earth and Mars within a minute; Jupiter sooner |
| orbit: period, size, eccentricity, plane | ✅ `lc_world::knowledge::arc`, below | inner planets within a few minutes; **not Saturn**, see below |
| mass | ✅ its moons, by Kepler's third law — and a close pass to range them, see below: Io goes round in 17 real seconds, Callisto in under three minutes, the Moon in four and a half, Titan in under three | minutes, for anything with a moon |
| density, so rock or gas | mass over volume | as soon as both are held |
| albedo and color | flux against the starlight falling on a disc of known size, per band | as soon as the size is held |
| temperature | thermal flux against the temperature it would have with no atmosphere | minutes |
| a surface under cloud | radio: thermal emission from a surface the clouds hide | minutes |

**The orbit fit.** ✅ **Built** (2026-09-22) as `knowledge::arc`. Gauss, then least squares, and
what it came to:

- **The ranges are searched, not the plane.** Crossing each ray with a candidate plane looks like
  the cheap way in and does not work at all: a ship inside a system and the planets it watches
  are within a few degrees of one plane, so every ray lies nearly *in* the candidate plane and
  crosses it nowhere that is not rounding. A plane tells you nothing about a body you are
  coplanar with.
- **Only two numbers are searched.** Guess the range at two of three spread looks; the third is
  closed form, because an orbit's plane contains the star, so `det[r1 r2 r3] = 0` and that
  determinant is linear in the third range. The three positions then give the plane outright, a
  conic with its focus at the star is the linear solve `1/r = A + B cos + C sin`, and regressing
  the times on the mean anomalies is a straight line whose slope is the period.
- **The period is measured and the star's mass falls out of it,** `mu = n^2 a^3`. No mass goes
  in. This is what this section wanted from "the star's mass is itself refined", and it arrives
  earlier than expected — with the first orbit rather than the second. Recovered to a part in
  1000 for the Sun and exactly for a star three times heavier.
- **Least squares is not optional.** A three-point solution passes *exactly* through three noisy
  rays, so it is an interpolation carrying their noise as a systematic: it sits 420 times its own
  noise floor until the six elements are settled against every look.
- **Arc, not noise, is what an orbit costs.** Over 71° the period comes out to 0.075% and
  bearings a hundred times worse change that by nothing, because the error is the fit's own
  convergence. Over 142° it is 8e-8, four orders better, at the noise floor. Nobody should buy a
  better telescope to get a better orbit; they should watch for longer.
- **A short arc gives no orbit, and that is the answer.** Three game months is 0.85% of Saturn's
  orbit, about three degrees, and the separated starts land on 2.7, 5.0, 2.6 and 230 AU with
  residuals within a factor of three of each other. **So the row above is wrong about Saturn**:
  three degrees does not give a percent, it gives four different answers. The fit refuses rather
  than reporting whichever scored best. Saturn needs a longer watch, and the done-when below is
  corrected to say so.
- **The test is rivalry, not the residual.** An earlier rule asked that the best fit sit near the
  bearings' own noise and it threw away the best orbits the fit makes — 284° of arc fits to
  a = 1.0001 with the runner-up five thousand times worse, decisive by any reading, and was
  refused for being three thousand times the noise floor. That is a statement about how far the
  search converged, not about what the arc supports.
- **Two attractors worth naming.** A short arc pulls toward *the ship's own orbit*: put the body
  on top of the observer and the range goes to zero, the parallax with it, and any orbit explains
  the bearings. Three of eight starts landed there, at exactly the 5 AU circle the ship was
  flying. And a finer grid is a *worse* search unless the kept starts are forced apart, because
  every one of the best eight is then a neighbor of the same spurious minimum.
- **`DVec3::angle_between` cannot be used for any of this.** It is an `acos` of a dot product,
  and for an angle of 3e-10 radians that product is `1 - 4.5e-20`, which is exactly 1.0 in f64.
  It returns zero for every bearing this fit tries to resolve, so the objective was blind below
  about 1e-8 radians — thirty times the noise it was meant to be measuring — and every fit
  plateaued there. `atan2` of the cross product keeps its digits all the way down.

One fit costs about 100 ms in a debug build, where this crate's own code is unoptimized, so
`FITS_PER_TICK` is one, the way `READS_PER_TICK` bounds log reading. Round-robin by whose orbit
is oldest, so a system of two hundred comes round in two hundred ticks -- ten real seconds --
which is far faster than any of their orbits change.

**Wired in** (2026-09-22). `Knowledge::looks_at` puts a body's bearings in the frame of where its
star is *believed* to be; `unfitted` picks whose turn it is; `fit_orbit` mints an `Orbit` with
`Method::Astrometric`, which `body_belief` reads back into a `Placed::Known`. Three things that
had to be got right, and one that was not:

- **Believed, not true.** The observer positions are the ship's own and exact; the star's is a
  parallax with its own error, and an error there shifts every look by the same vector and biases
  the orbit. A craft that has not measured its own sun fits nothing, which is the chain this
  document has described from the start, and the reason the survey measures the star every tick.
- **The elements have to mean what the reader means.** `Fitted` keeps its plane in
  `any_orthonormal_vector`'s basis, which nothing else shares; the record keeps an ascending node
  in simulation axes and periapsis measured round from it. A wrong conversion is a body drawn in
  the wrong place and nothing that complains, so the test puts the orbit through `placed_at` and
  checks it lands where the fit says, at eight points round three different orbits.
- **The error bars are the marginal ones,** found by moving each element until the fit is a
  chi-square worse *with the others re-settling*. Held fixed they come out eighty times too
  small, because the period and the axis trade against each other. They are still a few times
  optimistic, and [`spread`] says why: the re-settling is the same pattern search the fit uses
  and stops for the same reason.
- **An orbit has to be an orbit about a star,** and neither the geometry nor the timing says so
  on its own. `1/r = A + B cos + C sin` puts the semi-latus rectum at `1/A`, and three points
  nearly collinear in `(cos, sin)` put `A` near zero: nine hours of a generated system fitted to
  **2.9e16 AU with a plausible 158 day period**. Bounding the axis to the band the ranges were
  searched in moved it to 188 AU and 187 days -- inside the band, and a star of twenty-six
  million suns. So the implied mass is bounded too, at 0.02 to 300 solar. That is not circular
  even though the mass is one of the answers: the range of stars is a fact about stars. Both
  checks live in `residual`, because every candidate is scored there and nothing else is a
  chokepoint -- the 2.9e16 AU orbit *settled* its way out of the band, so checking only where the
  three-point solution lands catches nothing.

A body with no moon has no mass from this. Venus then stays of unknown mass, and its type comes
from everything else.

**What the readings conclude** is a hypothesis set, as the transit search's is: *airless rock*,
*rock with a thin atmosphere*, *rock under a thick atmosphere*, *temperate rock with oceans and
cloud*, *ice giant*, *gas giant*. With its evidence, as for Sol after 15 minutes. Every number
below is a reading of the **authored** Sol table: `Surface::classify` puts Venus, Mars and Titan
in one class and cannot tell them apart, so without that table these six hypotheses collapse to
the three the class already names.

| | size | evidence | leading reading |
|---|---|---|---|
| Venus | 0.95 Earth | albedo 0.7 and gray across B to I; cloud tops near 230 K; **radio from a 700 K surface** | rock under a thick atmosphere |
| Earth | 1.00 | albedo 0.3, blue; a 24-hour rotation with changing cloud; 255 K in the thermal infrared; the Moon gives a mass and a density of 5.5 | temperate rock with oceans and cloud |
| Mars | 0.53 | albedo 0.17, red; temperature near what no atmosphere would give; Phobos gives a density of 3.9 | rock with a thin atmosphere |
| Jupiter | 11.2 | density 1.3 from the Galilean moons; 6.5% oblate; emits 1.7 times what it absorbs; radio | gas giant |
| Saturn | 9.4 | density 0.69 from Titan; rings resolved; 10% oblate; a heat excess | gas giant |

**Room,** which the demo only just survives. A body's measurements are logs like a star's
photometry and count against room, which since the logs-only rebalance is all that room is. There
are `BANDS` = 7 bands and `SAMPLE_BYTES` is 24. Three game months is 2192 game hours, so at one
visit an hour:

| | |
|---|---|
| a starting ship's capacity: `ONBOARD_DATA_BYTES` 1 MiB, plus one data module at `DATA_ANCHOR_S / 1800 * 7 * 24` | 1.05 + 2.95 = **4.0 MB** |
| Sol's eight planets, hourly, three game months | **2.95 MB** |
| the same plus the seven moons the masses need — Luna, Phobos, the Galileans, Titan | **5.5 MB** |

The planets alone fit, with a quarter of the store spare. But the moons every mass in the table
above comes from overflow it half again, and the thirty-minute row of the timeline overflows on
planets alone. So the store is not the constraint at fifteen minutes and is the constraint at
thirty, which is too fine a margin to leave to chance: a done-when that ends in "Data full."
is not a demo.

So the cadence is part of the design, not an afterthought:

- **A body's log is digested every visit**, not when full. One visit is one row: a bearing, an
  angular diameter, seven fluxes. Reading it folds the row into the digest and frees it.
- **The digest is fixed-size per body:** the accumulated orbit fit — a symmetric matrix and a
  vector over the element set with `mu` as a parameter, on the order of a kilobyte — plus per-band
  flux means and variances and the rotation periodogram's bins. Fixed-size means eight planets
  cost the same in month three as in month one, which is the property that makes the fifteen
  minutes work, and whether the accumulated form survives `f64` at all is the open question under
  *Fitting the orbit* below.
- **Raw rows are kept only until the first fit converges,** because initial orbit determination
  needs the raw arc. `retain_raw` is the existing override for a player who wants it kept anyway.
- Reading a body is a small least-squares solve, not a period search over thousands of trials, so
  it gets its own budget per tick, apart from the one read a tick the transit search gets
  (`READS_PER_TICK`).

**Fitting the orbit,** which does not exist anywhere in the code today. The only fits in Rust are
`astrometry::triangulate` and the transit search's box least squares; there is no orbit
determination of any kind. What to build, and where:

- **Initial orbit determination from three bearings,** by Gauss's method. Three lines of sight
  from known observer positions and an assumed `mu` give a state vector, and
  `kepler::state::from_state` turns that into elements. Gauss is the right choice over Laplace
  here because the observer moves very little between visits an hour apart, which is the case
  Laplace's derivative form handles worst.
- **Then batch least squares** over every bearing held, with `mu` as a free parameter once two
  bodies are held — two orbits about one star over-determine its mass. `docs/scratch/fitlib.py`
  is the precedent and nearly the same problem: a Levenberg-damped `gauss_newton` fitting nine
  mean elements, `[n, a, e, i, raan0, argp0, M0, argp_rate, raan_rate]`, against a position series
  and matching `em-sim`'s propagation exactly. `em-sim`'s bundled Solar System elements came out
  of it.
- **Where it lives.** The math goes in `em-foundations`, radians only, beside
  `kepler::state::from_state` — osculating elements from a state vector, which is the natural seed
  — `kepler::semi_major_axis::third_law`, `kepler::angular_motion::mean` and the anomaly solvers.
  Exotic Matters can use an orbit fit as readily as Lightcone can. The records, the digest and the
  per-tick budget go in `lc-world::knowledge`. Note the public third-law functions are named for
  their output: `kepler::third_law` is a private module of shared constants, not a
  function, and there is no `mean_motion` at all.
- **The normal equations are a numerical hazard, and the code already knows it.**
  `astrometry::triangulate` deliberately does *not* solve its 3×3 normal-equation system, and its
  doc says why: the smallest eigenvalue is about 1e-12 of the largest, so inverting it in `f64`
  returns noise, and it regresses transverse position on slope instead. An orbit fit's normal
  matrix is worse conditioned than that one, not better — a short arc leaves `a` and `e` nearly
  degenerate. So accumulating normal equations as the digest cannot be done naively. Either keep
  the digest in a square-root form, as a QR or Cholesky factor updated per visit, which is the
  standard answer and is what keeps the condition number squared out of the stored state; or keep
  a decimated arc of bearings and re-fit. **This is the open question in the room budget**, since
  the whole fixed-size claim rests on the accumulated form being usable.

**Which point is which body.** A visit produces bearings to moving points, and nothing yet says
which detection belongs to which body. This is the linking problem and it is load-bearing: the
"existence and position within the first real second" claim is made or broken here, because a
survey that cannot keep a body's points together has a thousand one-point tracks and no orbits.

Each visit, in order:

1. **Predict** every held body from its current orbit belief to the visit's instant, with the
   belief's sigma grown by how long since it was last seen.
2. **Gate and assign**: a detection within a few sigma of a prediction is that body's. Where two
   predictions compete for one detection, the nearer in normalized distance takes it — a system's
   planets are far apart in the sky compared to their position errors, so the ambiguous case is
   rare and does not need a full assignment algorithm.
3. **Open a candidate track** for anything unassigned. A track with three visits is enough for an
   initial orbit; a track that never gets a second detection expires.
4. **Points that move with a body** rather than with the star are its **moons**, which is the same
   gate run in the body's frame, and is what gives the mass.

Confusion has a real cost and should: two planets that pass close together in the sky can have
their tracks swapped, which puts both orbits wrong until more visits separate them. That is an
observing hazard, not a bug, and the fit's residuals are what reveal it.

**Combining.** A body found by imaging and one found by transit are the same `BodyId`. The records
combine, and an `EdgeOnTo` constraint tightens an imaged pole.

**What the truth has to hold first.** A telescope cannot find what the model lacks. Most of this
is already there, and an earlier draft of this section underestimated it badly. What `Drawable`
(`lc-world/src/system.rs`) carries per body today:

| held | where |
|---|---|
| a class: gas giant, ice giant, ice, rock, weathered, scorched | `surface::Surface::classify`, from radius, mass and equilibrium temperature |
| geometric **and** Bond albedo, per class | `Surface::albedo`, `Surface::bond_albedo` |
| spin axis | `Drawable::pole`, from `em-sim`'s IAU rotations |
| internal heat, as a per-class ratio | `Surface::internal_heat_ratio`, feeding `Surface::effective_temperature`; `effective_k` against `equilibrium_k`, the gray balance |
| rings, with real radii, optical depths and particle albedo | `lc-world/src/rings.rs`, IAU and Cassini values — **Sol only**: `rings::for_body` is keyed on real body names, so no generated body has rings |
| rotation period and pole, for Sol | `em-sim/src/presets.rs`, 48 bodies with `BodyRotation::spinning` or `tidally_locked` |
| eccentricity, for generated planets | `sky/generate.rs:162`, `uniform_in(0.0, 0.12)`; belts have their own |

`em_spectra::Band` already runs `B, V, R, I, K, ThermalIr, Radio`, and the starfield shader
already evaluates every one of them and adds a second, thermal blackbody on top
(`starfield.wgsl:225-230`). So every channel the survey table wants exists and is computed. What
was missing is the four below. **All four are now built** — the star's pole in phase 1 and
the rest in phase 5 — and they are kept here because the reasoning is what a later reader needs:

- **Albedo per band.** There is one geometric albedo per class, and it is collapsed into a single
  `effective_radius_m` — a gray reflector. `effective_radius`' own doc says the consequence: a
  strongly colored body like Mars "comes out the star's color rather than its own." The bands are
  not the gap; per-band reflectance is. "Albedo 0.17, red" needs it, and `Surface::palette` is
  display-only and cannot serve.
- **An atmosphere.** Nothing models one; the only mentions in the codebase are two comments saying
  there isn't one. `Surface::Weathered` is Mars, Venus and Titan together, and the module says
  outright why it cannot do better: "a body's cloud deck is not derivable from its radius, mass
  and temperature, which is the whole basis of this module." Venus is the case it names, 0.76
  Bond albedo where its class gives 0.25. So the survey's Venus result — albedo 0.7, cloud tops
  at 230 K, a 700 K surface under them — **cannot** come from the classifier. It has to be
  authored, which is what the Sol table below is for, and generated systems need an atmosphere
  drawn from mass and insolation rather than inferred from the class.
- **Rotation for generated bodies.** Every one is `rotation: None` (`sky/generate.rs:341, 364,
  374, 395`). Sol has rotation and generated systems have none, so the rotation-period row of the
  survey table works for Sol and finds nothing anywhere else. Generated rings are absent the same
  way, and for the same reason.
- ~~**The star's own spin axis.**~~ **Done in phase 1:** `CatalogueStar::spin_axis`, its
  system's pole tilted up to 12°. It went on `CatalogueStar` rather than on `Star`, which is also
  a template for `Prior::host_like` and has no business carrying an orientation. Nothing reads it
  visibly yet — the star's map sphere is featureless — but it is where surface features will sit,
  and it is no longer `+Z` for every star in the galaxy.

Sol's major planets and large moons get an authored table of per-band reflectance, atmosphere and
what shows at its top. It lives in `lc-world`, beside `rings.rs`, which is already exactly this:
authored real values for Sol with a documented reason for each. A generated system's come from
rules on mass, radius and insolation. The generator also needs moons, where it does not already
make them, or no generated planet has a mass to find.

### Nothing on creation

**A new ship knows literally nothing** (decided 2026-09-22). No charts: not its home system's
planets, and not the stars around it either. Everything it knows it looked at or was told by
another craft. This replaces the charting office of [22-provenance.md](22-provenance.md), which
issued a new ship twenty light-years of star distances and would have issued its home planets.

So the first minutes of a new ship are looking: its own star is a bright bearing with no
distance, and the survey from inside is what turns it into a system.

**The frontier argument this reverses.** 22-provenance argued that "a player who starts with
nothing has no reason to fly anywhere", and issued twenty light-years of charts to put an edge on
the map. The survey from inside is the answer: a ship that knows nothing is not idle, it is in a
system full of unexamined planets that pay out within the first real minute, and the edge of the
map is then wherever its own telescope has reached. The reason to fly is that the next system's
planets need the same fifteen minutes, and somebody else's relayed orbit is worth checking.
Charts made the frontier by drawing a boundary; looking makes it by leaving everything past the
first system dark.

**What a ship knows about itself exactly.** A ship's own inertial state is truth, not knowledge:
dead reckoning is free, and `station()` handing the observatory the craft's true `position_ly`
(`lc-server/src/instruments.rs:104`) stays correct. What is believed is the *star's* position
relative to the ship. Without that split, an orbit fit's observer positions would quietly be
reading the generator, and a fit against known observer positions is the real problem anyway.

**Where charts are issued today,** and what happens to each in phase 6:

✅ **Done** (2026-09-23). Two calls went, being the two a player reaches; the method and the
fixtures stayed, as this table always said they would.

| site | what it is | phase 6 |
|---|---|---|
| `lc-server/src/instruments.rs` | the shard, on a craft's first tick | ✅ **removed.** The one that matters |
| `lc-client/src/app.rs` | the offline client's startup | ✅ **removed**, behind `--charted` |
| `lc-client/src/watch.rs` `Session::issue_charts` | the offline client's method | ✅ **kept.** The row above was the call; nine fixtures and the dev flag are the method |
| `lc-client/src/bin/snapshot.rs:69` | photographs | kept, behind the dev flag |
| `lc-client/examples/crossing.rs:52` | the crossing example | kept |
| `lc-client/tests/knows.rs:19,62` | two integration tests | kept |

Only the first three are paths a player reaches, and only those three go.

`observatory::issue_charts`, `CHARTS` and `CHART_ERROR` stay: tests and photographs need a way to
seed knowledge from truth, and the `CHARTS` witness is what `range.rs` renders as "the charts".
Seven client unit tests across `session.rs`, `action.rs`, `map_source.rs`, `hud.rs` and
`uplink.rs` call it as a fixture, plus two in `observatory.rs` itself, and all of them keep doing
so. What goes away is any call on a path a player reaches. `range.rs`' "Distance on the charts'
word" note goes quiet on its own once nothing issues them.

### From other craft

Body files travel in reports exactly as star files do: a report's entry for a system already
carries its members. A relayed orbit keeps its method and its lineage.

That is not an accident of this design — it is already the shape of the code.
`report::Entry` is documented as "everything one report carries about **one system**: the star and
whatever belongs to it", `report_upto`'s doc says "a planet rides with its star", and `Part`
holds exactly `sightings, claims, names, orbits, conclusions` and no logs. So a system is already
the unit a report is paged by. What is missing is a way for a player to *choose* one.

## Reporting one system on purpose

A player can send everything they know about one system to one craft, or to nobody in particular,
with the aim and the seal they choose.

**Most of this is built.** `Order::SendReport { to, aim, secrecy, idem }` exists, the radio
panel already has a **send survey** button beside its aim row and encrypt checkbox
(`radio_panel.rs:585`), `Server::compose` validates it, `Server::report_for` mints the body, and
`Knowledge::receive` folds it. Sealing already refuses correctly: a sealed broadcast is
`Refusal::Impossible` and a seal to a craft whose key is not held is `Refusal::NoKey`
(`radio.rs:151-158`). What is missing is only that a report has **no scope** — and the picker
that would give it one.

**Today a report is a backlog drain, and its content cannot be chosen.** `Reporting` keeps a
`Mark` per recipient — `{ at_s, after: Option<Subject> }`, keyed by ship id with `0` for the
broadcast — and `report_for` calls `report_upto`, which walks `self.backlog.after(since, sent_s)`
and takes the **oldest** systems first, up to `ENTRIES_PER_REPORT` (64), then moves the mark.
There is no `Subject` anywhere in the chain from `Order::SendReport` through `report_for` to
`report_upto`, so there is nowhere to say "tell Kestrel about Sol". Worse for this purpose, a
craft with nothing new is refused with `NothingNew` — right for a backlog, wrong for a deliberate
send, where the whole point may be that you think they did not hear you the first time.

So a **targeted report** sits beside the backlog rather than inside it:

| | backlog report | targeted report |
|---|---|---|
| what goes | the oldest 64 systems past the mark | one system, everything held about it |
| the mark | moves | **untouched** |
| nothing new | refused, `NothingNew` | sent anyway |
| who asks | a standing order, or a relay | a player, once |

The mark staying put is the part to get right. A mark means "how far through my own learning I
have told you", and a targeted report does not answer that question — it re-sends one system's
worth, most of which the recipient may already hold. Moving the mark would silently drop every
*other* system learned before it, which is the one outcome nobody would connect to the button
they pressed. Folding is idempotent ([22-provenance.md](22-provenance.md#moving-records-between-craft)),
so re-sending costs light and energy and nothing else.

In the code it is three small changes and no new wire shape:

1. **`Order::SendReport` gains `about: Option<Subject>`.** `None` keeps today's backlog behavior,
   so the existing button and any future relay order are unchanged. `lc_proto::Subject` is already
   on the wire — `Order::NameIt` and `Order::RetainRaw` both carry one.
2. **A sibling of `report_upto`**: gather every subject whose `Subject::system()` is this star,
   take each one's whole `Part` rather than the slice after a mark, and return a `Report` of
   exactly one `Entry`. `Entry` is already documented as "everything one report carries about one
   system", so the type does not change.
3. **`report_for` skips `Reporting::sent`** when the order was scoped, and does not map an empty
   result to `NothingNew`.

**The shard still writes it**, as it writes every report, because a client that composed its own
could report anything it liked. And the receive path needs nothing at all: `Knowledge::fold`
iterates parts and keys off `part.subject`, which is already any `Subject` including
`Body { star, body }`, so `Entry::system` is only a grouping key.

**It must fit `REPORT_LIMIT`,** 64 KiB, which is the one place a scoped report can still fail.
`report_for` currently halves its system limit until the JSON fits; a one-system report has
nothing to halve. A system with many bodies and a long tail of sightings can exceed it, so the
scoped path needs its own answer — drop the oldest sightings per body first, since a fitted orbit
makes its own input arcs redundant, and say in the sheet that it was trimmed.

### The three choices, which are already independent

[05-observation.md](05-observation.md#saying-something) settles this and the share sheet only has
to keep it: **addressed to**, **aimed** and **sealed** are three independent choices, and an
interface that conflates them is how a player broadcasts a private message in clear.

All three are already separate fields on the order, so nothing has to be invented:

| choice | field | for a system report |
|---|---|---|
| **addressed to** | `to: Option<ShipId>` | one craft, or `None` for a broadcast. A report is never acknowledged either way, so this decides who may decrypt it and nothing else |
| **aimed** | `aim: Aim` | `Omni`, `Ship(id)`, or `Star(id)`. A beam can miss, and a missed report is light that went past — nothing resends it |
| **sealed** | `secrecy: Secrecy` | `Open` or `Sealed`. Offered only when this ship holds that key, and the control says why when it does not |

Note `Aim::Ship` is refused with `NotInSight` for a craft never seen, and `Aim::Star` beams at a
whole system — the same aim the radio panel's "beam star" already sends, which is the one way to
report to somebody whose position you do not have.

The rules that already exist all apply unchanged, and the send surface is where they become
visible rather than inferred:

- **A broadcast cannot be sealed.** There is nobody to seal it to, and `compose` refuses it with
  `Impossible` rather than quietly sending it in the open.
- **An open report is read by everyone in earshot,** and this is where a report differs sharply
  from a message. A sealed report never lands on an eavesdropper at all — `redact` sets
  `Reported::body` to `None` and the landing is skipped — whereas an eavesdropper on an *open*
  report reads it and folds it into their own knowledge. Broadcasting a survey in clear is not
  leaking that you said something, it is giving away the survey. The surface says so plainly next
  to the seal, because it is the one choice here whose consequence a player cannot see afterwards.
- **A sealed message and a sealed report are hidden differently,** and both are deliberate: a
  message becomes `Body::Unreadable` and shows as fixed-length noise in Overheard, a report simply
  does not arrive. The report needs no length-hiding because it was never going in a transcript.

### What is in it, and what is not

The sheet states the payload before it goes, because a report's size is real: a full survey
is megabytes, a beam has a data rate, and every transmission is to cost stored energy
([23-factions.md](23-factions.md#cost)). Scoping a report to one system is the answer to the
bandwidth question 22-provenance raised and left open.

- **Records, never logs.** `Part` carries sightings, claims, names, orbits and conclusions. A
  craft's photometric samples never travel, which is also why a report cannot overrun the
  recipient's room the way the survey overruns the sender's.
- **The star and every body under it,** as much as is held. Three of eight planets known is three
  planets sent.
- **Unsettled transit candidates go too,** as the conclusions they are, with their probabilities.
  A candidate somebody else can confirm from another angle is worth more than a settled one they
  cannot, since two great circles cross at a pole.
- **The system plane is not sent, and cannot be.** It is a belief recomputed from the orbits and
  never stored. The receiver recomputes it from the orbits they now hold, which is why two craft
  holding the same orbits agree about the plane and why nobody can claim a better plane than
  their own orbits support. A craft that wants to improve somebody's plane sends orbits.
- **Nothing loses its witness.** Every record keeps the observer who measured it and gains a hop,
  so the recipient's Sources section reads *on craft 12's word, relayed once* rather than
  presenting a stranger's astrometry as their own.

### The surface

**It is a share sheet,** the one on every phone, and it should feel like one. A player who has
shared a photo already knows this interaction, and there is nothing about reporting a survey that
needs a new one invented for it.

That metaphor is not decoration; it settles the question the two entry points were raising.
Phones have both directions and so does this, because they are the same surface reached from
either side:

| phone | here | arrives with |
|---|---|---|
| a photo → **Share** → pick a person | the System window (`Y`), or a star in the telescope list | the system fixed, the recipient to pick |
| a conversation → **attach** → pick a photo | a conversation in the communications window (`C`) | the recipient fixed, the system to pick |

What carries over from the phone, and why each part is right here rather than merely familiar:

- **The thing being shared stays named at the top.** A share sheet shows the photo's thumbnail. A
  player about to give away a survey of Sol should be looking at the words *Sol* and what would go
  with it, since that is exactly the fact they might get wrong.
- **The sheet sits over what you were looking at,** rather than replacing it, and what is behind
  it goes quiet — dimmed and not clickable, still there. That is the answer to the modality
  question below, and the metaphor is what settles it.
- **Destinations are a list, and "anyone listening" is a row in it.** A phone lists people and
  AirDrop together; here a broadcast is a row beside the craft rather than a separate mode to
  switch into. The radio panel's party list already has exactly this shape — **Public** pinned
  above the craft, most recently heard from first — so the recipient picker is that list.
- **The common case is one tap, and the options are there for when you want them.** A phone's
  share sheet has the simple path plus an options row you expand. Aim and seal are that row.
- **"Sent" is all you get.** A phone tells you it shared and nothing more; a report is **sent, not
  delivered**, and no acknowledgement is coming. The metaphor is honest about this one where a
  messaging interface would not be.

One place the phone's habit is wrong, and worth naming so nobody copies it: a share sheet's
recipient row is ordered by **who you shared with recently**, and it reshuffles. Doc 13 already
warns about exactly this for the party list — "a list that reorders itself can be misclicked, and
it reorders exactly when a message lands". Ordering the picker by most-recently-heard-from is the
same hazard. Either hold the order still while the sheet is open, or order it by something that
does not move.

**On modality,** since a sheet over a panel raises it. Two rules bear on this and neither forbids
a sheet:

- [13-client-shell.md](13-client-shell.md#states)'s **"nothing is modal"** is an argument about
  *pausing*, not about transient surfaces. Its reason is that the clock never stops, so an
  overlay that blocks the world behind it claims something untrue, and its subject is `Overlay`
  not being an `AppState` — the escape menu, settings and the debug window. A share sheet blocks
  nothing and claims nothing; the rule is satisfied by the clock still running behind it.
- [18-ui-style.md](18-ui-style.md#one-surface-at-a-time)'s **"one surface at a time"** is the rule
  that actually applies, and it is a positive one: when something is modal, what is behind it
  **stands down** — gone, not dimmed and still readable — because two panels of similar size at
  the same place read as one muddled object. The sign-in modal is the worked precedent.

So the sheet is **opaque**, per doc 18's rule that anything over another panel is opaque. On how
far "stands down" goes, the share sheet decides it: **the parent stays drawn, dimmed and not
clickable.** Doc 18's worry is a second thing to read and a second set of buttons to try, and
disabling the parent's controls answers that without removing the very thing the player is
sharing — which is the point of it being visible at all. The sign-in case goes all the way to
*gone* because it is a modal the size of its parent, where two similar surfaces at one place read
as one muddled object; a small sheet over a large panel is not that case, and the phone has been
demonstrating the difference for fifteen years.

In implementation it is a `Panel` arm in `panels::open_panels`, since that is where every surface
in the client is drawn and a second mechanism would be the real cost. Nothing about living there
makes it less of a sheet: it opens with its context filled in, it sits over its parent, and it
closes when the report goes.

Top to bottom: **what is being shared**, the **recipient list**, an expandable **options** row
holding the aim and the seal, and one button that **says what it will do** — *Beam to Kestrel,
sealed*, or *Shout to anyone listening, open* — rather than **Send**. The button naming the act is
what keeps the three choices legible at the moment they take effect even when a player never
opened the options row, which is what makes a one-tap share compatible with
[05-observation.md](05-observation.md#saying-something)'s insistence that the choices stay
independent and visible.

**Defaults, which a share sheet cannot avoid having.** One tap means the options chose themselves,
and one of them decides whether a survey is given away. So the defaults are the **private** ones
and shouting is the deliberate act:

| | default | when |
|---|---|---|
| aim | beam at the recipient | they are in sight. `Aim::Ship` is refused `NotInSight` otherwise, so omni is then the only option and the beam row says why |
| seal | sealed | this ship holds their key. Otherwise open, and the row says it is open and what that means |

That polarity is the one that matches the rest of the design: 05-observation's "first contact is
loud by necessity" makes shouting the thing you do when you must, not the thing that happens
because you did not look. A player can still broadcast a survey in clear — 05-observation is
explicit that the interface should let them make the mistake — but it takes an act, and the button
will have said *Shout to anyone listening, open* before they made it.

Two pieces of the client are reused rather than rebuilt:

- **The recipient picker is the radio panel's party list**, Public pinned above the craft. Today a
  report's addressee is whichever conversation happens to be open, and there is no way to report
  to a craft without opening its conversation first.
- **The system picker is the star list**, and `Ui::selected` is already "one selection, whichever
  view you are looking at" — the telescope list, the sky and the map all send
  `Action::SelectTarget`. Shared from the System window, the system is just the selected one.

The client's own `Aimed` enum — `Omni`, `AtThem`, `AtTheSelectedStar` — is the existing mirror of
`Aim`, encoding "at whatever star the telescope is on" as an interface fact rather than a protocol
one. The options row uses it unchanged. Note `AtTheSelectedStar` is doing double duty once a
system is the thing being shared, and the two selections are not the same: a player can hold
Sol's survey while beaming at Vega, and the row has to say which star it is aiming at rather than
assume it is the one being reported.

**What comes back: nothing, and the surface has to say so.** A report is not a conversation and
writes no transcript row ([22-provenance.md](22-provenance.md#reports-on-the-air)). The sender
gets the shard's acceptance and no more; the recipient gets a notification — today
`"{who}: told you about {n} stars"` (`uplink.rs:629`) — and the knowledge arrives in their next
`Learned`. So the sheet closes on acceptance and leaves no thread to watch, because there is
nothing to watch: a report is **sent, not delivered**, no acknowledgement is coming, and the
amber unacknowledged triangle a message gets would be a lie here. A player who wants to be sure
sends it again. That notification should name the system for a scoped report rather than counting
stars, since "told you about 1 star" is a poor description of a survey of Sol.

## The system's plane

A belief on the **star's** subject, recomputed whenever an orbit changes, never stored:

- **Known** when at least one orbit has a `Known` orientation: the mean of the orbit poles,
  weighted by `1 / sigma^2` and by class, so a giant counts for more. That approximates the
  invariable plane without knowing masses. The error comes from the weighted scatter and the
  individual sigmas.
- **Constrained** when only `EdgeOnTo` orbits are held: a great circle, or two crossing at a
  point.
- **Unknown** otherwise.

### Where longitude starts

There is no vernal equinox, and the design does not borrow one. **Zero longitude is the
ascending node of the system's plane on the galactic plane**, the direction in which the system
plane crosses the galactic plane going north. It is:

- computable by anyone from knowledge alone, so two craft that solved the same system roughly
  agree on it;
- stable as the solution refines: it moves with the plane's error and not with which planet was
  found first;
- degenerate only when the two planes are within about a degree of each other. Then the zero
  falls back to the direction of the galactic center projected into the plane.

Sol's solved zero is therefore not the vernal equinox, and nothing should expect it to be.

Rejected: the periapsis of the first planet found. It changes meaning with discovery order and
differs between craft for no physical reason.

## Beliefs a reader gets

```rust
pub struct BodyBelief {
    pub letter_and_name: ...,           // as for stars
    pub kind: Hypotheses,               // rocky / giant / ... with probabilities
    pub period_s: Option<(f64, f64)>,
    pub semi_major_au: Option<(f64, f64)>,
    pub orientation: Orientation,       // the best held
    pub position_now: Placed,           // Known(point, sigma) | Shell(radius) | Unknown
    pub sources: ...,                   // as range::sources is for stars
}

pub enum SystemPlane { Unknown, Circle(DVec3), Known { pole: DVec3, sigma_rad: f64, zero: DVec3 } }
```

`Placed::Known` needs a full orientation and an epoch. `Shell` is an orbit of known size and
nothing else.

## What reads it

### The System panel

The body list shows **known bodies only**, outward by believed distance, planets lettered.
Transit candidates below `SETTLED` are listed after them, dimmed, with their probability. A system
with nothing known says so.

Beneath the list, a detail section shaped like the telescope's star section:

- the body's names, one per row, then an **Add Name** field;
- **Year:** `3.42 ± 0.01 d`;
- **Distance from star:** `0.041 ± 0.002 AU`;
- **Type:** `giant 94%, rocky 6%`;
- **Orientation:** `known ± 0.4°`, `edge-on to one line of sight`, or `unknown`;
- a small **Sources** section: *3 transits seen by this ship*, *orbit fitted to 14 bearings over
  38 days*, *on craft 12's word*, *relayed once*;
- then the courses, as now, but only the ones the knowledge supports (below).

A header row for the star gives the **system plane**: `solved from 3 orbits, ± 0.6°`, or
`unknown`.

### The map

- Only believed bodies are drawn.
- A body with `Placed::Known` is drawn where it is believed to be, with an **orbit ring in its own
  believed plane**.
- A body with `Shell` is a dashed circle facing the camera at its radius: a sphere seen edge-on,
  saying "somewhere at this distance".
- **Unconfirmed transit candidates are drawn,** in a fainter green than a settled body, at the
  distance their period gives.
- **A distance's error is drawn,** as an error bar running radially from the star along the
  presumed plane: the believed system plane when there is one, otherwise the plane that contains
  the line of sight the transit was seen along. A candidate's bar is usually wide, because its
  period may still be an alias and the star's mass is a prior; it shrinks as the belief does.

What that needs from `em-map`, which has no concept of any of it — grep it for dash, faint or
confidence and nothing comes back:

- **A per-body orbit ring** is a new primitive. `em_map::rings::decades` draws observer-centered
  scale rings only, and `frame.rs` is explicit that rings are measured from the observer and not
  from whatever the camera sits on.
- **Fainter-for-less-certain** needs a variant or a field. `ItemKind` is eight body types —
  `Star, Planet, Moon, Minor, Station, Ship, Population, Observer` — with no confidence on it.
- **The error bar already exists.** `MapItem::spread_ly`, an `Option<(DVec3, DVec3)>` built from
  `Distance::Measured`'s sigma along the line of sight, and already drawn as a segment
  (`map.rs:856`). A body's radial bar reuses it directly.
- **Dashes exist, but in the client.** `map.rs` has a dash ladder, `map.drops[dashes - 1]`, used
  for drop lines, where drifting further off the plane gains more dashes rather than longer ones.
  The spread segment is deliberately the solid member of that same ladder. So a dashed shell
  circle is a client change reusing existing meshes, not new machinery — it is only `em-map` that
  would need a way to ask for it.
- The reference plane option becomes **System plane**, the believed one, with zero longitude as
  above. When the plane is unknown the option is disabled and says why; Galactic is always
  available. `em_map::Plane` gains a **fieldless** `System` variant, with the basis passed in
  rather than held on it — see the wire table for why a basis-carrying variant is not possible.

### Courses

Offered from what is known, not from the generator:

| known | offered |
|---|---|
| full orbit and position | as now: go to it, orbit it equatorially or over the poles once its spin axis is known, otherwise at an altitude in the system plane |
| size only (`Shell`) | orbit the star at the planet's distance, in the ship's current plane of motion or the system plane if known |
| nothing | nothing |

The spin axis of a planet, which is what "equatorial" and "polar" mean, is its own record from
imaging: until it is held, those two options are replaced by "orbit at altitude".

**Courses are flown against believed positions** (decided 2026-09-22). A course aims at where
the crew believes the body will be, from its believed orbit, and arrives off by the belief's
error. That is the mechanic, not a flaw in it:

- While a ship flies, its telescope keeps working, and a better belief **re-plans the course**,
  as a pursuit re-plans against a moving quarry (`chase`). Approaching a planet is itself a
  survey with a shrinking baseline problem: the closer it gets, the better it knows where it is
  going.
- A ship that arrives where a planet was believed to be and finds nothing there is told so, and
  what it holds is only as good as its sources. Somebody else's stated orbit, relayed twice, is
  worth checking before trusting it with a flight.
- **An orbit around a body needs its mass.** Orbital speed at an altitude is set by it. A body
  with no mass held, like Venus without a moon, offers "hold station at altitude" rather than an
  orbit, until a mass is known.
- Crossings between stars follow the same rule. That is the approach of 22-provenance.md's
  *Navigation on beliefs*, and it is scheduled with the rest of this rather than deferred.

**What this costs on the server.** All of it is shard work, because the shard holds the
knowledge. Today:

- A `Course` carries a body **name string** (`Course::Orbit`, `Lagrange`, `Hangout`, `Rings`),
  resolved by `Course::resolve` against the simulation through `system.body_named`. A
  belief-driven course carries a `Subject` instead and is planned from the craft's own knowledge,
  refused with `Refusal::Impossible` when the craft does not `knows` it — which is the gate
  `Order::NameIt` and `Order::RetainRaw` already use (`instruments.rs:342`, `:350`).
- **Crossing between stars is `Order::Cross { star, accel_g, max_beta }`,** not a `Course`, and it
  resolves the catalogue id through `World::star_at` — the *shard's* catalogue, not the craft's
  knowledge. That is the one to gate first: it is the only order that names a place a craft may
  never have seen.
- **Body-relative courses already track moving bodies.** `Course::Orbit` and `Rings` resolve to a
  `Waypoint::Orbit` about `Anchor::Body`, placed every instant by `center_of_at`; `Lagrange` and
  `Hangout` are recomputed the same way. So "fly against a believed position" does not need a new
  tracking mechanism, only a believed center in place of a true one.
- What is missing is **an arbitrary standoff from a moving body.** Every body-relative course is
  an orbit, a ring plane or a libration point, and "hold station at altitude" — what a body with
  no known mass gets — is none of those. `Order::Intercept` holds station alongside a moving
  *craft* and is the closest existing shape.
- **Re-planning** has a precedent in `chase::decide`, which runs per pursuer each tick and gives
  a pursuit up when `chase::sighting` returns nothing. Note what that gate reads: light-delayed
  **truth**, refused as `Refusal::NotInSight`, not knowledge. A belief-driven course needs its own
  cadence and its own outcome for "you arrived and nothing is here".
- The gate matters because a modified client can already fly to truth by name today. The fix is
  that the shard plans the course, so the name never has to be trusted.

## Phases

1. **Fix the plane now, on truth.** ✅ **Built** (2026-09-22). The **Ecliptic** option keeps its
   name and stops hard-wiring `+Z`: the client supplies the local system's true pole, with zero
   longitude at the galactic node. No new `Plane` variant — that is phase 3, when the pole
   becomes a belief and the option is renamed. A stopgap which reads the generator, as the
   System panel already does. What it came to:

   - `CatalogueStar::system_pole` and `::spin_axis`, both derived rather than stored, so the
     `.lcsky` format is untouched. `system_pole` is where Sol's `+Z` is decided; reading
     `generate::pole_for` directly would hand Sol a random plane its JPL-fitted planets are not
     in. `spin_axis` is that pole tilted up to `SPIN_TILT_MAX_RAD`, 12°, uniform over the cap.
   - `LocalSystem` carries both as `pole` and `star_spin`. **They are kept apart on purpose:**
     the plane option is the planets' plane, and the star's map sphere is the star's own spin.
     The Sun's axis is 7.25° off the ecliptic, so a star spinning exactly with its planets would
     be the odd one out.
   - `em_map::Plane` stays the stored selector; `Plane::about(system_pole)` yields a `Datum`
     carrying `(u, v, n)`, which now owns `normal`, `basis`, `height_m`, `intersect`, `bearing`
     and `foot_ly`. The camera and `compose` take a `Datum`, so a plane and a basis that
     disagree is unrepresentable — and phase 3 hands that same seam a believed pole instead of a
     true one, which is the whole reason it is a seam.
   - `MapView` holds the resolved pole beside the camera, for the reason the rest of that struct
     is held together: its angles are measured against this basis, so the two cannot be a frame
     apart. `survey` writes it each frame; `ZERO` between the stars reads as `+Z`.
   - **Changed from the plan:** the spin axis went on `CatalogueStar`, not on `Star`. `Star` is
     also used as a *template* by `Prior::host_like`, and a template has no business carrying an
     orientation. Nothing else about phase 1 moved.
2. **Records only.** ✅ **Built** (2026-09-22). The full `Orbit` with `Orientation` and `Method`,
   `BodyBelief` and `SystemPlane`, and the readers that embedded the old `Orbit` deleted.
   **Charts stay** — see the note on ordering below. What it came to:

   - `Orbit` carries a period and an axis with their own sigmas, because they are not measured
     together. `Orbit::from_period` holds the one relationship every producer would otherwise
     re-derive: a third of the host mass's fractional error, since the period goes as `mu^-1/2`
     and the axis as `mu^1/3`.
   - `formats.rs` went from 410 lines to 47. `FILE_FORMAT` is 5 and `OLDEST_FILE_FORMAT` equals
     it, so an older file is refused loudly rather than read wrong quietly.
   - `knowledge/body.rs` holds both beliefs. `Placed::Known` turned out to be buildable now
     rather than in phase 6: a full orientation plus an epoch propagates through Kepler's
     equation, and the pole's own error carries the body along its ring. Without both it is a
     `Shell`.
   - Two details in the plane fold earn their code. A pole's sign is the direction of travel,
     which an edge-on reading does not settle, so poles are folded onto one half before
     averaging — otherwise one retrograde orbit cancels a prograde one into no plane at all. And
     poles more than `PLANE_SCATTER_LIMIT_RAD` apart report `Unknown` rather than averaging into
     a plane nothing lies in.
   - **Changed from the plan:** nothing in the shape, but `found_planet` now takes an `Orbit` and
     still stamps the witness itself, since it records what *this* craft found.
3. **The panel and the map read beliefs.** 🔶 **Part built** (2026-09-22). Known bodies only,
   candidates in fainter green, shells, distance error bars along the presumed plane, the System
   plane option from belief, and the detail section with sources.

   - ✅ **The plane option.** `em_map::Plane::Ecliptic` became `Plane::System`, *replaced* rather
     than added beside — em-map is used only by `lc-client`, so nothing would have asked for a
     fixed `+Z` frame again, and keeping one for a consumer that does not exist is speculation.
     So `other()` stays a two-way toggle rather than becoming a cycle, which is where this
     departs from the plan. `MapView` carries the `SystemPlane` belief rather than a bare pole,
     because the panel has to say *why* the option is unavailable, and the two ways of not having
     a plane read differently: nothing solved, against a pole known to lie on a circle.
   - ✅ **The System panel.** Reads `bodies_of`, with the detail section, unsettled candidates
     dimmed beneath, and the plane line above. Two lines exist to stop a number reading as more
     than it is: *edge-on to one line of sight*, and a transit's distance marked as resting on a
     prior. Courses still join back to a truth target by hashing generator keys, until phase 7.
     Belts are still the generator's, until phase 8.
   - ✅ **The map.** Draws `bodies_of`. **No new renderer work was needed**, against the
     expectation above: `outline::torus` at a right half-angle already produces a dashed sphere
     outline — the Oort cloud draws one — and its inner and outer radii carry the distance error
     as the shell's own thickness. So the *camera-facing dashed circle* and the *radial error
     bar* below are one existing shape, and an orbit of known size and unknown orientation is
     drawn as the sphere it is. A placed body draws its error **along** the ring rather than
     across it, since what is uncertain is how far round it has got. Keys are the ones truth's
     bodies had, joined through the generator key, or `primary` and the focus would name keys
     nothing draws.
   - ⬜ **Candidates are not drawn.** Their radius needs a period turned through a mass prior,
     and the client builds no `Prior` — adding one to draw faint rings is the wrong trade when
     the panel lists them already. The shard computes that radius at settle, so **sending it** is
     the cheap fix, and it belongs with a wire change rather than with a read.

   **The sky stays truth, and only the map is a chart of knowledge** (decided 2026-09-22). Not
   stated before and load-bearing. **The sky is a camera:** it reflects local truth without
   interpretation, which is what makes it the thing a craft discovers planets *with*.
   `starfield::Bodies` therefore keeps reading `drawables_at`. The map is the other kind of
   surface — a diagram of what has been worked out — and it is the one that draws beliefs. A sky
   filtered by knowledge would be a sky in which nothing could ever be found.
4. **Transits make bodies.** ✅ **Built** (2026-09-22). A settled transit calls `found_planet`,
   the period gives a distance through the mass prior, and the result is `EdgeOnTo`, crossed
   with other craft's. What it came to:

   - `lc-server/src/planets.rs`, because identity is the one part truth must answer and every
     number stays the craft's own. `read_logs` had been discarding the conclusion it got back.
   - `identify` matches within three sigma **or** two percent, whichever is wider: a box
     least-squares period from a long log can have a sigma of minutes, which the search's own
     grid does not deserve to be held to. A system's planets are decades apart in period, so
     nothing else sits inside that window.
   - `BodyId::phantom` is keyed by the witness as well as the star, so two craft's false
     positives never merge while one craft's repeated transits of its own land on one body.
   - `Prior::host_mass` **measures its own error** rather than stating one: the spread of `mu`
     across sampled stars within a factor of two in band luminosity. That error is what carries
     into the distance of every planet found by transit.
   - **Changed from the plan:** the doc wanted the host mass recorded on the `Conclusion` so a
     receiver could reproduce the radius. It is not, and does not need to be — the `Orbit`
     carries the axis itself, so a receiver reads the distance rather than recomputing it. The
     mass would only matter for re-deriving one, which nothing does.
5. **What a body is, in the truth.** ✅ **Built** (2026-09-22). What it came to:

   - `worlds.rs`, in the shape `rings.rs` established: an authored table of sixteen visited
     bodies keyed by the arena's id, each with geometric albedo per band, an atmosphere, what is
     at the top of it, and what it radiates over what it absorbs. Everything unvisited falls
     back to rules on its class, deliberately duller — a generated planet cannot be a Venus,
     because nothing about its radius, mass and temperature says it is one.
   - **What the reflectances are for is the ordering and the color**, as `rings.rs` says of its
     optical depths. Venus bright and nearly gray, Earth darker and blue, Mars darker still and
     red: those three statements are what a survey reads, and they are what the tests pin.
   - Two things the classifier could never have given: radio gets through a deck that stops the
     infrared, which is how a 737 K surface is found under cloud, and a giant carries its own
     heat so its temperature does not follow from its distance.
   - **Rotation** for generated planets: a period log-uniform by class, an obliquity gaussian
     with a heavy tail — six planets under 30° and Uranus at 98° — and retrograde expressed as
     an obliquity past a right angle rather than a negative rate.
   - **Moons**, which is what phase 6 owed the most. A telescope has one route to a planet's
     mass, a satellite through Kepler's third law, so every giant gets at least one and rocky
     planets usually none. They sit inside a third of the Hill radius, and **in the planet's
     equatorial plane** — which makes the obliquity measurable from outside, since the tilt of
     the moons' orbits is the tilt of the planet.
   - `Drawable` carries the resolved `World`, so the survey reads it off the body.
   - **Not done:** rings for generated planets. `rings::for_body` is keyed on real body names,
     so only Sol has any. Nothing in the survey's done-when needs one — Saturn's rings are
     Sol's — so it waits rather than being invented.

6. **Surveying a system from inside,** and charts go away. First the local star: a saturation
   ceiling, a bearing to the host regardless, a glare hole sized for planets, and the ship's
   parallax distance to its own sun, without which no mass prior runs. Then the Survey system
   duty, the visit's measurements, track association, the orbit fit, size, rotation, mass from
   moons, the type hypotheses, and a body's log digested every visit into a fixed-size digest.
   Only once this
   works does a new ship stop being issued charts, on every path listed under *Nothing on
   creation*, together with the photograph flag that replaces them.
   **Done when:** a ship in orbit 5 AU from Sol, surveying for three game months (15 real minutes
   at the design rate), believes Venus, Earth, Mars and Jupiter with periods to 0.1%, radii to
   1%, masses to 1% where a moon gives one, the leading type above
   99% and matching the table above, and the system plane to 0.1°. **Saturn is not in that
   list any more:** three game months is three degrees of its orbit and the fit refuses an arc
   that short, correctly. What it should hold of Saturn by then is a place and a size and no
   orbit, and an orbit once it has watched a good deal longer. Every one of them has a
   position within the first real second. Run it against the in-process shard (`local.rs`), not
   the offline client, which does not read logs. **Let it run to thirty minutes and check the
   ship is still not full** — fifteen minutes on the planets alone fits inside the starting store
   without any digest at all, so the shorter run does not test the cadence, and the moons and the
   thirty-minute mark are what do.
7. **Courses from beliefs.** Options gated by what is known, courses aimed at believed positions
   and re-planned as the belief improves, station-keeping where no mass is held, and crossings
   between stars aimed the same way. This is server work: a course has to carry a subject and be
   planned from the knowledge the shard holds.
8. **The rest of what a body is.** Belt planes from thermal imaging, and spectra finer than the
   bands, if a later instrument adds them.
9. **Reporting one system on purpose.** `about` on the order, the scoped gather beside
   `report_upto`, the mark left alone, the `REPORT_LIMIT` trim, and the share sheet with its three
   choices and both ways in. **Done when:** a craft can send everything it knows about one system
   to a named craft or to nobody, beamed or shouted, sealed or open; the mark to that recipient is
   unchanged afterwards; a scoped send with nothing new is not refused; a broadcast refuses a
   seal; an open one folds into an eavesdropper's knowledge as well as the addressee's; a sealed
   one does not land on the eavesdropper at all; and the recipient's Sources section names the
   sender with a hop.

**On the order of 9.** It is the smallest phase here — most of the machinery is already built and
the scoped path is a parameter, a gather and a picker. It needs only phase 3, since a report
carries records and a player has to see what they are sending. It is numbered last because
nothing depends on it, not because it should wait: it is worth pulling forward as soon as a system
holds anything worth telling somebody about, and phase 4 is the first point where that is a planet
rather than a distance. Sending a *body's* orbit is also what makes phase 4's cooperative
`EdgeOnTo` crossing actually reachable between two players.

**On the order of 2 and 6.** An earlier draft removed charts in phase 2, which would have left a
window of four phases in which a new ship had no star distances, no home planets and no way to
get either, because the survey that replaces charts is not built until 6. Charts are how the game
is playable in the meantime, so they are removed in the same phase as their replacement. The
records of phase 2 do not need them gone.

## Photographs, tests and the dev flags

A ship that knows nothing photographs nothing, and every existing body screenshot goes empty in
phase 6. Two flags break in a way worth naming:

- **`--focus <body>`** (`entry.rs:108`) takes a name straight into `Target::Body(String)`, and
  the inventory's key is the sim body's name falling back to its **generator key**
  (`system.rs:531`). Under beliefs the panel's `inventory().find(|e| &e.target == target)` matches
  nothing.
- **`--station rings:Saturn`** still resolves, because `Course::parse` and `Course::resolve` go
  through truth, but it leaves `ui.focus` on a row the panel no longer lists.

✅ **Built** as `--charted` (2026-09-23), in AGENTS.md's table beside `--at` and `--focus`.
`observatory::issue_charts` is exactly the mechanism, which is the second reason it survives the
phase that stopped calling it: the charting office kept as a dev tool.

**Three server tests were about charts and now say the opposite,** which is the change stated
where it is enforced rather than only here:

- `a_new_craft_starts_with_the_charts_of_where_it_is` is now
  `a_new_craft_knows_nothing_at_all`, and goes on to stare at the star and watch it arrive.
- `a_craft_restored_without_knowledge_is_issued_its_charts` is now
  `..._comes_back_empty_and_still_looking`: what a craft gets back is the telescope pointed
  where it was.
- `a_craft_names_only_what_it_knows` has to *look* at one of its two stars first, since the
  distinction it pins is between what a craft has seen and what it has not, and a craft is now
  handed neither.

And two client tests moved with them. `a_report_crosses_the_seam_and_is_learned_at_the_far_end`
now stares before it names, because a report about a star nobody has looked at is a report about
nothing — and it asks what the receiver *calls* the star rather than reading the first naming in
the file, because a craft that detects a star designates it before anybody names it, so the file
carries two namings from the same witness and only the ranking tells them apart.

Note that `observe_immediately` is not this. Despite the name it only skips the main menu
(`app.rs:401`); its user-facing spelling is `--observe` and it is not in the AGENTS.md table
either.

## Wire and storage changes

Collected, because they are spread across the phases. There is no compatibility to keep — the
game has no players — so each of these is a change in place, not a versioned addition:

| phase | change |
|---|---|
| 1 | ✅ **None.** `CatalogueStar::system_pole` and `::spin_axis` are derived, not stored, so neither the `.lcsky` catalogue nor the checkpoint changed. `SAVE_FORMAT` stays 9 |
| 2 | ✅ **Done.** `Orbit` grew, `Orientation` and `Method` are new, and `formats.rs`' three back-readers `FileV3`/`FileV2`/`FileV1` were deleted rather than repointed at a frozen `OrbitV4`. `FILE_FORMAT` and `OLDEST_FILE_FORMAT` are both 5 |
| 2 | `REPORT_FORMAT` stays 2. It carries the new `Orbit` by carrying `Part`, whose shape is unchanged, and the one shard is redeployed whole — a bump would only drop reports already in flight |
| 3 | `em_map::Plane` gains a fieldless `System` variant; `Plane::other()` becomes a cycle. It is `Copy + Eq + Hash` and a variant carrying a basis would break those derives and the ten `[Ecliptic, Galactic]` iterations. The basis is supplied by the caller through `MapFrame`. Only `lc-client` uses `em-map` |
| 6 | ✅ **`Orbit` grew `about: Option<BodyId>`** — what it goes round, `None` being the star — and `BodyBelief` grew `about` and `mass_kg` beside it. `FILE_FORMAT` is 8 and `REPORT_FORMAT` is 5 |
| 6 | ✅ **`Sighting` grew `range_m` and `spin_s`,** both `Option<(f64, f64)>`, beside `size`: the three things a close look measures and a distant one cannot. `Drawable` grew `spin_s` to have a rotation to measure, worked out from the arena's lock for a tidally locked body. `FILE_FORMAT` is 7 and `REPORT_FORMAT` is 4 |
| 6 | ✅ **`Sighting` grew a `size: Option<(f64, f64)>`** — an angular diameter and its sigma, `None` for a point source. `FILE_FORMAT` was 6 and `REPORT_FORMAT` 3. The report format *does* move here where phase 2 left it alone, because `Part` carries `Sighting` by value and its shape is what changed; `Reported::format` is checked strictly on landing, so reports in flight across the deploy fail to land, which is the right trade for one shard with no players |
| 6 | ✅ **`survey::Source` carries a `Subject` and a `diameter_rad`** rather than a `StarId`. Stars and bodies then live in one sky, which they have to: the host star glares on its own planets and only one list can be asked which of two sources outshines the other. `hidden_by` and `blended_with` return a `Subject`. Not stored and not on the wire — `Source` is built per look from the catalogue and from `visit::sources` |
| 6 | ✅ **`Duty::Survey { star, started_s }`** through every site this row lists, plus `Duty::surveying` and `Duty::visits`, an `Action::SurveySystem` and its button, and a pinned `golden::SURVEYING`. `PROTOCOL_VERSION` stays 36: a new variant appends a discriminant, so every pinned vector above it is byte-identical |
| 6 | the sites that were listed for a new `Duty` variant: the world enum (`survey.rs:306`) and its `target_at`, `slot_at`, `sweep`, `label`; `lc_proto::Duty` (`knowing.rs:52`) and `Duty::is_valid`; both `From` impls (`survey.rs:320`, `:340`); `Observatory::take_up` and `tick`; the `SetDuty` arm in `instruments.rs:322`; the golden vectors (`lib.rs:1232`, `:1251`, `:1359`; `golden.rs:208`, `:220`); and the client's three exhaustive matches in `telescope_panel.rs`, `action.rs` and `session.rs`. `persist.rs` needs no new arm — `SavedInstruments` carries the `Observatory` through serde wholesale — but the serialized shape changes |
| 7 | `Course` carries a `Subject` rather than a body name; `Order::Cross` gains a knowledge gate |
| 9 | `Order::SendReport` gains `about: Option<Subject>`, refused with `Impossible` when the craft does not `knows` that system. One new `Knowledge` method beside `report_upto`. **`REPORT_FORMAT` does not move**: `Report`, `Entry` and `Part` are unchanged, which is the point — and it must not move, because `Reported::format` is checked strictly on landing (`instruments.rs:215`), so a bump would make every report already in flight fail to land |

## Decided

- **A new ship knows nothing,** not even its home system or the stars around it (2026-09-22).
- **Unconfirmed candidates are drawn,** fainter, with distance error bars along the presumed plane
  (2026-09-22).
- **Courses fly against believed positions,** and re-plan as the belief improves (2026-09-22).
- **The sky is a camera and the map is a chart.** The sky reflects local truth without
  interpretation; only the map draws beliefs (2026-09-22).
- **A player can report one system on purpose,** to a craft or to nobody, beamed or shouted,
  sealed or open. A targeted report does not move the recipient's mark and is not refused for
  having nothing new (2026-09-22).
- **The send surface is a phone's share sheet,** deliberately familiar, reached from the system
  or from the conversation, with the parent dimmed behind it and the private options as the
  defaults (2026-09-22).
