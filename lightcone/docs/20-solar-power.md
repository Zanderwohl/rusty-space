# Solar power

Every hull is covered in collectors, so a ship earns energy from starlight when it holds still
near a star.

**Status: built.** `lc_world::solar` reads the form's shadow table and the flux; `Craft` walks the
segments and turns an idle ship's broadside to its star. `solar::intake_w` is what *arrives* at
the field ([30-the-field.md](30-the-field.md)), `G · L★ / (4π d²) · A(ŝ)`,
and `η` and the engines' rating are applied where it is converted: the `P` below is what storage
takes in while it has room and the rating is not reached. The first half of this doc is the
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
- **Size cuts both ways.** Collecting area goes as the square of hull length, while volume, mass,
  storage and living drain go as the cube. A bigger ship collects more in total and less per
  cubic meter, and has to be closer in to break even.
- **Heat bounds how close.** What arrives and is not stored is heat in the field, and a field
  that fills collapses ([30-the-field.md](30-the-field.md)). A full starting ship broadside is at
  its rated load at 0.05 AU, and a filling one can go closer.

### Collecting area is the hull's shadow

Panels over the whole hull deliver `η · flux · ∫ max(0, n·ŝ) dA`, where `n` is a surface normal
and `ŝ` the direction to the star. For any convex body that integral **is the projected area**,
the shadow the hull casts along `ŝ`. A form is not convex, and its projection is the better
number: a stack of plates shades itself, and the integral would count every plate.

So collection reads the form's **shadow table**, [29-ship-form.md](29-ship-form.md)'s projection of
its grid along 162 directions, interpolated between them. The direction is the star's in the ship's
own frame, in the attitude it holds. The table is cached with the form and measured again only when
the form changes.

For the starting form, 731 m long:

| attitude to the star | area | relative to broadside |
|---|---|---|
| **broadside** | 7.78 × 10⁴ m² | 1 |
| averaged over every orientation | 6.14 × 10⁴ m² | 0.79 |
| side-on, star along the beam | 5.50 × 10⁴ m² | 0.71 |
| nose-on, star along the nose | 2.63 × 10⁴ m² | 0.34 |

**Shape now pays.** A plate collects half as much again as a spindle of the same volume
(1.27 × 10⁵ m² against 8.5 × 10⁴ m²), so the choice between filling fast and turning fast is a
choice of form.

### Power

**P = G · η · L★ / (4π d²) · A(ŝ)**

| symbol | meaning | value |
|---|---|---|
| `η` | `conversion_efficiency` | 0.7 |
| `G` | balance gain | 1.79 × 10⁹, set by the anchor below |
| `L★` | the system primary's luminosity | the Sun: 3.83 × 10²⁶ W, which is 1361 W/m² at 1 AU |
| `d` | distance from the primary | |
| `A(ŝ)` | the shadow toward the star | from the form's table |

`G` is unphysical and says so. A module-energy is `mc²` for 155 000 tonnes, which is 0.036
seconds of the Sun's entire output. At a gain of one, the starting ship at 1 AU would take about
six billion years to collect one. The gain keeps the physics' shape — silhouette, inverse square,
the star's own luminosity — and chooses the scale.

### The anchor

> **The starting ship, broadside at 0.1 AU from a Sun-like star, fills its storage from empty
> in one game year, net of its living drain.**

A game year is a real hour at the design rate, so every time below is also real time.

The columns are the starting form, 30 ME of storage and a slot's volume of living space, and the same
form two and ten times as long in every part but the Mind (`default*2` and `default*10`).

| | 731 m (30 ME) | 1.46 km (240 ME) | 7.3 km (30 000 ME) |
|---|---|---|---|
| 0.05 AU | full in 15 min | 30 min | 2.5 h |
| **0.1 AU** | **1 h** | 2 h | 10 h |
| Mercury, 0.39 AU | 15 h | 31 h | 6.7 days |
| Venus, 0.72 AU | 53 h | 4.5 days | 26 days |
| Earth, 1 AU | 4.3 days, +0.29 ME/yr | 8.9 days | 2 months |
| Mars, 1.52 AU | 10 days | 23 days | 14 months |
| Belt, 2.7 AU | 40 days | 4 months | losing 5.9 ME/yr |
| Jupiter, 5.2 AU | 3 years, +0.001 ME/yr | losing 0.04 ME/yr | losing |
| **break-even** | **5.5 AU** | **3.9 AU** | **1.7 AU** |

The larger hulls are the starting form scaled, so 5% of each is living space and its shadow goes
as the square of its length: each fills in about the same time close in, where the drain is
nothing against the income, and breaks even closer to the star.

A 1 AU hop at 5 g costs the starting ship about 0.85 ME and takes 1.3 game days. Earning it
back takes 26 minutes at Mercury, 1.5 hours at Venus, 3 hours at Earth and 7 hours at Mars. A
full tank is about 35 such hops.

What it does to play:

- **Early flitting is easy, and refilling is the pressure.** The obvious refuel is a dive to
  about 0.1 AU: roughly 1 ME from Earth, then an hour.
- **Living drain matters at the edges.** The starting ship runs at a loss past 5.5 AU, just
  beyond Jupiter, a ship twice its length past the belt, and one ten times its length past Mars.
- **Crossings are gated by waiting.** A full-tank crossing at 0.46c is an hour at 0.1 AU or 15
  hours at Mercury, and the destination only refills a ship that dives into its star.

---

## Implementation

### Where it lives

| crate | what |
|---|---|
| `lc-world` | `solar.rs`: the shadow toward the star, the flux, and what arrives over a segment of time. `fitting.rs`: `Balance` carries `conversion_efficiency` and `solar_gain`; `Fitting` holds the settled segment's intake and folds it into the account. `craft.rs`: settling works out the next segment's power |
| `lc-proto` | `Balance` and `Fitting` carry their fields |
| `lc-server` | a checkpoint keeps the settled segment with the account |
| `lc-client` | the ledger and the HUD show collection; the client folds the same account |

### What it reads

- **Luminosity.** `LocalSystem::star_luminosity_w()` is the primary's, from the catalog's radius
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
pub conversion_efficiency: f64,   // η, 0.7; see 30-the-field.md
pub solar_gain: f64,         // G, derived in DEFAULT from the anchor
```

`Balance::DEFAULT` derives the gain the way it already derives engine thrust: from named anchor
constants (`SOLAR_ANCHOR_AU = 0.1` and one Julian year), the starting form's storage and drain, its
broadside and `SOLAR_CONSTANT_W_M2 = 1361`. The broadside needs the grid, which is not `const`, so it
is pinned as `STARTING_BROADSIDE_M2` by a test that solves the starting form again. Retuning is
then a change to the anchor, not to a magic number. The derivation uses the solar constant because
luminosity is not `const`. At run time the power reads the primary's own luminosity, so another
star's system follows its star.

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
  about 2.4 real seconds at the design rate), at every change of motive or grant, and where a
  refit step begins or ends.
  Those are the moments the account already settles.
- The grid is **absolute**: boundaries sit at multiples of `SOLAR_STEP_S` since the world origin,
  not at offsets from when a ship arrived. Server and client therefore cut the same segments
  from the same state without being told.

At settlement `Fitting` records what arrives for the segment that begins. The field's account
takes it from there ([30-the-field.md](30-the-field.md#conversion)): it is converted into storage at
`conversion_efficiency`, up to the engines' rating and while storage has room, and the rest is
heat. Storage filling partway through a segment splits it in closed form, so nothing is clamped
after the fact.

### Settling across boundaries

Settling a day at a time has a side effect on burns: the rest of a burn is re-priced at the
ship's mass after each day's drain, so a plan spends slightly less than it committed, and the
difference is refunded when it ends.

`Craft::advance` settles at each grid boundary it passes, in order, and computes the next
segment's power as it goes:

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

An idle ship turns the largest shadow its form casts to the star: [29-ship-form.md](29-ship-form.md)'s
broadside, with its roll. The picture agrees, and collection reads the attitude rather than assuming
it, in two halves.

**The roll.** Every hull rolls about its nose so that its broadside's part across the nose faces
the star: `hull::frame` rolls the height axis toward the star, then `solar::roll_rad` further, the
broadside's azimuth about the nose. That is not 29's `broadside_roll_rad`, which is the best roll
for a nose held square: a broadside that leans off square can lie at another azimuth altogether,
and a slab pitched nose-up whose beam beats its height would lose a quarter of its collection to
it. Roll about the nose changes no thrust, so every hull does it, under way or
not. With it, where the star lies in the ship's frame is fixed by one angle, the nose's to the star,
and `solar::toward_star` is that direction. With no star, or one along the nose where the roll
toward it is undetermined, it falls back to ecliptic north. A craft with no form is an ovoid, whose
broadside is its height axis, rolls by nothing further.

Rolling was not optional. The height axis used to be ecliptic north with the nose taken out, and a
star in the ecliptic can never lie along that, so a ship in the plane could not be drawn broadside
at all.

**The nose.** `Craft::facing_at` turns a fitted craft with no plan and a star to look at onto the
nearest heading at the broadside's angle to that star, at its hull's own slew rate from the attitude
its last order left it with. For the starting form and the plate that is within a degree of
perpendicular; a cluster, whose shadow is nearly as large from any side, leans its nose 50° toward
the star. A craft that has been idle since before anything is already there, having had
forever to turn. A ship given an order starts the plan's turn from wherever the broadside turn had
got to, which is what `Craft::remembering` already hands over. Nothing unfitted turns: a probe
keeps the attitude it was left with.

That changes what a `Presence` carries, which is honest — a ship charging is a ship turned to its
star — and it costs no protocol change, because `facing` is already on the wire.

A segment is priced at the attitude its midpoint holds, so a ship still turning back from a burn
collects what it presents, not what it will. The turn is minutes and a segment a day, so this is
small; it is there because the attitude is already closed form and assuming it would be one more
thing server and client could disagree about.

### Refits

The planner in `refit.rs` does not count solar income. It stays conservative: a refit it accepts
can still be paid for if income stops, for example because the ship leaves the system. Income
while refitting fills storage as usual. A refund that would have fit without it can overflow
storage, in which case the excess is lost, the same rule as any full ship.

### Client

- **HUD:** net income beside the energy bar while collecting — `+4.20 ME/yr` — and nothing
  while under way. A new ship on the local shard starts 5 AU out and reads `+0.002 ME/yr`: that far
  from the Sun it collects barely more than its living part drains.
- **The ledger:** `solar` and `net` rows, for the segment in force.
- **The editor's preview:** the starlight the draft would collect holding still where the ship is
  now, so a change of shape or of living space shows its effect on break-even before it is built
  ([29-ship-form.md](29-ship-form.md#what-else-it-shows)).

### Tests

- **The table** is 29's, checked there against the ovoid's analytic shadow, which is itself checked
  against the integral of the lit surface over a finely tessellated ellipsoid.
- **Idle presents the broadside.** For the starting form and every preset, the shadow toward the
  star at the idle attitude is the broadside to within 1%, and the drawn frame puts the star where
  `solar` says it is in the ship's frame.
- **Shape pays.** A plate collects more than a spindle of the same volume.
- **Flux.** Sol at 1 AU gives 1361 W/m² to within a percent.
- **The anchor.** The starting ship, broadside at 0.1 AU around Sol, fills from empty in one Julian
  year to within 1%: on the table at its idle attitude, at the broadside direction, and at
  directions two degrees off it, which fall between the table's vertices. Flown, it is half full
  after half a year. Break-even distances for the three hulls above come out at 5.5, 3.9 and
  1.7 AU.
- **Rules:**
  - Nothing is collected under way.
  - Nothing is collected between systems.
  - Storage stops at capacity, and what arrives past it is heat.
- **Determinism.** A ship holding an eccentric orbit, stepped at a frame's rate and at a tick's
  rate, ends with the same stored energy. That mirrors the existing
  `the_ship_clock_does_not_depend_on_how_finely_time_is_stepped`.
- **Break it on purpose.** Sample at the segment's start instead of its midpoint and check that
  the eccentric-orbit comparison against a fine numerical integral gets measurably worse. A test
  that passes either way is not testing the midpoint.

## Open

- **Multiple stars.** Only the primary is counted. A binary's companion contributes nothing.
- **Eclipses.** A planet between ship and star does not shade it. A ship in low orbit is in shadow
  for up to half of each orbit, which is a real effect and a small one against a segment a day
  long.
- **Collectors.** The structures [03-world-model.md](03-world-model.md) describes, and what this
  mechanic's waiting is meant to make worth building.
