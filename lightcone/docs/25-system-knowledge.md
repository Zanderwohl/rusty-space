# What a craft knows about a system

A system's planets, their orbits and the plane they share are **knowledge**, with provenance,
exactly as a star's distance is ([22-provenance.md](22-provenance.md)). Nothing a player sees about
a system reads the generator. This extends the transit search in
[24-standing-instruments.md](24-standing-instruments.md) from "there is probably a planet" to a
body with a name, an orbit and a place in a plane that was itself worked out.

Status: **design**. Nothing below is built except where it says so. Every claim about what exists
was checked against the code on 2026-09-22, and the symbols named are real; where a draft of this
document guessed wrong, the correction is in the text rather than quietly removed, because the
wrong guess was usually "that already exists" about something that does not.

## Where it is today

- The System panel lists `LocalSystem::inventory()`: every body the generator made, with its true
  orbit radius. The map draws the same truth, and courses resolve against it.
- The transit search concludes *a rocky or giant planet on a P-day orbit* about a **star**. No
  body is ever created from it: `Knowledge::found_planet` exists, letters a planet, and has no
  caller outside tests, not one. `Orbit` carries a single element, `semi_major_au`.
- The map's **Ecliptic** plane is `+Z`, the ecliptic of J2000, in every system
  (`em_map::Plane::normal`). Sol's planets lie in it because Sol is fitted against JPL. A
  generated system's planets and belts lie on `generate::pole_for(seed)`, and its star is given
  no spin axis, so the star spins about `+Z`, the map's default. The map is showing the star's
  equator, and the planets are tilted out of it. That is the bug that started this.
- Truth already holds a good deal of what the survey below wants to measure. See *What the truth
  has to hold first*, which lists it against the four things that are genuinely absent.

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
- **A transit `Conclusion` records the host mass it used.** Otherwise a craft receiving a relayed
  period cannot turn it into the same radius the sender did, and two craft would disagree about a
  planet's distance for a reason neither could see.

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

**The duty.** A new duty, **Survey system**, revisits each body it holds about once a game hour,
every eight ticks or so, and spends the rest of its time sweeping the space around the star for
new ones, against the star's glare (`survey::glare_radius_rad` and `survey::hidden_by` exist).

**The local star is in the way, and today it hides the system.** The sweep has no special case for
the craft's host star: `Sky::sources` iterates the whole catalogue and gives each star a flux of
`L / 4πd²`, and at `START_OFFSET_AU` — 5 AU, about 7.9e-5 ly — the host's flux is enormous.
`survey::hidden_by` then drops any source within `glare_radius_rad` of something brighter, which
is `resolution_rad * (SCATTER * bright / faint).sqrt().max(1.0)` — floored at the resolution, so
it can only ever widen the blind spot — with `SCATTER` at 1e-3, whose doc says what that comes to:
"arcseconds around a comparable star, tens of degrees around the local sun."
Nothing clamps brightness anywhere — `survey::look` has a detection floor at `DETECTION_SNR` and
no ceiling, and `Instrument::counts_from_flux` is unbounded above.

So a ship inside a system is currently blind across tens of degrees around exactly the point its
planets orbit. Three things are needed, and they are the first work of the survey duty, not a
detail of it:

- **A saturation ceiling,** so the host star's counts are what a real detector would give rather
  than an unbounded number that scales the glare radius without limit.
- **A bearing to the host star regardless.** It is the one source that must always be measurable,
  because the ship's distance to it is the prior for everything else. A saturated star still gives
  a centroid; it is its flux that is lost, not its position.
- **A glare hole sized for planets, not for stars behind them.** The current radius answers "can I
  see a faint star next to a bright one", where the contrast is astronomical. A planet at 5 AU is
  far brighter than a background star and much closer in, so the useful exclusion is a small inner
  radius plus a floor that falls off with separation, not one hard disc.

Mercury and Venus at inner elongations stay hidden, which is correct and is the same reason they
are hard from Earth. What is not correct is Jupiter being hidden.

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
is actually missing is four things:

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
- **The star's own spin axis.** `Star` is `{radius_m, teff_k, mu, limb_darkening}` and has no
  pole, which is the bug at the top of this document. Truth already holds the system's plane, as
  `GeneratedSystem::pole` (`sky/generate.rs:52`), so what phase 1 needs is for a star to spin
  about its system's pole rather than about `+Z` by default.

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

| site | what it is | phase 6 |
|---|---|---|
| `lc-server/src/instruments.rs:117` | the shard, on a craft's first tick | **removed.** The one that matters |
| `lc-client/src/watch.rs:36` `Session::issue_charts` | the offline client's own call | **removed** |
| `lc-client/src/app.rs:510` | the offline client's startup | **removed** |
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
makes its own input arcs redundant, and say in the send window that it was trimmed.

### The three choices, which are already independent

[05-observation.md](05-observation.md#saying-something) settles this and the send window only has
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

The send window states the payload before it goes, because a report's size is real: a full survey
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

**One surface, two ways in,** differing only in which field arrives filled. Two send dialogs that
drift apart is the failure worth designing against.

| from | arrives with |
|---|---|
| the System window (`Y`), or a star in the telescope list | the system fixed, the recipient to pick |
| a conversation in the communications window (`C`) | the recipient fixed, the system to pick |

**It is a window, not a modal, and that is a constraint rather than a preference.** The client
says outright in three places that nothing is modal (`ui.rs:6`, `app.rs:25`, `action.rs:896` —
"opening one does not close another"), and there is no popup, popover or `egui::Modal` anywhere
in `lc-client`. The only true modal in the codebase is the sign-in password panel, which is Bevy
UI in the menu and not in the game at all. So this is a new arm of `Panel` in
`panels::open_panels`, drawn in the same shared `egui::Window` every other panel uses, and it
opens and closes like one. It behaves like a popover in the ways that matter — it opens with its
context filled in and it closes when the report goes — without inventing a second kind of
surface.

It is **opaque**, because doc 18's rule is that anything over another panel is opaque: two
translucent surfaces at the same place blend into one object with both sets of text. Not being
modal is also right on its own terms — the clock never stops, and a player deciding who to tell
about Sol should still be able to watch the sky.

Its fields, top to bottom: the system and what would go, the recipient, the aim, the seal, and one
button that **says what it will do** — *Beam to Kestrel, sealed*, or *Shout to anyone listening,
open* — rather than **Send**. The button naming the act is what makes the three independent
choices legible at the moment they take effect, which is the entire reason for keeping them
independent, and it is where a player catches themselves about to give a survey away.

Two pieces of the client can be reused rather than rebuilt:

- **The recipient picker is the channel list.** Today a report's addressee is whichever
  conversation is open, and there is no way to report to a craft without opening its conversation.
  The same party list the radio panel draws is the picker.
- **The system picker is the star list**, and `Ui::selected` is already "one selection, whichever
  view you are looking at" — the telescope list, the sky and the map all send
  `Action::SelectTarget`. Opening this from the System window means the system is simply the
  selected one.

The client's own `Aimed` enum — `Omni`, `AtThem`, `AtTheSelectedStar` — is the existing mirror of
`Aim` and already encodes "at whatever star the telescope is on" as an interface fact rather than
a protocol one. It is what the aim row should use here too.

**What comes back: nothing, and the surface has to say so.** A report is not a conversation and
writes no transcript row ([22-provenance.md](22-provenance.md#reports-on-the-air)). The sender
gets the shard's acceptance and no more; the recipient gets a notification — today
`"{who}: told you about {n} stars"` (`uplink.rs:629`) — and the knowledge arrives in their next
`Learned`. So the window closes on acceptance and leaves no thread to watch, because there is
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

1. **Fix the plane now, on truth.** `Star` gains a spin axis, and a generated star spins about
   its system's pole give or take a few degrees; truth already holds that pole as
   `GeneratedSystem::pole`. The existing **Ecliptic** option keeps its name and stops hard-wiring
   `+Z`: the client supplies the local system's true pole, with zero longitude at the galactic
   node. No new `Plane` variant — that is phase 3, when the pole becomes a belief and the option
   is renamed. A stopgap which reads the generator, as the System panel already does.
2. **Records only.** The full `Orbit` with `Orientation` and `Method`, `BodyBelief` and
   `SystemPlane`, and the readers that embedded the old `Orbit` deleted. **Charts stay** — see
   the note on ordering below.
3. **The panel and the map read beliefs.** Known bodies only, candidates in fainter green,
   shells, distance error bars along the presumed plane, the System plane option from belief, and
   the detail section with sources. `em_map::Plane` gains a fieldless `System` variant, and
   `Plane::other()` becomes a cycle.
4. **Transits make bodies.** A settled transit calls `found_planet`, the period gives a distance
   through the mass prior, and the result is `EdgeOnTo`, crossed with other craft's.
5. **What a body is, in the truth.** Three of the four absent things, the star's own pole being
   phase 1's: reflectance per band, an atmosphere and what shows at the top of it, and rotation
   for generated bodies. Then the authored Sol table, which lives in `lc-world` beside `rings.rs`,
   generator rules for everything else, rings for generated planets — `rings::for_body` is keyed
   on real body names, so today only Sol has any — and moons for generated planets, without which
   no generated planet has a mass to find. Internal heat, Sol's rotations and body spin axes are
   already held.
6. **Surveying a system from inside,** and charts go away. First the local star: a saturation
   ceiling, a bearing to the host regardless, a glare hole sized for planets, and the ship's
   parallax distance to its own sun, without which no mass prior runs. Then the Survey system
   duty, the visit's measurements, track association, the orbit fit, size, rotation, mass from
   moons, the type hypotheses, and a body's log digested every visit into a fixed-size digest.
   Only once this
   works does a new ship stop being issued charts, on every path listed under *Nothing on
   creation*, together with the photograph flag that replaces them.
   **Done when:** a ship parked 5 AU from Sol, surveying for three game months (15 real minutes
   at the design rate), believes Venus, Earth, Mars, Jupiter and Saturn with periods to 0.1%
   (Saturn's to 1%), radii to 1%, masses to 1% where a moon gives one, the leading type above
   99% and matching the table above, and the system plane to 0.1°. Every one of them has a
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
   `report_upto`, the mark left alone, the `REPORT_LIMIT` trim, and the send window with its three
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

So phase 6 needs a dev flag that seeds a craft's knowledge from truth, and the mechanism already
exists: `observatory::issue_charts` is exactly that, which is the second reason it survives the
phase that stops calling it on player paths. The flag is the charting office kept as a dev tool.
It goes in AGENTS.md's table under **Checking your work**, beside `--at`, `--station` and
`--focus`.

Note that `observe_immediately` is not this. Despite the name it only skips the main menu
(`app.rs:401`); its user-facing spelling is `--observe` and it is not in the AGENTS.md table
either.

## Wire and storage changes

Collected, because they are spread across the phases. There is no compatibility to keep — the
game has no players — so each of these is a change in place, not a versioned addition:

| phase | change |
|---|---|
| 1 | `Star` gains a spin axis. Save shape changes; `SAVE_FORMAT` is 9 today |
| 2 | `Orbit` grows, `Orientation` and `Method` are new, and `formats.rs`' three back-readers `FileV3`/`FileV2`/`FileV1` are deleted rather than repointed at a frozen `OrbitV4`. `FILE_FORMAT` and `OLDEST_FILE_FORMAT` both become the new number |
| 2 | `REPORT_FORMAT`, 2 today (`radio.rs:115`), carries the new `Orbit` |
| 3 | `em_map::Plane` gains a fieldless `System` variant; `Plane::other()` becomes a cycle. It is `Copy + Eq + Hash` and a variant carrying a basis would break those derives and the ten `[Ecliptic, Galactic]` iterations. The basis is supplied by the caller through `MapFrame`. Only `lc-client` uses `em-map` |
| 6 | a new `Duty` variant: the world enum (`survey.rs:306`) and its `target_at`, `slot_at`, `sweep`, `label`; `lc_proto::Duty` (`knowing.rs:52`) and `Duty::is_valid`; both `From` impls (`survey.rs:320`, `:340`); `Observatory::take_up` and `tick`; the `SetDuty` arm in `instruments.rs:322`; the golden vectors (`lib.rs:1232`, `:1251`, `:1359`; `golden.rs:208`, `:220`); and the client's three exhaustive matches in `telescope_panel.rs`, `action.rs` and `session.rs`. `persist.rs` needs no new arm — `SavedInstruments` carries the `Observatory` through serde wholesale — but the serialized shape changes |
| 7 | `Course` carries a `Subject` rather than a body name; `Order::Cross` gains a knowledge gate |
| 9 | `Order::SendReport` gains `about: Option<Subject>`, refused with `Impossible` when the craft does not `knows` that system. One new `Knowledge` method beside `report_upto`. **`REPORT_FORMAT` does not move**: `Report`, `Entry` and `Part` are unchanged, which is the point — and it must not move, because `Reported::format` is checked strictly on landing (`instruments.rs:215`), so a bump would make every report already in flight fail to land |

## Decided

- **A new ship knows nothing,** not even its home system or the stars around it (2026-09-22).
- **Unconfirmed candidates are drawn,** fainter, with distance error bars along the presumed plane
  (2026-09-22).
- **Courses fly against believed positions,** and re-plan as the belief improves (2026-09-22).
- **A player can report one system on purpose,** to a craft or to nobody, beamed or shouted,
  sealed or open, from one send window with two ways into it. A targeted report does not move the
  recipient's mark and is not refused for having nothing new (2026-09-22).
