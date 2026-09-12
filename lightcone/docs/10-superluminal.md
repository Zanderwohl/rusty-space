# Superluminal trajectories: what it would cost

**Status: analysis only. Nothing in the current design assumes this, and nothing should be
built for it yet.** This document exists because the question was asked and the answer
affects which invariants are safe to write into `lc-spacetime` now.

The scenario: ships move faster than light. Light does not. `c` remains the speed of light
and of every signal; only some worldlines exceed it.

That distinction carries most of the result. Signals are unaffected, so event delivery is
unaffected. What changes is the mapping between a moving object and its images.

## What breaks, precisely

The retarded-time equation is

```
f(t_r) = t_r + |x_o - x_s(t_r)| - t_o = 0
f'(t_r) = 1 - n . v_s          n = unit vector from source to observer
```

For `|v_s| < 1`, `n . v_s < 1` always, so `f' > 0` strictly: `f` is monotone and has exactly
one root. That is the invariant [01-spacetime.md](01-spacetime.md) asserts, and it is the
only thing superluminal motion breaks.

For `|v_s| > 1`, `f'` changes sign. `f` is no longer monotone, and the root count is 0, 1 or
more. Roots are created and destroyed in pairs at folds, where `f' = 0`.

Verified numerically. For a source crossing at constant `beta` with perpendicular distance
`b`, closest approach at `t = 0`, the equation reduces to a quadratic:

```
(beta^2 - 1) t^2 + 2 t_o t + (b^2 - t_o^2) = 0

t_pm = [ -t_o +/- sqrt(beta^2 t_o^2 - (beta^2 - 1) b^2) ] / (beta^2 - 1)
```

| `t_o` | roots |
|---|---|
| `< b sqrt(1 - 1/beta^2)` | none — the object is not yet in the observer's past |
| `= b sqrt(1 - 1/beta^2)` | one, a double root: the caustic |
| `> b sqrt(1 - 1/beta^2)` | two |

Checked against brute-force bisection at `beta` of 1.2, 2 and 5 and across five decades of
`t_o`; the closed form and the numerical roots agree to 1e-3 everywhere. Sub-luminal cases
returned exactly one root in every trial.

Asymptotically `t_+ -> t_o / (beta + 1)` and `t_- -> -t_o / (beta - 1)`, confirmed to six
digits at `t_o = 1e6`. So one image runs forward through the object's history and the other
runs backward through it, without limit. That is the Picard Maneuver, and it is not
something that would need implementing — it is what the solver returns once it stops
assuming one root.

The first-arrival surface is the Cherenkov cone, half-angle `sin(theta) = 1 / beta`:
30 degrees at `beta = 2`, 11.5 degrees at `beta = 5`. Outside it, the object is invisible
because it has not entered the observer's past light cone. The cone sweeps over an observer
and the object appears from nothing, then splits.

Apparent brightness carries the factor `1 / |1 - n . v|`, which diverges at the fold. At
`beta = 2`, an observer 1e-7 past the caustic sees an amplification of 1500. The image pair
is born in a flash and separates. This is the same mathematics as a gravitational lensing
caustic and has the same numerical hazard: it must be clamped by the source's finite angular
size and the instrument's finite exposure, or it produces infinities.

## What does not break

| | why |
|---|---|
| the coordinate grid, units, `c = 1` | geometry, untouched |
| `interval2` and the Minkowski metric | untouched |
| light and signal propagation | signals still travel at `c` |
| the `deliveries` table and delivery scheduling | each **event** still has exactly one arrival time per observer, because its light travels at `c` from a fixed point |
| the event schema | unchanged |
| photometry, emission shells, populations | stars do not move superluminally |
| the `Worldline` trait signature | `position_at(t)` is still a function of server-frame `t` |

The delivery path surviving intact is the important one. The multi-image effect is about the
apparent continuous motion of an **object**, which is reconstruction. A discrete event
happens at one coordinate and its light reaches an observer once.

## The paradox question, and why this design is already immune

Superluminal signalling normally permits closed causal loops: A causes B across a spacelike
interval, B causes C across a spacelike interval in a different frame, and C precedes A in
the original frame.

This design cannot express that, and the reason is already in the code.

**A worldline is a function of server-frame `t`.** `position_at(t) -> DVec3` is total and
single-valued in `t`, so no object can move backward in server time, at any `beta`. Every
event is stamped with a server-frame `t`. Causation always runs from smaller `t` to larger
`t`. A loop would require placing an effect at a smaller `t` than its cause, and there is no
operation in the design that can do so.

So the engine is paradox-free by construction, not by checking. What it costs is explicit:

- `lc_precedes` currently means "timelike or lightlike separated, in that order", which is
  frame-independent and therefore physically meaningful. With superluminal ships it would
  have to mean "smaller `t` in the server frame", which is a **convention**.
- The game would have chosen a preferred frame in the strong sense — a foliation that
  superluminal causation respects. Physically that is a preferred-frame tachyon theory, which
  is self-consistent and is the standard way to have FTL without paradox.
- The design has already chosen a preferred frame for other reasons: players see server-frame
  time, there is no per-player frame, and simultaneity is already defined by fiat. So the
  incremental cost of this commitment is close to zero here, where in a frame-agnostic design
  it would be fatal.

The one observable consequence: superluminal travel would be anisotropic in the frame of a
fast-moving ship, so a sufficiently careful player could measure the preferred frame. That
is a feature if acknowledged and a bug if not.

## Actual work, by component

| component | change | size |
|---|---|---|
| `lc_spacetime::retarded_time` | returns a set of roots, not one | the real work |
| root isolation | partition each worldline segment at `n . v = 1`, then one Newton per monotone interval | moderate, and only possible because worldlines are already required to be closed-form |
| tangency handling | a double root at a caustic is a tangency, not a sign change; bracketing by sign alone misses it entirely, as the verification run demonstrated | small but easy to get wrong |
| renderer | draw `k` images per source, each with its own retarded state and beaming factor | moderate, and the visual payoff |
| caustic clamping | finite source size and exposure, or infinities | small |
| `LightConeCursor` | "a source's reachable events are a contiguous range in `(source_id, t)`" weakens to `k` ranges | small, if superluminal sources are segregated |
| `Separation` | spacelike no longer implies causally disconnected | rename, and audit every use |
| interception and beam aiming | the advanced-time solve becomes multi-root too | moderate |

Segregating superluminal sources is what keeps the cursor cheap. There are ~1e5 sources and
almost all of them are stars, which never move superluminally. Keep a separate small set for
the ones that can, solve those the expensive way, and the common path is unchanged.

Estimated shape: one module rewritten with care, one rendering feature added, one optimisation
weakened, one invariant restated. Bounded, and more interesting than difficult.

## The cost that is not technical

The engineering is affordable. The premise is what superluminal travel spends.

The entire information economy — telescopes, stale intelligence, probes reporting years late,
the impossibility of recalling a fleet — exists because nothing outruns light. A single FTL
courier bypasses all of it. The question is not whether the solver can handle it but how much
of the game is left afterwards.

It is a matter of degree, and the degree is computable:

| `beta` | Proxima, one way | what survives |
|---|---|---|
| 1 (light) | 4.25 real hours | everything |
| 2 | 2.1 hours | most of it; intelligence is still hours stale |
| 10 | 25 minutes | interstellar play becomes tactical rather than strategic |
| 100 | 2.5 minutes | a conventional RTS with unusual visuals |
| 1000 | 15 seconds | nothing |

At low `beta` the effects are also at their most spectacular, because the Cherenkov cone is
wide and the image separation is slow enough to watch. At `beta = 1.2` the cone half-angle is
56 degrees; at `beta = 100` it is 0.57 degrees and an observer sees the flash and the split
almost simultaneously. The interesting regime and the premise-preserving regime are the same
regime, which is fortunate.

## Recommendation

Do not build it. Do not foreclose it either.

Three cheap decisions now keep the option open at no cost:

1. **`retarded_time` returns a collection, not an `Option`.** Today it always has length
   0 or 1, and the caller pays nothing. Changing the signature later touches every call site;
   choosing it now costs one `SmallVec`.
2. **Do not write "exactly one root" into an assertion.** Write it as a debug assertion
   gated on the worldline's own `is_subluminal()`, so the invariant is checked where it holds
   and the code does not assume it globally.
3. **Keep `Separation` and `lc_precedes` documented as frame-independent *because* nothing
   is superluminal**, rather than as unconditional facts. Then the day the assumption changes,
   the affected code is already identified.

If superluminal travel is ever added, it should be as a rare, expensive, low-`beta` capability
— and the image-splitting should be the reason players notice it, not a side effect they are
asked to ignore.
