# Directed energy

Engines, radios, weapons and power lines are one thing: energy sent in a chosen direction.

**Status: designed, not built.** This is the Kzinti Lesson (Niven): a reaction drive is a weapon
in exact proportion to how good a drive it is. Here the drive is a photon rocket
([19-ship-fitting.md](19-ship-fitting.md)), so the lesson is literal. What a ship sends out is
light, and light lands on someone. [29-the-field.md](29-the-field.md) is where the energy comes
from and where it goes when it arrives. [05-observation.md](05-observation.md) is the aiming and
the geometry, already built for radio in `lc_world::signal`.

## Energy moves only as light

Every source is an emitter and every ship is a receiver.

| emitter | aimed by |
|---|---|
| a star | nobody: isotropic |
| a field's glow | nobody: isotropic |
| a collapse | nobody: isotropic, and all at once |
| an engine | its ship |
| a radio dish | its ship |

There is one intake, the field ([29-the-field.md](29-the-field.md)), and it does not care which
kind of emitter the light came from. Solar collection, refueling from an ally, and being attacked
differ only in the source and the numbers.

## Apertures

**Engine volume is aperture**, rated at `engine_density_w` per cubic meter: for a photon drive, the
thrust it gives times c. The starting drive section, 1.96 × 10⁶ m³, is rated **1.1 × 10²⁰ W**,
which is its 5 g. The rating bounds three things at once:

- the drive's exhaust power
- anything the ship emits on purpose
- conversion into storage, by reciprocity ([29-the-field.md](29-the-field.md))

An engine part's **aperture diameter** is the width of its open face: a frustum's wide end, a
cylinder's end. That sets the diffraction floor, so a broad drive section beams tighter than a
narrow one.

**Engine parts point fore or aft** ([28-ship-form.md](28-ship-form.md)):

- **Aft engines drive.** Acceleration is aft thrust over mass. Only aft engines count toward the
  rated acceleration.
- **Fore engines emit ahead.** They push the ship backward when used.
- **Fore and aft together, at equal power, emit with no net thrust.** A ship with engines at both
  ends can dump heat or beam power while holding station.

Moving engines forward buys aim and costs acceleration. That is a choice about what the ship is
for.

## The drive is the radiator

Heat is energy, and a photon rocket's exhaust can be any energy. So **while the drive is lit, its
exhaust comes from field heat first, and from storage only for what heat cannot supply.**

- A ship under way sheds heat as it goes. Flying cools. At 5 g the starting ship's exhaust is
  1.1 × 10²⁰ W, which empties a full field (10 ME) in about fifteen game days.
- **Heat is mass**, as stored energy is. `mass_kg_at` counts `Q / c²`, and the rocket law is
  unchanged. Only the source of the exhaust changes.
- **The commitment is unchanged.** A plan commits against storage when accepted.
  Whatever heat supplies instead is refunded at settlement, through the same refund path that
  `CutDrive` uses. A plan's cost can only come out lower than it said.
- The field's account gains a constant sink while lit: `dQ/dt = P_in − Q/τ − P_exhaust`, floored
  at zero. It is still closed form, with one more split where `Q` reaches the floor.

### Exhaust lands on whatever is behind

The exhaust is an emission like any other, so **it heats every craft in its cone.** The drive is not
focused: it spreads at `drive_spread_rad`, a fixed half-angle wider than any deliberate beam needs,
and focusing is what `Order::Emit` is for. The received power follows the beam formula below.

Where a full receiver, broadside to the drive, would sit exactly at its rated load, at the default
5°:

| burning ship, at 5 g | exhaust | cooking distance |
|---|---|---|
| 500 m | 1.1 × 10²⁰ W | 2.7 km |
| 5 km | 1.1 × 10²³ W | 84 km |
| 50 km | 1.1 × 10²⁶ W | 2 700 km |

A receiver's shadow and its rated load both scale with its size squared, so **the distance does not
depend on the receiver's size**. The same is true of a collapse's lethal radius. Inside that
distance, a receiver with nowhere to convert the heat walks up to collapse on the field's time
constant, a few real minutes.

What follows:

- **Every burn points at something.** A ship braking into a rendezvous points its exhaust at it. A
  starting ship's exhaust is safe for a companion at the usual 5 km standoff. A GSV leaving at 5 g
  cooks anything within 84 km behind it. Approach paths matter, and crowded space needs rules, which
  gives factions something real to legislate.
- **Empty storage helps here too.** Exhaust arrives as sustained power, so a receiver with room
  converts it up to its rating, and being behind an ally's drive can refuel you.
- **Delivery** is the fan-out below, with the burn as a continuous emission between ignition and
  cutoff. A receiver's heat input changes at the retarded times of those two events. Candidate
  receivers are only those within the distance where the flux falls to a millionth of the cooking
  flux, a thousand times the cooking distance.
- **The drawn plume's flare is `drive_spread_rad`**, so what a player sees is the cone that hurts.

## Emitting on purpose

`Order::Emit`:

| field | meaning |
|---|---|
| `aim` | `lc_proto::Aim`, as radio already uses: a direction, or a craft, which the server leads |
| `apertures` | fore, aft or both |
| `power_w` | at the start. At most the chosen apertures' rating |
| `wavelength_m` | anything from the radio dish's 3 cm down to 1 nm |
| `spread_rad` | the half-angle. **At least the diffraction floor** `λ / D`, and wider on request |
| `duration_s` | how long |

- **Source: heat first, then storage.** Emitting from heat is dumping. Emitting from storage is
  spending.
- **Recoil always points away from the beam.** Beaming anyone pushes you away from them.
- **With net thrust, an emit is a burn.** It is flown as a boost segment at constant proper
  acceleration, `P / (m c)`, with power falling as mass falls, exactly as the drive throttles now.
  It is refused while under way and refuses other plans while it runs, as burns do. A ship that
  must face a target turns to it first, at its slew rate.
- **With no net thrust** (fore and aft balanced), it is not a burn and may run while holding
  station, falling or drifting.
- It is refused during a refit, as every order that lights the drive is.

### Spread, and why it is a choice

The diffraction limit is only a floor. Against something that moves, a beam narrower than your
uncertainty about where the target will be is a beam that misses.

Where a target will be is predicted from light that is itself old ([05-observation.md](05-observation.md#aiming)).
A target free to change its thrust at `a` could be anywhere within **½ a (2d/c)²** of the
prediction by the time the beam arrives:

| distance | a 5 g target could be off by |
|---|---|
| 1 light-second | 100 m, a hull length: a tight beam hits |
| 10 light-seconds | 10 km |
| 1 light-minute | 350 km |

So **effective range comes from acceleration and size**, and nobody sets it. A small ship that
keeps changing its thrust is hard to hit beyond a few light-seconds. A GSV at a fraction of a g is
a target from much farther off. Widening the beam covers the uncertainty and spreads the power
over the area it covers.

Against a target that **shares its course**, the uncertainty is the course's own precision, and a
tight beam hits at any range. Communication is what makes long-range power beaming work.

### What arrives

The spot at distance `d` has radius `spread × d`. The receiver takes the fraction of it that its
shadow toward the emitter covers, at most all of it:

**P_received = P · min(1, A_shadow / (π (spread · d)²))**

Taking a 100 m aperture at the diffraction floor, the fraction a receiver collects:

| distance | wavelength | spot | 500 m ship | 5 km | 50 km |
|---|---|---|---|---|---|
| 1 AU | 1 µm | 1.5 km | 0.07 | 1 | 1 |
| 1 AU | 1 nm | 1.5 m | 1 | 1 | 1 |
| 100 AU | 1 µm | 150 km | 7 × 10⁻⁶ | 7 × 10⁻⁴ | 0.07 |
| 100 AU | 1 nm | 150 m | 1 | 1 | 1 |
| 4 ly | 1 nm | 380 km | 10⁻⁶ | 10⁻⁴ | 0.01 |

**Between stars, only the largest receivers are worth beaming to.** A GSV is an interstellar power
station's natural customer, and a probe is not.

**Every craft inside the cone receives**, not only the one aimed at. A beam is an event like a
transmission, and fans out the way [02-event-store.md](02-event-store.md) fans out radio: to every
worldline that crosses it. Fly through someone's power line and you are fed or cooked. Beams
become places, and places attract piracy, rules and factions.

**Nobody outside the cone sees a beam.** Vacuum scatters nothing. An observer inside it sees the
emitter as an extraordinarily bright point in the beam's band. That is both the stealth of a heat
dump aimed at empty space and its risk: it is a searchlight to anyone who happens to be downrange,
now or years from now.

## Three uses of one order

### Dumping heat

Aim at nothing, at whatever wavelength is tightest, from heat. The field cools at the emitted
power on top of what it radiates. A ship with engines at both ends dumps without moving. A ship
with engines only aft moves.

The shortest wavelength is the quietest. At 1 nm from a 100 m aperture, the beam is 10⁻¹¹ radians
wide. At four light-years that is a spot 380 km across. Unless somebody is in it, nobody ever
knows.

### Feeding an ally

Aim at their craft, with their course known. What arrives is converted at their rating and their
efficiency, while their storage has room. Past that it is heat.

- **The line between feeding and attacking is the receiver's conversion rating.** An ally who
  beams you harder than you can convert is hurting you, and **nobody can refuse light.** Being
  beamed is an act of trust.
- **Refueling takes time.** A supply ship fills you only as fast as you convert.
- The emit panel shows what you know of the receiver's rating and room, and warns when the beam
  would exceed them.

### Attacking

Aim at someone who did not agree to it. Nothing further is needed. What decides it:

| | attacker wants | defender wants |
|---|---|---|
| power | more than the defender's rated load, after conversion | rating and empty storage to convert it |
| range | close, where the lead uncertainty is small | far, and unpredictable |
| shadow | a broadside target | nose-on to the threat |
| headroom | a hot target | a cold field |

**Recoil decides how an attack is held.** A starting ship's full rating is its 5 g, so an attacker
beaming from its aft engines is flying away from its target at 5 g for as long as it fires. To
hold range it must balance: fore engines on the target, the same power out of the aft engines
into empty space. **Holding range costs twice what lands.**

An attacker with engines at both ends, delivering 1.1 × 10²⁰ W onto a starting ship from 1
light-second:

| defender | outcome |
|---|---|
| storage empty | converts it all. 30% of it is heat, 3.3 × 10¹⁹ W, under the rated load. It fills in 64 game days, about 11 real minutes, and only then starts to die |
| storage full | all 1.1 × 10²⁰ W is heat. Collapse in about 25 game days, 4 real minutes |
| empty, two such attackers | 1.1 × 10²⁰ W over the rating becomes heat outright. Collapse in about 16 days |

Killing a full ship that way costs a balanced attacker about 34 ME, which is more than a starting
ship holds, so a lone ship cannot beam a full peer to death. **Numbers win by focusing fire:**
rates add at the receiver, the defender's outflows do not grow, and each attacker pays only its
own share.

**A beam arrives with its own warning.** It travels at the speed of the light that would have
announced it. Defense is posture beforehand, never reaction.

## Radio

A radio transmission is an emit through the **comms dish**, `Transmitter::SHIP`: 30 m at 3 cm. That
is a separate aperture, not an engine, because a dish that talks should not also be a drive. Its
power is `SIGNAL_POWER_W`, a megawatt. Its recoil is nothing. It is charged from storage: a year of
transmitting is about 2 × 10⁻¹² ME.

`lc_world::signal`'s `Beam` and `Transmitter` are the geometry for all of it. What `Order::Emit`
adds to radio is power large enough to matter and no message.

## The star's gain

**`solar_gain` is on the star's energy output.** A star delivers `G · L★ / (4π d²)` of game energy
per unit area. Nothing downstream multiplies again: not conversion, not beams, not a collapse.

That gives starlight two faces. **An instrument sees `L★`.** A field receives `G · L★`. It is the
same split the field has between `Q / τ` and σT⁴A ([29-the-field.md](29-the-field.md)). Engines and
collapses have no gain. What they carry is mass-energy, and it is physical in both faces.

What a Dyson swarm re-beaming starlight carries, gained or physical, is deferred with the swarms.

## Balance

| setting | first guess | meaning |
|---|---|---|
| `drive_spread_rad` | 5° | the drive's exhaust half-angle |

## Protocol

- `Order::Emit` as above. `Order::Transmit` keeps its message fields and is charged.
- `Refusal::NoAperture` (both apertures asked of a ship with engines at one end only),
  `Refusal::OverRating`, and the existing `UnderWay` and `Refitting`.
- `Outbound::Illuminated`: a beam arriving, with its bearing, band and power, **when its light
  lands**. That is the only way a receiver learns of it. A receiver knows the bearing because the
  light came from there.
- `Presence` of an emitter seen from inside its cone carries its brightness in that band.

## Client

- **Emit window**, on `E`, with the same toolkit and rules as the other windows. Aim at a known
  craft, at the reticle, or along a bearing. Before sending, it shows:
  - the diffraction floor, the chosen spread, and the spot at the target
  - the **lead uncertainty**, from how old your knowledge of the target is and what it can
    accelerate at, drawn beside the spot, so a miss is visible before it is sent
  - the fraction expected to arrive, and what that does to the receiver if its rating is known
  - recoil, cost, and how much comes from heat and how much from storage
- **Incoming**, in the same window: every beam landing now, by bearing, band and power, and what
  it is doing to your field.
- **Map**: your own beams as cones, and received beams as bearing lines from where they came. A
  beam you are not in and did not send is not drawn, because you do not know about it.
- **The plume** brightens under a dump. A fore emission lights the bow. How that is drawn is in
  [31-ship-rendering.md](31-ship-rendering.md).

## Where it goes

| crate | new | changed |
|---|---|---|
| `lc-world` | `emit.rs`: the aperture rating, received fraction, lead uncertainty, the emit as a boost segment | `signal.rs` generalizes to any wavelength and aperture. `cost.rs` draws exhaust from heat first. `fitting.rs` counts heat in mass. Rated acceleration counts aft engines only |
| `lc-proto` | `Order::Emit`, `Outbound::Illuminated`, the refusals | `Presence` |
| `lc-server` | `emit.rs`, beside `radio.rs`: fan-out to every worldline in the cone, delivery at the retarded time, heat into the receiver's account | `radio.rs` charges transmissions |
| `lc-client` | `emit_panel.rs` | map overlays, plume, HUD |

## Tests

- **Exhaust from heat.** A burn from a hot ship refunds what heat supplied, and the ship ends
  colder, lighter and where the plan said.
- **Recoil.** A balanced emit leaves the worldline untouched. An aft-only emit moves the ship
  along `−aim` by the rapidity the rocket law gives for the energy.
- **Reciprocity.** A beam at exactly the receiver's rating, into empty storage, adds only the
  conversion loss as heat. At twice the rating, the excess is heat outright.
- **Fan-out.** A second craft inside the cone is fed. A third just outside it is not. Break the
  cone test on purpose and watch the fan-out test fail.
- **Light delay.** No receiver learns of a beam before its light lands, and a beam aimed at where
  a target was, when the target has since maneuvered, misses.

## Open

- **Direction of intake.** Whether catching a beam means pointing apertures at it, per
  [29-the-field.md](29-the-field.md).
- **Wavelength has no cost.** A 1 nm beam is always at least as good as a longer one against a
  cooperative target. A conversion efficiency that depends on wavelength would make it a trade.
- **Occlusion.** A planet between emitter and receiver does not block a beam yet, as it does not
  shade starlight.
- **Dyson swarms**, and whether their beams carry the star's gain.
