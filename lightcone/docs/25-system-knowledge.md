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
| `Naming` | as now: a planet letter from `found_planet`, and any given name | the crew, the charts |
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
    pub method: Method,                  // Transit, Astrometric, Claim
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

### From inside: imaging and orbit determination

Inside a system, planets are bright points near the star. A new duty, **Survey system**, images
the space around the star: detection against the star's glare (`survey::glare` exists), and a
bearing and reflected flux per body per pass.

- The ship's own motion gives parallax. Over an arc of the planet's orbit, triangulated positions
  fit a Keplerian orbit: period, size, eccentricity, and the full orientation. The fit tightens
  with the arc, so an outer planet takes longer to pin down than an inner one, and the panel
  says so.
- Moons are found the same way around a resolved planet, and belts from thermal imaging: the
  plane of a disc is its orientation.
- A body found by imaging and one found by transit are the same `BodyId`. The records combine,
  and an `EdgeOnTo` constraint tightens an astrometric pole.

### On somebody's word: the charts

A new ship is issued the charts of the volume it launched from (22-provenance.md). Its **home
system** comes with them: the major planets' orbits as `Claim`-method `Orbit` records from the
charting office, one hop, errors of a few percent on size and a degree or so on orientation.
Minor bodies are not charted. So a ship starting at Sol knows its planets from day one, as
something it was told, and can check them with its own telescope.

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

The body list shows **known bodies only**, outward by believed distance, planets lettered. A
transit candidate below `SETTLED` does not appear. A system with nothing known says so.

Beneath the list, a detail section shaped like the telescope's star section:

- the body's names, one per row, then an **Add Name** field;
- **Year:** `3.42 ± 0.01 d`;
- **Distance from star:** `0.041 ± 0.002 AU`;
- **Type:** `giant 94%, rocky 6%`;
- **Orientation:** `known ± 0.4°`, `edge-on to one line of sight`, or `unknown`;
- a small **Sources** section: *3 transits seen by this ship*, *orbit fitted to 14 bearings over
  38 days*, *on the charts' word*, *relayed once*;
- then the courses, as now, but only the ones the knowledge supports (below).

A header row for the star gives the **system plane**: `solved from 3 orbits, ± 0.6°`, or
`unknown`.

### The map

- Only believed bodies are drawn.
- A body with `Placed::Known` is drawn where it is believed to be, and its orbit ring lies in its
  own believed plane.
- A body with `Shell` is a dashed circle facing the camera at its radius: a sphere seen edge-on,
  saying "somewhere at this distance".
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

Flying still happens against the true system once a course is chosen, as a crossing to a star
still aims at the catalogue position (22-provenance.md, *Navigation on beliefs*). What changes is
which courses a player is offered. Aiming at the believed position and finding the planet is not
quite there is the same deferred mechanic as for stars.

## Phases

1. **Fix the plane now, on truth.** Generated stars spin about their system's pole, give or take
   a few degrees. The map's plane option uses the system's true pole, with zero longitude at the
   galactic node. That is a stopgap which reads the generator, as the System panel already does,
   and it is replaced in phase 3.
2. **Records and beliefs.** The full `Orbit` with `Orientation` and `Method`, knowledge format 5,
   `BodyBelief`, `SystemPlane`, and charts that issue the home system's planets.
3. **The panel and the map read beliefs.** Known bodies only, shells, the System plane option
   from belief, and the detail section with sources.
4. **Transits make bodies.** A settled transit calls `found_planet`, the period gives a distance
   through the mass prior, and the result is `EdgeOnTo`, crossed with other craft's.
5. **Imaging inside a system.** The Survey system duty, reflected-light detection, orbit
   determination, and spin axes.
6. **Courses from beliefs.** Options gated by what is known, and orbits of unknown orientation.
7. **More of what a body is.** Albedo and temperature from reflected and thermal flux, moons,
   and belt planes.

## Open

- **Home system on the charts:** planets and major moons as claims (recommended), or nothing, so
  a new ship has to find even its own neighbors.
- **Unconfirmed candidates:** hidden until settled (recommended), or drawn as a shell marked
  unconfirmed.
- **Flying on truth:** courses offered from belief but flown against truth for now
  (recommended), or aimed at the belief with its error, which is a mechanic of its own.
