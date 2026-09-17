# Solar power

Every hull is covered in collectors, so a ship earns energy from starlight when it holds still
near a star.

**Status: built,** except the attitude drawing below, which is deferred. `lc_world::solar`
holds the geometry and flux; `Craft` walks the segments. The first half of this doc is the
mechanic and the numbers behind it. The second half is how it fits into the energy account in
[19-ship-fitting.md](19-ship-fitting.md).

## The mechanic

Solar is the **fallback that never runs out**. No ship is ever stranded for good, but its
income is weak. What it pays for is short hops around the inner system. What it will not pay
for without a long wait is interstellar travel, a large hull, or life far from a star. That
wait is what makes collectors worth building.

- **Only when not under way.** A ship that is holding a station, falling or drifting turns its
  broad face to the star and collects. A ship flying a plan is pointed where the plan needs it
  and collects nothing.
- **Inverse square.** Income goes as `1/d²` from the star, so the place to refuel is close in.
  Getting there costs energy, and that trade is the game.
- **Size cuts both ways.** Collecting area goes as the square of hull length, while slots, mass,
  storage and living drain go as the cube. A bigger ship collects more in total and less per
  module, and has to be closer in to break even.
- **Heat is not modelled yet.** Nothing stops a ship from skimming the photosphere. A heat limit
  will reshape sun-diving later, and until then the effective refuelling point is about 0.1 AU,
  because that is what the anchor below is set against.

### Collecting area is the hull's shadow

Panels over the whole hull deliver `η · flux · ∫ max(0, n·ŝ) dA`, where `n` is a surface normal
and `ŝ` the direction to the star. For any convex body that integral **is the projected area**,
the shadow the hull casts along `ŝ`. For the ovoid, with semi-axes `a = L/2` along the nose,
`b = 0.3 L` across and `c = 0.1 L` deep:

**A(ŝ) = π √((b c sₓ)² + (a c s_y)² + (a b s_z)²)**

| attitude to the star | area | relative to broadside |
|---|---|---|
| **broadside**, star along the short axis | `π a b = 0.15 π L²` | 1 |
| averaged over every orientation (Cauchy: surface ÷ 4) | | 0.58 |
| side-on, star along the beam | `π a c` | 0.33 |
| nose-on, star along the nose | `π b c` | 0.20 |

Broadside is 43% of the hull's whole surface. The mechanic assumes broadside whenever a ship
collects.

### Power

**P = G · η · L★ / (4π d²) · 0.15 π L²**

| symbol | meaning | value |
|---|---|---|
| `η` | collector efficiency | 0.7 |
| `G` | balance gain | 1.18 × 10⁹, set by the anchor below |
| `L★` | the system primary's luminosity | the Sun: 3.83 × 10²⁶ W, which is 1361 W/m² at 1 AU |
| `d` | distance from the primary | |
| `L` | hull length | |

`G` is unphysical and says so. A module-energy is `mc²` for 155 000 tonnes, which is 0.036
seconds of the Sun's entire output. At a gain of one, a 500 m hull at 1 AU would take four
billion years to collect one. The gain keeps the physics' shape — silhouette, inverse square,
the star's own luminosity — and chooses the scale.

### The anchor

> **The starting ship, broadside at 0.1 AU from a Sun-like star, fills its storage from empty
> in one game year, net of its living drain.**

A game year is a real hour at the design rate, so every time below is also real time.

Starting ship: 20 slots, 30 ME of storage, 2 living modules. For the larger hulls the loadout
is scaled: 10% of slots living, 30% storage at 5 ME each.

| | 500 m (30 ME) | 1 km (240 ME) | 5 km (30 000 ME) |
|---|---|---|---|
| 0.05 AU | full in 15 min | 30 min | 2.5 h |
| **0.1 AU** | **1 h** | 2 h | 10 h |
| Mercury, 0.39 AU | 15 h | 30 h | 7 days |
| Venus, 0.72 AU | 54 h | 4.7 days | 33 days |
| Earth, 1 AU | 4.5 days, +0.28 ME/yr | 9.6 days | 4 months |
| Mars, 1.52 AU | 11 days | 28 days | losing 7 ME/yr |
| Belt, 2.7 AU | 2 months | about 6 years | losing |
| Jupiter, 5.2 AU | losing 0.009 ME/yr | losing | losing |
| **break-even** | **3.9 AU** | **2.7 AU** | **1.2 AU** |

A 1 AU hop at 5 g costs the starting ship about 0.85 ME and takes 1.3 game days. Earning it
back takes 26 minutes at Mercury, 1.5 hours at Venus, 3 hours at Earth and 8 hours at Mars. A
full tank is about 35 such hops.

What it does to play:

- **Early flitting is easy, and refilling is the pressure.** The obvious refuel is a dive to
  about 0.1 AU: roughly 1 ME from Earth, then an hour.
- **Living drain matters at the edges.** The starting ship runs at a loss past 3.9 AU, a 1 km
  ship past the belt, a 5 km ship past Earth.
- **Crossings are gated by waiting.** A full-tank crossing at 0.46c is an hour at 0.1 AU or 15
  hours at Mercury, and the destination only refills a ship that dives into its star.

---

## Implementation

### Where it lives

| crate | change |
|---|---|
| `lc-world` | new `solar.rs`: the silhouette, the flux, and the power a craft collects over a segment of time. `fitting.rs`: `Balance` gains `solar_efficiency` and `solar_gain`; `Fitting` gains the settled `solar_w` and folds it into stored energy. `craft.rs`: settling works out the next segment's power. |
| `lc-proto` | `Balance` and `Fitting` gain their fields. `PROTOCOL_VERSION` 20 → 21, goldens regenerated. |
| `lc-server` | `persist.rs`: `SAVE_FORMAT` 4 → 5, with a format 4 reader. `Fitting` is written through the proto type and postcard is positional, so the new field shifts every byte after it. |
| `lc-client` | the refit panel and HUD show collection; nothing else changes, because the client folds the same account |

The geometry and flux live in their own module, so `fitting.rs` stays under the cap at 568 lines
of code.

### What is already there

- **Luminosity.** `LocalSystem::star_luminosity_w()` is the primary's, from the catalogue's radius
  and temperature. Sol comes out within a percent of 3.828 × 10²⁶ W, which a test in `star.rs`
  already pins.
- **Where the star is.** `LocalSystem::star_position_at(seconds)` answers for any coordinate time
  without touching the propagated arena. That is the reading `AGENTS.md` asks for.
- **Where the ship is.** `motion::state_at` (or `Craft::position_at`) is closed-form in time.
- **Which system.** `Craft::system` is `Some` inside a shell and `None` between them. Between
  systems there is no income. At 1.6 ly the flux from any star rounds to nothing anyway.
- **Whether it may collect.** `ShipState::is_under_way()` is false for exactly the three
  motives that collect: `Holding`, `Falling` and `Drifting`.

Positions are light-years in `f64`. A unit in the last place is about 8 m at four light-years
from the origin, which is nothing against a distance of 0.05 AU.

### Balance

```rust
pub solar_efficiency: f64,   // η, 0.7
pub solar_gain: f64,         // G, derived in DEFAULT from the anchor
```

`Balance::DEFAULT` derives the gain the way it already derives engine thrust: from named anchor
constants (`SOLAR_ANCHOR_AU = 0.1` and one Julian year), the starting loadout and
`SOLAR_CONSTANT_W_M2 = 1361`. Retuning is then a change to the anchor, not to a magic number.
The derivation uses the solar constant because luminosity is not `const`. At run time the power
reads the primary's own luminosity, so another star's system follows its star.

### The distance problem, and segments

Everything in the account so far is closed-form because every term is either constant between
settlements or read off a motive. **Solar is neither.** A ship holding an orbit about Mercury
sees its distance from the Sun swing by 40% over Mercury's year (eccentricity 0.2), and a
falling arc changes distance continuously.

Integrating `1/d(t)²` exactly would need the ship's and the star's position everywhere between
two readings. Instead:

- **Income is constant over a segment**, at the power the ship would collect at the segment's
  **midpoint**. Midpoint rather than start makes the error second order: a segment's worth of
  a changing distance averages out to first order.
- **Segments end on a fixed grid** of coordinate time, `SOLAR_STEP_S` = one game day (86 400 s,
  about 2.4 real seconds at the design rate), and at every change of motive, loadout or grant.
  Those are the moments the account already settles.
- The grid is **absolute**: boundaries sit at multiples of `SOLAR_STEP_S` since the world origin,
  not at offsets from when a ship arrived. Server and client therefore cut the same segments
  from the same state without being told.

At settlement `Fitting` records `solar_w` for the segment that begins. `stored_j_at` gains a term
for it, with the clamps the drain already has:

- **Net power** is `solar_w − drain_w`.
- **When net is positive** it fills towards capacity and stops there. The excess is lost, as a
  real collector with nowhere to put it would lose it.
- **When net is negative** it drains free energy only, down to zero, as the drain does now.

Within one day-long segment, clamping the total rather than the path is exact whenever the clamp
is not reached, and off by at most one segment's income when it is.

### Settling across boundaries

Settling a day at a time has a side effect on burns: the rest of a burn is re-priced at the
ship's mass after each day's drain, so a plan spends slightly less than it committed, and the
difference is refunded when it ends.

`Craft::advance` already settles every frame while a refit runs. It now also settles at each
grid boundary it passes, in order, and computes the next segment's power as it goes:

```
while next boundary ≤ now:
    fitting.settle(motion, boundary)          // closes the segment at its own power
    fitting.solar_w = power at midpoint of [boundary, min(next boundary, now)]
settle(now)
```

Power for a segment is computed from the motive in force, so a change of motive mid-segment
settles at the change, which `Craft::remembering` already does, and starts a new partial segment
from there.

**A long leap** — a checkpoint caught up after hours of downtime, a background browser tab — would
walk many boundaries. `persist::catch_up` already steps a restored craft in tick-sized steps and
then takes the remainder in one leap. For solar, a leap longer than `SOLAR_MAX_SEGMENTS`
boundaries uses segments of `k × SOLAR_STEP_S`, the smallest multiple that fits under the cap.
That is coarser but bounded, and only the authority ever makes such a leap. A client that leaps
is corrected by the next `Fitted`.

### Attitude

The accounting assumes broadside. The **drawing** does not show it yet. `hull::attitude` builds
the hull's orientation from the nose alone, with roll fixed by ecliptic north, so a ship in the
ecliptic with its nose along the orbit already has its short axis to the Sun, and one pointing
sunward does not.

Two ways to make the picture agree, both deferred:

- **Aim idle ships.** `motion::aim_at` returns an aim for `Holding`, `Falling` and `Drifting`
  that puts the nose perpendicular to the star. That changes `facing`, and so what other players
  see in a `Presence`, which is honest: a ship charging is a ship turned to its star.
- **Carry roll.** Add an up vector to the attitude model, so broadside can be any nose direction.
  A bigger change: `ShipState::attitude`, `Presence::facing` and the hull transform all assume a
  nose alone.

Neither affects income, and income does not wait for either.

### Refits

The planner in `refit.rs` does not count solar income. It stays conservative: a refit it accepts
can still be paid for if income stops, for example because the ship leaves the system. Income
while refitting fills storage as usual. A refund that would have fit without it can overflow
storage, in which case the excess is lost, the same rule as any full ship.

### Client

- **HUD:** net income beside the energy bar while collecting — `+4.20 ME/yr` — and nothing
  while under way. A new ship on the local shard starts 5 AU out and reads `-0.008 ME/yr`: it
  collects, but not enough to cover two living modules that far from the Sun.
- **Refit panel:** `solar` and `net` rows in both tables. The top table shows the segment in
  force. After shows what the draft's hull would collect holding still where the ship is now, so
  growing a hull or adding living space shows its effect on break-even before it is built.

### Tests

The geometry is checked independently of the formula it replaces:

- **Silhouette against the integral.** Sum `max(0, n·ŝ) dA` over a finely tessellated ellipsoid and
  compare it to `A(ŝ)` for many directions. Separately, check that the average over random
  directions is a quarter of the surface area.
- **Flux.** Sol at 1 AU gives 1361 W/m² to within a percent.
- **The anchor.** The starting ship, broadside at 0.1 AU around Sol, fills from empty in one Julian
  year to within 1%. Break-even distances for 500 m, 1 km and 5 km come out at 3.9, 2.7 and
  1.2 AU.
- **Rules:**
  - Nothing is collected under way.
  - Nothing is collected between systems.
  - Collection stops at capacity.
  - A negative net stops at zero.
- **Determinism.** A ship holding an eccentric orbit, stepped at a frame's rate and at a tick's
  rate, ends with the same stored energy. That mirrors the existing
  `the_ship_clock_does_not_depend_on_how_finely_time_is_stepped`.
- **Break it on purpose.** Sample at the segment's start instead of its midpoint and check that
  the eccentric-orbit comparison against a fine numerical integral gets measurably worse. A test
  that passes either way is not testing the midpoint.
- **Persistence.** A save in format 4 loads with no settled solar, and the next settlement starts
  collecting.

## Open

- **Heat.** The mechanic's missing counter-pressure. Closer is always better until it exists.
- **Multiple stars.** Only the primary is counted. A binary's companion contributes nothing.
- **Eclipses.** A planet between ship and star does not shade it. A ship in low orbit is in shadow
  for up to half of each orbit, which is a real effect and a small one against a segment a day
  long.
- **Collectors.** The structures [03-world-model.md](03-world-model.md) describes, and what this
  mechanic's waiting is meant to make worth building.
