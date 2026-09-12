# Stellar photometry: the emission shell

## The model

A star is surrounded by a sphere at the system's shell radius. Every direction on that
sphere carries a radiance: the flux per unit solid angle that leaves the system in that
direction, in each of a few spectral bands.

```
L(n, t) = L_star(t) * product over occluders of (1 - f_i(n, t))
```

| term | meaning |
|---|---|
| `n` | unit direction out of the system |
| `L_star(t)` | intrinsic output: base luminosity, rotational modulation, flares, pulsation |
| `f_i(n, t)` | fraction of the stellar disc occluder `i` covers, as seen from direction `n` |

An observer at distance `d` in direction `n` receives `L(n, t_r) / d^2` at time
`t = t_r + d`, where `t_r` is the retarded time from [01-spacetime.md](01-spacetime.md).

This is the whole photometry model. Everything below is about how to evaluate it cheaply.

## Why it is not a stored texture

The intuitive implementation stores radiance per vertex on a subdivided icosphere and
interpolates. The resolution needed makes that impossible for the interesting case.

A transiting body blocks the star only for directions within a narrow band around its
orbital plane. The angular half-width is

```
sin(theta) = (R_star + R_body) / a
```

| occluder | half-width | full band |
|---|---|---|
| Earth at 1 AU | 4.70e-3 rad | 0.54 deg |
| Jupiter at 5.2 AU | 9.88e-4 rad | 0.11 deg |
| Mercury at 0.39 AU | 1.20e-2 rad | 1.4 deg |

Icosphere resolution, where vertices = `10 * 4^n + 2` and edge angle is about
`63.4 deg / 2^n`:

| level | vertices | faces | edge |
|---|---|---|---|
| 4 | 2 562 | 5 120 | 4.0 deg |
| 5 | 10 242 | 20 480 | 2.0 deg |
| 6 | 40 962 | 81 920 | 1.0 deg |
| 7 | 163 842 | 327 680 | 0.50 deg |
| 8 | 655 362 | 1 310 720 | 0.25 deg |

Resolving an Earth-analogue transit needs level 8 at minimum, and Jupiter at 5 AU needs
level 10. At level 8, four bands of `f16` is 5.0 MB per star. For 1000 observed stars that
is 5.0 GB, per time sample, of a signal that also varies in time. It does not work.

## Two evaluation paths

**Split by the angular scale of the occluder's signature, not by what kind of object it
is.**

### Analytic path: discrete occluders

For planets, moons, individual collectors, and any occluder whose signature is narrow:
evaluate `f_i(n, t_r)` directly.

```
1. propagate occluder i to t_r            (em_sim, Keplerian, closed form)
2. project its centre onto the plane normal to n
3. b = impact parameter, in units of R_star
4. f_i = overlap area of two discs, weighted by a limb-darkening profile
```

Cost is O(occluders) per star per observation direction, with one Kepler solve each. A
system with 20 occluders costs 20 Kepler solves per sample. A telescope taking 10 000
samples of one star costs 200 000 Kepler solves — milliseconds, and it is the same
`em_foundations::kepler` code the rest of the project already uses.

The key property: **this is evaluated only at directions actually observed.** One telescope
looking at one star needs one direction. The full sphere is never built.

Disc overlap with linear limb darkening `I(mu)/I(0) = 1 - u(1 - mu)`:

```
mu = sqrt(1 - r^2)   at fractional radius r on the disc
```

Integrate the profile over the lens-shaped overlap region. A closed form exists (Mandel &
Agol 2002) for the uniform and quadratic cases; use the quadratic one, since it is standard,
tested, and the coefficients are tabulated by spectral type.

### Baked path: statistical occluders

For dust, dyson swarms with thousands of elements, ringworlds, and shell segments: the
signature is broad, so bake it.

| occluder | angular scale | level |
|---|---|---|
| dust cloud | 10s of degrees | 3-4 |
| dyson swarm, partial | 5-30 deg | 4-5 |
| ringworld | 1-5 deg along one band | 6 |

At level 5 with four `f16` bands, a baked shell is 80 KB. Only systems containing such a
structure get one, and only those get shipped to clients.

Baking a swarm: rasterise each element's occlusion band onto the shell and accumulate. For
N elements this is O(N * covered_vertices) once, and the result is re-baked only when the
swarm's configuration changes — which is an event, and therefore already the trigger for
invalidating a cache.

### Combining

```
L(n, t) = L_star(t) * baked_shell(n, t) * product over discrete occluders of (1 - f_i(n,t))
```

Baked contribution is barycentric interpolation across the containing triangle. Discrete
contributions are exact. The two never overlap, because an occluder is assigned to exactly
one path at creation by its angular half-width against a threshold of about 2 degrees.

## Intrinsic variability

`L_star(t)` is a deterministic function of the star's parameters and `t`, so both server and
client can evaluate it without communication. Components:

| component | form | timescale |
|---|---|---|
| base luminosity | constant from spectral class | — |
| rotational modulation | sum of a few sinusoids from starspot groups | days to weeks |
| granulation and flicker | band-limited noise from a seeded PRNG keyed on `(star, t_bucket)` | hours |
| flares | Poisson process, seeded; exponential decay profile | minutes |
| pulsation, for variable classes | sinusoid or harmonic series from the class | hours to years |

Every stochastic component is drawn from a seeded generator keyed on the star ID and a time
bucket, never from wall-clock randomness. Otherwise two clients disagree about what a star
did, and the signal a player extracts from a light curve stops being reproducible.

## What this buys the game

The chain that makes observation meaningful:

1. A rival builds collectors around their star.
2. Collector area enters the star's occlusion, in the directions the collectors' orbits pass
   through.
3. An observer 30 ly away, in one of those directions, measures a dimming with a period
   equal to the collectors' orbital period.
4. The observer learns the rival's orbital radius (from the period and the star's mass), the
   total collector area (from the depth), and the inclination (from the transit duration) —
   at the retarded time, 30 in-game years ago.
5. The rival can reduce this by putting collectors in orbits whose planes miss the observer,
   which requires knowing where the observer is, which requires the same kind of
   observation.

None of that is scripted. It falls out of evaluating `L(n, t)` honestly.

## Data

Per star:

```rust
pub struct EmissionModel {
    pub luminosity: f64,              // W
    pub radius: f64,                  // m
    pub teff: f64,                    // K
    pub limb_darkening: [f32; 2],     // quadratic coefficients, by spectral type
    pub variability: Variability,     // deterministic, seeded
    pub discrete: Vec<OccluderRef>,   // bodies and structures, analytic path
    pub baked: Option<ShellHandle>,   // level 3-6 icosphere, when present
}
```

`ShellHandle` refers to a cached bake, invalidated by any event that changes a statistical
occluder. Shells are content-addressed by the hash of the occluder configuration, so an
unchanged swarm re-uses its bake across restarts.

## Bands

Start with four: a broad visible band and three others chosen to make spectroscopy
meaningful — something like `U`-ish, `V`-ish, `I`-ish, plus a thermal IR band that makes a
dyson swarm's waste heat visible. Waste heat is the mechanic that stops a swarm from being
invisible: total output is conserved, so occluded visible light reappears as infrared, and a
player who only watches the visible band misses it.

Four `f16` bands is the storage assumption used above. Increasing the count is linear in
baked shell size and free on the analytic path.

## Open

- Limb-darkening coefficients per spectral type. Claret tables are standard; decide whether
  to bundle a table or fit a two-parameter function of `Teff`.
- Threshold between analytic and baked paths. 2 degrees is a guess; it should be set by
  measuring where the analytic path's per-occluder cost exceeds the baked path's
  interpolation cost, at realistic occluder counts.
- Whether scattered light and reflection off planets matter. They are 1e-5 of the stellar
  flux for a hot Jupiter and less for everything else, so probably not, but a phase curve is
  an information channel if included.
- Non-spherical stars, and gravity darkening for rapid rotators. Deferred.
