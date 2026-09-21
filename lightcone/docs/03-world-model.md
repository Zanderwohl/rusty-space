# World model

## Hierarchy

```
World
 +- System            one star or multiple, plus everything inside its Oort shell
 |   +- Body          star, planet, moon, asteroid, comet  (em_sim::System)
 |   +- Structure     telescope, refinery, habitat, swarm element, transmitter
 |   +- Ship          player- or AI-controlled, may leave the shell
 +- Interstellar      ships and structures between shells
```

`System` here wraps `em_sim::system::System`, which is already the single source of truth
for body state in Exotic Matters. The same rule applies: nothing per-body is duplicated
outside it. Structures and ships are a separate table keyed by the body they orbit.

## The Oort shell

Each system has a radius, the **shell**, beyond which it is treated as a point source. The
shell does three jobs:

1. **Emission boundary.** The star's directional emission map (see
   [04-stellar-photometry.md](04-stellar-photometry.md)) is defined on this sphere. Light
   crossing it outward carries whatever the system's contents did to it.
2. **Simulation boundary.** Everything inside is one simulation domain with one writer.
   Domains interact only through light crossing the shell and objects crossing the shell.
3. **Coordinate boundary.** Local f64 meters are valid inside the shell and only inside it.

Sizing. The shell sits outside the system's outermost population — in the solar system's
case the Oort cloud, at roughly 1e5 AU = 1.58 ly, against a nearest star 4.25 ly away, so
shells at that radius do not intersect for typical spacings. Shell radius
scales with the star's mass and is clamped so that no two shells overlap: if two stars are
closer than the sum of their nominal radii, both shrink to half the separation, or they are
merged into one multi-star system.

Crossing the shell is an event. That is the natural place to cut the world:

| crossing | becomes |
|---|---|
| ship outbound | an event at the shell; the ship's worldline reparents to the interstellar frame |
| ship inbound | an event at the shell; reparents to the system's local frame |
| signal outbound | an emission on the shell with a direction and a beam pattern |
| signal inbound | a reception at the shell, propagated inward in local coordinates |

Because everything crossing is an event on a known surface, a system can be simulated,
paused, archived, or moved to another server process without any other domain noticing
beyond the latency of the crossing.

## Multi-star systems

**Decided: a binary is a barycenter with two children.** The barycenter carries the system,
and each star is a body orbiting it, which is a hierarchical two-body decomposition that
`em-sim` already propagates without modification. Planets orbit either a star (S-type) or the
barycenter (P-type), and the same decomposition covers both.

Consequences worth planning for:

- One shell per system, not per star. The shell radius is sized from the total mass.
- Two emission sources on that shell. Radiance is the sum, and it is not axisymmetric about
  anything, so a binary's shell has real angular structure independent of any occluder.
- **Mutual eclipses are coherent and periodic**, so they take the analytic path of
  [04-stellar-photometry.md](04-stellar-photometry.md), and they are enormous — an eclipsing
  binary dims by a fraction of order unity where a planet dims by 1e-4. Any observer at a
  suitable angle gets the orbital period, the mass ratio and the inclination for free, long
  before they could detect a planet. This is the strongest photometric signal in the game and
  it comes from real catalogue data.
- Close binaries whose separation is comparable to their radii need more than two-body
  Keplerian motion. Exclude contact and near-contact systems from generation rather than
  modeling them.

## Systems and star data

### The catalogue is test data, not the world

The shipped game is set in a **fictional galaxy** with authored features — globular clusters,
stellar nurseries, structures chosen for play rather than inherited from the sky. Real star
data is how the physics gets validated, not what the game ships.

That has one architectural consequence, and it is cheap now and expensive later:

- **Star data comes through a provider interface from the start.** `HygCatalogue` is one
  implementation; `AuthoredGalaxy` and `ProceduralGalaxy` are others. Nothing in world
  generation parses a CSV directly.
- **Star identity is a synthetic stable ID, never a catalogue ID.** An HYG number must not
  reach `source_id`, the event store, or the wire format. The importer assigns IDs; the
  catalogue's own numbers survive only as a provenance field.

Neither is work. Both are migrations if skipped.

The features an authored galaxy adds are mostly additive, with two that touch existing
assumptions and should be watched rather than solved now:

| feature | what it stresses |
|---|---|
| globular cluster | shell overlap. 1e5 stars in 10 pc gives a mean separation near 1 ly, against a nominal shell radius of 1.58 ly, so the clamp runs constantly and each shell touches many neighbors |
| stellar nursery | dust that belongs to no system. The occlusion model puts populations inside a shell; extended interstellar dust needs path extinction instead, which is a different calculation |
| authored structures | generation is currently seeded and deterministic; authored content is neither, so both paths must coexist |

Plotting the filtered catalogue as an HR diagram is the cheapest check that the import is
right — see [11-plotting.md](11-plotting.md). If the distance sentinel were not cut, the
main sequence would not appear.

### Source data

The validation catalogue is HYG v4.2, already in `assets/catalogs/hygdata_v42.csv` and read by
`src/catalog/`. It supplies position (RA/Dec/distance), spectral class, absolute magnitude,
and proper motion for ~120 000 stars. Positions convert to the ecliptic frame the sim
already uses; note the catalogue is equatorial and the existing conversion lives in the
star-field path.

Per-star derived data, generated once at world creation:

| field | from |
|---|---|
| mass, radius, effective temperature | spectral class lookup, `src/catalog/spectral.rs` |
| luminosity | radius and temperature, Stefan-Boltzmann |
| shell radius | mass, clamped against neighbors |
| planet set | procedural, seeded by the star's catalogue ID |

Planet generation is procedural and deterministic from a seed, so the same world ID always
produces the same system, and a client can generate a system locally rather than downloading
it. Generation must produce Keplerian elements that `em_sim` can propagate directly — the
output format is `em_sim::system::BodyDef`, not a bespoke one.

## Bodies

Reuse `em-sim` unchanged. A body is elements plus physical parameters; its state at any
coordinate time is analytic. The game adds:

| addition | purpose |
|---|---|
| composition | what mining yields |
| surface and atmosphere flags | whether landing, refining, habitation are possible |
| occluder role | radius and albedo, for the transit model |
| internal heat | radiated power over absorbed, which is 1 for everything that is not a giant |

Composition is generated with the system and does not change. Depletion is per-deposit
state, which does change, and is therefore event-backed.

## Structures

A structure is fixed relative to a parent body, or in its own orbit. All structures share:

| field | meaning |
|---|---|
| owner | player or faction |
| parent | body or orbit |
| state | building, active, damaged, derelict |
| proper time accumulator | drives its process rates |

Kinds, and the one property of each that matters to the physics rather than the economy:

| kind | physical property |
|---|---|
| telescope | aperture (sets SNR), band, pointing |
| transmitter | power, beam pattern (isotropic to tight) |
| receiver | effective area, noise floor |
| solar collector | area, distance from the star, occlusion it casts |
| refinery, fabricator | none; pure economy |
| swarm population | a distribution over orbital elements, not a roster; see below |
| ringworld, shell segment | large fixed occlusion over a solid angle |

Solar collectors, swarm elements and megastructures are the same thing to the light model:
area that blocks a fraction of the star's output in some set of directions. That is what
makes a rival's industry detectable from another system.

### Populations

A population is the model for any group of objects too numerous to track and statistically
steady in aggregate. Three come with every generated system, before a player builds
anything:

| population | typical parameters | photometrically |
|---|---|---|
| asteroid belt | narrow `a`, low inclination spread, moderate `e` | marginal |
| Kuiper analogue | wide `a`, low inclination, many small bodies | near the detection floor |
| Oort cloud | very wide `a`, isotropic inclination, `e` near 1 | invisible |

**Decided: population mass scales with stellar generation and metallicity.** A later-generation
star formed from enriched gas has more solid material available, so it gets more massive belts,
a denser Kuiper analogue, and richer volatiles. A Population II star gets almost nothing.

The catalogue does not carry `[Fe/H]`, so metallicity is synthesised rather than read: seed it
from galactic position — thin disc, thick disc, halo — plus the star's kinematics, which HYG
does carry as proper motion and radial velocity. Halo stars move fast relative to the local
standard of rest and are metal-poor; that correlation is strong enough to generate from and it
costs nothing. This also makes metal-rich systems worth traveling to, which is a resource
gradient derived from real data rather than sprinkled on top.

The Oort cloud earns its record for reasons that have nothing to do with light. It is
invisible in transit — a mean deficit of 2.8e-14, see
[04-stellar-photometry.md](04-stellar-photometry.md) — but it defines the shell radius, it
is a navigation consideration for anything crossing it, and it is where the volatiles are.
A player harvesting comets is harvesting a distribution: extraction reduces the population's
count, which is an event, and nothing is ever enumerated.

Player-built swarms use the same record with different parameters. So does dust, so does
debris from anything destroyed. One type, one integral, one storage cost.

### Swarms are populations, not entities

A swarm of a million collectors is stored as **one** record: a distribution over semi-major
axis, eccentricity and inclination, a pole, an element count, and a cross-section. The
orientation angles — node, argument of periapsis, mean anomaly — are not stored, because
assuming them uniform is what makes the swarm statistically steady and turns its occultation
into a closed-form integral. See [04-stellar-photometry.md](04-stellar-photometry.md).

The element count is an `f64` and may be fractional. Nothing enumerates the members.
Construction adds to the count and to the cross-section; losses subtract. Both are events.

An element leaves the population only when a player selects it for something specific — a
maneuvere, a transfer, a detachment — at which point it becomes a tracked body until it
rejoins. Bulk operations on a swarm are operations on the distribution's parameters, so
reconfiguring a million collectors is one event carrying a new inclination spread, not a
million events.

This is a hard rule, not an optimization. A design where a player can address individual
swarm members as entities has an unbounded object count, an unbounded source table, and an
observation cost that scales with someone else's industry.

## Ships

One ship per player is the player's viewpoint. Additional ships are autonomous: they carry
an order set, plot their own trajectory, and execute without further input, because the
round-trip latency to another system makes direct control impossible.

| field | notes |
|---|---|
| worldline | piecewise analytic arcs, see [01-spacetime.md](01-spacetime.md) |
| proper time | accumulated, drives onboard rates |
| delta-v budget, thrust, mass | standard rocket parameters |
| orders | a program, not a queue of clicks |
| known-world snapshot | what this ship has actually observed |

The **known-world snapshot** is the important one, and it now has a document of its own:
[22-provenance.md](22-provenance.md), which is where records, lineage and what a parallax costs
are written down. A ship far from its owner has a different
view of the universe than the owner does. When it reports back, the report is itself a
signal subject to delay, so the owner learns what the probe knew at the probe's emission
time, not what the probe knows now. Two ships can hold contradictory and simultaneously
correct beliefs about a third system. The data model must permit that: there is no global
"current state of system X" that a client reads. There is only "what this observer has
received about system X, and when".

## Resources

Sandbox extraction. Deposits are per-body, generated with the system, depleted by events.
Energy is a separate flow: collectors produce it as a function of area, distance and stellar
output, and it is consumed by fabrication, propulsion and transmission. Transmission power
is drawn from the same budget as everything else, which is what makes broadcasting a real
cost rather than a free action.

Ship modules, stored energy and what propulsion and construction cost are in
[19-ship-fitting.md](19-ship-fitting.md). A player's ship stores energy as mass and spends it as a
rocket whose exhaust is that energy; nothing yet collects any.

## Von Neumann probes

A probe is a ship whose order program includes construction of another probe. There is no
special mechanism; it falls out of ships that can build and ships that can be built.
Population growth is bounded by resource availability and by the light-delay on any recall
order, which is the interesting constraint: a self-replicating fleet 40 ly out cannot be
stopped in under 40 in-game years.

### Generation limits and drift

**Decided: replication orders carry a TTL.** A probe is built with a generation counter; it may
build children, which are one generation deeper, and at generation `N` the replication order
shuts itself down. The probe keeps working — it mines, builds, observes — it simply stops
making more of itself.

This bounds the population by construction rather than by a cap bolted on afterwards. With
branching factor `b`, a lineage totals `(b^(N+1) - 1) / (b - 1)` probes, so the limit is
exponential in `N` and both parameters need choosing together: `b = 2, N = 10` gives 2047;
`b = 2, N = 20` gives 2.1 million. Energy and mass conservation bound it further and should
bind first in normal play; the TTL is what guarantees termination when they do not.

**Drifters.** A replication has a small probability of copying the order set incorrectly. The
interesting corruption is a generation counter that fails to decrement: that lineage never
terminates, and it expands until something stops it. A drifter is not scripted hostility — it
is a probe running a slightly wrong copy of the player's own program, which is exactly what a
von Neumann failure mode looks like.

What makes it a real mechanic rather than a nuisance is the light delay. A drifter lineage 40
light-years out is discovered 40 in-game years after it started, by which point it has had 40
years to expand, and any order to stop it takes another 40 to arrive. A player's own probes
become the thing they have to hunt, and the hunt is bounded below by the speed of light.

Drift rates, TTL depth and branching factor are the three numbers that tune this, and all
three are per-design-decision rather than per-player.

## Persistence

| data | store |
|---|---|
| events | Postgres, partitioned; see [02-event-store.md](02-event-store.md) |
| source positions and worldlines | Postgres |
| generated systems | not stored; regenerated from seed |
| player accounts, ownership | Postgres |
| known-world snapshots | Postgres, per observer, as a compact fold over received events |

Generated systems are deliberately not stored. A world of 120 000 systems generated on
demand from a seed costs nothing until someone looks at one, and a seed is 8 bytes.
Anything a player *changes* inside a system becomes an event, which is stored, and replaying
those events over the generated baseline reconstructs the system exactly.

## Open

- Multi-star systems. `em-sim` propagates Keplerian orbits about a primary; a close binary
  needs either a hierarchical two-body decomposition or a restricted three-body treatment.
  The catalogue has many binaries and ignoring them removes a large fraction of real stars.
- ~~Whether a system's shell radius can be changed by players.~~ **Decided: immutable.** The
  shell is world geometry. A swarm large enough to argue otherwise is a later problem.
- ~~Granularity of swarm sub-populations.~~ **Decided: sub-populations exist and nest.** A
  population may belong to a meta-population, so a swarm built in waves keeps one record per
  wave, and a superstructure assembled from several orbital bands keeps one per band. Deficits
  add, so a meta-population's contribution is the sum of its members' and needs no separate
  representation. Observers see the superposition; the owner sees the parts.
- ~~Whether generated Oort and Kuiper populations vary per system.~~ **Decided: by stellar
  generation and metallicity**, as above. What remains is calibrating the yield curve.
- ~~Deposit granularity.~~ **Decided: per-body totals.** Enough resolution for the
  extraction loop, and it keeps a body's resource state to a handful of numbers.
