# Stellar photometry: the emission shell

## The model

A star is surrounded by a sphere at the system's shell radius. Every direction on that
sphere carries a radiance: the flux per unit solid angle that leaves the system in that
direction, in each of a few spectral bands.

```
L(n, t) = L_star(t) * (1 - D_cloud(n)) * product over discrete occluders of (1 - f_i(n, t))
```

| term | meaning |
|---|---|
| `n` | unit direction out of the system |
| `L_star(t)` | intrinsic output: base luminosity, rotational modulation, flares, pulsation |
| `D_cloud(n)` | statistical deficit from populations, baked into the shell, plus its flicker |
| `f_i(n, t)` | fraction of the stellar disc discrete occluder `i` covers, evaluated exactly |

An observer at distance `d` in direction `n` receives `L(n, t_r) / d^2` at time
`t = t_r + d`, where `t_r` is the retarded time from [01-spacetime.md](01-spacetime.md).

## What the shell can and cannot hold

The shell is a **statistically steady, time-averaged** structure. A vertex value is a
property of the direction, not of the moment.

That is the only line that matters, and it is about time, not about precision or angular
resolution:

- A **population** — a swarm, a belt, a dust cloud — is steady. Its elements are uniformly
  distributed in node, argument and phase, so the expected occultation from a given
  direction does not change as the elements orbit. Individual elements come and go; the
  distribution does not. It bakes exactly, and the residual scatter bakes too, as a
  variance.
- A **single body** — a planet, a moon, a station — is not steady. Its signal at a fixed
  direction is a coherent periodic box: absent for a year, 8.4e-5 deep for thirteen hours,
  absent again. A time-averaged shell records its duty-cycle average of about 1.3e-7 and
  throws away the period, the phase, the depth and the ingress shape, which is all of the
  information. Capturing it would need one shell per time sample.

So the split is by **coherence**, not by object type or by size. Anything whose signature is
a coherent time variation at a fixed direction is evaluated analytically. Anything whose
signature is stationary noise is baked.

### Precision

Store the **deficit**, not the transmittance, and store it in `f32`.

`f32` has a relative precision of 1.2e-7. `1 - 1e-8` rounds to exactly `1.0`, so a shell
holding transmittance silently discards every dip below 1e-7. The same shell holding the
deficit `1e-8` keeps seven significant digits of it, because `f32`'s exponent range is
irrelevant to the question — only its 24-bit mantissa is, and a small number spends all of
it on the small number.

`f16` is not an option: its smallest normal is 6.1e-5 and its relative precision is 1e-3, so
a 1e-8 deficit underflows to zero. Earlier drafts of this document assumed `f16`; that was
wrong, and the 5 MB-per-star figure it produced was answering a question about angular
resolution that does not arise once populations are treated statistically.

## Path 1: discrete occluders, analytic

Planets, moons, individual large structures. Evaluate `f_i(n, t_r)` directly.

```
1. propagate occluder i to t_r            (em_sim, Keplerian, closed form)
2. project its centre onto the plane normal to n
3. b = impact parameter, in units of R_star
4. f_i = overlap area of two discs, weighted by a limb-darkening profile
```

One Kepler solve per occluder per sample. A system with 20 discrete occluders costs 20
solves per observation, and the direction is the one actually being observed — the full
sphere is never built.

Disc overlap with quadratic limb darkening has a closed form (Mandel & Agol 2002); use it,
because it is standard, tested, and its coefficients are tabulated by spectral type.

The angular half-width of a transit signature is `sin(theta) = (R_star + R_body) / a`:

| occluder around a Sun-like star | half-width | depth |
|---|---|---|
| Mercury at 0.39 AU | 0.69 deg | 1.2e-5 |
| Earth at 1 AU | 0.27 deg | 8.4e-5 |
| Jupiter at 5.2 AU | 0.056 deg | 1.1e-2 |

These are narrow, but that is not why they are analytic. They are analytic because they are
coherent.

## Path 2: populations, statistical

A population is a distribution over orbital elements, uniform in longitude of ascending
node, argument of periapsis and mean anomaly. Those three angles are not stored per element;
they are exactly what makes the population steady, and integrating over them turns "which
element is in front of the star right now" into a probability.

What is stored per population:

| parameter | meaning |
|---|---|
| `p(a)` | semi-major axis distribution, usually narrow |
| `p(e)` | eccentricity distribution |
| `p(i)` | inclination distribution about the population's own pole |
| pole direction | the population's reference plane |
| `N`, `sigma` | element count and mean cross-section, or the product only |
| albedo, emissivity per band | what it does to the light rather than merely blocking it |

### The occultation integral

For an observer in direction `n`, an element at radius `r` occults the star when its
direction lies within a cone of half-angle `s = (R_star + r_elem) / r` about `n`, on the
observer's side. So with `Sigma(n)` the element sky density seen from the star, in elements
per steradian:

```
m(n) = Sigma(n) * pi * R_star^2 / a^2          expected elements on the disc
d(n) = m(n) * sigma / (pi * R_star^2)          expected fractional deficit
     = Sigma(n) * sigma / a^2
```

For an isotropic population this collapses to the obvious answer, which is a good check on
the algebra:

```
Sigma = N / 4pi     ->     d = N * sigma / (4 pi a^2)
```

The covering fraction. The expected dimming of an isotropic swarm is the swarm's total
cross-section divided by the area of the sphere it occupies, independent of direction.

For a population confined to inclinations near `i`, with random node and phase, the latitude
density is the standard result

```
p(phi | i) = cos(phi) / (pi * sqrt(sin^2(i) - sin^2(phi))),     |phi| < i
Sigma(phi | i) = N / (2 * pi^2 * sqrt(sin^2(i) - sin^2(phi)))
```

giving the donut: a deficit that peaks toward the population's plane and vanishes outside
its inclination limit. Verified by Monte Carlo against direct orbit sampling; agreement is
within Poisson error at every latitude away from the caustic (see below).

For eccentric populations, the time-averaged radial density of a single orbit is

```
p(r) = r / (pi * a * sqrt(a^2 e^2 - (r - a)^2)),     a(1-e) < r < a(1+e)
```

which integrates to 1 exactly, and which is convolved into `p(a)` before the occultation
integral. Both this and the latitude density carry an integrable inverse-square-root
singularity at their turning points.

### The caustic, and what sets shell resolution

`Sigma(phi)` diverges at `|phi| = i`: elements dwell at their turning latitude, so the sky
density spikes there. The divergence is integrable and the observable is not the point value
but the average of `Sigma` over the disc's cone of half-angle `s`. Monte Carlo shows the
difference directly: at `a/R_star = 20` and a delta-function inclination, sampling `Sigma`
at a point 0.05 rad inside the caustic underestimates the true count by 20%; at
`a/R_star = 100` the same point agrees to 2%.

Two consequences:

1. **Bake `m(n)`, the cone-averaged count, not `Sigma(n)`.** The shell then stores the
   observable, with the smoothing already folded in.
2. **Shell resolution is set by the inclination spread, not by the transit band width.**
   A population with `Delta i` of several degrees has no structure finer than that and bakes
   at level 4-5. A razor-thin ring has structure at `s = R_star / a`, which is 0.27 degrees
   at 1 AU, and needs either level 8 near the two caustic latitudes or an analytic edge term
   added on top of a coarse shell. Prefer the edge term.

### Flicker

The mean is not the whole signal. The number of elements on the disc at any instant is
Poisson with mean `m`, so the deficit fluctuates:

```
rms deficit / mean deficit = 1 / sqrt(m)
```

with a correlation time equal to the disc crossing time `t_cross = 2 R_star / v_perp`. This
is a real, observable, and cheaply generated signal: the client synthesises a realisation
from a seed keyed on `(star, population, time bucket)`, with the right mean, the right
variance, and the right autocorrelation. No element is ever instantiated.

Worked example, a swarm of 1.5e6 elements of 1e6 km^2 each at 1 AU around a Sun-like star:

| quantity | value |
|---|---|
| `s = R_star / a` | 4.65e-3 |
| `m`, elements on the disc | 8.1 |
| mean deficit `d` | 5.3e-6 |
| rms flicker | 1.9e-6, i.e. 35% of the mean |
| disc crossing time | 13.0 in-game hours = 5.3 real seconds |
| flicker knee frequency | 2.1e-5 Hz in-game |

Detection at 10 pc with a 1 m^2 aperture at 50% throughput: the mean deficit needs
`1/d^2 = 3.5e10` photons, so 600 s of in-game integration, 0.07 real seconds. The flicker
needs 4900 s in-game. Both are cheap nearby and scale as `d^2` with distance.

### The sparse limit

`m` is not always large. The same population record and the same integral cover the case
where elements almost never overlap, but the **description** of the signal changes, and a
synthesiser that assumes Gaussian fluctuation about a mean is simply wrong there.

The diagnostic is already stored: `rms / mean = 1 / sqrt(m)`. When it exceeds about 1/3, the
fluctuation is comparable to the thing it is fluctuating about, and the signal is not noise
around a mean — it is a sequence of isolated events.

| regime | `m` | character | what an observer measures |
|---|---|---|---|
| dense | >> 1 | elements overlap continuously | mean deficit, flicker amplitude, spectral knee |
| transition | ~ 1 | events merge and separate | both, badly |
| sparse | << 1 | isolated occultations, mostly nothing | event rate, per-event depth, per-event duration |

The solar system spans all three:

| population | `m` | mean deficit | single-event depth | event rate | duration |
|---|---|---|---|---|---|
| swarm, 1.5e6 x 1e6 km^2 at 1 AU | 8.1 | 5.3e-6 | 6.6e-7 | continuous | 13 h |
| Kuiper analogue, 1e9 x 50 km at 40 AU | 3.4 | 1.7e-8 | 5.2e-9 | continuous | 3.4 d |
| Hills cloud, 1e12 x 1 km at 5000 AU | 0.22 | 4.5e-13 | 2.1e-12 | 1 per 0.5 yr | 38 d |
| Oort cloud, 1e12 x 1 km at 20 000 AU | 0.014 | 2.8e-14 | 2.1e-12 | 1 per 15.5 yr | 77 d |

The Oort cloud is a population like any other. It is stored as one record, evaluated by the
same closed form, and costs nothing at runtime that the swarm does not also cost. It is also
photometrically invisible: a mean deficit of 2.8e-14 needs 1.3e27 photons to reach SNR 1,
and its individual events are 2.1e-12 deep and 77 in-game days long. This is the correct
answer — real Oort clouds around other stars are not detectable in transit either.

### Why the sparse branch has to exist anyway

The mean deficit is a fiction when `m << 1`. The star is not dimmed by that amount; it is
undimmed almost always and dimmed by the single-event depth occasionally. For the Oort cloud
both numbers are below every threshold, so the distinction does not matter. For a sparse
population of **large** elements it matters by three orders of magnitude, in the direction
that loses the signal:

| 1000 fragments of 1000 km radius at 2 AU | value |
|---|---|
| `m` | 1.4e-3 |
| mean deficit | 2.8e-9 — undetectable |
| single-event depth | **2.1e-6 — comfortably detectable** |
| event rate | 1 per 1.5 in-game years |
| event duration | 0.8 in-game days, 8 real seconds |

A synthesiser that reported the mean would declare this population invisible. What an
observer actually sees is a deep, isolated, unexplained dip once every year or two: the
signature of something large and artificial in an orbit, and one of the more alarming things
the game can show a player.

So the flicker synthesiser branches on `m`, which the shell already stores:

```
m >> 1   Gaussian noise, variance d^2/m, correlation time t_cross
m << 1   Poisson event train, rate m/t_cross, depth sigma/(pi R*^2), width t_cross
```

Both are O(1) to evaluate and both are seeded, so client and server agree on when the rare
events happened.

### Coherence, restated

The sparse limit sharpens the rule that separates the two paths. A single Oort body has a
period, so its signature is formally coherent — but the period is millions of years and it
will be seen once. Coherence is only useful if it **repeats within an observable baseline**.

| | repeats within a baseline | does not |
|---|---|---|
| single body | analytic: planets, moons, stations | statistical: one member of a sparse population |
| population | — | statistical: swarms, belts, clouds |

That is the test. Not size, not element count, not angular scale.

## Path 3: the baked shell

The shell is where path 2's populations are accumulated. It exists because summing is
cheaper than integrating, not because a population needs a sphere in principle.

A single axisymmetric population does not need a sphere at all: `m` depends only on latitude
about that population's pole, so it is a one-dimensional profile of a few hundred `f32`s,
or a closed form. The sphere is needed when the result is not axisymmetric:

- several populations with different poles, superposed;
- a partially built swarm, which is the interesting case — a swarm under construction is
  banded, clumped, or hemispherical, and its asymmetry is exactly what an observer can see;
- a swarm whose element distribution is deliberately shaped to avoid a known observer.

Deficits add in the optically-thin limit (`d << 1`), which is what makes accumulation
correct: rasterise each population's contribution once and sum. The shell is invalidated by
any event that changes a population's parameters, and content-addressed by the hash of those
parameters so an unchanged swarm reuses its bake across restarts.

Contents, per vertex:

| channel | type | meaning |
|---|---|---|
| `m` | f32 | cone-averaged element count on the disc; gives the flicker amplitude |
| `d_b` | f32 x bands | mean fractional deficit, per band |
| `t_cross` | f32 | mean disc crossing time; gives the flicker's correlation time |

At level 5 (10 242 vertices, 2.0 degree edges) with four bands that is 24 bytes per vertex,
240 KB per star, and only stars that actually have a population get one.

| level | vertices | edge | size, 4 bands |
|---|---|---|---|
| 3 | 642 | 7.9 deg | 15 KB |
| 4 | 2 562 | 4.0 deg | 60 KB |
| 5 | 10 242 | 2.0 deg | 240 KB |
| 6 | 40 962 | 0.99 deg | 960 KB |

Pick the level from the population's inclination spread. Most swarms sit at 4 or 5.

## Evaluating a sample

```
1. barycentric interpolation of (m, d_b, t_cross) at n              3 vertex reads
2. flicker realisation from the seeded generator at t_r             O(1)
3. discrete occluders: f_i(n, t_r) for each                         O(occluders)
4. L_star(t_r) from the variability model                           O(1)
5. combine
```

No part of this scales with population size. A swarm of 1.5e6 elements and a swarm of 1.5e9
cost the same to observe; they differ in `m`, `d` and the flicker amplitude, which is
exactly how they differ physically.

## Intrinsic variability

`L_star(t)` is a deterministic function of the star's parameters and `t`, so both server and
client evaluate it without communication.

| component | form | timescale |
|---|---|---|
| base luminosity | constant from spectral class | — |
| rotational modulation | sum of a few sinusoids from starspot groups | days to weeks |
| granulation and flicker | band-limited noise from a seeded PRNG | hours |
| flares | Poisson process, seeded; exponential decay profile | minutes |
| pulsation, for variable classes | sinusoid or harmonic series from the class | hours to years |

Every stochastic component, including population flicker, is drawn from a generator seeded
on the source and a time bucket, never from wall-clock randomness. Otherwise two clients
disagree about what a star did and a light curve stops being reproducible.

## Bands

Five, and the fifth changes the model.

| band | roughly | why it is there |
|---|---|---|
| B | 440 nm | colour, and the most extinction-sensitive band |
| V | 550 nm | the workhorse; magnitudes and depths are quoted here |
| K | 2.2 um | sees through dust that V cannot |
| thermal IR | 10 um | waste heat |
| radio | 21 cm | sees through everything, and carries signals |

The thermal IR band is not decoration. Total output is conserved, so light a swarm intercepts
reappears as waste heat; a player watching only V sees a star that is slightly dim, and a
player watching 10 um sees an excess with no natural explanation. That is the difference
between noticing a rival and identifying one.

### Occlusion is not achromatic

Adding a radio band forces a correction to the occlusion model. Geometric blocking is grey —
a solid body removes the same fraction at every wavelength — but **dust is not**. Interstellar
extinction follows roughly `A_lambda ~ 1/lambda` through the optical and falls away to nothing
in the radio:

| band | `A_lambda / A_V` | flux through `A_V = 1` | through `A_V = 5` |
|---|---|---|---|
| U | 1.53 | 0.244 | 0.0009 |
| B | 1.32 | 0.297 | 0.0023 |
| V | 1.00 | 0.398 | 0.0100 |
| I | 0.48 | 0.643 | 0.110 |
| K | 0.11 | 0.904 | 0.603 |
| 10 um | 0.06 | 0.946 | 0.759 |
| 21 cm | ~1e-10 | 1.000 | 1.000 |

So a cloud that removes 99% of a star's visible light removes 40% of its K band and none of
its 21 cm emission. In the galactic plane the ISM itself averages about 1.8 mag/kpc, which is
one magnitude of V extinction per 1800 ly of sightline, before any local cloud.

This is why `Population::band_response` exists. It is flat for anything solid and follows an
extinction curve for anything made of dust, and **the difference between those two shapes is
itself the observable**:

| dip shape | occluder |
|---|---|
| grey — same depth in every band | solid: planet, collector, swarm element, megastructure |
| reddening — much deeper in B than K, absent in radio | dust, debris, a natural cloud |
| grey in the optical with an IR excess | solid, and absorbing rather than merely blocking — engineering |

A civilisation that wants its swarm mistaken for a dust cloud has to make it reddening, which
means making it out of small particles, which means giving up the structural integrity that
made it a collector. The disguise has a physical price, and the game does not have to invent
one.

The radio band also stops the game from being a pure line-of-sight problem. A system behind a
dense cloud is invisible optically and perfectly ordinary at 21 cm, so dust is cover against
one kind of observation and no cover at all against another. See
[05-observation.md](05-observation.md) for what the radio band costs to use.

Adding bands costs linearly in shell size and nothing on the analytic path.

## What this buys the game

1. A rival builds collectors around their star.
2. Early on there are few, they are discrete, and they transit coherently. An observer in
   the right direction sees individual box-shaped events with a clean period.
3. As the swarm grows past the point where elements overlap on the disc, the individual
   events merge into a mean deficit plus correlated noise. The coherent signal is replaced
   by a statistical one, and the transition itself is observable.
4. From the mean deficit `d`, the rms flicker `f`, and the knee frequency, an observer
   recovers all three parameters separately:

   ```
   k     = f^2 / d                     deficit per element
   sigma = k * pi * R_star^2           element size
   m     = d / k                       elements on the disc
   a     = from the knee and the star's mass
   N     = 4 * m * a^2 / R_star^2      total element count
   ```

   Checked against the worked example above: it returns 1.0e12 m^2 and 1.50e6 elements.

5. So the observer learns how many objects of what size orbit at what radius, 30 in-game
   years ago. Not that a swarm exists — its scale, and therefore its owner's industrial
   capacity.
6. The rival can shape the swarm's inclination distribution to put a gap where they believe
   an observer sits, which requires knowing where the observer is, which requires the same
   kind of observation.

None of that is scripted. It falls out of evaluating the occultation integral honestly.

## Data

```rust
pub struct EmissionModel {
    pub luminosity: f64,              // W
    pub radius: f64,                  // m
    pub teff: f64,                    // K
    pub limb_darkening: [f32; 2],     // quadratic coefficients, by spectral type
    pub variability: Variability,     // deterministic, seeded
    pub discrete: Vec<OccluderRef>,   // coherent occluders, analytic path
    pub populations: Vec<Population>, // statistical, contribute to the shell
    pub shell: Option<ShellHandle>,   // baked, level 3-6, f32 deficits
}

pub struct Population {
    pub pole: DVec3,
    pub a: Distribution,              // semi-major axis
    pub e: Distribution,
    pub inc: Distribution,
    pub count: f64,                   // may be fractional; it is a density, not a roster
    pub cross_section: f64,           // m^2 per element
    pub band_response: [f32; BANDS],
}
```

`count` is deliberately an `f64` and deliberately not backed by a list of entities. A
population is a distribution. Instantiating its members is the thing this design exists to
avoid.

## Promotion and demotion

An object moves between paths when its coherence does.

| transition | trigger |
|---|---|
| discrete -> population | the group's expected count on the disc `m` approaches 1, so individual events overlap and stop being separable |
| discrete -> population | the body's period exceeds any observable baseline, so its coherence is unusable |
| population -> discrete | a player selects specific elements for a manoeuvre; they leave the distribution and become tracked bodies until they rejoin |

The threshold is observational as well as computational: a sufficiently good telescope can
separate events that a poor one cannot. Resolve this by keeping the statistical description
authoritative and letting instrument quality decide how much structure the client is shown,
rather than by switching representations per observer.

## Open

These are all accepted as open, to be settled during implementation rather than before it.
None of them blocks a first version.

- Limb-darkening coefficients per spectral type. Claret tables are standard; decide whether
  to bundle a table or fit a two-parameter function of `Teff`.
- The caustic edge term for thin rings. An analytic correction added to a coarse shell is
  preferable to raising the whole shell's level, but it needs writing.
- Correlated rather than Poisson statistics. Real swarms have structure — resonances, gaps,
  clumps — which makes the flicker non-Poisson and its spectrum informative in ways the
  current model does not capture. Probably a later refinement, and a good one.
- The transition band, `m` of roughly 0.3 to 3, where neither branch of the synthesiser is
  right. A Kuiper analogue sits there. Either interpolate, or generate the true Poisson event
  train in that band and accept the cost, since `m ~ 1` means few events to generate.
- Whether populations shadow each other. Two overlapping swarms at different radii are not
  independent; the optically-thin sum is wrong once total deficit approaches 1, which is
  precisely the regime a completed Dyson swarm occupies. Use `1 - exp(-tau)` with
  `tau = sum d_i` rather than the linear sum once any population exceeds a few percent.
- Non-spherical stars and gravity darkening for rapid rotators. Deferred.
