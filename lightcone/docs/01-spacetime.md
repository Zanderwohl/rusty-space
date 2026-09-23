# Spacetime coordinates and the relativity model

## The server frame

One inertial frame is privileged by fiat. Call it the **server frame**. Its origin in space
and time is arbitrary and fixed at world creation; a sensible choice is the barycenter of
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

### Time is stored once, in microseconds

**Decided: `i64` microseconds is the only stored time representation.** There is no per-system
epoch and no second absolute origin. `f64` seconds exists only as a locally computed
*difference*, which is what Kepler propagation actually consumes and the regime where `f64` is
precise.

The reason is the length of the game. Exotic Matters runs 2000 to 2400, so `f64` seconds from
J2000 holds up for its whole span. This game does not: at the 36 500-year world horizon the
`f64` ulp is 73 km of light travel against a 300 m grid, and a fictional galaxy has no J2000
for the origin to mean anything against.

**`em-foundations` is not modified.** `Instant` keeps its meaning and Exotic Matters keeps
working. `lc-spacetime` converts at the boundary, and only there:

```rust
// The argument to any em-sim call is an offset, never an absolute epoch.
fn propagation_time(t: i64, system_epoch: i64) -> Instant {
    Instant::from_seconds((t - system_epoch) as f64 * 1e-6)
}
```

The difference is small, so the `f64` is precise, and there is exactly one function that knows
how the two systems relate — the same discipline `em_foundations::time` applies between
`Instant` and `JulianDate`, for the same reason.

### Local tier: f64 meters from a system barycenter

Inside a star system, positions are `f64` meters relative to that system's barycenter. This is
exactly what `em-foundations` and `em-sim` already use, so orbital code needs no changes.

Precision, since this is why the split exists:

| distance from barycenter | f64 meter ulp |
|---|---|
| 1 AU (1.5e11 m) | 31 um |
| 1000 AU | 31 mm |
| 100 000 AU (1.58 ly, Oort edge) | 2.0 m |
| 1 ly in a *global* meter frame | 2.0 m |
| 100 ly in a *global* meter frame | 128 m |

Local meters stay precise because the demand for precision falls off with distance from the
star at the same rate the representation does. Global meters do not, which is why the global
tier is an integer grid instead.

### Converting between tiers

```
global = system_origin_global + round(local_meters / 299.792458)
```

The rounding costs up to 150 m, i.e. 0.5 us of coordinate time, i.e. 57 us of real time.
That is below any timescale a player can perceive and below the resolution of every
gameplay rule.

**Rule: the global grid is the index; local arithmetic is the truth.** Causality *within*
a system is evaluated in local f64 seconds and meters. Causality *between* systems is
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
| star | fixed point, or linear proper motion | HYG catalog in `assets/catalogs/` |
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
increasing. Assert it as a debug assertion gated on `Worldline::is_subluminal()`, so the
invariant is checked where it holds rather than assumed everywhere. A second root means a
worldline exceeded `c`, which today is a bug and is the only thing that would have to change
if it ever were not — see [10-superluminal.md](10-superluminal.md).

`position_at(t)` being a total, single-valued function of server-frame `t` is also what makes
closed causal loops unrepresentable: nothing can move backward in `t`, so no effect can be
placed before its cause. That property is free, and it is worth not losing.

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
   and its crew ages half as fast. The UI shows both, labeled, and never transforms the
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

/// Solve t_r + |x_o - w(t_r)| = t_o.
///
/// Returns a collection, not an Option. For a sub-luminal worldline it always holds 0 or 1
/// root and the caller pays nothing for the generality. The signature is chosen now because
/// changing it later touches every call site, and superluminal motion would make it 0, 1 or
/// more. See 10-superluminal.md.
pub fn retarded_times(observer: Coord, w: &dyn Worldline) -> SmallVec<[f64; 2]>;

pub trait Worldline {
    fn position_at(&self, t: f64) -> DVec3;   // local meters, or global if unparented
    fn velocity_at(&self, t: f64) -> DVec3;
    fn defined_over(&self) -> Range<f64>;

    /// Every worldline in the current design returns true. The single-root invariant is
    /// asserted against this, not assumed globally.
    fn is_subluminal(&self) -> bool { true }
}
```

`Coord` is `Copy`, `Ord` on `t` only, and has no arithmetic operators that could mix a
duration with an instant — the same discipline `em_foundations::time` applies to `Instant`
and `JulianDate`, and for the same reason.

## Decided

**Stars get proper motion eventually, not yet.** The catalog is frozen for now, so every
interstellar retarded-time solve is exact and closed-form. It is planned, so the architecture
must not assume static stars: a star gets a `Worldline` like everything else, currently
returning a constant. Nothing may read a star's position as a field. When proper motion is
switched on, every interstellar solve becomes iterative and the cost is real — Barnard's Star
moves 10.3 arcsec/yr, which at 8766x is visible inside a single session, so it is worth
having.

**The world terminates at a boundary.** The playable galaxy fits inside the `2^60` invariant's
36 500 ly. Beyond the boundary is a skybox of distant galaxies, which are rendered and never
simulated. A third coordinate tier for intergalactic range is possible and is not expected to
be needed; the boundary is a design choice made early so that nothing comes to depend on its
absence.

**Causality is frame-independent because nothing is superluminal.** State it that way in the
code rather than as an unconditional fact — the two differ the moment anything exceeds `c`.
See [10-superluminal.md](10-superluminal.md) for what that would cost and for the three
cheap decisions that keep the option open.

## Subtract before you narrow, in time as well as space

`lc_world::boost` is the change of inertial frame, in seconds and light-seconds so that `c` is
one. It is used for matching velocity with another craft — see `lc_world::pursuit` — and it
carries one lesson worth stating on its own.

A frame is pinned to an event, and a long burn carries the ship a long way from it: shedding
`0.9999c` at five gravities takes most of a century and leaves the anchor hundreds of
light-years astern. Transforming the ship's position out of the frame and then subtracting the
quarry's to recover a five-kilometer standoff is a difference of two numbers of order a hundred
light-years, and `f64` has nothing left at that ratio.

The fix is not a better solver. Ask for the *offset* rather than the position, and the enormous
`γt'` term cancels symbolically — `separation_in_world` is that cancellation written out, and it
is the same discipline `em-render` applies to render coordinates, applied to the time axis as
well as the space ones.
