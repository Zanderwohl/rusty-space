# What a craft knows about a system

A system's planets, their orbits and the plane they share are **knowledge**, with provenance,
exactly as a star's distance is ([22-provenance.md](22-provenance.md)). Nothing a player sees about
a system reads the generator. This extends the transit search in
[24-standing-instruments.md](24-standing-instruments.md) from "there is probably a planet" to a
body with a name, an orbit and a place in a plane that was itself worked out.

Status: **design**. Nothing below is built except where it says so.

## Where it is today

- The System panel lists `LocalSystem::inventory()`: every body the generator made, with its true
  orbit radius. The map draws the same truth, and courses resolve against it.
- The transit search concludes *a rocky or giant planet on a P-day orbit* about a **star**. No
  body is ever created from it: `Knowledge::found_planet` exists, letters a planet, and has no
  caller outside tests. `Orbit` holds only `semi_major_au`.
- The map's **Ecliptic** plane is `+Z`, the ecliptic of J2000, in every system
  (`em_map::Plane::normal`). Sol's planets lie in it because Sol is fitted against JPL. A
  generated system's planets and belts lie on `generate::pole_for(seed)`, and its star is given
  no spin axis, so the star spins about `+Z`, the map's default. The map is showing the star's
  equator, and the planets are tilted out of it. That is the bug that started this.

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
never reaches a player.

| record | holds | from |
|---|---|---|
| `Sighting` | a bearing to the body and its reflected flux, as for a star | imaging inside the system |
| `Orbit` | elements with errors, and how much of the orientation is known (below) | a transit fit, an astrometric fit, or a claim |
| `Naming` | as now: a planet letter from `found_planet`, and any given name | the crew, other craft |
| `Conclusion` | what kind of body: rocky or giant, and later temperature and albedo | transit depth, reflected and thermal flux |

`Orbit` grows from one number to what a fit actually yields:

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
    Known { pole: DVec3, sigma_rad: f64, node: f64, periapsis: f64 },
}
```

The format bump is knowledge format 5, with a reader for the old one-number `Orbit`, and report
format 3.

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
in the sky after the star, and a four-meter mirror resolves them outright. From 5 AU, Venus is a
disc of about 3 arcseconds and Jupiter about 40, against a diffraction limit of 0.035. So this is
not the transit search's statistics at the noise floor. It is looking.

**The duty.** A new duty, **Survey system**, revisits each body it holds about once a game hour,
every eight ticks or so, and spends the rest of its time sweeping the space around the star for
new ones, against the star's glare (`survey::glare` exists).

**What one visit measures,** per body:

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
| orbit: period, size, eccentricity, plane, velocity | an angles-only fit (Gauss, then least squares) with the star's gravity; the star's mass is itself refined once two orbits are held | inner planets within a few minutes; Saturn's 3° arc to about a percent by 15 |
| mass | its moons, by Kepler's third law: Io goes round in 17 real seconds, Callisto in under three minutes, the Moon in four and a half, Titan in under three | minutes, for anything with a moon |
| density, so rock or gas | mass over volume | as soon as both are held |
| albedo and color | flux against the starlight falling on a disc of known size, per band | as soon as the size is held |
| temperature | thermal flux against the temperature it would have with no atmosphere | minutes |
| a surface under cloud | radio: thermal emission from a surface the clouds hide | minutes |

A body with no moon has no mass from this. Venus then stays of unknown mass, and its type comes
from everything else.

**What the readings conclude** is a hypothesis set, as the transit search's is: *airless rock*,
*rock with a thin atmosphere*, *rock under a thick atmosphere*, *temperate rock with oceans and
cloud*, *ice giant*, *gas giant*. With its evidence, as for Sol after 15 minutes:

| | size | evidence | leading reading |
|---|---|---|---|
| Venus | 0.95 Earth | albedo 0.7 and gray across B to I; cloud tops near 230 K; **radio from a 700 K surface** | rock under a thick atmosphere |
| Earth | 1.00 | albedo 0.3, blue; a 24-hour rotation with changing cloud; 255 K in the thermal infrared; the Moon gives a mass and a density of 5.5 | temperate rock with oceans and cloud |
| Mars | 0.53 | albedo 0.17, red; temperature near what no atmosphere would give; Phobos gives a density of 3.9 | rock with a thin atmosphere |
| Jupiter | 11.2 | density 1.3 from the Galilean moons; 6.5% oblate; emits 1.7 times what it absorbs; radio | gas giant |
| Saturn | 9.4 | density 0.69 from Titan; rings resolved; 10% oblate; a heat excess | gas giant |

**Room.** A body's measurements are logs like a star's photometry and count against room. A
visit an hour in eight bands, for eight planets, is about 3 MB in three months, which is the
starting ship's whole store. So a body's log is read and consumed often, into a digest that keeps
what the fit needs: the least-squares normal equations for its orbit, per-band flux means, and
the rotation periodogram's bins. Reading a body is a small least-squares problem, not a period
search over thousands of trials, so it has its own budget per tick, apart from the one read a
tick the transit search gets.

**Combining.** A body found by imaging and one found by transit are the same `BodyId`. The records
combine, and an `EdgeOnTo` constraint tightens an imaged pole.

**What the truth has to hold first.** A telescope cannot find what the model lacks. Today a body
has a mass, a radius, an orbit and, for Sol, a display color. Each body needs:

- a geometric albedo per band;
- an atmosphere (none, thin, thick, or envelope) and what shows at the top of it (rock, ice,
  ocean, cloud);
- a rotation period and axis;
- internal heat;
- rings, with their radii and optical depth.

Sol's major planets and large moons get an authored table of their real values. A generated
system's come from rules on mass, radius and the starlight falling on it. The generator also
needs moons, where it does not already make them, or no generated planet has a mass to find.

### Nothing on creation

**A new ship knows literally nothing** (decided 2026-09-22). No charts: not its home system's
planets, and not the stars around it either. Everything it knows it looked at or was told by
another craft. This replaces the charting office of [22-provenance.md](22-provenance.md), which
issued a new ship twenty light-years of star distances and would have issued its home planets.

So the first minutes of a new ship are looking: its own star is a bright bearing with no
distance, and the survey from inside is what turns it into a system.

### From other craft

Body files travel in reports exactly as star files do: a report's entry for a system already
carries its members. A relayed orbit keeps its method and its lineage.

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
- A body with `Placed::Known` is drawn where it is believed to be, and its orbit ring lies in its
  own believed plane.
- A body with `Shell` is a dashed circle facing the camera at its radius: a sphere seen edge-on,
  saying "somewhere at this distance".
- **Unconfirmed transit candidates are drawn,** in a fainter green than a settled body, at the
  distance their period gives.
- **A distance's error is drawn,** as an error bar running radially from the star along the
  presumed plane: the believed system plane when there is one, otherwise the plane that contains
  the line of sight the transit was seen along. A candidate's bar is usually wide, because its
  period may still be an alias and the star's mass is a prior; it shrinks as the belief does.
- The reference plane option becomes **System plane**, the believed one, with zero longitude as
  above. When the plane is unknown the option is disabled and says why; Galactic is always
  available. `em_map::Plane` gains a variant carrying a basis rather than hard-wiring `+Z`. Only
  `lc-client` uses `em-map`.

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

## Phases

1. **Fix the plane now, on truth.** Generated stars spin about their system's pole, give or take
   a few degrees. The map's plane option uses the system's true pole, with zero longitude at the
   galactic node. That is a stopgap which reads the generator, as the System panel already does,
   and it is replaced in phase 3.
2. **Records and beliefs.** The full `Orbit` with `Orientation` and `Method`, knowledge format 5,
   `BodyBelief` and `SystemPlane`. New ships stop being issued charts.
3. **The panel and the map read beliefs.** Known bodies only, candidates in fainter green,
   shells, distance error bars along the presumed plane, the System plane option from belief, and
   the detail section with sources.
4. **Transits make bodies.** A settled transit calls `found_planet`, the period gives a distance
   through the mass prior, and the result is `EdgeOnTo`, crossed with other craft's.
5. **What a body is, in the truth.** Albedo per band, atmosphere, rotation, internal heat, rings,
   an authored table for Sol, generator rules for everything else, and moons for generated
   planets.
6. **Surveying a system from inside.** The Survey system duty, the visit's measurements, the
   orbit fit, size, rotation, mass from moons, the type hypotheses, and body logs digested often.
   **Done when:** a ship parked 5 AU from Sol, surveying for three game months (15 real minutes
   at the design rate), believes Venus, Earth, Mars, Jupiter and Saturn with periods to 0.1%
   (Saturn's to 1%), radii to 1%, masses to 1% where a moon gives one, the leading type above
   99% and matching the table above, and the system plane to 0.1°. Every one of them has a
   position within the first real second.
7. **Courses from beliefs.** Options gated by what is known, courses aimed at believed positions
   and re-planned as the belief improves, station-keeping where no mass is held, and crossings
   between stars aimed the same way.
8. **The rest of what a body is.** Belt planes from thermal imaging, and spectra finer than the
   bands, if a later instrument adds them.

## Decided

- **A new ship knows nothing,** not even its home system or the stars around it (2026-09-22).
- **Unconfirmed candidates are drawn,** fainter, with distance error bars along the presumed plane
  (2026-09-22).
- **Courses fly against believed positions,** and re-plan as the belief improves (2026-09-22).
