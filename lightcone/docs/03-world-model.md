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
3. **Coordinate boundary.** Local f64 metres are valid inside the shell and only inside it.

Sizing. The Sun's Oort cloud reaches roughly 1e5 AU = 1.58 ly, and the nearest star is
4.25 ly away, so shells at that radius do not intersect for typical spacings. Shell radius
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

## Systems and star data

The starting catalogue is HYG v4.2, already in `assets/catalogs/hygdata_v42.csv` and read by
`src/catalog/`. It supplies position (RA/Dec/distance), spectral class, absolute magnitude,
and proper motion for ~120 000 stars. Positions convert to the ecliptic frame the sim
already uses; note the catalogue is equatorial and the existing conversion lives in the
star-field path.

Per-star derived data, generated once at world creation:

| field | from |
|---|---|
| mass, radius, effective temperature | spectral class lookup, `src/catalog/spectral.rs` |
| luminosity | radius and temperature, Stefan-Boltzmann |
| shell radius | mass, clamped against neighbours |
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
| swarm element | area and orbit, contributing to the star's occlusion map |
| ringworld, shell segment | large fixed occlusion over a solid angle |

Solar collectors, swarm elements and megastructures are the same thing to the light model:
area that blocks a fraction of the star's output in some set of directions. That is what
makes a rival's industry detectable from another system.

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

The **known-world snapshot** is the important one. A ship far from its owner has a different
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

## Von Neumann probes

A probe is a ship whose order program includes construction of another probe. There is no
special mechanism; it falls out of ships that can build and ships that can be built.
Population growth is bounded by resource availability and by the light-delay on any recall
order, which is the interesting constraint: a self-replicating fleet 40 ly out cannot be
stopped in under 40 in-game years.

Practical limit: the server must cap the total object count per player, or the source table
and the BVH grow without bound. Cap by energy and mass conservation first, and impose a hard
numeric cap as a backstop.

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
- Whether a system's shell radius can be changed by players (a large enough swarm arguably
  redefines what escapes), or is immutable world geometry.
- Deposit granularity: per-body totals, or per-site with positions on the surface.
