# Spacetime coordinates and the relativity model

## The server frame

One inertial frame is privileged by fiat. Call it the **server frame**. Its origin in space
and time is arbitrary and fixed at world creation; a sensible choice is the barycentre of
the starting system at the moment the world is generated. The server's coordinate time `t`
advances monotonically at 8766x real time.

All stored coordinates are server-frame coordinates. No other frame is ever stored,
transmitted, or shown to a player.

This is a convention, not a claim about physics. Its consequences must be stated plainly in
the code, because they are the kind of thing that gets forgotten:

- "Simultaneous" means "equal `t` in the server frame". Two spacelike-separated events that
  the server calls simultaneous are not simultaneous in a moving ship's frame. Nothing in
  the game depends on the ordering of spacelike pairs, and nothing may be allowed to.
- Causal ordering (timelike and lightlike pairs) is frame-independent and therefore safe to
  depend on. Every rule that cares about "before" must be expressed against a timelike or
  lightlike separation.

## Units

Two coordinate tiers. Each is exact where it is used and lossy where it is not.

### Global tier: the integer light-microsecond grid

| quantity | unit | type |
|---|---|---|
| coordinate time | microsecond | `i64` |
| coordinate position | light-microsecond (299.792458 m) | `i64` x3 |

With these units **c = 1**, so the light-cone test is integer arithmetic with no constant
in it:

```
s2 = dt*dt - (dx*dx + dy*dy + dz*dz)
s2 > 0  timelike   (causally connected)
s2 == 0 lightlike  (exactly on the cone)
s2 < 0  spacelike  (no causal contact)
```

Range and resolution:

| | value |
|---|---|
| `i64` full range | +/- 9.22e18 units = +/- 292 000 years, +/- 292 000 ly |
| spatial resolution | 299.79 m |
| temporal resolution | 1 us of coordinate time = 114 ns of real time |

**Invariant: every coordinate satisfies `|c| < 2^60`** (1.15e18 units = 36 500 ly, 36 500
years). This bounds `dt^2 + dr^2` below 5.3e36, so `s2` computed in `i128` cannot overflow.
Without the bound, four squared `i64` extremes sum past the `i128` ceiling. The bound is
checked on construction of a coordinate, not at every comparison.

### Local tier: f64 metres from a system barycentre

Inside a star system, positions are `f64` metres relative to that system's barycentre, and
times are `f64` seconds relative to a per-system epoch. This is exactly what
`em-foundations` and `em-sim` already use, so orbital code needs no changes.

Precision, since this is why the split exists:

| distance from barycentre | f64 metre ulp |
|---|---|
| 1 AU (1.5e11 m) | 31 um |
| 1000 AU | 31 mm |
| 100 000 AU (1.58 ly, Oort edge) | 2.0 m |
| 1 ly in a *global* metre frame | 2.0 m |
| 100 ly in a *global* metre frame | 128 m |

Local metres stay precise because the demand for precision falls off with distance from the
star at the same rate the representation does. Global metres do not, which is why the global
tier is an integer grid instead.

### Converting between tiers

```
global = system_origin_global + round(local_metres / 299.792458)
```

The rounding costs up to 150 m, i.e. 0.5 us of coordinate time, i.e. 57 us of real time.
That is below any timescale a player can perceive and below the resolution of every
gameplay rule.

**Rule: the global grid is the index; local arithmetic is the truth.** Causality *within*
a system is evaluated in local f64 seconds and metres. Causality *between* systems is
evaluated on the integer grid. A query that spans the boundary uses the grid to select
candidates and local arithmetic to refine, in that order.

## Worldlines

A worldline is a function from coordinate time to position. Every object that can emit,
receive, or occlude must have one.

**Hard constraint: every worldline must be evaluable at arbitrary past coordinate time**,
in closed form or from a stored spline. Retarded-time solving (below) evaluates positions at
times that are not the current tick and were not necessarily visited in order.

This forbids storing ship motion only as an integrator state. It is already satisfied by:

| kind | representation | source |
|---|---|---|
| star | fixed point, or linear proper motion | HYG catalogue in `assets/catalogs/` |
| planet, moon, comet | Keplerian elements | `em_sim::motive` |
| ship under thrust | piecewise analytic arcs | `em_sim::trajectory::Path`, `em_sim::patch` |
| structure | fixed relative to its parent body | — |

Ship arcs under constant proper acceleration are hyperbolic motion in the server frame:

```
x(t) = x0 + (c^2/a) * (sqrt(1 + (a*t/c)^2) - 1)
v(t) = a*t / sqrt(1 + (a*t/c)^2)
tau  = (c/a) * asinh(a*t/c)
```

Closed form in both directions, which is what the constraint requires.

## Retarded time

The central operation. Given an observer at coordinate `(t_o, x_o)` and a source with
worldline `x_s(t)`, find the emission time `t_r` whose light arrives at the observer:

```
t_r + |x_o - x_s(t_r)| = t_o        (c = 1)
```

Implicit, because the source moves while its light is in flight. Solve by fixed-point
iteration seeded with the static answer:

```
t_r <- t_o - |x_o - x_s(t_o)|
repeat: t_r <- t_o - |x_o - x_s(t_r)|
```

The iteration contracts at rate `beta = |v_s|/c`. For stars (`beta < 1e-3`) one iteration
reaches machine precision. For ships at `beta = 0.5` it needs ~35 iterations for f64, so use
Newton on `f(t_r) = t_r + |x_o - x_s(t_r)| - t_o` instead, whose derivative is
`1 - n . v_s` with `n` the unit vector from source to observer. Newton converges in 3-4
steps at any `beta < 1`.

There is exactly one solution for any sub-luminal worldline, because `f` is strictly
increasing. Assert that; a second root means a worldline went superluminal and is a bug.

## What relativity actually does in the game

Four effects, and nothing else.

1. **Light delay.** Everything a player observes is retarded. This is the whole game.
2. **Doppler shift.** Radio from a moving transmitter arrives at
   `f_obs = f_src * sqrt((1-beta)/(1+beta))` for pure recession, with the general form
   `f_obs = f_src / (gamma * (1 - n . beta))`. Determines whether a receiver tuned to a
   band still hears a fast-moving ship, and lets a receiver infer radial velocity.
3. **Aberration.** At `beta = 0.5` the apparent direction to a star shifts by up to 30
   degrees. Applied in the client when rendering from a fast ship; applied on the server
   when deciding what a tight beam actually hits.
4. **Proper time.** A ship accumulates `tau = integral dt/gamma`. Onboard processes —
   construction, refining, computation — advance on `tau`. The player's clock is `t`. A ship
   that runs at `beta = 0.87` (`gamma = 2`) builds at half rate as measured by the player,
   and its crew ages half as fast. The UI shows both, labelled, and never transforms the
   world into the ship's frame.

Nothing else from SR appears. There is no length contraction of rendered objects (it would
be a rendering of the ship's frame, which the game does not have), no relativistic mass, no
frame switching.

## Types

Sketch, in `lc-spacetime`:

```rust
/// Server-frame coordinate on the integer light-microsecond grid. c = 1.
pub struct Coord { pub t: i64, pub x: i64, pub y: i64, pub z: i64 }

/// Invariant interval squared, in i128. Sign classifies the pair.
pub fn interval2(a: Coord, b: Coord) -> i128;

pub enum Separation { Timelike, Lightlike, Spacelike }

/// Solve t_r + |x_o - w(t_r)| = t_o. Returns None if the worldline has no coverage there.
pub fn retarded_time(observer: Coord, w: &dyn Worldline) -> Option<f64>;

pub trait Worldline {
    fn position_at(&self, t: f64) -> DVec3;   // local metres, or global if unparented
    fn velocity_at(&self, t: f64) -> DVec3;
    fn defined_over(&self) -> Range<f64>;
}
```

`Coord` is `Copy`, `Ord` on `t` only, and has no arithmetic operators that could mix a
duration with an instant — the same discipline `em_foundations::time` applies to `Instant`
and `JulianDate`, and for the same reason.

## Open

- Do stars get proper motion, or is the catalogue frozen? Proper motion makes every
  interstellar retarded-time solve iterative instead of exact. Barnard's Star moves 10.3
  arcsec/yr, which at 8766x is visible within a session.
- Does the world wrap or terminate at a boundary? The `2^60` invariant permits 36 500 ly,
  far past any plausible playable volume, so a soft boundary is a design choice rather than
  a representation limit.
