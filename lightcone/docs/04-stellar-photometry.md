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

Four to start: a broad visible band, two others chosen to make colour meaningful, and a
thermal infrared band. The IR band is not decoration. Total output is conserved, so light a
swarm intercepts reappears as waste heat; a player watching only the visible band sees a
star that is slightly dim, and a player watching the IR sees a star with an excess that has
no natural explanation. That is the difference between noticing a rival and identifying one.

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
| population -> discrete | a player selects specific elements for a manoeuvre; they leave the distribution and become tracked bodies until they rejoin |

The threshold is observational as well as computational: a sufficiently good telescope can
separate events that a poor one cannot. Resolve this by keeping the statistical description
authoritative and letting instrument quality decide how much structure the client is shown,
rather than by switching representations per observer.

## Open

- Limb-darkening coefficients per spectral type. Claret tables are standard; decide whether
  to bundle a table or fit a two-parameter function of `Teff`.
- The caustic edge term for thin rings. An analytic correction added to a coarse shell is
  preferable to raising the whole shell's level, but it needs writing.
- Correlated rather than Poisson statistics. Real swarms have structure — resonances, gaps,
  clumps — which makes the flicker non-Poisson and its spectrum informative in ways the
  current model does not capture. Probably a later refinement, and a good one.
- Whether populations shadow each other. Two overlapping swarms at different radii are not
  independent; the optically-thin sum is wrong once total deficit approaches 1, which is
  precisely the regime a completed Dyson swarm occupies. Use `1 - exp(-tau)` with
  `tau = sum d_i` rather than the linear sum once any population exceeds a few percent.
- Non-spherical stars and gravity darkening for rapid rotators. Deferred.
